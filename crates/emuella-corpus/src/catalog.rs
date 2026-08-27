use crate::model::{
    AssetInventoryManifest, AssetRecord, CatalogueManifest, InspectionExpectation, PackKind,
    PackManifest, ReviewState, SuiteManifest,
};
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
    pub tree_sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct InventoryReport {
    pub output: PathBuf,
    pub asset_count: usize,
    pub total_bytes: u64,
    pub tree_sha256: String,
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
            let mut manifest: PackManifest = parse_toml(&path)?;
            if let Some(inventory_path) = &manifest.asset_inventory {
                if !manifest.assets.is_empty() {
                    return Err(CatalogueError::message(format!(
                        "pack {} has both inline assets and an external asset inventory",
                        manifest.id
                    )));
                }
                validate_relative_path("asset inventory", inventory_path)?;
                let inventory: AssetInventoryManifest = parse_toml(&root.join(inventory_path))?;
                if inventory.schema_version != 1 {
                    return Err(CatalogueError::message(format!(
                        "pack {} uses unsupported asset inventory schema version {}",
                        manifest.id, inventory.schema_version
                    )));
                }
                if inventory.pack_id != manifest.id || inventory.pack_version != manifest.version {
                    return Err(CatalogueError::message(format!(
                        "asset inventory {} identifies {}@{}, expected {}@{}",
                        inventory_path,
                        inventory.pack_id,
                        inventory.pack_version,
                        manifest.id,
                        manifest.version
                    )));
                }
                manifest.assets = inventory.assets;
            }
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
            if let Some(inspection) = &suite.inspection {
                validate_inspection_plan(suite, inspection, &self.packs)?;
            }
            if let Some(comparison) = &suite.decoded_pixel_comparison {
                validate_decoded_pixel_comparison_plan(suite, comparison, &self.packs)?;
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
        if let Some(expected) = &pack.materialization.expected_tree_sha256 {
            let actual_assets = inventory_assets(&root)?;
            let actual = tree_sha256(&actual_assets);
            if &actual != expected {
                return Err(CatalogueError::message(format!(
                    "tree SHA-256 mismatch for {}: expected {}, found {}",
                    root.display(),
                    expected,
                    actual
                )));
            }
            report.tree_sha256 = Some(actual);
        }
        Ok(report)
    }

    pub fn write_inventory(
        &self,
        id: &str,
        root: &Path,
        output: &Path,
    ) -> Result<InventoryReport, CatalogueError> {
        let pack = self
            .pack(id)
            .ok_or_else(|| CatalogueError::message(format!("unknown pack ID {id}")))?;
        let assets = inventory_assets(root)?;
        if assets.is_empty() {
            return Err(CatalogueError::message(format!(
                "cannot inventory an empty tree: {}",
                root.display()
            )));
        }
        let inventory = AssetInventoryManifest {
            schema_version: 1,
            pack_id: pack.id.clone(),
            pack_version: pack.version.clone(),
            assets,
        };
        let encoded = toml::to_string_pretty(&inventory).map_err(|error| {
            CatalogueError::message(format!("failed to encode asset inventory: {error}"))
        })?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                CatalogueError::message(format!(
                    "failed to create inventory directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        fs::write(output, encoded).map_err(|error| {
            CatalogueError::message(format!(
                "failed to write asset inventory {}: {error}",
                output.display()
            ))
        })?;
        let total_bytes = inventory.assets.iter().map(|asset| asset.bytes).sum();
        Ok(InventoryReport {
            output: output.to_path_buf(),
            asset_count: inventory.assets.len(),
            total_bytes,
            tree_sha256: tree_sha256(&inventory.assets),
        })
    }
}

