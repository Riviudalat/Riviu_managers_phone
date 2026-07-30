use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{
    ArtifactInventory, AuthRequirement, BaselineDiff, BaselineSource, HttpMethod,
    RouteEvidenceStatus, SessionRequirement,
};
use crate::{archive, baseline, codesign, dwarf, macho, redact, routes, SCHEMA_VERSION};

const WDA_BASELINE_VERSION: &str = "15.1.4";
const WDA_BASELINE_GIT_HEAD: &str = "20b705f8f96dee2939c022de6352720a311adb71";
const WDA_BASELINE_PACKAGE: &str = "appium-webdriveragent";
const WDA_BASELINE_INTEGRITY: &str =
    "sha512-1tPVzIVPsBKynbTFqJyk3Hrf/FZ6kDmeP81P24hJ6q3gYHd2ljsI6OYEhINSbzxDdDmgTuWyYoUa1YtFvZC8oA==";

pub fn inventory(ipa: &Path) -> Result<ArtifactInventory> {
    let archive = archive::read_ipa(ipa)?;
    let mut machos = archive::macho_candidates(&archive)
        .into_iter()
        .map(|(path, bytes)| macho::inspect(path, bytes))
        .collect::<Result<Vec<_>>>()?;
    machos.sort_by(|left, right| left.path.cmp(&right.path));

    let mut dwarf = archive::macho_candidates(&archive)
        .into_iter()
        .filter(|(path, _)| path.contains("/Contents/Resources/DWARF/"))
        .map(|(path, bytes)| dwarf::inspect(path, bytes))
        .collect::<Result<Vec<_>>>()?;
    dwarf.sort_by(|left, right| left.path.cmp(&right.path));

    let mut provisioning_entitlements = BTreeMap::new();
    for (_, bytes) in archive::mobileprovision_candidates(&archive) {
        for (key, value) in codesign::profile_entitlements(bytes)? {
            if provisioning_entitlements.insert(key, value).is_some() {
                bail!("multiple provisioning profiles define the same entitlement");
            }
        }
    }

    let mut inventory = ArtifactInventory {
        schema_version: SCHEMA_VERSION,
        artifact: archive.artifact,
        entries: archive.entries,
        bundles: archive.bundles,
        machos,
        dwarf,
        provisioning_entitlements,
        redaction_count: 0,
    };
    inventory.redaction_count = count_redaction_markers(&serde_json::to_value(&inventory)?);
    Ok(inventory)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).context("serialize deterministic JSON")?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

pub fn write_text(path: &Path, value: &str) -> Result<()> {
    let mut bytes = value.trim_end_matches(['\r', '\n']).as_bytes().to_vec();
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

pub fn verify_baseline(lock_path: &Path, archive_path: &Path) -> Result<()> {
    let lock = baseline::read_lock(lock_path)?;
    let metadata = archive_path
        .metadata()
        .with_context(|| format!("read baseline archive metadata: {}", archive_path.display()))?;
    if metadata.len() > 1024 * 1024 * 1024 {
        bail!("baseline archive exceeds 1 GiB size limit");
    }
    let bytes = fs::read(archive_path)
        .with_context(|| format!("read baseline archive: {}", archive_path.display()))?;
    baseline::verify_integrity(&bytes, &lock.integrity)
}

pub fn baseline_diff(
    inventory_path: &Path,
    source_path: &Path,
    lock_path: &Path,
    archive_path: &Path,
) -> Result<BaselineDiff> {
    let inventory_bytes = fs::read(inventory_path)
        .with_context(|| format!("read artifact inventory: {}", inventory_path.display()))?;
    let inventory: ArtifactInventory =
        serde_json::from_slice(&inventory_bytes).context("parse artifact inventory JSON")?;
    if inventory.schema_version != SCHEMA_VERSION {
        bail!("unsupported artifact inventory schema version");
    }
    let lock = baseline::read_lock(lock_path)?;
    let archive_bytes = read_baseline_archive(archive_path)?;
    baseline::verify_integrity(&archive_bytes, &lock.integrity)?;
    let verified_source = baseline::verify_source_archive(source_path, &archive_bytes)?;
    let oracle = oracle_source(&inventory);
    Ok(baseline::compare_sources(
        &lock,
        &verified_source.source,
        &oracle,
        &sha256_bytes(&archive_bytes),
        &sha256_bytes(&inventory_bytes),
        &verified_source.sha256,
    ))
}

pub struct GateAInputs<'a> {
    pub ipa: &'a Path,
    pub inventory: &'a Path,
    pub baseline: &'a Path,
    pub routes: &'a Path,
    pub manifest: &'a Path,
    pub baseline_source: &'a Path,
    pub baseline_archive: &'a Path,
    pub baseline_lock: &'a Path,
}

