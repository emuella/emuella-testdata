use crate::model::{
    AssetInventoryManifest, AssetRecord, CatalogueManifest, DecodedPixelDerivedSetCodingMode,
    DecodedPixelDerivedSetId, DecodedPixelDerivedSetSelection, DecodedPixelNormalisationStep,
    InspectionExpectation, PackKind, PackManifest, RenderedColourSpace, RenderedReferenceLayout,
    ReviewState, SuiteManifest,
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
            if let Some(comparison) = &suite.rendered_pixel_comparison {
                validate_rendered_pixel_comparison_plan(suite, comparison, &self.packs)?;
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

struct ExpectedDs0Case {
    reference_number: &'static str,
    coding_mode: DecodedPixelDerivedSetCodingMode,
    variants: &'static [ExpectedDs0Variant],
}

struct ExpectedDs0Variant {
    b_magb: u8,
    peak_error_limit: u64,
    mean_squared_error_limit: f64,
}

// Fail-closed transcription of the Class-0 Profile-0 DS0 BSET matrix and final
// inclusive limits from ISO/IEC 15444-4:2024, Tables C.1 and C.1bis (PDF pages
// 31 and 32). Limits include the bit-depth-scaled Part 4 additions.
const DS0_EXPECTED_CASES: &[ExpectedDs0Case] = &[
    ExpectedDs0Case {
        reference_number: "01",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
    ExpectedDs0Case {
        reference_number: "02",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 1,
                mean_squared_error_limit: 0.001,
            },
            ExpectedDs0Variant {
                b_magb: 12,
                peak_error_limit: 0,
                mean_squared_error_limit: 0.0,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "03",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 17,
                mean_squared_error_limit: 0.15,
            },
            ExpectedDs0Variant {
                b_magb: 14,
                peak_error_limit: 0,
                mean_squared_error_limit: 0.0,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "04",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 35,
                mean_squared_error_limit: 55.9,
            },
            ExpectedDs0Variant {
                b_magb: 12,
                peak_error_limit: 33,
                mean_squared_error_limit: 55.8,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "05",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 54,
                mean_squared_error_limit: 68.0,
            },
            ExpectedDs0Variant {
                b_magb: 12,
                peak_error_limit: 54,
                mean_squared_error_limit: 68.0,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "06",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 266,
                mean_squared_error_limit: 1035.96875,
            },
            ExpectedDs0Variant {
                b_magb: 15,
                peak_error_limit: 109,
                mean_squared_error_limit: 743.0,
            },
            ExpectedDs0Variant {
                b_magb: 18,
                peak_error_limit: 109,
                mean_squared_error_limit: 743.0,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "06",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtMix,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 109,
                mean_squared_error_limit: 743.0,
            },
            ExpectedDs0Variant {
                b_magb: 18,
                peak_error_limit: 109,
                mean_squared_error_limit: 743.0,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "07",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 13,
                mean_squared_error_limit: 0.43765625,
            },
            ExpectedDs0Variant {
                b_magb: 15,
                peak_error_limit: 11,
                mean_squared_error_limit: 0.34029296875,
            },
            ExpectedDs0Variant {
                b_magb: 16,
                peak_error_limit: 10,
                mean_squared_error_limit: 0.34,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "08",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 10,
                mean_squared_error_limit: 6.89578125,
            },
            ExpectedDs0Variant {
                b_magb: 15,
                peak_error_limit: 7,
                mean_squared_error_limit: 6.72,
            },
            ExpectedDs0Variant {
                b_magb: 16,
                peak_error_limit: 7,
                mean_squared_error_limit: 6.72,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "09",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 4,
            mean_squared_error_limit: 1.47,
        }],
    },
    ExpectedDs0Case {
        reference_number: "10",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 10,
            mean_squared_error_limit: 2.84,
        }],
    },
    ExpectedDs0Case {
        reference_number: "11",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 10,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
    ExpectedDs0Case {
        reference_number: "12",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
    ExpectedDs0Case {
        reference_number: "13",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
    ExpectedDs0Case {
        reference_number: "14",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
    ExpectedDs0Case {
        reference_number: "15",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[
            ExpectedDs0Variant {
                b_magb: 11,
                peak_error_limit: 17,
                mean_squared_error_limit: 0.15,
            },
            ExpectedDs0Variant {
                b_magb: 14,
                peak_error_limit: 0,
                mean_squared_error_limit: 0.0,
            },
        ],
    },
    ExpectedDs0Case {
        reference_number: "15",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtMix,
        variants: &[ExpectedDs0Variant {
            b_magb: 8,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
    ExpectedDs0Case {
        reference_number: "16",
        coding_mode: DecodedPixelDerivedSetCodingMode::HtOnly,
        variants: &[ExpectedDs0Variant {
            b_magb: 11,
            peak_error_limit: 0,
            mean_squared_error_limit: 0.0,
        }],
    },
];

fn validate_rendered_pixel_comparison_plan(
    suite: &SuiteManifest,
    comparison: &crate::model::RenderedPixelComparisonPlan,
    packs: &BTreeMap<String, LocatedPack>,
) -> Result<(), CatalogueError> {
    let selected = suite
        .packs
        .iter()
        .find(|selected| selected.id == comparison.pack_id)
        .ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} rendered-pixel plan names unselected pack {}",
                suite.id, comparison.pack_id
            ))
        })?;
    let pack = packs.get(&comparison.pack_id).ok_or_else(|| {
        CatalogueError::message("rendered-pixel comparison pack disappeared from catalogue")
    })?;
    if selected.version != pack.manifest.version {
        return Err(CatalogueError::message(format!(
            "suite {} rendered-pixel plan pack version is not selected",
            suite.id
        )));
    }
    if pack.manifest.review_state != ReviewState::Locked {
        return Err(CatalogueError::message(format!(
            "suite {} rendered-pixel plan requires locked pack {}",
            suite.id, comparison.pack_id
        )));
    }
    if comparison.standard.trim().is_empty()
        || comparison.clauses.is_empty()
        || comparison
            .clauses
            .iter()
            .any(|clause| clause.trim().is_empty())
        || comparison.clauses.iter().collect::<BTreeSet<_>>().len() != comparison.clauses.len()
        || !is_lower_hex(&comparison.retrieval_commit, 40)
        || comparison.cases.is_empty()
    {
        return Err(CatalogueError::message(format!(
            "suite {} has incomplete rendered-pixel comparison authority or cases",
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
        validate_id("rendered-pixel case", &case.id)?;
        if !case_ids.insert(case.id.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats rendered-pixel case ID {}",
                suite.id, case.id
            )));
        }
        validate_relative_path("rendered-pixel input", &case.input)?;
        validate_relative_path("rendered-pixel reference", &case.reference)?;
        if case.input == case.reference {
            return Err(CatalogueError::message(format!(
                "suite {} rendered-pixel case {} uses one path as input and reference",
                suite.id, case.id
            )));
        }
        if !input_paths.insert(case.input.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats rendered-pixel input {}",
                suite.id, case.input
            )));
        }
        if !reference_paths.insert(case.reference.as_str()) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats rendered-pixel reference {}",
                suite.id, case.reference
            )));
        }
        let input = inventory.get(case.input.as_str()).ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} rendered-pixel case {} input is absent from the locked inventory",
                suite.id, case.id
            ))
        })?;
        let reference = inventory.get(case.reference.as_str()).ok_or_else(|| {
            CatalogueError::message(format!(
                "suite {} rendered-pixel case {} reference is absent from the locked inventory",
                suite.id, case.id
            ))
        })?;
        if Path::new(&input.path)
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("jp2")
            || Path::new(&reference.path)
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("tif")
        {
            return Err(CatalogueError::message(format!(
                "suite {} rendered-pixel case {} requires a .jp2 input and .tif reference",
                suite.id, case.id
            )));
        }
        if input.media_type != "image/jp2" || reference.media_type != "image/tiff" {
            return Err(CatalogueError::message(format!(
                "suite {} rendered-pixel case {} has an unsupported inventory media type",
                suite.id, case.id
            )));
        }
        if case.width == 0
            || case.height == 0
            || case.components != 3
            || case.bits_per_sample != 8
            || case.rendered_colour_space != RenderedColourSpace::Srgb
            || case.reference_layout != RenderedReferenceLayout::TiffRgbU8Contiguous
            || !case.peak_error_limit.is_finite()
            || case.peak_error_limit < 0.0
        {
            return Err(CatalogueError::message(format!(
                "suite {} rendered-pixel case {} has unsupported full-frame shape, colour, layout, or error limit",
                suite.id, case.id
            )));
        }
    }
    Ok(())
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
    let required_order_dependent = [
        DecodedPixelNormalisationStep::ResolutionReduction,
        DecodedPixelNormalisationStep::RecoverFirstCodestreamComponent,
        DecodedPixelNormalisationStep::RoundToNearestInteger,
        DecodedPixelNormalisationStep::ClipToDeclaredSampleRange,
        DecodedPixelNormalisationStep::ReferenceBitDepthArithmeticShift,
        DecodedPixelNormalisationStep::ReferenceGridSubsampling,
        DecodedPixelNormalisationStep::UpperLeftReferenceCrop,
    ];
    let required_order_independent = BTreeSet::from([
        DecodedPixelNormalisationStep::PlanarComponentDeinterleave,
        DecodedPixelNormalisationStep::BigEndianByteOrder,
        DecodedPixelNormalisationStep::SignExtendToByteBoundary,
    ]);
    if let Some(normalisation) = &comparison.output_normalisation {
        if normalisation.order_dependent != required_order_dependent
            || normalisation
                .order_independent
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                != required_order_independent
            || normalisation.order_independent.len() != required_order_independent.len()
        {
            return Err(CatalogueError::message(format!(
                "suite {} has an incomplete or incorrectly ordered decoded-output normalisation contract",
                suite.id
            )));
        }
    } else if !comparison.derived_sets.is_empty() {
        return Err(CatalogueError::message(format!(
            "suite {} derived-set contract lacks decoded-output normalisation",
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
        if case.resolution_reduction > 5
            || (!case.output_window && (case.output_origin_x != 0 || case.output_origin_y != 0))
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
    let base_cases = comparison
        .cases
        .iter()
        .map(|case| (case.id.as_str(), case))
        .collect::<BTreeMap<_, _>>();
    let base_choice_groups = comparison
        .choice_groups
        .iter()
        .map(|group| (group.id.as_str(), group))
        .collect::<BTreeMap<_, _>>();
    let mut derived_set_ids = BTreeSet::new();
    let mut derived_case_keys = BTreeSet::new();
    for derived_set in &comparison.derived_sets {
        if !derived_set_ids.insert(derived_set.id) {
            return Err(CatalogueError::message(format!(
                "suite {} repeats decoded-pixel derived set {:?}",
                suite.id, derived_set.id
            )));
        }
        if derived_set.id != DecodedPixelDerivedSetId::Ds0
            || derived_set.profile != 0
            || derived_set.compliance_class != 0
            || derived_set.selection
                != DecodedPixelDerivedSetSelection::GreatestBMagbNotExceedingMMagb
            || derived_set.cases.is_empty()
        {
            return Err(CatalogueError::message(format!(
                "suite {} has an unsupported derived-set identity, profile, class, or selection rule",
                suite.id
            )));
        }
        let expected_case_ids = DS0_EXPECTED_CASES
            .iter()
            .map(|case| {
                let mode = match case.coding_mode {
                    DecodedPixelDerivedSetCodingMode::HtOnly => "htonly",
                    DecodedPixelDerivedSetCodingMode::HtMix => "htmix",
                };
                format!(
                    "annex-c/class0-profile0/ds0-{mode}/p0-{}",
                    case.reference_number
                )
            })
            .collect::<BTreeSet<_>>();
        let actual_case_ids = derived_set
            .cases
            .iter()
            .map(|case| case.id.as_str())
            .collect::<BTreeSet<_>>();
        if actual_case_ids
            != expected_case_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
        {
            return Err(CatalogueError::message(format!(
                "suite {} DS0 Class-0 Profile-0 contract does not contain the exact 18 HTONLY and HTMIX case points",
                suite.id
            )));
        }
        for derived_case in &derived_set.cases {
            validate_id("decoded-pixel derived-set case", &derived_case.id)?;
            validate_id(
                "decoded-pixel derived-set reference case",
                &derived_case.reference_case_id,
            )?;
            if !case_ids.insert(derived_case.id.as_str()) {
                return Err(CatalogueError::message(format!(
                    "suite {} repeats decoded-pixel case, group, or derived-set ID {}",
                    suite.id, derived_case.id
                )));
            }
            if !derived_case_keys.insert((
                derived_case.reference_case_id.as_str(),
                derived_case.coding_mode,
            )) {
                return Err(CatalogueError::message(format!(
                    "suite {} repeats coding mode for decoded-pixel reference case {}",
                    suite.id, derived_case.reference_case_id
                )));
            }
            let reference_number = derived_case
                .reference_case_id
                .strip_prefix("annex-c/class0-profile0/p0-")
                .filter(|number| {
                    number.len() == 2 && number.bytes().all(|byte| byte.is_ascii_digit())
                })
                .ok_or_else(|| {
                    CatalogueError::message(format!(
                        "suite {} derived-set case {} does not reference a Class-0 Profile-0 case",
                        suite.id, derived_case.id
                    ))
                })?;
            let expected_case_id = match derived_case.coding_mode {
                DecodedPixelDerivedSetCodingMode::HtOnly => {
                    format!("annex-c/class0-profile0/ds0-htonly/p0-{reference_number}")
                }
                DecodedPixelDerivedSetCodingMode::HtMix => {
                    format!("annex-c/class0-profile0/ds0-htmix/p0-{reference_number}")
                }
            };
            if derived_case.id != expected_case_id || derived_case.variants.is_empty() {
                return Err(CatalogueError::message(format!(
                    "suite {} derived-set case {} has an inconsistent identity or no variants",
                    suite.id, derived_case.id
                )));
            }
            let scalar_reference = base_cases.get(derived_case.reference_case_id.as_str());
            let choice_reference = base_choice_groups.get(derived_case.reference_case_id.as_str());
            if scalar_reference.is_some() == choice_reference.is_some() {
                return Err(CatalogueError::message(format!(
                    "suite {} derived-set case {} must resolve to exactly one scalar or choice reference contract",
                    suite.id, derived_case.id
                )));
            }
            let expected_case = DS0_EXPECTED_CASES
                .iter()
                .find(|expected| {
                    expected.reference_number == reference_number
                        && expected.coding_mode == derived_case.coding_mode
                })
                .ok_or_else(|| {
                    CatalogueError::message(format!(
                        "suite {} derived-set case {} is absent from the canonical DS0 matrix",
                        suite.id, derived_case.id
                    ))
                })?;
            if derived_case.variants.len() != expected_case.variants.len() {
                return Err(CatalogueError::message(format!(
                    "suite {} derived-set case {} does not contain the canonical DS0 variant matrix",
                    suite.id, derived_case.id
                )));
            }
            let mut previous_b_magb = None;
            for (variant, expected_variant) in derived_case
                .variants
                .iter()
                .zip(expected_case.variants.iter())
            {
                validate_relative_path("decoded-pixel derived-set input", &variant.input)?;
                if previous_b_magb.is_some_and(|previous| previous >= variant.b_magb) {
                    return Err(CatalogueError::message(format!(
                        "suite {} derived-set case {} variants are not strictly ordered by B_MAGB",
                        suite.id, derived_case.id
                    )));
                }
                previous_b_magb = Some(variant.b_magb);
                if !input_paths.insert(variant.input.as_str()) {
                    return Err(CatalogueError::message(format!(
                        "suite {} repeats decoded-pixel input {}",
                        suite.id, variant.input
                    )));
                }
                let input = inventory.get(variant.input.as_str()).ok_or_else(|| {
                    CatalogueError::message(format!(
                        "suite {} derived-set case {} input is absent from the locked inventory",
                        suite.id, derived_case.id
                    ))
                })?;
                let mode_prefix = match derived_case.coding_mode {
                    DecodedPixelDerivedSetCodingMode::HtOnly => "ht",
                    DecodedPixelDerivedSetCodingMode::HtMix => "hm",
                };
                let expected_input = format!(
                    "files/htj2k_bsets_profile0/p0_{reference_number}_bset/ds0_{mode_prefix}_{reference_number}_b{}.j2k",
                    expected_variant.b_magb
                );
                if variant.b_magb != expected_variant.b_magb || input.path != expected_input {
                    return Err(CatalogueError::message(format!(
                        "suite {} derived-set case {} input does not match the canonical mode, profile, case, or B_MAGB variant",
                        suite.id, derived_case.id
                    )));
                }
                if scalar_reference.is_some_and(|reference| {
                    !variant.alternative_limits.is_empty()
                        || variant.component_limits.len() != 1
                        || variant.component_limits[0].component != reference.component
                        || variant.component_limits[0].peak_error_limit < reference.peak_error_limit
                        || variant.component_limits[0].mean_squared_error_limit
                            < reference.mean_squared_error_limit
                        || variant.component_limits[0].peak_error_limit
                            != expected_variant.peak_error_limit
                        || variant.component_limits[0]
                            .mean_squared_error_limit
                            .to_bits()
                            != expected_variant.mean_squared_error_limit.to_bits()
                        || !valid_derived_limits(
                            variant.component_limits[0].mean_squared_error_limit,
                        )
                }) {
                    return Err(CatalogueError::message(format!(
                        "suite {} derived-set case {} has limits inconsistent with its component reference",
                        suite.id, derived_case.id
                    )));
                }
                if let Some(reference) = choice_reference {
                    let expected_alternatives = reference
                        .alternatives
                        .iter()
                        .map(|alternative| alternative.id.as_str())
                        .collect::<BTreeSet<_>>();
                    let actual_alternatives = variant
                        .alternative_limits
                        .iter()
                        .map(|limits| limits.alternative_id.as_str())
                        .collect::<BTreeSet<_>>();
                    if !variant.component_limits.is_empty()
                        || variant.alternative_limits.len() != reference.alternatives.len()
                        || actual_alternatives != expected_alternatives
                        || variant.alternative_limits.iter().any(|limits| {
                            let Some(alternative) = reference
                                .alternatives
                                .iter()
                                .find(|alternative| alternative.id == limits.alternative_id)
                            else {
                                return true;
                            };
                            limits.peak_error_limit < alternative.peak_error_limit
                                || limits.mean_squared_error_limit
                                    < alternative.mean_squared_error_limit
                                || limits.peak_error_limit != expected_variant.peak_error_limit
                                || limits.mean_squared_error_limit.to_bits()
                                    != expected_variant.mean_squared_error_limit.to_bits()
                                || !valid_derived_limits(limits.mean_squared_error_limit)
                        })
                    {
                        return Err(CatalogueError::message(format!(
                            "suite {} derived-set case {} has limits inconsistent with its choice reference",
                            suite.id, derived_case.id
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

fn valid_derived_limits(mean_squared_error_limit: f64) -> bool {
    mean_squared_error_limit.is_finite() && mean_squared_error_limit >= 0.0
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

    fn rendered_fixture() -> (
        SuiteManifest,
        crate::model::RenderedPixelComparisonPlan,
        Catalogue,
    ) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        let suite = catalogue
            .suites
            .get("layer2/conformance-jpeg-2000")
            .expect("rendered comparison suite exists")
            .manifest
            .clone();
        let comparison = suite
            .rendered_pixel_comparison
            .clone()
            .expect("rendered comparison plan exists");
        (suite, comparison, catalogue)
    }

    fn rendered_schema() -> serde_json::Value {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../../schema/suite.schema.json"))
                .expect("suite schema is valid JSON");
        schema["properties"]["rendered_pixel_comparison"].clone()
    }

    fn rendered_schema_instance() -> serde_json::Value {
        let (_, comparison, _) = rendered_fixture();
        serde_json::to_value(comparison).expect("rendered comparison serialises as JSON")
    }

    fn changed_schema_instance(
        base: &serde_json::Value,
        update: impl FnOnce(&mut serde_json::Value),
    ) -> serde_json::Value {
        let mut changed = base.clone();
        update(&mut changed);
        changed
    }

    fn changed_rendered_comparison(
        base: &crate::model::RenderedPixelComparisonPlan,
        update: impl FnOnce(&mut crate::model::RenderedPixelComparisonPlan),
    ) -> crate::model::RenderedPixelComparisonPlan {
        let mut changed = base.clone();
        update(&mut changed);
        changed
    }

    fn bounded_schema_accepts(schema: &serde_json::Value, instance: &serde_json::Value) -> bool {
        let schema = schema
            .as_object()
            .expect("bounded schema node is an object");
        for keyword in schema.keys() {
            assert!(
                matches!(
                    keyword.as_str(),
                    "type"
                        | "additionalProperties"
                        | "required"
                        | "properties"
                        | "minItems"
                        | "uniqueItems"
                        | "items"
                        | "minLength"
                        | "pattern"
                        | "const"
                        | "minimum"
                        | "maximum"
                ),
                "unsupported bounded schema keyword {keyword}"
            );
        }

        if schema
            .get("const")
            .is_some_and(|expected| expected != instance)
        {
            return false;
        }
        let Some(expected_type) = schema.get("type").and_then(serde_json::Value::as_str) else {
            return true;
        };
        match expected_type {
            "object" => {
                let Some(instance) = instance.as_object() else {
                    return false;
                };
                let properties = schema
                    .get("properties")
                    .and_then(serde_json::Value::as_object)
                    .expect("object schema has properties");
                if schema
                    .get("required")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|required| {
                        required.iter().any(|field| {
                            !instance.contains_key(
                                field.as_str().expect("required field name is a string"),
                            )
                        })
                    })
                {
                    return false;
                }
                for (field, value) in instance {
                    if let Some(field_schema) = properties.get(field) {
                        if !bounded_schema_accepts(field_schema, value) {
                            return false;
                        }
                    } else if schema
                        .get("additionalProperties")
                        .is_some_and(|allowed| allowed == false)
                    {
                        return false;
                    }
                }
                true
            }
            "array" => {
                let Some(instance) = instance.as_array() else {
                    return false;
                };
                if schema
                    .get("minItems")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|minimum| instance.len() < minimum as usize)
                {
                    return false;
                }
                if schema
                    .get("uniqueItems")
                    .is_some_and(|unique| unique == true)
                    && instance.iter().enumerate().any(|(index, value)| {
                        instance[index + 1..]
                            .iter()
                            .any(|candidate| candidate == value)
                    })
                {
                    return false;
                }
                let item_schema = schema.get("items").expect("array schema has items");
                instance
                    .iter()
                    .all(|value| bounded_schema_accepts(item_schema, value))
            }
            "string" => {
                let Some(instance) = instance.as_str() else {
                    return false;
                };
                if schema
                    .get("minLength")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|minimum| instance.chars().count() < minimum as usize)
                {
                    return false;
                }
                schema
                    .get("pattern")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|pattern| bounded_pattern_matches(pattern, instance))
            }
            "integer" => {
                let Some(value) = instance.as_f64().filter(|value| value.fract() == 0.0) else {
                    return false;
                };
                bounded_numeric_limits_accept(schema, value)
            }
            "number" => instance
                .as_f64()
                .is_some_and(|value| bounded_numeric_limits_accept(schema, value)),
            other => panic!("unsupported bounded schema type {other}"),
        }
    }

    fn bounded_numeric_limits_accept(
        schema: &serde_json::Map<String, serde_json::Value>,
        value: f64,
    ) -> bool {
        !schema
            .get("minimum")
            .and_then(serde_json::Value::as_f64)
            .is_some_and(|minimum| value < minimum)
            && !schema
                .get("maximum")
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|maximum| value > maximum)
    }

    fn bounded_pattern_matches(pattern: &str, value: &str) -> bool {
        match pattern {
            r"\S" => value.chars().any(|character| !character.is_whitespace()),
            r"^[0-9a-f]{40}(?![\s\S])" => {
                value.len() == 40
                    && value
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }
            r"^(?!.*(?:^|/)\.\.(?:/|$))[a-z0-9](?:[a-z0-9./-]*[a-z0-9.-])?(?![\s\S])" => {
                !value.is_empty()
                    && value
                        .bytes()
                        .next()
                        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                    && !value.ends_with('/')
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'-' | b'/' | b'.')
                    })
                    && value.split('/').all(|component| component != "..")
            }
            r"^(?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)*[A-Za-z0-9_-][A-Za-z0-9._-]*\.jp2(?![\s\S])" => {
                bounded_portable_path_matches(value, ".jp2")
            }
            r"^(?:[A-Za-z0-9_-][A-Za-z0-9._-]*/)*[A-Za-z0-9_-][A-Za-z0-9._-]*\.tif(?![\s\S])" => {
                bounded_portable_path_matches(value, ".tif")
            }
            other => panic!("unsupported bounded schema pattern {other}"),
        }
    }

    fn bounded_portable_path_matches(value: &str, extension: &str) -> bool {
        value.ends_with(extension)
            && value.split('/').all(|component| {
                component
                    .bytes()
                    .next()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                    && component.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
                    })
            })
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
    fn deserialises_and_validates_annex_g_rendered_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join("suites/conformance-jpeg-2000.toml"))
            .expect("suite source is readable");
        let suite: SuiteManifest = toml::from_str(&source).expect("suite deserialises");
        assert_eq!(suite.revision, 20);
        let comparison = suite
            .rendered_pixel_comparison
            .as_ref()
            .expect("rendered comparison plan exists");
        assert_eq!(comparison.pack_id, "jpeg-2000/conformance");
        assert_eq!(comparison.standard, "ISO/IEC 15444-4:2024");
        assert_eq!(
            comparison.clauses,
            [
                "Annex G",
                "G.1",
                "G.2",
                "G.3",
                "G.4.2",
                "G.4.3",
                "Table G.1"
            ]
        );
        assert_eq!(
            comparison.retrieval_commit,
            "725ecba70e5d03eff3f6ce9626bb9cb08dd4e0c7"
        );
        assert_eq!(comparison.cases.len(), 1);
        let case = &comparison.cases[0];
        assert_eq!(case.id, "annex-g/jp2/file3");
        assert_eq!(case.input, "files/testfiles_jp2/file3.jp2");
        assert_eq!(case.reference, "files/reference_jp2/jp2_3.tif");
        assert_eq!((case.width, case.height), (480, 640));
        assert_eq!((case.components, case.bits_per_sample), (3, 8));
        assert_eq!(case.rendered_colour_space, RenderedColourSpace::Srgb);
        assert_eq!(
            case.reference_layout,
            RenderedReferenceLayout::TiffRgbU8Contiguous
        );
        assert_eq!(case.peak_error_limit, 4.0);

        let catalogue = Catalogue::open(root).expect("repository catalogue opens");
        catalogue.check().expect("repository catalogue checks");
        let assets = &catalogue.packs["jpeg-2000/conformance"].manifest.assets;
        let input = assets
            .iter()
            .find(|asset| asset.path == case.input)
            .expect("input is locked");
        let reference = assets
            .iter()
            .find(|asset| asset.path == case.reference)
            .expect("reference is locked");
        assert_eq!(
            input.sha256,
            "fe922461d6928f9b9c86c222a133c42c19d119351400d5e8dd6a1e60db437e66"
        );
        assert_eq!(
            reference.sha256,
            "512a8827b98d71051c3cba52b96a323e879870ba90dc254016befaa1aa90dbd5"
        );
    }

    #[test]
    fn rendered_contract_schema_executes_valid_and_adversarial_instances() {
        let schema = rendered_schema();
        let valid = rendered_schema_instance();
        assert!(bounded_schema_accepts(&schema, &valid));

        let one_character_id = changed_schema_instance(&valid, |instance| {
            instance["cases"][0]["id"] = serde_json::json!("a");
        });
        validate_id("rendered-pixel case", "a")
            .expect("catalogue accepts a one-character rendered case ID");
        assert!(
            bounded_schema_accepts(&schema, &one_character_id),
            "schema must accept every one-character ID accepted by validate_id"
        );

        let model_overflow = changed_schema_instance(&valid, |instance| {
            instance["cases"][0]["width"] = serde_json::json!(4_294_967_296_u64);
        });
        serde_json::from_value::<crate::model::RenderedPixelComparisonPlan>(model_overflow)
            .expect_err("u32 model must reject overflowing geometry");

        let invalid = [
            (
                "missing required plan field",
                changed_schema_instance(&valid, |instance| {
                    instance
                        .as_object_mut()
                        .expect("plan is an object")
                        .remove("standard");
                }),
            ),
            (
                "wrong geometry type",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["width"] = serde_json::json!("480");
                }),
            ),
            (
                "wrong component constant",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["components"] = serde_json::json!(2);
                }),
            ),
            (
                "wrong colour constant",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["rendered_colour_space"] = serde_json::json!("Display P3");
                }),
            ),
            (
                "blank standard",
                changed_schema_instance(&valid, |instance| {
                    instance["standard"] = serde_json::json!(" \t\n");
                }),
            ),
            (
                "blank clause",
                changed_schema_instance(&valid, |instance| {
                    instance["clauses"][0] = serde_json::json!("   ");
                }),
            ),
            (
                "duplicate clause",
                changed_schema_instance(&valid, |instance| {
                    instance["clauses"] = serde_json::json!(["G.1", "G.1"]);
                }),
            ),
            (
                "invalid retrieval commit",
                changed_schema_instance(&valid, |instance| {
                    instance["retrieval_commit"] =
                        serde_json::json!("725ECBA70E5D03EFF3F6CE9626BB9CB08DD4E0C7");
                }),
            ),
            (
                "unsafe case ID",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["id"] = serde_json::json!("a/../b");
                }),
            ),
            (
                "zero width",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["width"] = serde_json::json!(0);
                }),
            ),
            (
                "zero height",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["height"] = serde_json::json!(0);
                }),
            ),
            (
                "u32-overflow width",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["width"] = serde_json::json!(4_294_967_296_u64);
                }),
            ),
            (
                "u32-overflow height",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["height"] = serde_json::json!(4_294_967_296_u64);
                }),
            ),
            (
                "negative peak limit",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["peak_error_limit"] = serde_json::json!(-0.1);
                }),
            ),
            (
                "unknown plan field",
                changed_schema_instance(&valid, |instance| {
                    instance
                        .as_object_mut()
                        .expect("plan is an object")
                        .insert("unknown".to_owned(), serde_json::json!(true));
                }),
            ),
            (
                "unknown case field",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]
                        .as_object_mut()
                        .expect("case is an object")
                        .insert("unknown".to_owned(), serde_json::json!(true));
                }),
            ),
            (
                "parent-traversing input",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["input"] = serde_json::json!("../file3.jp2");
                }),
            ),
            (
                "absolute input",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["input"] =
                        serde_json::json!("/files/testfiles_jp2/file3.jp2");
                }),
            ),
            (
                "Windows-absolute input",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["input"] = serde_json::json!("C:/files/file3.jp2");
                }),
            ),
            (
                "backslash input",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["input"] =
                        serde_json::json!(r"files\testfiles_jp2\file3.jp2");
                }),
            ),
            (
                "uppercase input extension",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["input"] =
                        serde_json::json!("files/testfiles_jp2/file3.JP2");
                }),
            ),
            (
                "parent-traversing reference",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["reference"] = serde_json::json!("../jp2_3.tif");
                }),
            ),
            (
                "absolute reference",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["reference"] =
                        serde_json::json!("/files/reference_jp2/jp2_3.tif");
                }),
            ),
            (
                "backslash reference",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["reference"] =
                        serde_json::json!(r"files\reference_jp2\jp2_3.tif");
                }),
            ),
            (
                "uppercase reference extension",
                changed_schema_instance(&valid, |instance| {
                    instance["cases"][0]["reference"] =
                        serde_json::json!("files/reference_jp2/jp2_3.TIF");
                }),
            ),
        ];
        for (label, instance) in invalid {
            assert!(
                !bounded_schema_accepts(&schema, &instance),
                "schema accepted {label}"
            );
        }
    }

    #[test]
    fn rendered_schema_leaves_cross_record_relations_to_catalogue() {
        let schema = rendered_schema();
        let valid = rendered_schema_instance();

        let duplicate_identity = changed_schema_instance(&valid, |instance| {
            let mut second = instance["cases"][0].clone();
            second["input"] = serde_json::json!("files/testfiles_jp2/file4.jp2");
            second["reference"] = serde_json::json!("files/reference_jp2/jp2_4.tif");
            instance["cases"]
                .as_array_mut()
                .expect("cases is an array")
                .push(second);
        });
        assert!(bounded_schema_accepts(&schema, &duplicate_identity));

        let duplicate_input = changed_schema_instance(&valid, |instance| {
            let mut second = instance["cases"][0].clone();
            second["id"] = serde_json::json!("annex-g/jp2/file4");
            second["reference"] = serde_json::json!("files/reference_jp2/jp2_4.tif");
            instance["cases"]
                .as_array_mut()
                .expect("cases is an array")
                .push(second);
        });
        assert!(bounded_schema_accepts(&schema, &duplicate_input));

        let duplicate_reference = changed_schema_instance(&valid, |instance| {
            let mut second = instance["cases"][0].clone();
            second["id"] = serde_json::json!("annex-g/jp2/file4");
            second["input"] = serde_json::json!("files/testfiles_jp2/file4.jp2");
            instance["cases"]
                .as_array_mut()
                .expect("cases is an array")
                .push(second);
        });
        assert!(bounded_schema_accepts(&schema, &duplicate_reference));

        let (suite, comparison, catalogue) = rendered_fixture();
        let mut duplicate_identity = comparison.clone();
        let mut second = comparison.cases[0].clone();
        second.input = "files/testfiles_jp2/file4.jp2".to_owned();
        second.reference = "files/reference_jp2/jp2_4.tif".to_owned();
        duplicate_identity.cases.push(second);
        let error =
            validate_rendered_pixel_comparison_plan(&suite, &duplicate_identity, &catalogue.packs)
                .expect_err("catalogue must reject a repeated case ID");
        assert!(error.to_string().contains("repeats rendered-pixel case ID"));

        let mut duplicate_input = comparison.clone();
        let mut second = comparison.cases[0].clone();
        second.id = "annex-g/jp2/file4".to_owned();
        second.reference = "files/reference_jp2/jp2_4.tif".to_owned();
        duplicate_input.cases.push(second);
        let error =
            validate_rendered_pixel_comparison_plan(&suite, &duplicate_input, &catalogue.packs)
                .expect_err("catalogue must reject a repeated input");
        assert!(error.to_string().contains("repeats rendered-pixel input"));

        let mut duplicate_reference = comparison;
        let mut second = duplicate_reference.cases[0].clone();
        second.id = "annex-g/jp2/file4".to_owned();
        second.input = "files/testfiles_jp2/file4.jp2".to_owned();
        duplicate_reference.cases.push(second);
        let error =
            validate_rendered_pixel_comparison_plan(&suite, &duplicate_reference, &catalogue.packs)
                .expect_err("catalogue must reject a repeated reference");
        assert!(
            error
                .to_string()
                .contains("repeats rendered-pixel reference")
        );
    }

    #[test]
    fn rejects_missing_or_unlocked_rendered_pack() {
        let (suite, mut comparison, catalogue) = rendered_fixture();
        comparison.pack_id = "jpeg-2000/missing".to_owned();
        let error = validate_rendered_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("unselected pack must fail");
        assert!(error.to_string().contains("names unselected pack"));

        let (mut suite, mut comparison, catalogue) = rendered_fixture();
        comparison.pack_id = "jpeg-2000/missing".to_owned();
        suite.packs.push(crate::model::SuitePack {
            id: comparison.pack_id.clone(),
            version: "1".to_owned(),
            required: true,
        });
        let error = validate_rendered_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("missing catalogue pack must fail");
        assert!(error.to_string().contains("disappeared from catalogue"));

        let (suite, comparison, mut catalogue) = rendered_fixture();
        catalogue
            .packs
            .get_mut("jpeg-2000/conformance")
            .expect("pack exists")
            .manifest
            .review_state = ReviewState::Reviewed;
        let error = validate_rendered_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("unlocked pack must fail");
        assert!(error.to_string().contains("requires locked pack"));
    }

    #[test]
    fn rejects_blank_rendered_authority() {
        let (suite, comparison, catalogue) = rendered_fixture();
        for invalid in [
            changed_rendered_comparison(&comparison, |comparison| {
                comparison.standard = " \t".to_owned();
            }),
            changed_rendered_comparison(&comparison, |comparison| {
                comparison.clauses[0] = " \n".to_owned();
            }),
        ] {
            let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
                .expect_err("blank rendered authority must fail");
            assert!(error.to_string().contains("incomplete rendered-pixel"));
        }
    }

    #[test]
    fn rejects_absent_or_unsafe_rendered_paths() {
        let (suite, comparison, catalogue) = rendered_fixture();
        for (input, reference) in [
            ("files/missing.jp2", comparison.cases[0].reference.as_str()),
            (comparison.cases[0].input.as_str(), "files/missing.tif"),
        ] {
            let mut invalid = comparison.clone();
            invalid.cases[0].input = input.to_owned();
            invalid.cases[0].reference = reference.to_owned();
            let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
                .expect_err("absent inventory path must fail");
            assert!(
                error
                    .to_string()
                    .contains("absent from the locked inventory")
            );
        }

        for (input, reference) in [
            ("../file3.jp2", comparison.cases[0].reference.as_str()),
            (comparison.cases[0].input.as_str(), "/tmp/jp2_3.tif"),
        ] {
            let mut invalid = comparison.clone();
            invalid.cases[0].input = input.to_owned();
            invalid.cases[0].reference = reference.to_owned();
            let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
                .expect_err("unsafe path must fail");
            assert!(
                error.to_string().contains("unsafe traversal")
                    || error.to_string().contains("non-empty relative path")
            );
        }
    }

    #[test]
    fn rejects_duplicate_or_same_rendered_paths_and_cases() {
        let (suite, comparison, catalogue) = rendered_fixture();

        let mut invalid = comparison.clone();
        invalid.cases.push(invalid.cases[0].clone());
        let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
            .expect_err("duplicate case ID must fail");
        assert!(error.to_string().contains("repeats rendered-pixel case ID"));

        let mut invalid = comparison.clone();
        let mut second = invalid.cases[0].clone();
        second.id = "annex-g/jp2/file4".to_owned();
        second.reference = "files/reference_jp2/jp2_4.tif".to_owned();
        invalid.cases.push(second);
        let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
            .expect_err("duplicate input must fail");
        assert!(error.to_string().contains("repeats rendered-pixel input"));

        let mut invalid = comparison.clone();
        let mut second = invalid.cases[0].clone();
        second.id = "annex-g/jp2/file4".to_owned();
        second.input = "files/testfiles_jp2/file4.jp2".to_owned();
        invalid.cases.push(second);
        let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
            .expect_err("duplicate reference must fail");
        assert!(
            error
                .to_string()
                .contains("repeats rendered-pixel reference")
        );

        let mut invalid = comparison;
        invalid.cases[0].reference = invalid.cases[0].input.clone();
        let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
            .expect_err("same input and reference must fail");
        assert!(
            error
                .to_string()
                .contains("one path as input and reference")
        );
    }

    #[test]
    fn rejects_wrong_rendered_extensions_or_inventory_media_types() {
        let (suite, comparison, catalogue) = rendered_fixture();
        for (input, reference) in [
            (
                "files/testfiles_jph/file1_b11.jph",
                comparison.cases[0].reference.as_str(),
            ),
            (
                comparison.cases[0].input.as_str(),
                "files/reference_class0_profile0/c0p0_01.pgx",
            ),
        ] {
            let mut invalid = comparison.clone();
            invalid.cases[0].input = input.to_owned();
            invalid.cases[0].reference = reference.to_owned();
            let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
                .expect_err("wrong extension must fail");
            assert!(
                error
                    .to_string()
                    .contains("requires a .jp2 input and .tif reference")
            );
        }

        for path in [
            comparison.cases[0].input.as_str(),
            comparison.cases[0].reference.as_str(),
        ] {
            let (suite, comparison, mut catalogue) = rendered_fixture();
            catalogue
                .packs
                .get_mut("jpeg-2000/conformance")
                .expect("pack exists")
                .manifest
                .assets
                .iter_mut()
                .find(|asset| asset.path == path)
                .expect("asset exists")
                .media_type = "application/octet-stream".to_owned();
            let error =
                validate_rendered_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
                    .expect_err("wrong inventory media type must fail");
            assert!(error.to_string().contains("inventory media type"));
        }
    }

    #[test]
    fn rejects_wrong_rendered_shape_and_error_limits() {
        let (suite, comparison, catalogue) = rendered_fixture();
        for (width, height, components, bits_per_sample) in [
            (0, 640, 3, 8),
            (480, 0, 3, 8),
            (480, 640, 2, 8),
            (480, 640, 3, 16),
        ] {
            let mut invalid = comparison.clone();
            let case = &mut invalid.cases[0];
            (
                case.width,
                case.height,
                case.components,
                case.bits_per_sample,
            ) = (width, height, components, bits_per_sample);
            let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
                .expect_err("unsupported shape must fail");
            assert!(error.to_string().contains("unsupported full-frame shape"));
        }

        for limit in [-0.1, f64::INFINITY, f64::NAN] {
            let mut invalid = comparison.clone();
            invalid.cases[0].peak_error_limit = limit;
            let error = validate_rendered_pixel_comparison_plan(&suite, &invalid, &catalogue.packs)
                .expect_err("negative or non-finite limit must fail");
            assert!(error.to_string().contains("error limit"));
        }
    }

    #[test]
    fn rejects_wrong_rendered_colour_layout_and_unknown_fields_during_deserialisation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join("suites/conformance-jpeg-2000.toml"))
            .expect("suite source is readable");
        for invalid in [
            source.replacen(
                "rendered_colour_space = \"sRGB\"",
                "rendered_colour_space = \"Display P3\"",
                1,
            ),
            source.replacen(
                "reference_layout = \"tiff-rgb-u8-contiguous\"",
                "reference_layout = \"tiff-planar\"",
                1,
            ),
            source.replacen(
                "[rendered_pixel_comparison]\n",
                "[rendered_pixel_comparison]\nunknown_plan_field = true\n",
                1,
            ),
            source.replacen(
                "[[rendered_pixel_comparison.cases]]\n",
                "[[rendered_pixel_comparison.cases]]\nunknown_case_field = true\n",
                1,
            ),
        ] {
            toml::from_str::<SuiteManifest>(&invalid)
                .expect_err("invalid rendered contract must not deserialise");
        }
    }

    #[test]
    fn rendered_contract_preserves_native_and_derived_set_plans() {
        let (suite, _, catalogue) = rendered_fixture();
        let decoded = suite
            .decoded_pixel_comparison
            .as_ref()
            .expect("native comparison plan remains present");
        assert_eq!(decoded.cases.len(), 14);
        assert_eq!(decoded.choice_groups.len(), 2);
        assert_eq!(decoded.derived_sets.len(), 1);
        assert_eq!(decoded.derived_sets[0].cases.len(), 18);
        validate_decoded_pixel_comparison_plan(&suite, decoded, &catalogue.packs)
            .expect("native and DS0 contracts remain valid");
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
            .expect("scalar cases support five reduction levels");

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison
            .cases
            .iter_mut()
            .find(|case| case.id == "annex-c/class0-profile0/p0-04")
            .expect("P0.04 scalar case exists")
            .resolution_reduction = 5;
        validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect("scalar cases admit five reduction levels");
        comparison
            .cases
            .iter_mut()
            .find(|case| case.id == "annex-c/class0-profile0/p0-04")
            .expect("P0.04 scalar case exists")
            .resolution_reduction = 6;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("scalar cases remain bounded to five reduction levels");
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
    fn records_p0_06_reduced_scalar_contract() {
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
            .find(|case| case.id == "annex-c/class0-profile0/p0-06")
            .expect("P0.06 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_06.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_06.pgx"
        );
        assert_eq!(case.component, 0);
        assert_eq!(case.resolution_reduction, 3);
        assert_eq!((case.width, case.height), (65, 17));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 109);
        assert_eq!(case.mean_squared_error_limit, 743.0);
    }

    #[test]
    fn records_p0_07_scalar_output_window_contract() {
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
            .find(|case| case.id == "annex-c/class0-profile0/p0-07")
            .expect("P0.07 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_07.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_07.pgx"
        );
        assert_eq!(case.component, 0);
        assert!(case.output_window);
        assert_eq!((case.output_origin_x, case.output_origin_y), (0, 0));
        assert_eq!(case.resolution_reduction, 0);
        assert_eq!((case.width, case.height), (128, 128));
        assert_eq!(case.bits_per_sample, 8);
        assert!(case.signed);
        assert_eq!(case.peak_error_limit, 10);
        assert_eq!(case.mean_squared_error_limit, 0.34);
    }

    #[test]
    fn records_p0_08_reduction_five_scalar_contract() {
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
            .find(|case| case.id == "annex-c/class0-profile0/p0-08")
            .expect("P0.08 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_08.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_08.pgx"
        );
        assert_eq!(case.component, 0);
        assert!(!case.output_window);
        assert_eq!((case.output_origin_x, case.output_origin_y), (0, 0));
        assert_eq!(case.resolution_reduction, 5);
        assert_eq!((case.width, case.height), (17, 96));
        assert_eq!(case.bits_per_sample, 8);
        assert!(case.signed);
        assert_eq!(case.peak_error_limit, 7);
        assert_eq!(case.mean_squared_error_limit, 6.72);
    }

    #[test]
    fn records_p0_10_scalar_contract() {
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
            .find(|case| case.id == "annex-c/class0-profile0/p0-10")
            .expect("P0.10 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_10.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_10.pgx"
        );
        assert_eq!(case.component, 0);
        assert!(!case.output_window);
        assert_eq!((case.output_origin_x, case.output_origin_y), (0, 0));
        assert_eq!(case.resolution_reduction, 0);
        assert_eq!((case.width, case.height), (64, 64));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 10);
        assert_eq!(case.mean_squared_error_limit, 2.84);
    }

    #[test]
    fn records_p0_13_scalar_contract() {
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
            .find(|case| case.id == "annex-c/class0-profile0/p0-13")
            .expect("P0.13 scalar case exists");
        assert_eq!(case.input, "files/codestreams_profile0/p0_13.j2k");
        assert_eq!(
            case.reference,
            "files/reference_class0_profile0/c0p0_13.pgx"
        );
        assert_eq!(case.component, 0);
        assert!(!case.output_window);
        assert_eq!((case.output_origin_x, case.output_origin_y), (0, 0));
        assert_eq!(case.resolution_reduction, 0);
        assert_eq!((case.width, case.height), (1, 1));
        assert_eq!(case.bits_per_sample, 8);
        assert!(!case.signed);
        assert_eq!(case.peak_error_limit, 0);
        assert_eq!(case.mean_squared_error_limit, 0.0);
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
    fn records_and_selects_all_ds0_profile0_case_points() {
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
        assert_eq!(plan.derived_sets.len(), 1);
        let derived_set = &plan.derived_sets[0];
        assert_eq!(derived_set.id, DecodedPixelDerivedSetId::Ds0);
        assert_eq!(derived_set.profile, 0);
        assert_eq!(derived_set.compliance_class, 0);
        assert_eq!(derived_set.cases.len(), 18);
        assert_eq!(
            derived_set
                .cases
                .iter()
                .map(|case| case.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "annex-c/class0-profile0/ds0-htonly/p0-01",
                "annex-c/class0-profile0/ds0-htonly/p0-02",
                "annex-c/class0-profile0/ds0-htonly/p0-03",
                "annex-c/class0-profile0/ds0-htonly/p0-04",
                "annex-c/class0-profile0/ds0-htonly/p0-05",
                "annex-c/class0-profile0/ds0-htonly/p0-06",
                "annex-c/class0-profile0/ds0-htmix/p0-06",
                "annex-c/class0-profile0/ds0-htonly/p0-07",
                "annex-c/class0-profile0/ds0-htonly/p0-08",
                "annex-c/class0-profile0/ds0-htonly/p0-09",
                "annex-c/class0-profile0/ds0-htonly/p0-10",
                "annex-c/class0-profile0/ds0-htonly/p0-11",
                "annex-c/class0-profile0/ds0-htonly/p0-12",
                "annex-c/class0-profile0/ds0-htonly/p0-13",
                "annex-c/class0-profile0/ds0-htonly/p0-14",
                "annex-c/class0-profile0/ds0-htonly/p0-15",
                "annex-c/class0-profile0/ds0-htmix/p0-15",
                "annex-c/class0-profile0/ds0-htonly/p0-16",
            ]
        );
        assert_eq!(
            derived_set
                .cases
                .iter()
                .map(|case| case.variants.len())
                .sum::<usize>(),
            30
        );
        let derived_cases = derived_set
            .cases
            .iter()
            .map(|case| (case.id.as_str(), case))
            .collect::<BTreeMap<_, _>>();

        let p0_02_b11 = &derived_cases["annex-c/class0-profile0/ds0-htonly/p0-02"].variants[0]
            .component_limits[0];
        assert_eq!(
            (
                p0_02_b11.peak_error_limit,
                p0_02_b11.mean_squared_error_limit
            ),
            (1, 0.001)
        );
        let p0_03_b11 = &derived_cases["annex-c/class0-profile0/ds0-htonly/p0-03"].variants[0]
            .alternative_limits;
        assert!(p0_03_b11.iter().all(|limits| {
            limits.peak_error_limit == 17 && limits.mean_squared_error_limit == 0.15
        }));
        let p0_04_b11 = &derived_cases["annex-c/class0-profile0/ds0-htonly/p0-04"].variants[0]
            .component_limits[0];
        assert_eq!(
            (
                p0_04_b11.peak_error_limit,
                p0_04_b11.mean_squared_error_limit
            ),
            (35, 55.9)
        );

        let p0_06 = derived_cases["annex-c/class0-profile0/ds0-htonly/p0-06"];
        assert!(p0_06.select_variant(10).is_none());
        assert_eq!(p0_06.select_variant(11).expect("B11 selected").b_magb, 11);
        assert_eq!(p0_06.select_variant(14).expect("B11 selected").b_magb, 11);
        assert_eq!(p0_06.select_variant(15).expect("B15 selected").b_magb, 15);
        assert_eq!(p0_06.select_variant(17).expect("B15 selected").b_magb, 15);
        assert_eq!(p0_06.select_variant(18).expect("B18 selected").b_magb, 18);
        assert_eq!(
            p0_06.select_variant(u8::MAX).expect("B18 selected").b_magb,
            18
        );
        let b11_limits = &p0_06.variants[0].component_limits[0];
        assert_eq!(b11_limits.peak_error_limit, 266);
        assert_eq!(b11_limits.mean_squared_error_limit, 1035.96875);

        let p0_07 = derived_cases["annex-c/class0-profile0/ds0-htonly/p0-07"];
        assert_eq!(
            p0_07
                .variants
                .iter()
                .map(|variant| {
                    let limits = &variant.component_limits[0];
                    (
                        variant.b_magb,
                        limits.peak_error_limit,
                        limits.mean_squared_error_limit,
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (11, 13, 0.43765625),
                (15, 11, 0.34029296875),
                (16, 10, 0.34),
            ]
        );
        let p0_08 = derived_cases["annex-c/class0-profile0/ds0-htonly/p0-08"];
        assert_eq!(
            p0_08
                .variants
                .iter()
                .map(|variant| {
                    let limits = &variant.component_limits[0];
                    (
                        variant.b_magb,
                        limits.peak_error_limit,
                        limits.mean_squared_error_limit,
                    )
                })
                .collect::<Vec<_>>(),
            vec![(11, 10, 6.89578125), (15, 7, 6.72), (16, 7, 6.72)]
        );

        let p0_15_htonly = derived_cases["annex-c/class0-profile0/ds0-htonly/p0-15"];
        assert!(
            p0_15_htonly.variants[0]
                .alternative_limits
                .iter()
                .all(|limits| {
                    limits.peak_error_limit == 17 && limits.mean_squared_error_limit == 0.15
                })
        );
        let p0_15_htmix = derived_cases["annex-c/class0-profile0/ds0-htmix/p0-15"];
        assert!(p0_15_htmix.select_variant(7).is_none());
        let selected = p0_15_htmix.select_variant(18).expect("B8 selected");
        assert_eq!(selected.b_magb, 8);
        assert_eq!(selected.alternative_limits.len(), 2);
        assert!(selected.alternative_limits.iter().all(|limits| {
            limits.peak_error_limit == 0 && limits.mean_squared_error_limit == 0.0
        }));
    }

    #[test]
    fn rejects_invalid_ds0_derived_set_contracts() {
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
        comparison.derived_sets[0].profile = 1;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("DS0 Profile-1 contract must fail");
        assert!(
            error
                .to_string()
                .contains("unsupported derived-set identity")
        );

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[1].variants.reverse();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("descending B_MAGB variants must fail");
        assert!(error.to_string().contains("canonical"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0]
            .cases
            .iter_mut()
            .find(|case| case.id == "annex-c/class0-profile0/ds0-htonly/p0-06")
            .expect("P0.06 exists")
            .variants
            .pop();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("removing the highest B_MAGB variant must fail");
        assert!(error.to_string().contains("canonical DS0 variant matrix"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        let p0_06 = comparison.derived_sets[0]
            .cases
            .iter_mut()
            .find(|case| case.id == "annex-c/class0-profile0/ds0-htonly/p0-06")
            .expect("P0.06 exists");
        p0_06
            .variants
            .push(p0_06.variants.last().expect("P0.06 has variants").clone());
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("adding a DS0 variant must fail");
        assert!(error.to_string().contains("canonical DS0 variant matrix"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[1].variants[1].b_magb = 13;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("changing a canonical B_MAGB variant must fail");
        assert!(error.to_string().contains("canonical mode"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[1].variants[0].input =
            "files/htj2k_bsets_profile0/p0_02_bset/ds0_ht_02_b12.j2k".to_owned();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("a locked but non-canonical variant path must fail");
        assert!(error.to_string().contains("canonical mode"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[0].variants[0].input =
            "files/htj2k_bsets_profile0/not-in-inventory.j2k".to_owned();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("unlocked derived input must fail");
        assert!(
            error
                .to_string()
                .contains("absent from the locked inventory")
        );

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[0].variants[0]
            .component_limits
            .clear();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("missing component limits must fail");
        assert!(error.to_string().contains("component reference"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[2].variants[0].alternative_limits[1].alternative_id =
            "full-resolution-window".to_owned();
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("duplicate choice alternative limits must fail");
        assert!(error.to_string().contains("choice reference"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[1].variants[0].component_limits[0].peak_error_limit += 1;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("increasing a scalar peak limit must fail");
        assert!(error.to_string().contains("component reference"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[1].variants[0].component_limits[0]
            .mean_squared_error_limit = 0.0;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("decreasing a scalar MSE limit must fail");
        assert!(error.to_string().contains("component reference"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[2].variants[0].alternative_limits[0]
            .mean_squared_error_limit = 0.16;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("increasing an alternative MSE limit must fail");
        assert!(error.to_string().contains("choice reference"));

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.derived_sets[0].cases[2].variants[0].alternative_limits[0].peak_error_limit = 16;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("decreasing an alternative peak limit must fail");
        assert!(error.to_string().contains("choice reference"));
    }

    #[test]
    fn keeps_v1_scalar_plans_compatible_but_requires_derived_normalisation() {
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
        comparison.derived_sets.clear();
        comparison.output_normalisation = None;
        validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect("schema-v1 scalar-only plans may omit normalisation");

        let mut comparison = suite
            .decoded_pixel_comparison
            .clone()
            .expect("comparison plan exists");
        comparison.output_normalisation = None;
        let error = validate_decoded_pixel_comparison_plan(&suite, &comparison, &catalogue.packs)
            .expect_err("derived-set plans require normalisation");
        assert!(
            error
                .to_string()
                .contains("lacks decoded-output normalisation")
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