fn validate_decoded_pixel_comparison_plan(
    suite: &SuiteManifest,
    comparison: &crate::model::DecodedPixelComparisonPlan,
    packs: &BTreeMap<String, LocatedPack>,
) -> Result<(), CatalogueError> {
    let selected = suite
        .packs
        .iter()
        .find(|selected| selected.id == comparison.pack_id)
        .ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} decoded-pixel plan names unselected pack {}",
                suite.id, comparison.pack_id
            ))
        })?;
    let pack = packs.get(&comparison.pack_id).ok_or_else(|| {
        CatalogueError::message("decoded-pixel comparison pack disappeared from catalogue")
    })?;
    if selected.version != pack.manifest.version {
        return Err(CatalogueError::message(format!(
            "suite {} decoded-pixel plan pack version is not selected",
            suite.id
        )));
    }
    if comparison.standard.trim().is_empty()
        || comparison.clauses.is_empty()
        || comparison
            .clauses
            .iter()
            .any(|clause| clause.trim().is_empty())
        || !is_lower_hex(&comparison.retrieval_commit, 40)
        || (comparison.cases.is_empty() && comparison.choice_groups.is_empty())
    {
        return Err(CatalogueError::message(format!(
            "suite {} has incomplete decoded-pixel comparison authority or cases",
            suite.id
        )));
    }

    let inventory = pack
        .manifest
        .assets
        .iter()
        .map(|asset| (asset.path.as_str(), asset))
        .collect::<BTreeMap<_, _>>();
    let mut case_ids = BTreeSet::new();
    let mut input_paths = BTreeSet::new();
    let mut reference_paths = BTreeSet::new();
    for case in &comparison.cases {
        validate_id("decoded-pixel case", &case.id)?;
        if !case_ids.insert(case.id.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats decoded-pixel case ID {}",
                suite.id, case.id
            )));
        }
        validate_relative_path("decoded-pixel input", &case.input)?;
        validate_relative_path("decoded-pixel reference", &case.reference)?;
        if case.input == case.reference {
            return Err(CatalogueError::message(format!(
                "suite {} decoded-pixel case {} uses one path as input and reference",
                suite.id, case.id
            )));
        }
        if !input_paths.insert(case.input.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats decoded-pixel input {}",
                suite.id, case.input
            )));
        }
        let input = inventory.get(case.input.as_str()).ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} decoded-pixel case {} input is absent from the locked inventory",
                suite.id, case.id
            ))
        })?;
        let reference = inventory.get(case.reference.as_str()).ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} decoded-pixel case {} reference is absent from the locked inventory",
                suite.id, case.id
            ))
        })?;
        if !reference_paths.insert(case.reference.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats decoded-pixel reference {}",
                suite.id, case.reference
            )));
        }
        if Path::new(&input.path)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("j2k")
            || Path::new(&reference.path)
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("pgx")
        {
            return Err(CatalogueError::message(format!(
                "suite {} decoded-pixel case {} requires a .j2k input and .pgx reference",
                suite.id, case.id
            )));
        }
        if case.resolution_reduction > 3
            || case.width == 0
            || case.height == 0
            || !(1..=32).contains(&case.bits_per_sample)
            || !case.mean_squared_error_limit.is_finite()
            || case.mean_squared_error_limit < 0.0
        {
            return Err(CatalogueError::message(format!(
                "suite {} decoded-pixel case {} has unsupported geometry, sample format, resolution, or error limits",
                suite.id, case.id
            )));
        }
    }
    for group in &comparison.choice_groups {
        validate_id("decoded-pixel choice group", &group.id)?;
        if !case_ids.insert(group.id.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats decoded-pixel case or group ID {}",
                suite.id, group.id
            )));
        }
        validate_relative_path("decoded-pixel input", &group.input)?;
        if !input_paths.insert(group.input.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats decoded-pixel input {}",
                suite.id, group.input
            )));
        }
        let input = inventory.get(group.input.as_str()).ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} decoded-pixel group {} input is absent from the locked inventory",
                suite.id, group.id
            ))
        })?;
        if Path::new(&input.path)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("j2k")
        {
            return Err(CatalogueError::message(format!(
                "suite {} decoded-pixel group {} requires a .j2k input",
                suite.id, group.id
            )));
        }
        if group.alternatives.is_empty()
            || group.minimum_passing_alternatives == 0
            || usize::from(group.minimum_passing_alternatives) > group.alternatives.len()
        {
            return Err(CatalogueError::message(format!(
                "suite {} decoded-pixel group {} has an invalid alternative pass requirement",
                suite.id, group.id
            )));
        }
        let mut alternative_ids = BTreeSet::new();
        for alternative in &group.alternatives {
            validate_id("decoded-pixel alternative", &alternative.id)?;
            if !alternative_ids.insert(alternative.id.as_str()) {
                return Err(CatalogueError::message(format!(
                    "suite {} decoded-pixel group {} repeats alternative ID {}",
                    suite.id, group.id, alternative.id
                )));
            }
            validate_relative_path("decoded-pixel reference", &alternative.reference)?;
            if group.input == alternative.reference {
                return Err(CatalogueError::message(format!(
                    "suite {} decoded-pixel group {} uses one path as input and reference",
                    suite.id, group.id
                )));
            }
            let reference = inventory
                .get(alternative.reference.as_str())
                .ok_or_else(|| {
                    CatalogueError::message(format!(
                        "suite {} decoded-pixel group {} alternative {} reference is absent from the locked inventory",
                        suite.id, group.id, alternative.id
                    ))
                })?;
            if !reference_paths.insert(alternative.reference.as_str()) {
                return Err(CatalogueError::message(format!(
                    "suite {} repeats decoded-pixel reference {}",
                    suite.id, alternative.reference
                )));
            }
            if Path::new(&reference.path)
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("pgx")
            {
                return Err(CatalogueError::message(format!(
                    "suite {} decoded-pixel group {} alternative {} requires a .pgx reference",
                    suite.id, group.id, alternative.id
                )));
            }
            if alternative.resolution_reduction > 1
                || alternative.width == 0
                || alternative.height == 0
                || !(1..=32).contains(&alternative.bits_per_sample)
                || !alternative.mean_squared_error_limit.is_finite()
                || alternative.mean_squared_error_limit < 0.0
            {
                return Err(CatalogueError::message(format!(
                    "suite {} decoded-pixel group {} alternative {} has unsupported geometry, sample format, resolution, or error limits",
                    suite.id, group.id, alternative.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_inspection_plan(
    suite: &SuiteManifest,
    inspection: &crate::model::InspectionPlan,
    packs: &BTreeMap<String, LocatedPack>,
) -> Result<(), CatalogueError> {
    let selected = suite
        .packs
        .iter()
        .find(|selected| selected.id == inspection.pack_id)
        .ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} inspection plan names unselected pack {}",
                suite.id, inspection.pack_id
            ))
        })?;
    let pack = packs
        .get(&inspection.pack_id)
        .ok_or_else(|| CatalogueError::message("inspection pack disappeared from catalogue"))?;
    if selected.version != pack.manifest.version {
        return Err(CatalogueError::message(format!(
            "suite {} inspection plan pack version is not selected",
            suite.id
        )));
    }
    if inspection.extensions.is_empty() || inspection.classifications.is_empty() {
        return Err(CatalogueError::message(format!(
            "suite {} inspection plan has an empty selection or classification set",
            suite.id
        )));
    }
    validate_expected_diagnostic(
        &suite.id,
        "inspection default",
        inspection.expected,
        inspection.diagnostic_contains.as_deref(),
    )?;

    let mut extensions = BTreeSet::new();
    for extension in &inspection.extensions {
        if !matches!(extension.as_str(), ".j2k" | ".htj2k" | ".jp2" | ".jph") {
            return Err(CatalogueError::message(format!(
                "suite {} inspection plan has unsupported extension {extension:?}",
                suite.id
            )));
        }
        if !extensions.insert(extension.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} inspection plan repeats extension {extension}",
                suite.id
            )));
        }
    }

    for classification in &inspection.classifications {
        match (&classification.path, &classification.path_prefix) {
            (Some(path), None) => validate_relative_path("inspection classification path", path)?,
            (None, Some(prefix)) => {
                validate_relative_path("inspection classification path prefix", prefix)?
            }
            _ => {
                return Err(CatalogueError::message(format!(
                    "suite {} inspection classification must set exactly one of path or path_prefix",
                    suite.id
                )));
            }
        }
        if classification.cohort.trim().is_empty() {
            return Err(CatalogueError::message(format!(
                "suite {} inspection classification has an empty cohort",
                suite.id
            )));
        }
    }

    let candidates = pack
        .manifest
        .assets
        .iter()
        .filter(|asset| {
            Path::new(&asset.path)
                .extension()
                .and_then(|extension| extension.to_str())
                .map(|extension| format!(".{extension}").to_ascii_lowercase())
                .is_some_and(|extension| extensions.contains(extension.as_str()))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(CatalogueError::message(format!(
            "suite {} inspection plan selects no inventory assets",
            suite.id
        )));
    }

    let candidate_paths = candidates
        .iter()
        .map(|asset| asset.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut rule_matches = vec![0_usize; inspection.classifications.len()];
    for asset in &candidates {
        let matching = inspection
            .classifications
            .iter()
            .enumerate()
            .filter(|(_, classification)| {
                classification
                    .path
                    .as_deref()
                    .is_some_and(|path| path == asset.path)
                    || classification
                        .path_prefix
                        .as_deref()
                        .is_some_and(|prefix| asset.path.starts_with(prefix))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(CatalogueError::message(format!(
                "suite {} inspection candidate {} matches {} classifications, expected one",
                suite.id,
                asset.path,
                matching.len()
            )));
        }
        rule_matches[matching[0]] += 1;
    }
    if let Some((index, _)) = rule_matches
        .iter()
        .enumerate()
        .find(|(_, matches)| **matches == 0)
    {
        return Err(CatalogueError::message(format!(
            "suite {} inspection classification {} matches no candidates",
            suite.id, index
        )));
    }

    let mut override_paths = BTreeSet::new();
    for override_record in &inspection.overrides {
        if !candidate_paths.contains(override_record.path.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} inspection override names unselected path {}",
                suite.id, override_record.path
            )));
        }
        if !override_paths.insert(override_record.path.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} inspection plan repeats override {}",
                suite.id, override_record.path
            )));
        }
        validate_expected_diagnostic(
            &suite.id,
            &format!("inspection override {}", override_record.path),
            override_record.expected,
            override_record.diagnostic_contains.as_deref(),
        )?;
    }
    Ok(())
}