pub fn gate_a(inputs: GateAInputs<'_>) -> Result<(bool, String)> {
    let reported_inventory: ArtifactInventory = read_json(inputs.inventory, "artifact inventory")?;
    let inventory = inventory(inputs.ipa)?;
    let baseline: BaselineDiff = read_json(inputs.baseline, "baseline delta")?;
    let verified_baseline = baseline_diff(
        inputs.inventory,
        inputs.baseline_source,
        inputs.baseline_lock,
        inputs.baseline_archive,
    )?;
    let manifest: serde_json::Value = read_json(inputs.manifest, "agent manifest")?;
    let manifest_hash = manifest
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .context("agent manifest has no string sha256")?;

    let mut checks = Vec::new();
    checks.push((
        "IPA inventory evidence chain",
        reported_inventory == inventory,
        format!(
            "recomputed {} entries and {} Mach-O images from artifact bytes",
            inventory.entries.len(),
            inventory.machos.len()
        ),
    ));
    checks.push((
        "Artifact hash",
        inventory
            .artifact
            .sha256
            .eq_ignore_ascii_case(manifest_hash),
        format!("{} bytes", inventory.artifact.size),
    ));

    let (dsyms, runtime_images): (Vec<_>, Vec<_>) = inventory
        .machos
        .iter()
        .partition(|image| image.path.contains("/Contents/Resources/DWARF/"));
    let macho_ok = inventory.machos.len() == 4
        && dsyms.len() == 1
        && runtime_images.len() == 3
        && inventory
            .machos
            .iter()
            .all(|image| image.architecture == "aarch64")
        && runtime_images.iter().all(|image| image.crypt_id.is_some())
        && dsyms.iter().all(|image| image.crypt_id.is_none());
    checks.push((
        "Mach-O inventory",
        macho_ok,
        format!(
            "{} images; {} runtime; {} dSYM",
            inventory.machos.len(),
            runtime_images.len(),
            dsyms.len()
        ),
    ));

    let signing_ok = !inventory.provisioning_entitlements.is_empty()
        && runtime_images
            .iter()
            .any(|image| !image.entitlements.is_empty())
        && !contains_sensitive_key(&serde_json::to_value(&inventory.provisioning_entitlements)?)
        && runtime_images.iter().all(|image| {
            serde_json::to_value(&image.entitlements)
                .map(|value| !contains_sensitive_key(&value))
                .unwrap_or(false)
        });
    checks.push((
        "Signing metadata",
        signing_ok,
        format!(
            "{} provisioning entitlements; {} signed runtime images",
            inventory.provisioning_entitlements.len(),
            runtime_images
                .iter()
                .filter(|image| !image.entitlements.is_empty())
                .count()
        ),
    ));

    let dwarf_ok = inventory.dwarf.iter().any(|image| {
        image.compile_units > 0
            && image.subprograms > 0
            && image.line_sequences > 0
            && image.line_rows > 0
            && image
                .functions
                .iter()
                .any(|function| !function.ranges.is_empty())
    });
    let compile_units = inventory
        .dwarf
        .iter()
        .map(|image| image.compile_units)
        .sum::<usize>();
    let subprograms = inventory
        .dwarf
        .iter()
        .map(|image| image.subprograms)
        .sum::<usize>();
    let line_rows = inventory
        .dwarf
        .iter()
        .map(|image| image.line_rows)
        .sum::<usize>();
    checks.push((
        "DWARF evidence",
        dwarf_ok,
        format!("{compile_units} compile units; {subprograms} subprograms; {line_rows} line rows"),
    ));

    let baseline_chain_ok = baseline == verified_baseline;
    checks.push((
        "Baseline evidence chain",
        baseline_chain_ok,
        format!(
            "npm integrity verified; archive SHA-256 {}; source {}; inventory {}",
            verified_baseline.archive_sha256,
            verified_baseline.source_sha256,
            verified_baseline.inventory_sha256
        ),
    ));

    let framework_version = inventory
        .bundles
        .iter()
        .find(|bundle| bundle.bundle_id.as_deref() == Some("com.facebook.WebDriverAgentLib"))
        .and_then(|bundle| bundle.short_version.as_deref());
    let baseline_ok = verified_baseline.schema_version == SCHEMA_VERSION
        && verified_baseline.package == WDA_BASELINE_PACKAGE
        && verified_baseline.package_version == WDA_BASELINE_VERSION
        && verified_baseline.git_head == WDA_BASELINE_GIT_HEAD
        && verified_baseline.integrity == WDA_BASELINE_INTEGRITY
        && framework_version == Some(verified_baseline.package_version.as_str());
    checks.push((
        "WDA baseline",
        baseline_ok,
        format!(
            "framework {}; baseline {}",
            framework_version.unwrap_or("missing"),
            verified_baseline.package_version
        ),
    ));

    let exported_symbols = inventory
        .machos
        .iter()
        .map(|image| image.exported_symbols.len())
        .sum::<usize>();
    let delta_ok = exported_symbols > 0
        && baseline::provenance_complete(&verified_baseline.baseline_source)
        && baseline::provenance_complete(&verified_baseline.oracle_source)
        && (!verified_baseline.class_oracle_only.is_empty()
            || !verified_baseline.method_oracle_only.is_empty()
            || !verified_baseline.route_oracle_only.is_empty());
    checks.push((
        "Static delta evidence",
        delta_ok,
        format!(
            "{exported_symbols} exported symbols; oracle/baseline-only: {}/{} classes, {}/{} methods, {}/{} routes; complete source/image provenance",
            verified_baseline.class_oracle_only.len(),
            verified_baseline.class_baseline_only.len(),
            verified_baseline.method_oracle_only.len(),
            verified_baseline.method_baseline_only.len(),
            verified_baseline.route_oracle_only.len(),
            verified_baseline.route_baseline_only.len()
        ),
    ));

    let contract = routes::read_contract(inputs.routes);
    let (contract, route_evidence) = match contract {
        Ok(contract) => {
            let oracle_routes = inventory
                .machos
                .iter()
                .flat_map(|image| image.route_candidates.iter().cloned())
                .collect::<Vec<_>>();
            let baseline_routes = verified_baseline
                .baseline_source
                .route_candidates
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let evidence = routes::cross_check(&contract, oracle_routes, baseline_routes)?;
            (Some(contract), evidence)
        }
        Err(_) => (None, Vec::new()),
    };
    let path_confirmed_routes = route_evidence
        .iter()
        .filter(|route| {
            route.method.is_some() && route.status == RouteEvidenceStatus::PathConfirmed
        })
        .count();
    let contract_routes = contract.as_ref().map_or(0, |value| value.routes.len());
    let route_ok = contract_routes > 0 && path_confirmed_routes == contract_routes;
    let oracle_only_routes = route_evidence
        .iter()
        .filter(|route| route.status == RouteEvidenceStatus::OracleOnly)
        .count();
    let baseline_only_routes = route_evidence
        .iter()
        .filter(|route| route.status == RouteEvidenceStatus::BaselineOnly)
        .count();
    let shared_undocumented_routes = route_evidence
        .iter()
        .filter(|route| {
            route.method.is_none() && route.status == RouteEvidenceStatus::PathConfirmed
        })
        .count();
    checks.push((
        "Route path inventory",
        route_ok,
        format!(
            "{contract_routes} contract routes; {path_confirmed_routes} paths confirmed; {oracle_only_routes} oracle-only, {baseline_only_routes} baseline-only, {shared_undocumented_routes} shared undocumented candidates; method/auth/session/body remain contract assertions"
        ),
    ));

    let redaction_ok = verify_redaction(&[
        inputs.inventory.to_path_buf(),
        inputs.baseline.to_path_buf(),
    ])
    .is_ok();
    checks.push((
        "Report redaction",
        redaction_ok,
        "inventory and baseline reports scanned".into(),
    ));

    let passed = checks.iter().all(|(_, passed, _)| *passed);
    let mut markdown = String::from("# Gate A - RT-MMO Forensic Inventory\n\n");
    for (name, passed, evidence) in &checks {
        markdown.push_str(&format!(
            "- [{}] **{}**: {}\n",
            if *passed { "x" } else { " " },
            name,
            evidence
        ));
    }
    if let Some(contract) = &contract {
        markdown.push_str("\n## Contract Route Path Evidence\n\n");
        markdown.push_str(
            "| Method | Path | Auth | Session | Body | Status | Path evidence | Contract source |\n",
        );
        markdown.push_str("|---|---|---|---|---|---|---|---|\n");
        for route in &contract.routes {
            let normalized = routes::normalize_path(&route.path);
            let evidence = route_evidence.iter().find(|candidate| {
                candidate.method == Some(route.method) && candidate.path == normalized
            });
            let (status, source, contract_source) =
                evidence.map_or(("missing", "none", "none"), |value| {
                    (
                        route_status(value.status),
                        value.evidence.as_str(),
                        value.contract_evidence.as_deref().unwrap_or("none"),
                    )
                });
            markdown.push_str(&format!(
                "| {} | `{}` | {} | {} | {} | {} | {} | {} |\n",
                http_method(route.method),
                normalized,
                auth_requirement(route.auth),
                session_requirement(route.session),
                request_body(route.request_body.as_ref()),
                status,
                source,
                contract_source
            ));
        }
        let additional = route_evidence
            .iter()
            .filter(|candidate| candidate.method.is_none())
            .collect::<Vec<_>>();
        if !additional.is_empty() {
            markdown.push_str("\n## Additional Route Path Evidence\n\n");
            markdown.push_str("| Path | Status | Evidence |\n");
            markdown.push_str("|---|---|---|\n");
            for route in additional {
                markdown.push_str(&format!(
                    "| `{}` | {} | {} |\n",
                    route.path,
                    route_status(route.status),
                    route.evidence
                ));
            }
        }
    }
    markdown.push_str(
        "\n## Evidence Boundary\n\nThe runtime images are stripped and the bundled dSYM exposes only surviving runner symbols. Gate A records exported symbols, DWARF ranges/line tables, filtered Objective-C metadata, route paths, typed contract assertions, and provenance. Path-confirmed does not prove the declared HTTP method, auth, session, or body semantics, and Gate A does not claim a recovered feature call graph. Project 2 must add a contract or probe before implementing any feature-specific delta.\n",
    );
    markdown.push_str(&format!(
        "\nDecision: {}\n",
        if passed { "PASS" } else { "BLOCKED" }
    ));
    if redact::all(&markdown).1 != 0 {
        bail!("Gate A Markdown failed redaction before publication");
    }
    Ok((passed, markdown))
}

