use crate::model::{CatalogueManifest, PackKind, PackManifest, ReviewState, SuiteManifest};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

#[derive(Debug)]
pub struct CatalogueError(pub(crate) String);

impl CatalogueError {
    pub(crate) fn message(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CatalogueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for CatalogueError {}

#[derive(Debug)]
pub struct Catalogue {
    root: PathBuf,
    config: CatalogueManifest,
    packs: BTreeMap<String, LocatedPack>,
    suites: BTreeMap<String, LocatedSuite>,
}

#[derive(Debug)]
struct LocatedPack {
    path: PathBuf,
    manifest: PackManifest,
}

#[derive(Debug)]
struct LocatedSuite {
    path: PathBuf,
    manifest: SuiteManifest,
}

#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    pub pack_count: usize,
    pub suite_count: usize,
    pub asset_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct VerificationReport {
    pub pack_id: String,
    pub root: PathBuf,
    pub checked_assets: usize,
    pub checked_bytes: u64,
}

impl Catalogue {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, CatalogueError> {
        let root = root.as_ref().canonicalize().map_err(|error| {
            CatalogueError::message(format!(
                "failed to resolve catalogue root {}: {error}",
                root.as_ref().display()
            ))
        })?;
        let config_path = root.join("catalog.toml");
        let config: CatalogueManifest = parse_toml(&config_path)?;
        if config.schema_version != 1 {
            return Err(CatalogueError::message(format!(
                "unsupported catalogue schema version {}",
                config.schema_version
            )));
        }

        let mut packs = BTreeMap::new();
        for path in toml_files(&root.join(&config.manifest_root))? {
            let manifest: PackManifest = parse_toml(&path)?;
            let id = manifest.id.clone();
            if let Some(previous) = packs.insert(
                id.clone(),
                LocatedPack {
                    path: path.clone(),
                    manifest,
                },
            ) {
                return Err(CatalogueError::message(format!(
                    "duplicate pack ID {id} in {} and {}",
                    previous.path.display(),
                    path.display()
                )));
            }
        }

        let mut suites = BTreeMap::new();
        for path in toml_files(&root.join(&config.suite_root))? {
            let manifest: SuiteManifest = parse_toml(&path)?;
            let id = manifest.id.clone();
            if let Some(previous) = suites.insert(
                id.clone(),
                LocatedSuite {
                    path: path.clone(),
                    manifest,
                },
            ) {
                return Err(CatalogueError::message(format!(
                    "duplicate suite ID {id} in {} and {}",
                    previous.path.display(),
                    path.display()
                )));
            }
        }

        Ok(Self {
            root,
            config,
            packs,
            suites,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn name(&self) -> &str {
        &self.config.name
    }

    pub fn description(&self) -> &str {
        &self.config.description
    }

    pub fn cache_environment(&self) -> &str {
        &self.config.cache_environment
    }

    pub fn pack_ids(&self) -> impl Iterator<Item = &str> {
        self.packs.keys().map(String::as_str)
    }

    pub fn suite_ids(&self) -> impl Iterator<Item = &str> {
        self.suites.keys().map(String::as_str)
    }

    pub fn pack(&self, id: &str) -> Option<&PackManifest> {
        self.packs.get(id).map(|located| &located.manifest)
    }

    pub fn suite(&self, id: &str) -> Option<&SuiteManifest> {
        self.suites.get(id).map(|located| &located.manifest)
    }

    pub fn default_materialization_root(&self, id: &str) -> Result<PathBuf, CatalogueError> {
        let pack = self
            .pack(id)
            .ok_or_else(|| CatalogueError::message(format!("unknown pack ID {id}")))?;
        Ok(self.root.join(&pack.materialization.directory))
    }

    pub fn check(&self) -> Result<CheckReport, CatalogueError> {
        let mut report = CheckReport {
            pack_count: self.packs.len(),
            suite_count: self.suites.len(),
            ..CheckReport::default()
        };

        if self.config.name.trim().is_empty() || self.config.description.trim().is_empty() {
            return Err(CatalogueError::message(
                "catalogue name and description must not be empty",
            ));
        }
        validate_relative_path("generated_root", &self.config.generated_root)?;

        for located in self.packs.values() {
            let pack = &located.manifest;
            validate_pack(pack, &self.root, &located.path, &mut report)?;
            report.asset_count += pack.assets.len();
        }

        for located in self.suites.values() {
            let suite = &located.manifest;
            validate_id("suite", &suite.id)?;
            if suite.schema_version != 1 {
                return Err(CatalogueError::message(format!(
                    "{} uses unsupported suite schema version {}",
                    located.path.display(),
                    suite.schema_version
                )));
            }
            if !(1..=3).contains(&suite.layer) {
                return Err(CatalogueError::message(format!(
                    "suite {} has invalid layer {}",
                    suite.id, suite.layer
                )));
            }
            if suite.name.trim().is_empty()
                || suite.description.trim().is_empty()
                || suite.purposes.is_empty()
                || suite.packs.is_empty()
            {
                return Err(CatalogueError::message(format!(
                    "suite {} lacks required descriptive content",
                    suite.id
                )));
            }

            let mut referenced = BTreeSet::new();
            for selected in &suite.packs {
                if !referenced.insert(&selected.id) {
                    return Err(CatalogueError::message(format!(
                        "suite {} references pack {} more than once",
                        suite.id, selected.id
                    )));
                }
                let pack = self.packs.get(&selected.id).ok_or_else(|| {
                    CatalogueError::message(format!(
                        "suite {} references unknown pack {}",
                        suite.id, selected.id
                    ))
                })?;
                if pack.manifest.version != selected.version {
                    return Err(CatalogueError::message(format!(
                        "suite {} requests {} version {}, catalogue has {}",
                        suite.id, selected.id, selected.version, pack.manifest.version
                    )));
                }
                if suite.gating && pack.manifest.review_state != ReviewState::Locked {
                    return Err(CatalogueError::message(format!(
                        "gating suite {} includes non-locked pack {}",
                        suite.id, selected.id
                    )));
                }
            }
        }

        Ok(report)
    }

    pub fn verify(
        &self,
        id: &str,
        root: Option<&Path>,
    ) -> Result<VerificationReport, CatalogueError> {
        let pack = self
            .pack(id)
            .ok_or_else(|| CatalogueError::message(format!("unknown pack ID {id}")))?;
        if pack.assets.is_empty() {
            return Err(CatalogueError::message(format!(
                "pack {id} has no locked asset inventory to verify"
            )));
        }
        let root = root
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.root.join(&pack.materialization.directory));
        let mut report = VerificationReport {
            pack_id: id.to_owned(),
            root: root.clone(),
            ..VerificationReport::default()
        };
        for asset in &pack.assets {
            validate_relative_path("asset path", &asset.path)?;
            let path = root.join(&asset.path);
            let metadata = fs::metadata(&path).map_err(|error| {
                CatalogueError::message(format!("failed to inspect {}: {error}", path.display()))
            })?;
            if !metadata.is_file() {
                return Err(CatalogueError::message(format!(
                    "asset is not a regular file: {}",
                    path.display()
                )));
            }
            if metadata.len() != asset.bytes {
                return Err(CatalogueError::message(format!(
                    "size mismatch for {}: expected {}, found {}",
                    path.display(),
                    asset.bytes,
                    metadata.len()
                )));
            }
            let digest = sha256_file(&path)?;
            if digest != asset.sha256 {
                return Err(CatalogueError::message(format!(
                    "SHA-256 mismatch for {}: expected {}, found {}",
                    path.display(),
                    asset.sha256,
                    digest
                )));
            }
            report.checked_assets += 1;
            report.checked_bytes += metadata.len();
        }
        Ok(report)
    }
}

fn validate_pack(
    pack: &PackManifest,
    root: &Path,
    source_path: &Path,
    report: &mut CheckReport,
) -> Result<(), CatalogueError> {
    if pack.schema_version != 1 {
        return Err(CatalogueError::message(format!(
            "{} uses unsupported pack schema version {}",
            source_path.display(),
            pack.schema_version
        )));
    }
    validate_id("pack", &pack.id)?;
    if pack.version.trim().is_empty()
        || pack.name.trim().is_empty()
        || pack.description.trim().is_empty()
        || pack.codecs.is_empty()
        || pack.purposes.is_empty()
    {
        return Err(CatalogueError::message(format!(
            "pack {} lacks required descriptive content",
            pack.id
        )));
    }
    if pack.license.expression.trim().is_empty()
        || pack.license.name.trim().is_empty()
        || pack.license.evidence_url.trim().is_empty()
        || pack.license.reviewed_on.trim().is_empty()
    {
        return Err(CatalogueError::message(format!(
            "pack {} has an incomplete licence record",
            pack.id
        )));
    }
    for license_file in &pack.license.local_files {
        validate_relative_path("licence file", license_file)?;
        if !root.join(license_file).is_file() {
            return Err(CatalogueError::message(format!(
                "pack {} references missing licence file {}",
                pack.id, license_file
            )));
        }
    }
    validate_relative_path("materialization directory", &pack.materialization.directory)?;
    if let Some(digest) = &pack.materialization.expected_tree_sha256 {
        validate_sha256("expected tree SHA-256", digest, &pack.id)?;
    }

    match pack.kind {
        PackKind::Generated => {
            if pack
                .materialization
                .generator
                .as_deref()
                .unwrap_or("")
                .is_empty()
            {
                return Err(CatalogueError::message(format!(
                    "generated pack {} has no generator",
                    pack.id
                )));
            }
        }
        PackKind::External | PackKind::Derived => {
            let source = pack.source.as_ref().ok_or_else(|| {
                CatalogueError::message(format!("external pack {} has no source", pack.id))
            })?;
            if source.landing_page.trim().is_empty() || source.terms_url.trim().is_empty() {
                return Err(CatalogueError::message(format!(
                    "external pack {} has incomplete source URLs",
                    pack.id
                )));
            }
            if let Some(digest) = &source.archive_sha256 {
                validate_sha256("archive SHA-256", digest, &pack.id)?;
            }
        }
    }

    if pack.review_state == ReviewState::Locked && pack.assets.is_empty() {
        return Err(CatalogueError::message(format!(
            "locked pack {} has no asset inventory",
            pack.id
        )));
    }
    if pack.review_state != ReviewState::Locked {
        report.warnings.push(format!(
            "pack {}@{} is {:?}, not locked",
            pack.id, pack.version, pack.review_state
        ));
    }

    let mut asset_paths = BTreeSet::new();
    for asset in &pack.assets {
        validate_relative_path("asset path", &asset.path)?;
        validate_sha256("asset SHA-256", &asset.sha256, &pack.id)?;
        if asset.media_type.trim().is_empty() || asset.semantics.trim().is_empty() {
            return Err(CatalogueError::message(format!(
                "pack {} asset {} lacks media type or semantics",
                pack.id, asset.path
            )));
        }
        if !asset_paths.insert(&asset.path) {
            return Err(CatalogueError::message(format!(
                "pack {} lists asset {} more than once",
                pack.id, asset.path
            )));
        }
    }
    Ok(())
}

fn validate_id(kind: &str, id: &str) -> Result<(), CatalogueError> {
    validate_relative_path(&format!("{kind} ID"), id)?;
    if id.is_empty()
        || id.starts_with('.')
        || id.ends_with('/')
        || id.chars().any(|character| {
            !(character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '-' | '/' | '.'))
        })
    {
        return Err(CatalogueError::message(format!("invalid {kind} ID {id:?}")));
    }
    Ok(())
}

fn validate_relative_path(label: &str, value: &str) -> Result<(), CatalogueError> {
    let path = Path::new(value);
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(CatalogueError::message(format!(
            "{label} must be a non-empty relative path: {value:?}"
        )));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(CatalogueError::message(format!(
            "{label} contains unsafe traversal: {value:?}"
        )));
    }
    Ok(())
}

fn validate_sha256(label: &str, digest: &str, pack_id: &str) -> Result<(), CatalogueError> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CatalogueError::message(format!(
            "pack {pack_id} has invalid {label}: {digest:?}"
        )));
    }
    Ok(())
}

