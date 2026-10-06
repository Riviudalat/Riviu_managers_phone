//! One bounded shared cache. Cached evidence never replaces a fresh creation guard.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) const METADATA_TTL: Duration = Duration::from_secs(300);
pub(super) const READINESS_TTL: Duration = Duration::from_secs(30);

pub(super) struct PreparationContext {
    pub request_id: String,
    pub preparation_id: String,
    pub fresh: bool,
    events: riviu_core::EventBus,
    started: Instant,
    revision: AtomicU64,
}
impl PreparationContext {
    pub fn new(
        events: riviu_core::EventBus,
        request_id: String,
        preparation_id: String,
        fresh: bool,
    ) -> Self {
        Self {
            events,
            request_id,
            preparation_id,
            fresh,
            started: Instant::now(),
            revision: AtomicU64::new(0),
        }
    }
}
tokio::task_local! { pub(super) static CONTEXT: PreparationContext; }

pub(super) async fn stage(db: &Arc<Database>, stage: &'static str) -> anyhow::Result<()> {
    let request = CONTEXT
        .try_with(|context| context.fresh.then(|| context.request_id.clone()))
        .ok()
        .flatten();
    if let Some(request) = request {
        db.storage_write(move |db| {
            db.advance_publish_start(&request, "preparing", "preparing", stage, None)
        })
        .await?;
    }
    Ok(())
}

pub(super) fn progress(
    udid: &str,
    stage: &str,
    state: &str,
    completed: u32,
    error: Option<String>,
) {
    let _ = CONTEXT.try_with(|context| {
        context
            .events
            .emit(riviu_core::AppEvent::PublishPreflightProgress {
                request_id: context.request_id.clone(),
                preparation_id: context.preparation_id.clone(),
                udid: udid.into(),
                stage: stage.into(),
                state: state.into(),
                completed_checks: completed,
                total_checks: 4,
                elapsed_ms: context.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                error,
                revision: context.revision.fetch_add(1, Ordering::Relaxed) + 1,
            })
    });
}

#[derive(Clone)]
pub(super) struct DeviceEvidence {
    pub key: String,
    pub build: (String, String, String),
    pub build_at: Instant,
    pub observation: super::preflight::PreflightDeviceObservation,
    pub readiness_at: Instant,
}
#[derive(Clone)]
struct ManifestEvidence {
    fingerprint: String,
    at: Instant,
    manifest: PublishFolderManifest,
}
#[derive(Clone)]
struct PreparedEvidence {
    request: riviu_core::PublishPreflightRequest,
    prepared: super::preflight::PreparedPublishPreflight,
    guard_fingerprint: String,
    generation: u64,
    at: Instant,
}
#[derive(Default)]
struct PreparationCache {
    db: std::sync::Weak<Database>,
    receiver: Option<tokio::sync::broadcast::Receiver<riviu_core::AppEvent>>,
    roster: HashMap<String, String>,
    devices: HashMap<String, DeviceEvidence>,
    manifests: HashMap<String, ManifestEvidence>,
    prepared: HashMap<String, PreparedEvidence>,
    generation: u64,
}

pub(super) async fn guard_fingerprint(
    db: &Arc<Database>,
    request: &riviu_core::PublishPreflightRequest,
) -> anyhow::Result<String> {
    let udids = request.udids.clone();
    db.storage_read(move |db| {
        let metas = db
            .list_device_metas()?
            .into_iter()
            .filter(|meta| udids.contains(&meta.udid))
            .collect::<Vec<_>>();
        let groups = db.list_groups()?;
        let mut bindings = Vec::with_capacity(udids.len());
        let mut holds = Vec::new();
        for udid in &udids {
            // Preflight is read-only now: a hold is part of the fingerprint instead of an
            // error, so releasing it after confirmation invalidates any cached preparation.
            holds.extend(
                db.publish_device_guard(udid)?
                    .blocking
                    .into_iter()
                    .map(|hold| (hold.assignment_id, hold.updated_at)),
            );
            bindings.push(db.device_app_binding(udid, "tiktok")?);
        }
        Ok(execution::frame_sha256(&serde_json::to_vec(
            &serde_json::json!({"metas":metas,"groups":groups,"bindings":bindings,"holds":holds}),
        )?))
    })
    .await
}

/// Only public configuration identity is retained. Tokens stay inside the existing
/// writer check; every invocation fences any reused proof against this binding.
pub(super) async fn sheet_binding(
    db: &Arc<Database>,
    enabled: bool,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !enabled {
        return Ok(None);
    }
    db.storage_read(|db| {
        Ok(Some(serde_json::json!({
            "provider": db.get_setting(riviu_core::db::SHEET_PROVIDER_SETTING)?,
            "connection": db.google_sheet_connection()?,
            "url": db.get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)?,
            "authorizationGeneration": db.get_setting("google.sheets.authorization-generation")?,
            "migration": db.get_setting(riviu_core::db::GOOGLE_MIGRATION_SETTING)?,
            "layout": db.get_setting(riviu_core::publish_sheet::INTERNAL_REPORTING_SETTING)?,
        })))
    })
    .await
}

