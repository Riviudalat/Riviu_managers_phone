//! Unregistered compatibility entry points retained for calibration.

use super::*;

// Compatibility-only Rust entry points. They are intentionally absent from Tauri's command
// registry and `api.ts`; production callers must use `publish_execute`, whose one-shot effect
// boundary covers transfer through Sheet settlement.
#[tauri::command]
pub fn publish_prepare(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    match state
        .db
        .mark_publish_campaign_ready(&campaign_id)
        .map_err(err)?
    {
        Some(riviu_core::PublishCampaignState::Ready) => {
            announce(&state.events, &state.db, &campaign_id);
        }
        Some(actual) => {
            return Err(err(format!(
                "campaign is already terminal or in flight: {actual:?}"
            )))
        }
        None => return Err(err("publish campaign not found")),
    }
    state
        .db
        .get_publish_campaign(&campaign_id)
        .map_err(err)?
        .ok_or_else(|| err("campaign disappeared after prepare"))
}

#[tauri::command]
pub async fn publish_transfer(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    transfer_publish_campaign_inner(
        state.control.clone(),
        state.db.clone(),
        state.events.clone(),
        state.active_agent_bundle_id.clone(),
        campaign_id,
    )
    .await
    .map_err(err)
}

#[tauri::command]
pub async fn publish_post(
    state: State<'_, AppState>,
    campaign_id: String,
) -> Result<PublishCampaignDetail, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    post_publish_campaign_inner(
        state.control.clone(),
        state.db.clone(),
        Arc::new(state.streams.clone()),
        state.events.clone(),
        campaign_id,
    )
    .await
    .map_err(err)
}