fn parse_toml<T>(path: &Path) -> Result<T, CatalogueError>
where
    T: serde::de::DeserializeOwned,
{
    let source = fs::read_to_string(path).map_err(|error| {
        CatalogueError::message(format!("failed to read {}: {error}", path.display()))
    })?;
    toml::from_str(&source).map_err(|error| {
        CatalogueError::message(format!("failed to parse {}: {error}", path.display()))
    })
}

fn toml_files(root: &Path) -> Result<Vec<PathBuf>, CatalogueError> {
    let mut files = Vec::new();
    collect_toml_files(root, &mut files).map_err(|error| {
        CatalogueError::message(format!("failed to walk {}: {error}", root.display()))
    })?;
    files.sort();
    Ok(files)
}

fn collect_toml_files(root: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_toml_files(&path, files)?;
        } else if file_type.is_file() && path.extension().is_some_and(|value| value == "toml") {
            files.push(path);
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, CatalogueError> {
    let mut file = fs::File::open(path).map_err(|error| {
        CatalogueError::message(format!("failed to open {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            CatalogueError::message(format!("failed to read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_digest(hasher.finalize().as_slice()))
}

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_parent_traversal() {
        assert!(validate_relative_path("test", "../escape").is_err());
        assert!(validate_relative_path("test", "safe/path").is_ok());
    }

    #[test]
    fn formats_sha256_as_lowercase_hex() {
        assert_eq!(hex_digest(&[0x00, 0xab, 0xff]), "00abff");
    }

    #[test]
    fn repository_catalogue_is_internally_consistent() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let report = catalogue.check().expect("repository catalogue checks");
        assert!(report.pack_count >= 9);
        assert!(report.suite_count >= 7);
        assert!(report.asset_count >= 18);
        let verification = catalogue
            .verify("common/generated-core", None)
            .expect("generated core verifies");
        assert_eq!(verification.checked_assets, 18);
    }
}