fn read_baseline_archive(path: &Path) -> Result<Vec<u8>> {
    let metadata = path
        .metadata()
        .with_context(|| format!("read baseline archive metadata: {}", path.display()))?;
    if metadata.len() > 1024 * 1024 * 1024 {
        bail!("baseline archive exceeds 1 GiB size limit");
    }
    fs::read(path).with_context(|| format!("read baseline archive: {}", path.display()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn http_method(value: HttpMethod) -> &'static str {
    match value {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Delete => "DELETE",
    }
}

fn auth_requirement(value: AuthRequirement) -> &'static str {
    match value {
        AuthRequirement::Exempt => "exempt",
        AuthRequirement::Protected => "protected",
    }
}

fn session_requirement(value: SessionRequirement) -> &'static str {
    match value {
        SessionRequirement::None => "none",
        SessionRequirement::Required => "required",
    }
}

fn route_status(value: RouteEvidenceStatus) -> &'static str {
    match value {
        RouteEvidenceStatus::PathConfirmed => "path-confirmed",
        RouteEvidenceStatus::DocumentedOnly => "documented-only",
        RouteEvidenceStatus::BaselineOnly => "baseline-only",
        RouteEvidenceStatus::OracleOnly => "oracle-only",
    }
}

fn request_body(value: Option<&crate::model::RequestBodyContract>) -> String {
    value.map_or_else(
        || "none".into(),
        |body| format!("required: `{}`", body.required.join("`, `")),
    )
}

