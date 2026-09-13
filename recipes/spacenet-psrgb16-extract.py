#!/usr/bin/env python3
"""Bounded offline extraction of exactly locked SpaceNet source members."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import tarfile

LOCK = Path(__file__).with_name("spacenet-psrgb16-v1.sources.json")
MAX_MEMBERS = 10_000
MAX_TOTAL_BYTES = 4 * 1024 ** 3
MAX_MEMBER_BYTES = 32 * 1024 ** 2
MAX_SELECTED_BYTES = 512 * 1024 ** 2


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def safe_name(name):
    p = PurePosixPath(name)
    if not name or p.is_absolute() or '..' in p.parts or p.as_posix() != name or '\\' in name:
        raise ValueError("unsafe archive member path")
    return name


def inspect_archive(archive, selected):
    """Validate all entries and selected hashes before any extraction side effect."""
    expected = {safe_name(r['member']): r for r in selected}
    if len(expected) != len(selected) or not expected:
        raise ValueError("duplicate or empty locked selection")
    if any('/RGB-PanSharpen/' not in p or not p.endswith('.tif') for p in expected):
        raise ValueError("selection must contain only RGB-PanSharpen TIFFs")
    if any(type(r['bytes']) is not int or not 0 < r['bytes'] <= MAX_MEMBER_BYTES for r in selected):
        raise ValueError("selected member exceeds byte budget")
    if sum(r['bytes'] for r in selected) > MAX_SELECTED_BYTES:
        raise ValueError("selected extraction exceeds byte budget")
    seen, matched = set(), set()
    count, total = 0, 0
    with tarfile.open(archive, 'r|gz') as stream:
        for member in stream:
            count += 1
            total += member.size
            if count > MAX_MEMBERS or total > MAX_TOTAL_BYTES or member.size > MAX_MEMBER_BYTES or member.size < 0:
                raise ValueError("archive scan exceeds finite budget")
            safe_name(member.name)
            if member.name in seen:
                raise ValueError("duplicate archive member")
            seen.add(member.name)
            if not (member.isdir() or member.isreg()) or member.issparse():
                raise ValueError("archive links, sparse or special entries are forbidden")
            if member.name not in expected:
                continue
            record = expected[member.name]
            if not member.isreg() or member.size != record['bytes']:
                raise ValueError("selected member type or size mismatch")
            with stream.extractfile(member) as source:
                actual = hashlib.file_digest(source, 'sha256').hexdigest()
            if actual != record['sha256']:
                raise ValueError("selected member hash mismatch")
            matched.add(member.name)
    if matched != set(expected):
        raise ValueError("locked selected member missing")
    return {'archive_members': count, 'decompressed_member_bytes': total,
            'selected_members': len(matched), 'selected_bytes': sum(r['bytes'] for r in selected)}


def run(source_root, extract=False):
    if source_root.is_symlink():
        raise ValueError("source root must not be a symlink")
    source_root = source_root.resolve(strict=True)
    lock = json.loads(LOCK.read_text())
    assets = {a['path']: a for a in lock['assets']}
    archive = source_root / 'SN3_roads_sample.tar.gz'
    for name in ('SN3_roads_sample.tar.gz', 'LICENSE.md'):
        p = source_root / name
        if p.is_symlink() or not p.is_file() or p.stat().st_size != assets[name]['bytes'] or digest(p) != assets[name]['sha256']:
            raise ValueError("preserved archive or supplied notice identity mismatch")
    if extract:
        entries = list(source_root.rglob('*'))
        if any(p.is_symlink() or not (p.is_file() or p.is_dir()) for p in entries):
            raise ValueError("special source entry")
        if {p.relative_to(source_root).as_posix() for p in entries if p.is_file()} != {'SN3_roads_sample.tar.gz', 'LICENSE.md'}:
            raise ValueError("extraction requires archive and notice only; existing member outputs are refused")
    result = inspect_archive(archive, lock['members'])
    if extract:
        selected = {a['member']: a for a in lock['members']}
        with tarfile.open(archive, 'r|gz') as stream:
            for member in stream:
                if member.name not in selected:
                    continue
                record = selected[member.name]
                if not member.isreg() or member.size != record['bytes']:
                    raise ValueError("archive changed after preflight")
                output = source_root / safe_name(member.name)
                output.parent.mkdir(parents=True, exist_ok=True)
                if any(p.is_symlink() for p in [output, *output.parents]):
                    raise ValueError("symlink in output path")
                with stream.extractfile(member) as source, output.open('xb') as destination:
                    while block := source.read(1024 * 1024):
                        destination.write(block)
                if output.stat().st_size != record['bytes'] or digest(output) != record['sha256']:
                    raise ValueError("extracted member identity mismatch")
    return dict(result, archive_sha256=assets['SN3_roads_sample.tar.gz']['sha256'],
                action='extract' if extract else 'verify-archive',
                recipe_sha256=digest(Path(__file__)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=('extract', 'verify-archive'))
    parser.add_argument('--source-root', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(run(args.source_root, args.command == 'extract'), sort_keys=True))


if __name__ == '__main__':
    main()
