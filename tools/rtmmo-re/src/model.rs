use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileDigest {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BundleInfo {
    pub plist_path: String,
    pub bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub short_version: Option<String>,
    pub build_version: Option<String>,
    pub minimum_os_version: Option<String>,
    pub dt_xcode: Option<String>,
    pub dt_sdk_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MachOInfo {
    pub path: String,
    pub sha256: String,
    pub architecture: String,
    pub is_64: bool,
    pub little_endian: bool,
    pub uuid: Option<String>,
    pub crypt_id: Option<u32>,
    pub linked_dylibs: Vec<String>,
    pub sections: Vec<String>,
    pub symbol_count: usize,
    pub exported_symbols: Vec<String>,
    pub objc_classes: Vec<String>,
    pub objc_methods: Vec<String>,
    pub route_candidates: Vec<String>,
    pub entitlements: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct DwarfRange {
    pub begin: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct DwarfFunction {
    pub name: String,
    pub ranges: Vec<DwarfRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DwarfInfo {
    pub path: String,
    pub compile_units: usize,
    pub subprograms: usize,
    pub attribute_errors: usize,
    pub line_sequences: usize,
    pub line_rows: usize,
    pub line_files: Vec<String>,
    pub source_paths: Vec<String>,
    pub function_names: Vec<String>,
    pub functions: Vec<DwarfFunction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInventory {
    pub schema_version: u32,
    pub artifact: FileDigest,
    pub entries: Vec<FileDigest>,
    pub bundles: Vec<BundleInfo>,
    pub machos: Vec<MachOInfo>,
    pub dwarf: Vec<DwarfInfo>,
    pub provisioning_entitlements: BTreeMap<String, serde_json::Value>,
    pub redaction_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineLock {
    pub package: String,
    pub version: String,
    pub git_head: String,
    pub tarball: String,
    pub integrity: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineSource {
    pub objc_classes: BTreeSet<String>,
    pub objc_methods: BTreeSet<String>,
    pub route_candidates: BTreeSet<String>,
    pub class_provenance: BTreeMap<String, Vec<String>>,
    pub method_provenance: BTreeMap<String, Vec<String>>,
    pub route_provenance: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BaselineDiff {
    pub schema_version: u32,
    pub package: String,
    pub package_version: String,
    pub git_head: String,
    pub integrity: String,
    pub archive_sha256: String,
    pub inventory_sha256: String,
    pub source_sha256: String,
    pub baseline_source: BaselineSource,
    pub oracle_source: BaselineSource,
    pub class_overlap: Vec<String>,
    pub class_baseline_only: Vec<String>,
    pub class_oracle_only: Vec<String>,
    pub method_overlap: Vec<String>,
    pub method_baseline_only: Vec<String>,
    pub method_oracle_only: Vec<String>,
    pub route_overlap: Vec<String>,
    pub route_baseline_only: Vec<String>,
    pub route_oracle_only: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Delete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AuthRequirement {
    Exempt,
    Protected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionRequirement {
    None,
    Required,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JsonValueKind {
    Array,
    Boolean,
    Number,
    Object,
    String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RequestBodyContract {
    pub required: Vec<String>,
    pub properties: BTreeMap<String, JsonValueKind>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteContractEntry {
    pub method: HttpMethod,
    pub path: String,
    pub auth: AuthRequirement,
    pub session: SessionRequirement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<RequestBodyContract>,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteContract {
    pub schema_version: u32,
    pub routes: Vec<RouteContractEntry>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RouteEvidenceStatus {
    PathConfirmed,
    DocumentedOnly,
    BaselineOnly,
    OracleOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteEvidence {
    pub method: Option<HttpMethod>,
    pub path: String,
    pub status: RouteEvidenceStatus,
    pub evidence: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_evidence: Option<String>,
}
