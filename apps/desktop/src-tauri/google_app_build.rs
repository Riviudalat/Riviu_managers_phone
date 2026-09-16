//! Build-time application configuration. Never reads a user's credential store.
use std::{env, fs, path::PathBuf};

const CONFIG_ENV: &str = "RIVIU_GOOGLE_OAUTH_CONFIG_JSON";
const CONFIG_FILE_ENV: &str = "RIVIU_GOOGLE_OAUTH_CONFIG_FILE";
const REQUIRED_ENV: &str = "RIVIU_REQUIRE_GOOGLE_CONFIG";
const INVALID: &str = "Google build config invalid: provide Desktop clientId, pickerApiKey and numeric projectNumber; clientSecret is optional. Account tokens are not allowed.";

pub fn validate(raw: &str, required: bool) -> Result<String, &'static str> {
    if raw.trim().is_empty() {
        return if required {
            Err("Google build config missing: set RIVIU_GOOGLE_OAUTH_CONFIG_JSON or google-oauth.local.json before building a release.")
        } else {
            Ok(String::new())
        };
    }
    if raw.len() > 16 * 1024 {
        return Err(INVALID);
    }
    let value: serde_json::Value = serde_json::from_str(raw).map_err(|_| INVALID)?;
    let object = value.as_object().ok_or(INVALID)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "clientId" | "clientSecret" | "pickerApiKey" | "projectNumber"
        )
    }) {
        return Err(INVALID);
    }
    let text = |key| {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .ok_or(INVALID)
    };
    let client = text("clientId")?;
    let prefix = client
        .strip_suffix(".apps.googleusercontent.com")
        .ok_or(INVALID)?;
    if prefix.is_empty()
        || client.len() > 256
        || !prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(INVALID);
    }
    let valid_key =
        |s: &str| !s.trim().is_empty() && s.len() <= 4096 && !s.chars().any(char::is_control);
    if !valid_key(text("pickerApiKey")?) {
        return Err(INVALID);
    }
    if let Some(secret) = object.get("clientSecret").filter(|v| !v.is_null()) {
        if !secret.as_str().is_some_and(valid_key) {
            return Err(INVALID);
        }
    }
    let project = text("projectNumber")?;
    if project.is_empty() || project.len() > 32 || !project.bytes().all(|b| b.is_ascii_digit()) {
        return Err(INVALID);
    }
    serde_json::to_string(&value).map_err(|_| INVALID)
}

pub fn stage() -> Result<(), &'static str> {
    for name in [CONFIG_ENV, CONFIG_FILE_ENV, REQUIRED_ENV] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    println!("cargo:rerun-if-changed=google_app_build.rs");
    let path = env::var_os(CONFIG_FILE_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"))
                .join("google-oauth.local.json")
        });
    println!("cargo:rerun-if-changed={}", path.display());
    // An explicitly present empty/invalid CI variable must fail, not fall back
    // to an unrelated local file that would hide a broken release configuration.
    let raw = match env::var(CONFIG_ENV) {
        Ok(value) => value,
        Err(env::VarError::NotUnicode(_)) => return Err(INVALID),
        Err(env::VarError::NotPresent) => match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && env::var_os(CONFIG_FILE_ENV).is_none() =>
            {
                String::new()
            }
            Err(_) => return Err("Cannot read Google application build config file"),
        },
    };
    let required = env::var("PROFILE").as_deref() == Ok("release")
        || env::var(REQUIRED_ENV).as_deref() == Ok("1");
    let normalized = validate(&raw, required)?;
    let destination = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"))
        .join("google-oauth-config.json");
    // Use include_str! instead of cargo:rustc-env to keep the JSON out of logs.
    fs::write(destination, normalized).map_err(|_| "Cannot stage Google application build config")
}

#[cfg(test)]
mod tests {
    use super::*;
    const COMPLETE: &str = r#"{"clientId":"fixture.apps.googleusercontent.com","pickerApiKey":"fixture-key","projectNumber":"12345"}"#;
    #[test]
    fn required_builds_reject_missing_incomplete_and_account_tokens() {
        assert!(validate("", true).is_err());
        assert_eq!(validate("", false).unwrap(), "");
        for raw in [
            "broken",
            r#"{"clientId":"fixture.apps.googleusercontent.com"}"#,
            r#"{"clientId":"fixture.apps.googleusercontent.com","pickerApiKey":"key","projectNumber":"123","refreshToken":"secret-value"}"#,
        ] {
            let error = validate(raw, true).unwrap_err();
            assert!(!error.contains("secret-value"));
        }
    }
    #[test]
    fn valid_application_config_roundtrips_with_or_without_desktop_secret() {
        assert!(validate(COMPLETE, true).is_ok());
        let mut value: serde_json::Value = serde_json::from_str(COMPLETE).unwrap();
        value["clientSecret"] = "fixture-secret".into();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&validate(&value.to_string(), true).unwrap())
                .unwrap(),
            value
        );
    }
}
