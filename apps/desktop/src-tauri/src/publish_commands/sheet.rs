//! Sheet delivery configuration and credential-aware updates.

use super::*;

pub(super) fn settle_publish_sheet_delivery_and_announce(
    db: &Database,
    events: &riviu_core::events::EventBus,
    row: &riviu_core::db::SheetOutboxRow,
    input_digest: Option<&str>,
    target_snapshot: Option<&riviu_core::ResolvedTargetSnapshot>,
) -> anyhow::Result<bool> {
    match db.settle_publish_sheet_delivery(
        &row.assignment_id,
        &row.campaign_id,
        row.revision,
        input_digest,
        target_snapshot,
    )? {
        riviu_core::db::SheetOutboxSettlement::StaleRevision => Ok(false),
        riviu_core::db::SheetOutboxSettlement::DeliveredWithoutCampaign => Ok(true),
        riviu_core::db::SheetOutboxSettlement::Delivered(_) => {
            announce(events, db, &row.campaign_id);
            Ok(true)
        }
    }
}

/// Finish the exact Sheet revision that was delivered, then converge the durable operation view.
/// A stale CAS is an ordinary refusal and emits nothing because a newer row is still owed.
pub(crate) fn mark_publish_sheet_sent_and_reconcile(
    db: &Database,
    events: &riviu_core::events::EventBus,
    row: &riviu_core::db::SheetOutboxRow,
) -> anyhow::Result<bool> {
    let (input_digest, target_snapshot) = publish_reconciliation_identity(db, &row.campaign_id)?;
    settle_publish_sheet_delivery_and_announce(
        db,
        events,
        row,
        input_digest.as_deref(),
        target_snapshot.as_ref(),
    )
}

pub(super) async fn deliver_assignment_sheet_row(
    db: &Database,
    events: &riviu_core::events::EventBus,
    assignment_id: &str,
) -> Result<(), String> {
    let Some(row) = db
        .pending_publish_sheet_row(assignment_id)
        .map_err(|error| error.to_string())?
    else {
        return Ok(());
    };
    let settings = db
        .publish_sheet_delivery_settings()
        .map_err(|error| error.to_string())?;
    let webhook = settings.webhook_url;
    let token = settings.token;
    if !riviu_core::publish_sheet::is_acceptable_webhook(webhook.trim()) || token.trim().is_empty()
    {
        return Err(
            "sheet_not_ready: bài đã đăng và link đang nằm trong outbox pending; cấu hình webhook HTTPS cùng token để gửi Sheet"
                .into(),
        );
    }
    let payload = riviu_core::publish_sheet::SheetRow {
        token,
        post_url: row.post_url.clone(),
        poster: row.poster.clone(),
        partners: row.partners.clone(),
        assignment_id: row.assignment_id.clone(),
        posted_at: row.posted_at.clone(),
    };
    let metadata = if settings.internal_reporting {
        match db.internal_publish_report(assignment_id) {
            Ok(report) => report
                .filter(|report| report.metadata.status == "Đã xác minh")
                .map(|row| row.metadata),
            Err(error) => {
                log::warn!("Sheet nội bộ: chưa đọc được metadata {assignment_id} ({error:#})");
                None
            }
        }
    } else {
        None
    };
    if let Err(error) =
        riviu_core::publish_sheet::push_row_with_metadata(&webhook, &payload, metadata.as_ref())
            .await
    {
        let reason = error.to_string();
        let marked = db
            .mark_publish_sheet_failed(&row.assignment_id, row.revision, &reason)
            .map_err(|error| error.to_string())?;
        if !marked
            && db
                .pending_publish_sheet_row(&row.assignment_id)
                .map_err(|error| error.to_string())?
                .is_none()
        {
            return Ok(());
        }
        return Err(reason);
    }
    let marked = mark_publish_sheet_sent_and_reconcile(db, events, &row)
        .map_err(|error| error.to_string())?;
    if !marked
        && db
            .pending_publish_sheet_row(&row.assignment_id)
            .map_err(|error| error.to_string())?
            .is_some()
    {
        return Err("Sheet accepted the row but its outbox revision changed".into());
    }
    Ok(())
}

/// What the Sheet delivery is configured with — minus the token itself.
///
/// `has_token` and never the token: the value is a bearer credential, and a screen that can
/// display it is a screen that screenshots, logs and support photos leak it from. The page
/// only needs to know whether one is set.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishSheetConfig {
    pub webhook_url: String,
    pub has_token: bool,
    pub internal_reporting: bool,
    pub sheet_url: String,
}