fn validate_expected_diagnostic(
    suite_id: &str,
    label: &str,
    expected: InspectionExpectation,
    diagnostic: Option<&str>,
) -> Result<(), CatalogueError> {
    match (expected, diagnostic) {
        (InspectionExpectation::Reject, Some(diagnostic)) if !diagnostic.trim().is_empty() => {
            Ok(())
        }
        (InspectionExpectation::Reject, _) => Err(CatalogueError::message(format!(
            "suite {suite_id} rejected {label} lacks a diagnostic"
        ))),
        (InspectionExpectation::Accept, None) => Ok(()),
        (InspectionExpectation::Accept, Some(_)) => Err(CatalogueError::message(format!(
            "suite {suite_id} accepted {label} must not name a diagnostic"
        ))),
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
    if let Some(inventory_path) = &pack.asset_inventory {
        validate_relative_path("asset inventory", inventory_path)?;
        if !root.join(inventory_path).is_file() {
            return Err(CatalogueError::message(format!(
                "pack {} references missing asset inventory {}",
                pack.id, inventory_path
            )));
        }
    }
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
    if pack.review_state == ReviewState::Locked && pack.kind != PackKind::Generated {
        let source = pack.source.as_ref().expect("external source checked above");
        if source.archive_filename.is_none() || source.archive_sha256.is_none() {
            return Err(CatalogueError::message(format!(
                "locked external pack {} has no archive filename and SHA-256",
                pack.id
            )));
        }
        if pack.materialization.expected_tree_sha256.is_none() {
            return Err(CatalogueError::message(format!(
                "locked external pack {} has no expected tree SHA-256",
                pack.id
            )));
        }
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
    if let Some(source) = &pack.source
        && let (Some(filename), Some(digest)) = (&source.archive_filename, &source.archive_sha256)
    {
        let archive = pack
            .assets
            .iter()
            .find(|asset| asset.path == *filename)
            .ok_or_else(|| {
                CatalogueError::message(format!(
                    "pack {} archive {} is absent from its asset inventory",
                    pack.id, filename
                ))
            })?;
        if archive.sha256 != *digest {
            return Err(CatalogueError::message(format!(
                "pack {} archive digest disagrees with its asset inventory",
                pack.id
            )));
        }
    }
    Ok(())
}

fn inventory_assets(root: &Path) -> Result<Vec<AssetRecord>, CatalogueError> {
    let root = root.canonicalize().map_err(|error| {
        CatalogueError::message(format!(
            "failed to resolve inventory root {}: {error}",
            root.display()
        ))
    })?;
    let mut files = Vec::new();
    collect_regular_files(&root, &root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
        .into_iter()
        .map(|(relative, path)| {
            let metadata = fs::metadata(&path).map_err(|error| {
                CatalogueError::message(format!("failed to inspect {}: {error}", path.display()))
            })?;
            Ok(AssetRecord {
                media_type: media_type(&relative).to_owned(),
                semantics: asset_semantics(&relative).to_owned(),
                path: relative,
                bytes: metadata.len(),
                sha256: sha256_file(&path)?,
            })
        })
        .collect()
}

fn collect_regular_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), CatalogueError> {
    for entry in fs::read_dir(directory).map_err(|error| {
        CatalogueError::message(format!("failed to walk {}: {error}", directory.display()))
    })? {
        let entry = entry.map_err(|error| {
            CatalogueError::message(format!("failed to read directory entry: {error}"))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| {
            CatalogueError::message(format!("failed to inspect {}: {error}", path.display()))
        })?;
        if file_type.is_dir() {
            collect_regular_files(root, &path, files)?;
        } else if file_type.is_file() {
            let relative = path.strip_prefix(root).expect("walk remains under root");
            let relative = relative.to_str().ok_or_else(|| {
                CatalogueError::message(format!(
                    "inventory path is not valid UTF-8: {}",
                    relative.display()
                ))
            })?;
            files.push((relative.replace(std::path::MAIN_SEPARATOR, "/"), path));
        } else {
            return Err(CatalogueError::message(format!(
                "inventory tree contains a non-regular entry: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn tree_sha256(assets: &[AssetRecord]) -> String {
    let mut hasher = Sha256::new();
    for asset in assets {
        hasher.update(asset.sha256.as_bytes());
        hasher.update(b"\t");
        hasher.update(asset.bytes.to_string().as_bytes());
        hasher.update(b"\t");
        hasher.update(asset.path.as_bytes());
        hasher.update(b"\n");
    }
    hex_digest(hasher.finalize().as_slice())
}

fn media_type(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|value| value.to_str()) {
        Some("zip") => "application/zip",
        Some("j2k") => "image/j2c",
        Some("jp2") => "image/jp2",
        Some("jph") => "image/jph",
        Some("jpx") => "image/jpx",
        Some("pgm") => "image/x-portable-graymap",
        Some("tif") => "image/tiff",
        Some("xml") => "application/xml",
        Some("txt" | "ttx" | "desc" | "pf" | "licence") | None => "text/plain",
        _ => "application/octet-stream",
    }
}

fn asset_semantics(path: &str) -> &'static str {
    if path == "electronic_insert.zip" {
        "Unmodified authoritative ISO electronic-insert archive."
    } else if path.ends_with("COPYRIGHT.txt") || path.ends_with("README.licence") {
        "Embedded rights and attribution notice; preserve unchanged with the pack."
    } else if path.contains("htj2k_bsets") && path.ends_with(".j2k") {
        "HTJ2K conformance codestream."
    } else if path.contains("codestreams_") && path.ends_with(".j2k") {
        "JPEG 2000 conformance codestream."
    } else if path.contains("testfiles_jp2") && path.ends_with(".jp2") {
        "JP2 file-format conformance test file."
    } else if path.contains("testfiles_jph") && path.ends_with(".jph") {
        "JPH file-format conformance test file."
    } else if path.contains("testfiles_jpx") && path.ends_with(".jpx") {
        "JPX file-format conformance test file."
    } else if path.contains("reference_") {
        "Reference image for conformance comparison."
    } else if path.contains("descriptions_") {
        "Conformance test description or comparison parameters."
    } else {
        "Supporting file from the ISO/IEC 15444-4:2024 electronic insert."
    }
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

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

    fn inspection_fixture() -> (SuiteManifest, crate::model::InspectionPlan, Catalogue) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let suite = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("inspection suite exists")
            .manifest
            .clone();
        let inspection = suite.inspection.clone().expect("inspection plan exists");
        (suite, inspection, catalogue)
    }

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
    fn tree_digest_commits_to_hash_size_and_path() {
        let assets = vec![AssetRecord {
            path: "a/file.bin".to_owned(),
            bytes: 3,
            sha256: "00aaff0000000000000000000000000000000000000000000000000000000000".to_owned(),
            media_type: "application/octet-stream".to_owned(),
            semantics: "ignored by the byte-tree identity".to_owned(),
        }];
        assert_eq!(
            tree_sha256(&assets),
            "b471b7359e6e37530180322f745db7fb3f2c72f30fb20d055efa6e6165c8b305"
        );
    }

    #[test]
    fn rejects_duplicate_inspection_extensions() {
        let (suite, mut inspection, catalogue) = inspection_fixture();
        inspection.extensions.push(".j2k".to_owned());
        let error = validate_inspection_plan(&suite, &inspection, &catalogue.packs)
            .expect_err("duplicate extension must fail");
        assert!(error.to_string().contains("repeats extension .j2k"));
    }

    #[test]
    fn rejects_ambiguous_and_dead_inspection_classifications() {
        let (suite, mut inspection, catalogue) = inspection_fixture();
        inspection
            .classifications
            .push(inspection.classifications[0].clone());
        let error = validate_inspection_plan(&suite, &inspection, &catalogue.packs)
            .expect_err("overlapping classifications must fail");
        assert!(error.to_string().contains("matches 2 classifications"));

        let (suite, mut inspection, catalogue) = inspection_fixture();
        let mut dead = inspection.classifications[0].clone();
        dead.path = None;
        dead.path_prefix = Some("files/not-present/".to_owned());
        inspection.classifications.push(dead);
        let error = validate_inspection_plan(&suite, &inspection, &catalogue.packs)
            .expect_err("dead classification must fail");
        assert!(error.to_string().contains("matches no candidates"));
    }

    #[test]
    fn rejects_incomplete_negative_inspection_expectations() {
        let (suite, mut inspection, catalogue) = inspection_fixture();
        inspection.expected = InspectionExpectation::Reject;
        inspection.diagnostic_contains = None;
        let error = validate_inspection_plan(&suite, &inspection, &catalogue.packs)
            .expect_err("negative expectation without diagnostic must fail");
        assert!(error.to_string().contains("lacks a diagnostic"));
    }

    #[test]
    fn rejects_inspection_override_outside_selection() {
        let (suite, mut inspection, catalogue) = inspection_fixture();
        inspection.overrides.push(crate::model::InspectionOverride {
            path: "files/not-selected.txt".to_owned(),
            expected: InspectionExpectation::Reject,
            diagnostic_contains: Some("expected diagnostic".to_owned()),
        });
        let error = validate_inspection_plan(&suite, &inspection, &catalogue.packs)
            .expect_err("unselected override must fail");
        assert!(error.to_string().contains("names unselected path"));
    }

    #[test]
    fn rejects_invalid_decoded_pixel_contracts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let suite = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .clone();
        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");

        comparison.cases[0].reference = "files/not-in-inventory.pgx".to_owned();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("missing reference must fail");
        assert!(
            error
                .to_string()
                .contains("absent from the locked inventory")
        );

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.cases[0].mean_squared_error_limit = f64::INFINITY;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("non-finite limit must fail");
        assert!(error.to_string().contains("unsupported geometry"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.choice_groups[0].minimum_passing_alternatives = 0;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("choice group must require a passing alternative");
        assert!(error.to_string().contains("pass requirement"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.choice_groups[0].minimum_passing_alternatives = 3;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("choice group cannot require more passes than alternatives");
        assert!(error.to_string().contains("pass requirement"));

        let comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect("scalar cases support three reduction levels");

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison
            .cases
            .iter_mut()
            .find(|case| case.id == "annex-c/class0-profile0/p0-04")
            .expect("P0.04 scalar case exists")
            .resolution_reduction = 4;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("scalar cases remain bounded to three reduction levels");
        assert!(error.to_string().contains("unsupported geometry"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.choice_groups[0].alternatives[0].resolution_reduction = 2;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("choice alternatives remain bounded to P0.03 reductions");
        assert!(error.to_string().contains("unsupported geometry"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.choice_groups[0].alternatives[1].id =
            comparison.choice_groups[0].alternatives[0].id.clone();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("choice group alternative IDs must be unique");
        assert!(error.to_string().contains("repeats alternative ID"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.choice_groups[0].alternatives[1].reference = comparison.choice_groups[0]
            .alternatives[0]
            .reference
            .clone();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("decoded-pixel references must be unique");
        assert!(
            error
                .to_string()
                .contains("repeats decoded-pixel reference")
        );
    }

    #[test]
    fn records_p0_03_window_and_resolution_choices() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let plan = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .decoded_pixel_comparison
            .as_ref()
            .expect("comparison plan exists");
        let group = plan
            .choice_groups
            .iter()
            .find(|group| group.id == "annex-c/class0-profile0/p0-03")
            .expect("P0.03 choice group exists");
        assert_eq!(group.minimum_passing_alternatives, 1);
        assert_eq!(group.alternatives.len(), 2);
        assert_eq!(
            group
                .alternatives
                .iter()
                .map(|alternative| (
                    alternative.resolution_reduction,
                    alternative.output_origin_x,
                    alternative.output_origin_y,
                    alternative.width,
                    alternative.height,
                ))
                .collect::<Vec<_>>(),
            vec![(0, 0, 0, 128, 128), (1, 0, 0, 128, 128)]
        );
    }

    #[test]
    fn records_p0_14_reduced_scalar_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let plan = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .decoded_pixel_comparison
            .as_ref()
            .expect("comparison plan exists");
        let case = plan
            .cases
            .iter()
            .find(|case| case.id == "annex-c/class0-profile0/p0-14")
            .expect("P0.14 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_14.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_14.pgx"
        );
        assert_eq!(case.component, 0);
        assert_eq!(case.resolution_reduction, 2);
        assert_eq!((case.width, case.height), (13, 13));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 0);
        assert_eq!(case.mean_squared_error_limit, 0.0);
    }

    #[test]
    fn records_p0_09_reduced_scalar_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let plan = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .decoded_pixel_comparison
            .as_ref()
            .expect("comparison plan exists");
        let case = plan
            .cases
            .iter()
            .find(|case| case.id == "annex-c/class0-profile0/p0-09")
            .expect("P0.09 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_09.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_09.pgx"
        );
        assert_eq!(case.component, 0);
        assert_eq!(case.resolution_reduction, 2);
        assert_eq!((case.width, case.height), (5, 10));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 4);
        assert_eq!(case.mean_squared_error_limit, 1.47);
    }

    #[test]
    fn records_p0_04_reduced_scalar_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let plan = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .decoded_pixel_comparison
            .as_ref()
            .expect("comparison plan exists");
        let case = plan
            .cases
            .iter()
            .find(|case| case.id == "annex-c/class0-profile0/p0-04")
            .expect("P0.04 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_04.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_04.pgx"
        );
        assert_eq!(case.component, 0);
        assert_eq!(case.resolution_reduction, 3);
        assert_eq!((case.width, case.height), (80, 60));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 33);
        assert_eq!(case.mean_squared_error_limit, 55.8);
    }

    #[test]
    fn records_p0_05_reduced_scalar_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let plan = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .decoded_pixel_comparison
            .as_ref()
            .expect("comparison plan exists");
        let case = plan
            .cases
            .iter()
            .find(|case| case.id == "annex-c/class0-profile0/p0-05")
            .expect("P0.05 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_05.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_05.pgx"
        );
        assert_eq!(case.component, 0);
        assert_eq!(case.resolution_reduction, 3);
        assert_eq!((case.width, case.height), (128, 128));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 54);
        assert_eq!(case.mean_squared_error_limit, 68.0);
    }

    #[test]
    fn records_p0_15_window_and_resolution_choices() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let plan = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("comparison suite exists")
            .manifest
            .decoded_pixel_comparison
            .as_ref()
            .expect("comparison plan exists");
        let group = plan
            .choice_groups
            .iter()
            .find(|group| group.id == "annex-c/class0-profile0/p0-15")
            .expect("P0.15 choice group exists");
        assert_eq!(group.input, "files/codestreams_profile0/p0_15.j2k");
        assert_eq!(group.minimum_passing_alternatives, 1);
        assert_eq!(
            group
                .alternatives
                .iter()
                .map(|alternative| (
                    alternative.id.as_str(),
                    alternative.reference.as_str(),
                    alternative.component,
                    alternative.resolution_reduction,
                    alternative.output_origin_x,
                    alternative.output_origin_y,
                    alternative.width,
                    alternative.height,
                    alternative.bits_per_sample,
                    alternative.signed,
                    alternative.peak_error_limit,
                    alternative.mean_squared_error_limit,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "full-resolution-window",
                    "files/reference_class0_profile0/c0p0_15r0.pgx",
                    0,
                    0,
                    0,
                    0,
                    128,
                    128,
                    4,
                    true,
                    0,
                    0.0,
                ),
                (
                    "one-level-reduced",
                    "files/reference_class0_profile0/c0p0_15r1.pgx",
                    0,
                    1,
                    0,
                    0,
                    128,
                    128,
                    4,
                    true,
                    0,
                    0.0,
                ),
            ]
        );
    }

    #[test]
    fn repository_catalogue_is_internally_consistent() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        assert_eq!(catalogue.cache_environment(), "EMUELLA_TESTDATA_CACHE");
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
