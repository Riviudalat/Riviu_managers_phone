//! Declarative compatibility packs. Loading a pack never dispatches device work.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetSpec {
    pub id: String,
    pub screen: String,
    pub labels: Vec<String>,
    pub resource_suffixes: Vec<String>,
    pub anchors: Vec<String>,
    pub excluded_labels: Vec<String>,
    pub effectful: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompatibilityPack {
    pub schema_version: u32,
    pub engine_api_version: u32,
    pub id: String,
    pub revision: u64,
    pub packages: Vec<String>,
    pub targets: Vec<TargetSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixtures: Vec<ProfileFixture>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileFixture {
    pub target: String,
    pub package: String,
    pub width: u32,
    pub height: u32,
    pub xml: String,
    pub expected_count: usize,
}
impl CompatibilityPack {
    pub fn parse(bytes: &[u8]) -> anyhow::Result<Self> {
        anyhow::ensure!(bytes.len() <= 1024 * 1024, "profile_too_large");
        let pack: Self = serde_json::from_slice(bytes)?;
        anyhow::ensure!(
            pack.schema_version == 1 && pack.engine_api_version == 1 && pack.revision > 0,
            "profile_version_incompatible"
        );
        anyhow::ensure!(
            !pack.id.is_empty() && !pack.packages.is_empty() && pack.targets.len() <= 128,
            "profile_invalid"
        );
        let mut ids = std::collections::HashSet::new();
        for target in &pack.targets {
            anyhow::ensure!(
                ids.insert(&target.id) && !target.id.is_empty() && !target.screen.is_empty(),
                "profile_duplicate_target"
            );
            anyhow::ensure!(
                !target.labels.is_empty() || !target.resource_suffixes.is_empty(),
                "profile_target_empty"
            );
            for text in target
                .labels
                .iter()
                .chain(&target.resource_suffixes)
                .chain(&target.anchors)
                .chain(&target.excluded_labels)
            {
                anyhow::ensure!(
                    !text.trim().is_empty() && text.len() <= 256,
                    "profile_label_invalid"
                );
            }
        }
        Ok(pack)
    }
    pub fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }
    pub fn target(&self, id: &str) -> Option<&TargetSpec> {
        self.targets.iter().find(|t| t.id == id)
    }
    pub fn verify_fixtures(&self) -> anyhow::Result<usize> {
        anyhow::ensure!(
            !self.fixtures.is_empty() && self.fixtures.len() <= 256,
            "profile_fixtures_missing"
        );
        for fixture in &self.fixtures {
            anyhow::ensure!(
                self.packages.contains(&fixture.package),
                "profile_fixture_package"
            );
            let target = self
                .target(&fixture.target)
                .ok_or_else(|| anyhow::anyhow!("profile_fixture_target"))?;
            let tree = super::tree::Tree::parse(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: fixture.xml.clone(),
            })?;
            let app = super::AppContext {
                package: fixture.package.clone(),
                version: String::new(),
                system_locale: String::new(),
                observed_language: None,
                width: fixture.width,
                height: fixture.height,
            };
            let found = super::resolver::resolve(&tree, &app, target).len();
            anyhow::ensure!(
                found == fixture.expected_count,
                "profile_fixture_failed: {} expected {} found {found}",
                fixture.target,
                fixture.expected_count
            );
        }
        Ok(self.fixtures.len())
    }
}
