//! Short-lived evidence from a completed writer check, bound to local authorization.
//! A visual/read-only Sheet result cannot populate this cache. Delivery still reads
//! the remote owner and receipt through the existing shared writer protocol.
use super::*;
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(300);

struct Proof {
    binding: String,
    result: SheetCheckResult,
    observed_at: Instant,
}

#[derive(Default)]
struct ProofCache(Option<Proof>);

impl ProofCache {
    fn get(&mut self, binding: &str, now: Instant) -> Option<SheetCheckResult> {
        if self.0.as_ref().is_some_and(|proof| {
            proof.binding == binding && now.saturating_duration_since(proof.observed_at) < TTL
        }) {
            return self.0.as_ref().map(|proof| proof.result.clone());
        }
        self.0 = None;
        None
    }

    fn remember(&mut self, binding: String, result: &SheetCheckResult, observed_at: Instant) {
        self.0 = (result.connection_verified && result.reporting_ready).then(|| Proof {
            binding, result: result.clone(), observed_at,
        });
    }
}

static CACHE: std::sync::LazyLock<parking_lot::Mutex<ProofCache>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(ProofCache::default()));

pub(super) fn invalidate() {
    CACHE.lock().0 = None;
}

async fn binding(db: &Database, url: &str) -> anyhow::Result<Option<String>> {
    if !db.sheet_uses_google_direct()? {
        return Ok(None);
    }
    let Some(connection) = db.google_sheet_connection()? else { return Ok(None) };
    let parsed = riviu_core::publish_sheet::parse_sheet_url(url)?;
    if parsed.spreadsheet_id != connection.target.spreadsheet_id
        || parsed.sheet_gid != connection.target.sheet_gid {
        return Ok(None);
    }
    let Some(tokens) = db.google_oauth_tokens()? else { return Ok(None) };
    if tokens.needs_refresh() || !tokens.has_sheets_scope() || tokens.account_id != connection.account_id {
        return Ok(None);
    }
    let session = sessions().lock().await;
    if session.phase != "idle" || session.staged.is_some() || session.error.is_some() {
        return Ok(None);
    }
    let config = app_config::configured(db)?;
    Ok(Some(serde_json::to_string(&serde_json::json!({
        "connection": connection,
        "clientId": config.map(|config| config.client_id),
        "authorizationGeneration": db.get_setting("google.sheets.authorization-generation")?,
        "sessionGeneration": session.generation,
        "provider": db.get_setting(riviu_core::db::SHEET_PROVIDER_SETTING)?,
        "url": db.get_setting(riviu_core::publish_sheet::SHEET_URL_SETTING)?,
        "layout": db.get_setting(riviu_core::publish_sheet::INTERNAL_REPORTING_SETTING)?,
        "migration": db.get_setting(GOOGLE_MIGRATION_SETTING)?,
    }))?))
}

pub(super) async fn get(db: &Database, url: &str) -> anyhow::Result<Option<SheetCheckResult>> {
    let Some(binding) = binding(db, url).await? else {
        invalidate();
        return Ok(None);
    };
    Ok(CACHE.lock().get(&binding, Instant::now()))
}

pub(super) async fn remember(db: &Database, url: &str, result: &SheetCheckResult) -> anyhow::Result<()> {
    if let Some(binding) = binding(db, url).await? {
        CACHE.lock().remember(binding, result, Instant::now());
    } else {
        invalidate();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_proof_is_reused_only_for_the_same_unexpired_binding() {
        let mut cache = ProofCache::default();
        let mut checked = riviu_core::publish_sheet::parse_sheet_url(
            "https://docs.google.com/spreadsheets/d/fixture-book/edit#gid=0",
        ).unwrap();
        checked.connection_verified = true;
        checked.reporting_ready = true;
        let now = Instant::now();
        cache.remember("account/target/epoch/auth-generation".into(), &checked, now);
        assert_eq!(cache.get("account/target/epoch/auth-generation", now + Duration::from_secs(20)), Some(checked.clone()));
        assert_eq!(cache.get("changed-binding", now + Duration::from_secs(21)), None);
        assert_eq!(cache.get("account/target/epoch/auth-generation", now + Duration::from_secs(22)), None);
        cache.remember("account/target/epoch/auth-generation".into(), &checked, now);
        assert_eq!(cache.get("account/target/epoch/auth-generation", now + TTL), None);
        let mut refused = checked;
        refused.reporting_ready = false;
        cache.remember("account/target/epoch/auth-generation".into(), &refused, now);
        assert_eq!(cache.get("account/target/epoch/auth-generation", now), None);
    }
}