pub(super) fn publish_sheet_config_of(db: &Database) -> Result<PublishSheetConfig, CommandError> {
    let settings = db.publish_sheet_delivery_settings().map_err(err)?;
    let webhook_url = settings.webhook_url;
    let has_token = !settings.token.trim().is_empty();
    Ok(PublishSheetConfig {
        webhook_url,
        has_token,
        internal_reporting: settings.internal_reporting,
        sheet_url: db
            .get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)
            .map_err(err)?
            .unwrap_or_default(),
    })
}

#[tauri::command]
pub fn publish_sheet_get_config(
    state: State<'_, AppState>,
) -> Result<PublishSheetConfig, CommandError> {
    publish_sheet_config_of(&state.db)
}

#[tauri::command]
pub async fn publish_sheet_check(
    state: State<'_, AppState>,
    sheet_url: String,
) -> Result<riviu_core::publish_sheet::SheetCheckResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let settings = state.db.publish_sheet_delivery_settings().map_err(err)?;
    let result = riviu_core::publish_sheet::check_sheet(&sheet_url, &settings)
        .await
        .map_err(err)?;
    if result.readable {
        state
            .db
            .set_setting(
                riviu_core::publish_sheet::SHEET_URL_SETTING,
                &result.sheet_url,
            )
            .map_err(err)?;
    }
    Ok(result)
}

#[tauri::command]
pub async fn publish_sheet_prepare(
    state: State<'_, AppState>,
    sheet_url: String,
) -> Result<riviu_core::publish_sheet::SheetCheckResult, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let settings = state.db.publish_sheet_delivery_settings().map_err(err)?;
    let result = riviu_core::publish_sheet::prepare_sheet(&sheet_url, &settings)
        .await
        .map_err(err)?;
    state
        .db
        .set_setting(
            riviu_core::publish_sheet::SHEET_URL_SETTING,
            &result.sheet_url,
        )
        .map_err(err)?;
    if result.layout.as_deref() == Some("internal") && !settings.internal_reporting {
        state
            .db
            .set_publish_sheet_config_with_reporting(
                &settings.webhook_url,
                Some(&settings.token),
                Some(true),
            )
            .map_err(err)?;
    }
    Ok(result)
}

/// Whether saving this config would hand one endpoint's credential to another.
///
/// **A token belongs to the endpoint it was issued for.** `token: None` means "keep the
/// stored one", which is what lets an operator fix a typo in the URL without re-pasting a
/// credential — but the same convenience, applied to a *different* endpoint, sends webhook
/// A's bearer token to webhook B in the request body. Whoever answers at B then holds a
/// token that writes into the operator's sheet. So the pairing is a refusal, not a warning:
/// changing the URL requires saying what the token for that URL is (or clearing it).
///
/// Pure, and separate from the command, because it is the one decision here worth a test —
/// the rest is two `set_setting` calls.
pub(super) fn token_must_be_restated(stored_url: &str, new_url: &str, token: Option<&str>) -> bool {
    token.is_none() && stored_url.trim() != new_url.trim()
}

/// Save the webhook URL and the token **together**.
///
/// `token: None` keeps the stored one, and is accepted only while the URL is unchanged —
/// see [`token_must_be_restated`]. An empty string clears the token on purpose. The URL is
/// refused unless `is_acceptable_webhook` takes it (HTTPS with a host) or it is empty:
/// empty is the off switch the sweeper honours, not an error.
///
/// **Both writes go in one transaction.** They were two `set_setting` calls, and the sweeper
/// reads the pair every tick — so a crash between them, or a tick landing in the gap, could
/// see a new URL beside an old token. One transaction removes the window entirely rather
/// than making it small.
#[tauri::command]
pub fn publish_sheet_save_config(
    state: State<'_, AppState>,
    webhook_url: String,
    token: Option<String>,
    internal_reporting: Option<bool>,
) -> Result<PublishSheetConfig, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    let webhook_url = webhook_url.trim().to_string();
    if !webhook_url.is_empty() && !riviu_core::publish_sheet::is_acceptable_webhook(&webhook_url) {
        return Err(err(format!(
            "webhook không nhận được: cần HTTPS kèm host thật — token và link bài đi trong \
             body, http:// là gửi chúng trần trụi ({webhook_url})"
        )));
    }
    let stored = publish_sheet_config_of(&state.db)?;
    if token_must_be_restated(&stored.webhook_url, &webhook_url, token.as_deref()) {
        return Err(err(
            "đổi webhook thì phải nhập lại token: token là của endpoint cũ, gửi nó sang \
             endpoint mới là trao cho bên đó quyền ghi vào sheet. Điền token của webhook \
             mới, hoặc để trống ô token và bấm Xoá token nếu endpoint mới không cần.",
        ));
    }
    state
        .db
        .set_publish_sheet_config_with_reporting(
            &webhook_url,
            token.as_deref().map(str::trim),
            internal_reporting,
        )
        .map_err(err)?;
    publish_sheet_config_of(&state.db)
}