pub fn verify_redaction(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        let bytes = fs::read(path).with_context(|| {
            format!("read report for redaction verification: {}", path.display())
        })?;
        let text = std::str::from_utf8(&bytes)
            .with_context(|| format!("report is not UTF-8: {}", path.display()))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("report");
        let is_json = path.extension().and_then(|value| value.to_str()) == Some("json");
        let raw_text = if is_json {
            // The pinned upstream commit is typed provenance. Decoded JSON scanning below
            // only exempts it when it is the exact value of the gitHead field.
            text.replace(WDA_BASELINE_GIT_HEAD, "<allowed-git-head>")
        } else {
            text.to_owned()
        };
        if redact::all(&raw_text).1 != 0 {
            bail!("redaction verification failed in raw report bytes: {name}");
        }
        let decoded_leak_count = if is_json {
            scan_json_strings(text).with_context(|| format!("validate report JSON: {name}"))?
        } else {
            0
        };
        if decoded_leak_count != 0 {
            bail!("redaction verification failed: {name}");
        }
    }
    Ok(())
}

fn scan_json_strings(text: &str) -> Result<usize> {
    let mut leak_count = 0;
    let mut deserializer = serde_json::Deserializer::from_str(text);
    JsonScanSeed {
        leak_count: &mut leak_count,
        allow_git_head: false,
    }
    .deserialize(&mut deserializer)
    .context("parse JSON without duplicate keys")?;
    deserializer.end().context("parse trailing JSON content")?;
    Ok(leak_count)
}

