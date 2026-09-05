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
    let webhook = db
        .get_setting(riviu_core::publish_sheet::WEBHOOK_URL_SETTING)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let token = db
        .get_setting(riviu_core::publish_sheet::WEBHOOK_TOKEN_SETTING)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
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
    };
    if let Err(error) = riviu_core::publish_sheet::push_row(&webhook, &payload).await {
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
}

pub(super) fn publish_sheet_config_of(db: &Database) -> Result<PublishSheetConfig, CommandError> {
    let webhook_url = db
        .get_setting(riviu_core::publish_sheet::WEBHOOK_URL_SETTING)
        .map_err(err)?
        .unwrap_or_default();
    let has_token = db
        .get_setting(riviu_core::publish_sheet::WEBHOOK_TOKEN_SETTING)
        .map_err(err)?
        .map(|token| !token.trim().is_empty())
        .unwrap_or(false);
    Ok(PublishSheetConfig {
        webhook_url,
        has_token,
    })
}

#[tauri::command]
pub fn publish_sheet_get_config(
    state: State<'_, AppState>,
) -> Result<PublishSheetConfig, CommandError> {
    publish_sheet_config_of(&state.db)
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
        .set_publish_sheet_config(&webhook_url, token.as_deref().map(str::trim))
        .map_err(err)?;
    publish_sheet_config_of(&state.db)
}
