//! Sheet delivery configuration and credential-aware updates.

use super::*;

#[cfg(test)]
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
#[cfg(test)]
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
    let Some(claim) = db
        .claim_bound_sheet_delivery(
            riviu_core::db::SheetDeliveryKind::Canonical,
            Some(assignment_id),
            chrono::Utc::now().timestamp_millis(),
        )
        .map_err(|e| e.to_string())?
    else {
        if db
            .pending_publish_sheet_row(assignment_id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("sheet_not_ready: Sheet vẫn đang chờ gửi hoặc cần xử lý; chưa nhận được xác nhận hàng hiện tại".into());
        }
        return Ok(());
    };
    deliver_bound_claim(db, events, claim)
        .await
        .map_err(|e| e.to_string())
}

async fn deliver_bound_claim(
    db: &Database,
    events: &riviu_core::events::EventBus,
    claim: riviu_core::db::SheetDeliveryClaim,
) -> anyhow::Result<()> {
    use riviu_core::db::{SheetDeliveryPayload, SheetOutboxSettlement};
    let mut transport_accepted = false;
    let delivered = async {
        let settings = db.publish_sheet_delivery_settings()?;
        anyhow::ensure!(
            !settings.webhook_url.trim().is_empty() && !settings.token.trim().is_empty(),
            "Sheet chưa cấu hình kết nối ghi"
        );
        match &claim.payload {
            SheetDeliveryPayload::Canonical { row, metadata } => {
                let payload = riviu_core::publish_sheet::SheetRow {
                    token: settings.token.clone(),
                    post_url: row.post_url.clone(),
                    poster: row.poster.clone(),
                    partners: row.partners.clone(),
                    assignment_id: row.assignment_id.clone(),
                    posted_at: row.posted_at.clone(),
                };
                riviu_core::publish_sheet::push_bound_canonical(
                    &settings.webhook_url,
                    &payload,
                    &claim.target,
                    row.revision,
                    metadata.as_ref(),
                )
                .await?;
            }
            SheetDeliveryPayload::Report(row) => {
                riviu_core::publish_sheet::push_bound_internal_report(
                    &settings.webhook_url,
                    &settings.token,
                    row,
                    &claim.target,
                )
                .await?
            }
        }
        transport_accepted = true;
        let (campaign, input_digest, target_snapshot) = match &claim.payload {
            SheetDeliveryPayload::Canonical { row, .. } => {
                let (digest, target) = publish_reconciliation_identity(db, &row.campaign_id)?;
                (Some(row.campaign_id.as_str()), digest, target)
            }
            SheetDeliveryPayload::Report(_) => (None, None, None),
        };
        let settlement = db.settle_bound_sheet_delivery(
            &claim,
            input_digest.as_deref(),
            target_snapshot.as_ref(),
            chrono::Utc::now().timestamp_millis(),
        )?;
        if matches!(settlement, SheetOutboxSettlement::Delivered(_)) {
            if let Some(campaign) = campaign {
                announce(events, db, campaign);
            }
        }
        anyhow::ensure!(
            !matches!(settlement, SheetOutboxSettlement::StaleRevision),
            "Sheet đã trả kết quả nhưng quyền gửi đã thay đổi; hàng hiện tại vẫn được giữ"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if let Err(error) = &delivered {
        let retryable = transport_accepted
            || riviu_core::publish_sheet::sheet_delivery_error_is_retryable(error);
        db.fail_bound_sheet_delivery(
            &claim,
            &format!("{error:#}"),
            retryable,
            chrono::Utc::now().timestamp_millis(),
        )?;
        if let SheetDeliveryPayload::Canonical { row, .. } = &claim.payload {
            announce(events, db, &row.campaign_id);
        }
    }
    delivered
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
    if result.connection_verified {
        state
            .db
            .set_setting(
                riviu_core::publish_sheet::SHEET_URL_SETTING,
                &result.sheet_url,
            )
            .map_err(err)?;
        let internal = result.layout.as_deref() == Some("internal");
        if internal != settings.internal_reporting {
            state
                .db
                .set_publish_sheet_config_with_reporting(
                    &settings.webhook_url,
                    Some(&settings.token),
                    Some(internal),
                )
                .map_err(err)?;
        }
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
    if !result.connection_verified {
        return Err(err("Sheet chưa xác minh kết nối ghi"));
    }
    state
        .db
        .set_setting(
            riviu_core::publish_sheet::SHEET_URL_SETTING,
            &result.sheet_url,
        )
        .map_err(err)?;

    if (result.layout.as_deref() == Some("internal")) != settings.internal_reporting {
        state
            .db
            .set_publish_sheet_config_with_reporting(
                &settings.webhook_url,
                Some(&settings.token),
                Some(result.layout.as_deref() == Some("internal")),
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

/// Independent canonical/report work with a shared durable per-assignment claim.
/// Reports can occupy only one slot, leaving capacity for a newly verified link.
pub(crate) async fn run_bound_sheet_worker(
    db: Arc<Database>,
    events: riviu_core::events::EventBus,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    use riviu_core::db::SheetDeliveryKind;
    use sha2::Digest;
    let mut tasks = tokio::task::JoinSet::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _=tick.tick()=>{},
            result=tasks.join_next(),if !tasks.is_empty()=>{
                if let Some(Err(error))=result {log::warn!("Sheet delivery worker failed: {error}");}
            },
        }
        if stop.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        let settings = match db.publish_sheet_delivery_settings() {
            Ok(settings) => settings,
            Err(error) => {
                log::warn!("Sheet: không đọc được kết nối: {error:#}");
                continue;
            }
        };
        if settings.webhook_url.trim().is_empty() || settings.token.trim().is_empty() {
            continue;
        }
        let mut digest = sha2::Sha256::new();
        digest.update(settings.webhook_url.as_bytes());
        digest.update([0]);
        digest.update(settings.token.as_bytes());
        let fingerprint = format!("{:x}", digest.finalize());
        let now = chrono::Utc::now().timestamp_millis();
        if let Err(error) = db.configure_bound_sheet_delivery(&fingerprint, now) {
            log::warn!("Sheet: không cập nhật được lịch gửi: {error:#}");
            continue;
        }
        for kind in [
            SheetDeliveryKind::Canonical,
            SheetDeliveryKind::Report,
            SheetDeliveryKind::Canonical,
        ] {
            if tasks.len() >= 2 {
                break;
            }
            match db.claim_bound_sheet_delivery(kind, None, now) {
                Ok(Some(claim)) => {
                    let db = db.clone();
                    let events = events.clone();
                    tasks.spawn(async move {
                        if let Err(error) = deliver_bound_claim(&db, &events, claim).await {
                            log::warn!("Sheet chưa gửi được: {error:#}");
                        }
                    });
                }
                Ok(None) => {}
                Err(error) => log::warn!("Sheet: không nhận được hàng đến hạn: {error:#}"),
            }
        }
    }
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            log::warn!("Sheet shutdown delivery failed: {error}");
        }
    }
}
