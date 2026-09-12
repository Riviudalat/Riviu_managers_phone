//! Supervised perception sidecar. This module has no device-effect entrypoints.
use anyhow::Context;
use riviu_core::{
    db::Database,
    ui_automation::{profile::CompatibilityPack, GuiReasoner, GuiRequest, GuiResponse},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, Command},
    sync::{Mutex, Semaphore},
};

const KEY: &str = "gui_service.config.v1";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct GuiConfig {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub max_requests: u32,
}
impl Default for GuiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: String::new(),
            model: String::new(),
            max_requests: 20,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuiStatus {
    pub config: GuiConfig,
    pub running: bool,
    pub provider_ready: bool,
    pub protocol_version: u32,
    pub last_error: Option<String>,
}

struct Running {
    child: Child,
    _stdin: ChildStdin,
    url: String,
    token: String,
    fingerprint: String,
}
pub struct GuiService {
    db: Arc<Database>,
    resource_dir: Option<PathBuf>,
    data_dir: PathBuf,
    running: Mutex<Option<Running>>,
    starts: Mutex<Vec<Instant>>,
    capacity: Semaphore,
    last_error: parking_lot::Mutex<Option<String>>,
    trace_lock: parking_lot::Mutex<()>,
}
impl GuiService {
    pub fn new(db: Arc<Database>, resource_dir: Option<PathBuf>, data_dir: PathBuf) -> Self {
        Self {
            db,
            resource_dir,
            data_dir,
            running: Mutex::new(None),
            starts: Mutex::new(Vec::new()),
            capacity: Semaphore::new(2),
            last_error: parking_lot::Mutex::new(None),
            trace_lock: parking_lot::Mutex::new(()),
        }
    }
    pub fn config(&self) -> anyhow::Result<GuiConfig> {
        self.db
            .get_setting(KEY)?
            .filter(|s| !s.is_empty())
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .unwrap_or_else(|| Ok(GuiConfig::default()))
    }
    pub async fn save(&self, config: GuiConfig) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=100).contains(&config.max_requests),
            "Ngân sách phải từ 1 đến 100 request"
        );
        if !config.base_url.is_empty() {
            validate_provider_url(&config.base_url)?;
        }
        self.db.set_setting(KEY, &serde_json::to_string(&config)?)?;
        self.stop().await;
        Ok(())
    }
    pub async fn status(&self) -> anyhow::Result<GuiStatus> {
        let config = self.config()?;
        let settings = self.db.get_nurture_settings()?;
        let mut state = self.running.lock().await;
        let running = state
            .as_mut()
            .is_some_and(|s| matches!(s.child.try_wait(), Ok(None)));
        Ok(GuiStatus {
            config,
            running,
            provider_ready: !settings.api_key.is_empty(),
            protocol_version: 1,
            last_error: self.last_error.lock().clone(),
        })
    }
    pub async fn stop(&self) {
        if let Some(mut child) = self.running.lock().await.take() {
            let _ = child.child.kill().await;
            let _ = child.child.wait().await;
        }
    }
    fn executable(&self) -> anyhow::Result<(PathBuf, Vec<String>)> {
        if let Some(resource) = &self.resource_dir {
            let filename = if cfg!(windows) {
                "riviu-gui-service.exe"
            } else {
                "riviu-gui-service"
            };
            let path = resource.join("sidecars/gui-service").join(filename);
            if path.is_file() {
                verify_runtime(path.parent().context("runtime parent")?)?;
                return Ok((path, Vec::new()));
            }
        }
        if cfg!(debug_assertions) {
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../sidecars/gui-service");
            let python = source.join(".venv/Scripts/python.exe");
            if python.is_file() {
                return Ok((
                    python,
                    vec![source.join("serve.py").to_string_lossy().into_owned()],
                ));
            }
        }
        anyhow::bail!("gui_service_missing: thiếu dịch vụ nhận diện trong bộ cài")
    }
    async fn connection(&self) -> anyhow::Result<(String, String)> {
        let config = self.config()?;
        anyhow::ensure!(config.enabled, "gui_disabled: nhận diện AI đang tắt");
        let settings = self.db.get_nurture_settings()?;
        let base = if config.base_url.is_empty() {
            settings.base_url
        } else {
            config.base_url
        };
        let model = if config.model.is_empty() {
            settings.model
        } else {
            config.model
        };
        validate_provider_url(&base)?;
        anyhow::ensure!(
            !settings.api_key.is_empty() && !model.is_empty(),
            "gui_provider_unconfigured: cấu hình khóa và model AI trước"
        );
        let provider =
            serde_json::json!({"base_url":base,"model":model,"api_key":settings.api_key});
        let fingerprint = CompatibilityPack::sha256(serde_json::to_string(&provider)?.as_bytes());
        let mut state = self.running.lock().await;
        if let Some(running) = state.as_mut() {
            if running.child.try_wait()?.is_none() && running.fingerprint == fingerprint {
                return Ok((running.url.clone(), running.token.clone()));
            }
        }
        if let Some(mut old) = state.take() {
            let _ = old.child.kill().await;
            let _ = old.child.wait().await;
        }
        {
            let mut starts = self.starts.lock().await;
            starts.retain(|t| t.elapsed() < Duration::from_secs(600));
            anyhow::ensure!(
                starts.len() < 3,
                "gui_restart_budget: dịch vụ lỗi lặp; kiểm tra chẩn đoán"
            );
            starts.push(Instant::now());
        }
        let (exe, args) = self.executable()?;
        std::fs::create_dir_all(&self.data_dir)?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.data_dir.join("service.log"))?;
        let mut cmd = Command::new(exe);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(log)
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x08000000);
        let mut child = cmd.spawn().context("khởi động dịch vụ GUI")?;
        let mut stdin = child.stdin.take().context("service stdin")?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        stdin
            .write_all(
                format!(
                    "{}\n",
                    serde_json::json!({"token":token,"provider":provider})
                )
                .as_bytes(),
            )
            .await?;
        stdin.flush().await?;
        let mut reader = BufReader::new(child.stdout.take().context("service stdout")?);
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut line)).await??;
        anyhow::ensure!(line.len() < 4096, "gui_handshake_invalid");
        let handshake: serde_json::Value = serde_json::from_str(&line)?;
        anyhow::ensure!(handshake["protocolVersion"] == 1, "gui_protocol_mismatch");
        let port = handshake["port"]
            .as_u64()
            .filter(|p| *p > 0 && *p <= 65535)
            .context("service port")?;
        let url = format!("http://127.0.0.1:{port}");
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(response) = http
                .get(format!("{url}/health/ready"))
                .bearer_auth(&token)
                .send()
                .await
            {
                if let Ok(ready) = response.json::<serde_json::Value>().await {
                    if ready["protocolVersion"] == 1 && ready["ready"] == true {
                        break;
                    }
                }
            }
            anyhow::ensure!(
                Instant::now() < deadline && child.try_wait()?.is_none(),
                "gui_startup_failed"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        *state = Some(Running {
            child,
            _stdin: stdin,
            url: url.clone(),
            token: token.clone(),
            fingerprint,
        });
        Ok((url, token))
    }
    pub fn import_pack(&self, bytes: &[u8]) -> anyhow::Result<String> {
        let pack = CompatibilityPack::parse(bytes)?;
        pack.verify_fixtures()?;
        let adapter = pack
            .packages
            .first()
            .and_then(|p| riviu_core::app_automation::adapter(p))
            .context("profile_app_unknown")?;
        let baseline = adapter.pack();
        anyhow::ensure!(
            pack.id == baseline.id && pack.packages == baseline.packages,
            "profile_identity_changed"
        );
        for target in &pack.targets {
            let original = baseline
                .target(&target.id)
                .context("profile_target_unknown")?;
            anyhow::ensure!(
                target.effectful == original.effectful && target.screen == original.screen,
                "profile_effect_contract_changed"
            );
            anyhow::ensure!(
                pack.fixtures
                    .iter()
                    .any(|f| f.target == target.id && f.expected_count == 1)
                    && pack
                        .fixtures
                        .iter()
                        .any(|f| f.target == target.id && f.expected_count != 1),
                "profile_positive_and_negative_required: {}",
                target.id
            );
        }
        anyhow::ensure!(
            pack.id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
            "profile_id_invalid"
        );
        let folder = self.data_dir.join("packs");
        std::fs::create_dir_all(&folder)?;
        let hash = CompatibilityPack::sha256(bytes);
        let path = folder.join(format!("{}-{}-{hash}.json", pack.id, pack.revision));
        std::fs::write(&path, bytes)?;
        let key = format!("gui.profile.{}", pack.id);
        if let Some(previous) = self.db.get_setting(&key)? {
            self.db.set_setting(&format!("{key}.previous"), &previous)?;
        }
        self.db
            .set_setting(&format!("gui.profile.{}", pack.id), &path.to_string_lossy())?;
        Ok(hash)
    }
}
fn validate_provider_url(raw: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(raw)?;
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "provider_url_credentials"
    );
    anyhow::ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "provider_url_requires_https"
    );
    Ok(())
}
#[async_trait::async_trait]
impl GuiReasoner for GuiService {
    fn compatibility_pack_for(
        &self,
        package: &str,
        scope: Option<&riviu_core::ui_automation::GuiScope>,
    ) -> Option<CompatibilityPack> {
        let scope = scope?;
        let key = format!(
            "gui.pinned.{}",
            CompatibilityPack::sha256(format!("{}:{package}", scope.run_id).as_bytes())
        );
        if let Some(raw) = self.db.get_setting(&key).ok().flatten() {
            return CompatibilityPack::parse(raw.as_bytes()).ok();
        }
        let pack = self
            .compatibility_pack(package)
            .or_else(|| riviu_core::app_automation::adapter(package).map(|a| a.pack()))?;
        self.db
            .set_setting(&key, &serde_json::to_string(&pack).ok()?)
            .ok()?;
        Some(pack)
    }
    async fn resolve(&self, request: GuiRequest) -> anyhow::Result<GuiResponse> {
        let config = self.config()?;
        anyhow::ensure!(config.enabled, "gui_disabled");
        anyhow::ensure!(
            !self.db.get_nurture_settings()?.api_key.is_empty(),
            "gui_provider_unconfigured"
        );
        let max = config.max_requests;
        self.db.claim_gui_request(&request, max)?;
        self.record(&request, None, "requested");
        let deadline = Duration::from_millis(request.remaining_ms.min(30_000));
        let result = tokio::time::timeout(deadline, async {
            let _permit = self.capacity.acquire().await?;
            let (url, token) = self.connection().await?;
            let response = reqwest::Client::builder()
                .timeout(deadline)
                .build()?
                .post(format!("{url}/v1/gui/resolve"))
                .bearer_auth(token)
                .json(&request)
                .send()
                .await?;
            anyhow::ensure!(
                response.status().is_success(),
                "gui_service_http_{}",
                response.status().as_u16()
            );
            let response: GuiResponse = response.json().await?;
            response.validate_binding(&request)?;
            Ok::<_, anyhow::Error>(response)
        })
        .await
        .map_err(|_| anyhow::anyhow!("gui_deadline"))
        .and_then(|result| result);
        if let Err(error) = &result {
            *self.last_error.lock() = Some(error.to_string());
            self.record(&request, None, &error.to_string());
        }
        let reason = result
            .as_ref()
            .err()
            .map(ToString::to_string)
            .unwrap_or_default();
        self.db
            .settle_gui_request(&request, result.as_ref().ok(), &reason)?;
        result
    }
    fn compatibility_pack(&self, package: &str) -> Option<CompatibilityPack> {
        let adapter = riviu_core::app_automation::adapter(package)?;
        let builtin = adapter.pack();
        let path = self
            .db
            .get_setting(&format!("gui.profile.{}", builtin.id))
            .ok()??;
        let path = PathBuf::from(path);
        let parent = self.data_dir.join("packs").canonicalize().ok()?;
        if !path.canonicalize().ok()?.starts_with(parent) {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        if !path
            .file_stem()?
            .to_str()?
            .ends_with(&CompatibilityPack::sha256(&bytes))
        {
            return None;
        }
        let pack = CompatibilityPack::parse(&bytes).ok()?;
        (pack.id == builtin.id && pack.packages.iter().any(|p| p == package)).then_some(pack)
    }
    fn record(&self, request: &GuiRequest, response: Option<&GuiResponse>, reason: &str) {
        use std::io::Write;
        let _guard = self.trace_lock.lock();
        let result = (|| -> anyhow::Result<()> {
            std::fs::create_dir_all(&self.data_dir)?;
            let pictures = self.data_dir.join("observations");
            std::fs::create_dir_all(&pictures)?;
            prune_images(&pictures, &self.db.gui_protected_images()?)?;
            if !request.screenshot.is_empty()
                && request.screenshot_sha256.len() == 64
                && request
                    .screenshot_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                use base64::Engine;
                let bytes =
                    base64::engine::general_purpose::STANDARD.decode(&request.screenshot)?;
                if CompatibilityPack::sha256(&bytes) == request.screenshot_sha256 {
                    std::fs::write(
                        pictures.join(format!("{}.png", request.screenshot_sha256)),
                        bytes,
                    )?;
                }
            }
            let mut request = request.clone();
            request.screenshot.clear();
            let trace = self.data_dir.join("trace.jsonl");
            if trace.metadata().is_ok_and(|m| m.len() > 8 * 1024 * 1024) {
                std::fs::copy(&trace, self.data_dir.join("trace.previous.jsonl"))?;
                std::fs::write(&trace, [])?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(trace)?;
            writeln!(
                file,
                "{}",
                serde_json::json!({"at":chrono::Utc::now(),"request":request,"response":response,"reason":reason})
            )?;
            Ok(())
        })();
        if result.is_err() {
            log::warn!("gui_trace_unavailable");
        }
    }
}

fn verify_runtime(root: &Path) -> anyhow::Result<()> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Manifest {
        protocol_version: u32,
        files: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        path: String,
        bytes: u64,
        sha256: String,
    }
    let manifest: Manifest =
        serde_json::from_slice(&std::fs::read(root.join("gui-service-manifest.json"))?)?;
    anyhow::ensure!(
        manifest.protocol_version == 1 && !manifest.files.is_empty(),
        "gui_manifest_invalid"
    );
    let root = root.canonicalize()?;
    for entry in manifest.files {
        let path = root.join(entry.path).canonicalize()?;
        anyhow::ensure!(path.starts_with(&root), "gui_manifest_path");
        let data = std::fs::read(path)?;
        anyhow::ensure!(
            data.len() as u64 == entry.bytes && CompatibilityPack::sha256(&data) == entry.sha256,
            "gui_resource_hash_mismatch"
        );
    }
    Ok(())
}

fn prune_images(
    directory: &Path,
    protected: &std::collections::HashSet<String>,
) -> anyhow::Result<()> {
    let root = directory.canonicalize()?;
    let mut entries = std::fs::read_dir(&root)?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let metadata = e.metadata().ok()?;
            if !metadata.is_file() || e.path().extension().is_none_or(|x| x != "png") {
                return None;
            }
            Some((e.path(), metadata.len(), metadata.modified().ok()?))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(_, _, time)| *time);
    let mut total: u64 = entries.iter().map(|(_, bytes, _)| bytes).sum();
    for (path, bytes, time) in entries {
        let hash = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if hash.len() != 64 || protected.contains(hash) {
            continue;
        }
        if total > 1024 * 1024 * 1024
            || time
                .elapsed()
                .is_ok_and(|age| age > Duration::from_secs(7 * 24 * 3600))
        {
            let actual = path.canonicalize()?;
            anyhow::ensure!(actual.starts_with(&root), "gui_cleanup_path");
            std::fs::remove_file(actual)?;
            total = total.saturating_sub(bytes);
        }
    }
    Ok(())
}

mod commands;
pub use commands::*;
#[cfg(test)]
mod tests;
