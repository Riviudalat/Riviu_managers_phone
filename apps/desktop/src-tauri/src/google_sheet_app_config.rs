//! Distributable application identity, separate from each PC's Google session.
use anyhow::Context;
use riviu_core::{db::Database, google_oauth::GoogleOAuthClientConfig};

pub(super) fn configured(db: &Database) -> anyhow::Result<Option<GoogleOAuthClientConfig>> {
    resolve(
        db.google_oauth_config()?,
        Some(include_str!(concat!(
            env!("OUT_DIR"),
            "/google-oauth-config.json"
        ))),
    )
}

fn resolve(
    local: Option<GoogleOAuthClientConfig>,
    bundled: Option<&str>,
) -> anyhow::Result<Option<GoogleOAuthClientConfig>> {
    if let Some(config) = local {
        config.validate()?;
        return Ok(Some(config));
    }
    let Some(value) = bundled.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    anyhow::ensure!(
        value.len() <= 16 * 1024,
        "Cấu hình Google đi kèm bản build quá lớn"
    );
    // Do not echo parser input, keys or credentials in errors. deny_unknown_fields
    // on the core type also rejects session/access/refresh tokens in this payload.
    let config: GoogleOAuthClientConfig = serde_json::from_str(value)
        .map_err(|_| anyhow::anyhow!("Cấu hình Google đi kèm bản build không hợp lệ"))?;
    config
        .validate()
        .context("Cấu hình Google đi kèm bản build không hợp lệ")?;
    Ok(Some(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    const BUNDLE: &str = r#"{"clientId":"fixture.apps.googleusercontent.com","clientSecret":"app-secret","pickerApiKey":"picker-key","projectNumber":"12345"}"#;

    #[test]
    fn compiled_configuration_works_without_a_local_credential_store() {
        let db = Database::open(":memory:").unwrap();
        let config = configured(&db).unwrap();
        let bundled = include_str!(concat!(env!("OUT_DIR"), "/google-oauth-config.json"));
        if bundled.is_empty() {
            assert!(config.is_none());
        } else {
            let config = config.unwrap();
            assert!(config.picker_api_key.is_some() && config.project_number.is_some());
            assert!(db.google_oauth_config().unwrap().is_none());
            assert!(db.google_oauth_tokens().unwrap().is_none());
        }
    }

    #[test]
    fn clean_pc_gets_app_identity_without_account_or_tokens() {
        let config = resolve(None, Some(BUNDLE)).unwrap().unwrap();
        assert_eq!(config.client_id, "fixture.apps.googleusercontent.com");
        assert_eq!(config.project_number.as_deref(), Some("12345"));
        assert!(resolve(None, None).unwrap().is_none());
        assert!(resolve(None, Some("  ")).unwrap().is_none());
    }

    #[test]
    fn local_identity_survives_a_binary_update_with_different_or_invalid_bundle() {
        let mut local = resolve(None, Some(BUNDLE)).unwrap().unwrap();
        local.client_id = "custom.apps.googleusercontent.com".into();
        for bundled in [BUNDLE, "invalid", ""] {
            assert_eq!(
                resolve(Some(local.clone()), Some(bundled)).unwrap(),
                Some(local.clone())
            );
        }
    }

    #[test]
    fn rejects_invalid_bundle_and_session_tokens_without_echoing_values() {
        for raw in [
            "not-json",
            r#"{"clientId":"invalid"}"#,
            r#"{"clientId":"fixture.apps.googleusercontent.com","refreshToken":"private-token"}"#,
            r#"{"clientId":"fixture.apps.googleusercontent.com","projectNumber":"bad"}"#,
        ] {
            let error = resolve(None, Some(raw)).unwrap_err().to_string();
            assert!(!error.contains("private-token"));
        }
    }
}