#[derive(Default)]
pub(crate) struct InternalSheetSync {
    configuration: String,
    cursor: Option<String>,
    acknowledged: HashMap<String, String>,
    retry: HashMap<String, (Instant, u32)>,
}

impl InternalSheetSync {
    pub(crate) fn configure(
        &mut self,
        settings: &riviu_core::publish_sheet::SheetDeliverySettings,
    ) {
        use sha2::Digest;
        let mut hash = sha2::Sha256::new();
        hash.update(settings.webhook_url.as_bytes());
        hash.update([0]);
        hash.update(settings.token.as_bytes());
        hash.update([u8::from(settings.internal_reporting)]);
        let fingerprint = format!("{:x}", hash.finalize());
        if self.configuration != fingerprint {
            self.configuration = fingerprint;
            self.cursor = None;
            self.acknowledged.clear();
            self.retry.clear();
        }
    }

    fn due(&self, id: &str, digest: &str, now: Instant) -> bool {
        self.acknowledged.get(id).is_none_or(|old| old != digest)
            && self.retry.get(id).is_none_or(|(at, _)| *at <= now)
    }

    fn failed(&mut self, id: &str, now: Instant) {
        let attempts = self
            .retry
            .get(id)
            .map_or(1, |(_, count)| count.saturating_add(1));
        let seconds = 45u64.saturating_mul(1u64 << attempts.min(4)).min(900);
        self.retry
            .insert(id.into(), (now + Duration::from_secs(seconds), attempts));
    }

    fn accepted(&mut self, id: String, digest: String) {
        self.retry.remove(&id);
        self.acknowledged.insert(id, digest);
    }
}

/// DB-only projection and bounded transport; never changes Publish or classic outbox state.
pub(crate) async fn sync_internal_sheet_reports(
    db: &Database,
    settings: &riviu_core::publish_sheet::SheetDeliverySettings,
    sync: &mut InternalSheetSync,
) -> anyhow::Result<()> {
    use sha2::Digest;
    sync.configure(settings);
    if !settings.internal_reporting {
        return Ok(());
    }
    let page = db.internal_publish_report_batch(sync.cursor.as_deref(), 50)?;
    for (id, reason) in &page.errors {
        log::warn!("Sheet nội bộ: bỏ qua hàng dữ liệu lỗi {id}: {reason}");
    }
    let rows = page.rows;
    let page_len = rows.len();
    let mut sent = 0usize;
    let mut consumed = 0usize;
    for row in rows {
        sync.cursor = Some(row.assignment_id.clone());
        consumed += 1;
        let digest = format!("{:x}", sha2::Sha256::digest(serde_json::to_vec(&row)?));
        if !sync.due(&row.assignment_id, &digest, Instant::now()) {
            continue;
        }
        sent += 1;
        match riviu_core::publish_sheet::push_internal_report(
            &settings.webhook_url,
            &settings.token,
            &row,
        )
        .await
        {
            Ok(()) => sync.accepted(row.assignment_id, digest),
            Err(error) => {
                sync.failed(&row.assignment_id, Instant::now());
                log::warn!(
                    "Sheet nội bộ: chưa cập nhật hàng {}: {error:#}",
                    row.assignment_id
                );
            }
        }
        if sent >= 5 {
            break;
        }
    }
    if consumed == page_len {
        sync.cursor = if page.has_more {
            page.next_cursor
        } else {
            None
        };
    }
    Ok(())
}

#[cfg(test)]
mod internal_tests {
    use super::*;
    #[test]
    fn internal_report_cache_retries_changed_rows_and_resets_for_credentials_or_restart() {
        let settings = riviu_core::publish_sheet::SheetDeliverySettings {
            webhook_url: "https://example.com/a".into(),
            token: "a".into(),
            internal_reporting: true,
        };
        let mut sync = InternalSheetSync::default();
        sync.configure(&settings);
        let now = Instant::now();
        assert!(sync.due("a", "v1", now));
        sync.accepted("a".into(), "v1".into());
        assert!(!sync.due("a", "v1", now));
        assert!(sync.due("a", "v2", now));
        sync.failed("poison", now);
        assert!(!sync.due("poison", "v1", now));
        assert!(sync.due("next", "v1", now));
        assert!(sync.due("poison", "v1", now + Duration::from_secs(901)));
        sync.configure(&settings);
        assert!(!sync.due("a", "v1", now));
        let mut changed = settings.clone();
        changed.token = "b".into();
        sync.configure(&changed);
        assert!(sync.due("a", "v1", now));
        sync.accepted("a".into(), "v1".into());
        changed.internal_reporting = false;
        sync.configure(&changed);
        assert!(sync.acknowledged.is_empty());
        let mut restarted = InternalSheetSync::default();
        restarted.configure(&settings);
        assert!(restarted.due("a", "v1", now));
    }
}
