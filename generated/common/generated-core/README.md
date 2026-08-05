# Generated core image pack

These fixtures were generated deterministically by `emuella-corpus` from
Emuella-authored algorithms. They are licensed under Apache-2.0 with the
catalogue software and are safe to copy into Apache-2.0 test distributions.

The pack intentionally uses simple, independently inspectable formats:

- PGM/PPM for unsigned 8- and 16-bit integer samples;
- PAM for RGBA frames;
- PGX for signed 12-bit JPEG 2000 component samples; and
- PFM for little-endian scene-linear floating-point RGB.

File headers are part of the fixture bytes. Canonical sample semantics are
recorded in the pack manifest; codec tests should distinguish source-format
parsing from codec behavior when reporting failures.
