use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogueManifest {
    pub schema_version: u32,
    pub name: String,
    pub description: String,
    pub manifest_root: String,
    pub suite_root: String,
    pub generated_root: String,
    pub cache_environment: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub review_state: ReviewState,
    pub kind: PackKind,
    pub codecs: Vec<String>,
    pub purposes: Vec<String>,
    #[serde(default)]
    pub standards: Vec<StandardReference>,
    pub license: LicenseRecord,
    pub rights: RightsRecord,
    #[serde(default)]
    pub source: Option<SourceRecord>,
    pub materialization: MaterializationRecord,
    #[serde(default)]
    pub asset_inventory: Option<String>,
    #[serde(default)]
    pub assets: Vec<AssetRecord>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetInventoryManifest {
    pub schema_version: u32,
    pub pack_id: String,
    pub pack_version: String,
    pub assets: Vec<AssetRecord>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewState {
    Planned,
    Reviewed,
    Locked,
    Withdrawn,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PackKind {
    Generated,
    External,
    Derived,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StandardReference {
    pub identifier: String,
    pub role: String,
    #[serde(default)]
    pub clauses: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseRecord {
    pub expression: String,
    pub name: String,
    #[serde(default)]
    pub local_files: Vec<String>,
    pub evidence_url: String,
    pub reviewed_on: String,
    pub notes: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Permitted,
    Prohibited,
    Conditional,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RightsRecord {
    pub access: Permission,
    pub redistribution: Permission,
    pub modification: Permission,
    pub commercial_use: Permission,
    pub publish_derivatives: Permission,
    pub publish_benchmarks: Permission,
    pub ml_training: Permission,
    pub weights_redistribution: Permission,
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRecord {
    pub landing_page: String,
    #[serde(default)]
    pub download_url: Option<String>,
    pub terms_url: String,
    #[serde(default)]
    pub upstream_revision: Option<String>,
    #[serde(default)]
    pub archive_filename: Option<String>,
    #[serde(default)]
    pub archive_sha256: Option<String>,
    pub requires_manual_acquisition: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializationRecord {
    pub directory: String,
    #[serde(default)]
    pub generator: Option<String>,
    #[serde(default)]
    pub expected_tree_sha256: Option<String>,
    pub layout: String,
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRecord {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub media_type: String,
    pub semantics: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuiteManifest {
    pub schema_version: u32,
    pub id: String,
    pub revision: u32,
    pub name: String,
    pub description: String,
    pub layer: u8,
    pub purposes: Vec<String>,
    pub gating: bool,
    pub missing_policy: MissingPolicy,
    pub packs: Vec<SuitePack>,
    #[serde(default)]
    pub inspection: Option<InspectionPlan>,
    #[serde(default)]
    pub decoded_pixel_comparison: Option<DecodedPixelComparisonPlan>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedPixelComparisonPlan {
    pub pack_id: String,
    pub standard: String,
    pub clauses: Vec<String>,
    pub retrieval_commit: String,
    pub cases: Vec<DecodedPixelComparisonCase>,
    #[serde(default)]
    pub choice_groups: Vec<DecodedPixelComparisonChoiceGroup>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedPixelComparisonCase {
    pub id: String,
    pub input: String,
    pub reference: String,
    pub component: u16,
    pub resolution_reduction: u8,
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u8,
    pub signed: bool,
    pub peak_error_limit: u64,
    pub mean_squared_error_limit: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedPixelComparisonChoiceGroup {
    pub id: String,
    pub input: String,
    pub minimum_passing_alternatives: u16,
    pub alternatives: Vec<DecodedPixelComparisonAlternative>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DecodedPixelComparisonAlternative {
    pub id: String,
    pub reference: String,
    pub component: u16,
    pub resolution_reduction: u8,
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u8,
    pub signed: bool,
    pub peak_error_limit: u64,
    pub mean_squared_error_limit: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionPlan {
    pub pack_id: String,
    pub extensions: Vec<String>,
    pub expected: InspectionExpectation,
    #[serde(default)]
    pub diagnostic_contains: Option<String>,
    pub classifications: Vec<InspectionClassification>,
    #[serde(default)]
    pub overrides: Vec<InspectionOverride>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionExpectation {
    Accept,
    Reject,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum InspectionFormat {
    J2k,
    Htj2k,
    Jp2,
    Jph,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionClassification {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub path_prefix: Option<String>,
    pub format: InspectionFormat,
    pub cohort: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionOverride {
    pub path: String,
    pub expected: InspectionExpectation,
    #[serde(default)]
    pub diagnostic_contains: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum MissingPolicy {
    Fail,
    Skip,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuitePack {
    pub id: String,
    pub version: String,
    pub required: bool,
}