pub(super) async fn remember_prepared(
    db: &Arc<Database>,
    request: riviu_core::PublishPreflightRequest,
    prepared: &super::preflight::PreparedPublishPreflight,
    expected_guard: &str,
) -> anyhow::Result<()> {
    if !prepared.report.can_execute {
        return Ok(());
    }
    let Some(id) = CONTEXT
        .try_with(|context| context.preparation_id.clone())
        .ok()
    else {
        return Ok(());
    };
    let guard_fingerprint = guard_fingerprint(db, &request).await?;
    if guard_fingerprint != expected_guard {
        return Ok(());
    }
    let mut cache = CACHE.lock();
    cache.drain();
    if cache.prepared.len() >= 16 {
        cache.prepared.clear();
    }
    let generation = cache.generation;
    cache.prepared.insert(
        id,
        PreparedEvidence {
            request,
            prepared: prepared.clone(),
            guard_fingerprint,
            generation,
            at: Instant::now(),
        },
    );
    Ok(())
}

pub(super) async fn reuse_prepared(
    control: &DeviceControlPlane,
    registry: &riviu_core::DeviceRegistry,
    db: &Arc<Database>,
    request: &riviu_core::PublishPreflightRequest,
    manifest: &PublishFolderManifest,
    sheet: &super::preflight::VerifiedSheetChoice,
) -> anyhow::Result<Option<super::preflight::PreparedPublishPreflight>> {
    use futures_util::stream::{self, StreamExt};
    let id = CONTEXT
        .try_with(|context| context.preparation_id.clone())
        .ok();
    let cached = {
        let mut cache = CACHE.lock();
        cache.drain();
        if cache
            .db
            .upgrade()
            .is_none_or(|prior| !Arc::ptr_eq(&prior, db))
        {
            return Ok(None);
        }
        let matches = |entry: &&PreparedEvidence| {
            entry.at.elapsed() < READINESS_TTL
                && entry.generation == cache.generation
                && entry.request == *request
        };
        id.as_ref()
            .and_then(|id| cache.prepared.get(id))
            .filter(matches)
            .cloned()
            .or_else(|| {
                cache
                    .prepared
                    .values()
                    .filter(matches)
                    .max_by_key(|entry| entry.at)
                    .cloned()
            })
    };
    let Some(cached) = cached else {
        return Ok(None);
    };
    // A refused or changed Sheet answer is passed to the full report builder.
    // Never return a cached canExecute=true over this request's writer refusal.
    if sheet.as_ref().ok() != Some(&cached.prepared.report.sheet_delivery) {
        return Ok(None);
    }
    let mut bundles = request
        .bundle_ids
        .iter()
        .map(|id| manifest.bundles.iter().find(|b| &b.id == id).cloned())
        .collect::<Option<Vec<_>>>();
    let Some(bundles) = bundles.as_mut() else {
        return Ok(None);
    };
    let overrides: HashMap<_, _> = request.caption_overrides.clone().into_iter().collect();
    execution::apply_caption_overrides(bundles, Some(&overrides))?;
    if *bundles != cached.prepared.bundles
        || guard_fingerprint(db, request).await? != cached.guard_fingerprint
    {
        return Ok(None);
    }
    let work = stream::iter(request.udids.clone().into_iter().map(|udid| async move {
        progress(&udid, "checkingDevices", "running", 0, None);
        anyhow::ensure!(
            registry.get(&udid).is_some(),
            "Máy không còn trong roster: {udid}"
        );
        anyhow::ensure!(
            control.current_work_owner(&udid).is_none(),
            "Máy đang có chủ mới: {udid}"
        );
        control.verify_automation_transport(&udid).await?;
        // Lock/readiness is critical and fresh; parsed package metadata and storage threshold
        // remain reusable for 30 seconds. The dispatcher rechecks again before transfer/Post.
        control.verify_automation_readiness(&udid).await?;
        progress(&udid, "checkingDevices", "passed", 4, None);
        Ok::<_, anyhow::Error>(())
    }))
    .buffer_unordered(request.udids.len().max(1));
    tokio::pin!(work);
    while let Some(result) = work.next().await {
        result?;
    }
    if guard_fingerprint(db, request).await? != cached.guard_fingerprint {
        return Ok(None);
    }
    {
        let mut cache = CACHE.lock();
        cache.drain();
        if cache.generation != cached.generation || cached.at.elapsed() >= READINESS_TTL {
            return Ok(None);
        }
    }
    let mut prepared = cached.prepared;
    for udid in &request.udids {
        anyhow::ensure!(
            control.current_work_owner(udid).is_none(),
            "Máy đang có chủ mới: {udid}"
        );
    }
    if guard_fingerprint(db, request).await? != cached.guard_fingerprint {
        return Ok(None);
    }
    {
        let mut cache = CACHE.lock();
        cache.drain();
        if cache.generation != cached.generation || cached.at.elapsed() >= READINESS_TTL {
            return Ok(None);
        }
    }
    // Keep the approved immutable digest, and only return after the critical guards above.
    prepared.report.can_execute = true;
    Ok(Some(prepared))
}
static CACHE: std::sync::LazyLock<parking_lot::Mutex<PreparationCache>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(PreparationCache::default()));