struct JsonScanSeed<'a> {
    leak_count: &'a mut usize,
    allow_git_head: bool,
}

impl<'de> DeserializeSeed<'de> for JsonScanSeed<'_> {
    type Value = ();

    fn deserialize<Deserializer>(
        self,
        deserializer: Deserializer,
    ) -> Result<(), Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonScanVisitor {
            leak_count: self.leak_count,
            allow_git_head: self.allow_git_head,
        })
    }
}

struct JsonScanVisitor<'a> {
    leak_count: &'a mut usize,
    allow_git_head: bool,
}

impl JsonScanVisitor<'_> {
    fn scan(&mut self, value: &str) {
        if !(self.allow_git_head && value == WDA_BASELINE_GIT_HEAD) {
            *self.leak_count += redact::all(value).1;
        }
    }
}

impl<'de> Visitor<'de> for JsonScanVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<Error>(self, _: bool) -> Result<(), Error> {
        Ok(())
    }

    fn visit_i64<Error>(self, _: i64) -> Result<(), Error> {
        Ok(())
    }

    fn visit_u64<Error>(self, _: u64) -> Result<(), Error> {
        Ok(())
    }

    fn visit_f64<Error>(self, _: f64) -> Result<(), Error> {
        Ok(())
    }

    fn visit_unit<Error>(self) -> Result<(), Error> {
        Ok(())
    }

    fn visit_none<Error>(self) -> Result<(), Error> {
        Ok(())
    }

    fn visit_some<Deserializer>(self, deserializer: Deserializer) -> Result<(), Deserializer::Error>
    where
        Deserializer: serde::Deserializer<'de>,
    {
        JsonScanSeed {
            leak_count: self.leak_count,
            allow_git_head: self.allow_git_head,
        }
        .deserialize(deserializer)
    }

    fn visit_str<Error>(mut self, value: &str) -> Result<(), Error> {
        self.scan(value);
        Ok(())
    }

    fn visit_string<Error>(mut self, value: String) -> Result<(), Error> {
        self.scan(&value);
        Ok(())
    }

    fn visit_seq<Access>(self, mut sequence: Access) -> Result<(), Access::Error>
    where
        Access: SeqAccess<'de>,
    {
        while sequence
            .next_element_seed(JsonScanSeed {
                leak_count: &mut *self.leak_count,
                allow_git_head: false,
            })?
            .is_some()
        {}
        Ok(())
    }

    fn visit_map<Access>(self, mut map: Access) -> Result<(), Access::Error>
    where
        Access: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
            }
            *self.leak_count += redact::all(&key).1;
            let allow_git_head = key == "gitHead";
            map.next_value_seed(JsonScanSeed {
                leak_count: &mut *self.leak_count,
                allow_git_head,
            })?;
        }
        Ok(())
    }
}