fn identity(device: &riviu_core::DeviceInfo) -> String {
    format!(
        "{:?}/{:?}/{:?}/{:?}",
        device.platform, device.connection, device.status, device.last_error
    )
}
impl PreparationCache {
    fn drain(&mut self) {
        while let Some(receiver) = &mut self.receiver {
            match receiver.try_recv() {
                Ok(riviu_core::AppEvent::DevicesUpdated { devices }) => {
                    let next: HashMap<_, _> = devices
                        .iter()
                        .map(|d| (d.udid.clone(), identity(d)))
                        .collect();
                    if next != self.roster {
                        self.devices
                            .retain(|udid, _| next.get(udid) == self.roster.get(udid));
                        self.roster = next;
                        self.generation += 1;
                    }
                }
                Ok(riviu_core::AppEvent::DeviceUpdated { device }) => {
                    let key = identity(&device);
                    if self.roster.get(&device.udid) != Some(&key) {
                        self.devices.remove(&device.udid);
                        self.roster.insert(device.udid, key);
                        self.generation += 1;
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                    self.devices.clear();
                    self.generation += 1;
                }
                Err(_) => break,
            }
        }
    }
}
pub(super) fn initialize(
    db: &Arc<Database>,
    events: &riviu_core::EventBus,
    registry: &riviu_core::DeviceRegistry,
) {
    let mut cache = CACHE.lock();
    if cache
        .db
        .upgrade()
        .is_none_or(|prior| !Arc::ptr_eq(&prior, db))
    {
        *cache = PreparationCache {
            db: Arc::downgrade(db),
            receiver: Some(events.subscribe()),
            roster: registry
                .list()
                .iter()
                .map(|d| (d.udid.clone(), identity(d)))
                .collect(),
            ..Default::default()
        };
    }
    cache.drain();
}
pub(super) fn device(db: &Arc<Database>, udid: &str, key: &str) -> (u64, Option<DeviceEvidence>) {
    let mut cache = CACHE.lock();
    cache.drain();
    if cache
        .db
        .upgrade()
        .is_none_or(|prior| !Arc::ptr_eq(&prior, db))
    {
        return (cache.generation, None);
    }
    let result = cache
        .devices
        .get(udid)
        .filter(|entry| entry.key == key && entry.build_at.elapsed() < METADATA_TTL)
        .cloned();
    (cache.generation, result)
}
pub(super) fn remember_device(
    db: &Arc<Database>,
    udid: &str,
    generation: u64,
    evidence: DeviceEvidence,
) {
    let mut cache = CACHE.lock();
    cache.drain();
    if cache.generation == generation
        && cache
            .db
            .upgrade()
            .is_some_and(|prior| Arc::ptr_eq(&prior, db))
    {
        if cache.devices.len() >= 500 {
            cache.devices.clear();
        }
        cache.devices.insert(udid.into(), evidence);
    }
}

/// Hash the same shallow source tree before reusing parsed/decoded manifests. Timestamps alone
/// cannot prove that a caption, workbook, or image stayed unchanged.
fn source_fingerprint(root: &str) -> anyhow::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let root = Path::new(root);
    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        paths.push(path.clone());
        if path.is_dir() {
            for child in fs::read_dir(&path)? {
                paths.push(child?.path());
            }
        }
    }
    paths.sort();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    for path in paths {
        let name = path.strip_prefix(root)?.to_string_lossy();
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        let metadata = fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "Nguồn không được chứa symlink"
        );
        digest.update([u8::from(metadata.is_file())]);
        if metadata.is_file() {
            digest.update(metadata.len().to_le_bytes());
            let mut file = fs::File::open(&path)?;
            loop {
                let len = file.read(&mut buffer)?;
                if len == 0 {
                    break;
                }
                digest.update(&buffer[..len]);
            }
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn scan_source(root: String) -> anyhow::Result<PublishFolderManifest> {
    let fingerprint = source_fingerprint(&root)?;
    {
        let cache = CACHE.lock();
        if let Some(entry) = cache
            .manifests
            .get(&root)
            .filter(|entry| entry.at.elapsed() < METADATA_TTL && entry.fingerprint == fingerprint)
        {
            return Ok(entry.manifest.clone());
        }
    }
    let manifest = scan_publish_folder(&root, PublishScanOptions::default())?;
    anyhow::ensure!(
        source_fingerprint(&root)? == fingerprint,
        "Nguồn đã thay đổi trong lúc kiểm tra; quét lại"
    );
    let mut cache = CACHE.lock();
    if cache.manifests.len() >= 8 {
        cache.manifests.clear();
    }
    cache.manifests.insert(
        root,
        ManifestEvidence {
            fingerprint,
            at: Instant::now(),
            manifest: manifest.clone(),
        },
    );
    Ok(manifest)
}