fn count_redaction_markers(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::String(value) => {
            value.matches("<home>").count()
                + value.matches("<redacted-agent-token>").count()
                + value.matches("<redacted-device-id>").count()
        }
        serde_json::Value::Array(values) => values.iter().map(count_redaction_markers).sum(),
        serde_json::Value::Object(values) => values.values().map(count_redaction_markers).sum(),
        _ => 0,
    }
}

fn oracle_source(inventory: &ArtifactInventory) -> BaselineSource {
    let mut source = BaselineSource::default();
    for image in &inventory.machos {
        for value in &image.objc_classes {
            insert_provenance(
                &mut source.objc_classes,
                &mut source.class_provenance,
                value,
                &image.path,
            );
        }
        for value in &image.objc_methods {
            insert_provenance(
                &mut source.objc_methods,
                &mut source.method_provenance,
                value,
                &image.path,
            );
        }
        for value in &image.route_candidates {
            insert_provenance(
                &mut source.route_candidates,
                &mut source.route_provenance,
                value,
                &image.path,
            );
        }
    }
    source
}

fn insert_provenance(
    values: &mut BTreeSet<String>,
    provenance: &mut BTreeMap<String, Vec<String>>,
    value: &str,
    path: &str,
) {
    values.insert(value.to_owned());
    let paths = provenance.entry(value.to_owned()).or_default();
    if !paths.iter().any(|existing| existing == path) {
        paths.push(path.to_owned());
        paths.sort();
    }
}

fn read_json<T: DeserializeOwned>(path: &Path, label: &str) -> Result<T> {
    let bytes = fs::read(path).with_context(|| format!("read {label}: {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {label} JSON"))
}

fn contains_sensitive_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(values) => values.iter().any(|(key, value)| {
            let key = key
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            key.contains("password")
                || key.contains("certificate")
                || key.contains("provisioneddevices")
                || key.contains("udid")
                || contains_sensitive_key(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_sensitive_key),
        _ => false,
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("create report directory: {}", parent.display()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("report output must end in a UTF-8 filename")?;
    let temporary = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("create temporary report: {}", temporary.display()))?;
        file.write_all(bytes).context("write temporary report")?;
        file.sync_all().context("flush temporary report")?;
        replace_file(&temporary, path).context("atomically publish report")
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).context("rename temporary report")
}
