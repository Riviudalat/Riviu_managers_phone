//! Free Apple ID signing helpers (anisette-based free signing via sidecar).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::{DateTime, Duration, Utc};
use riviu_core::{AppleIdConfig, WdaStatus};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

const SERVICE: &str = "riviu-managers-phone";
const ACCOUNT_USER: &str = "apple-id-email";
const ACCOUNT_PASS: &str = "apple-id-password";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignRequest {
    pub udid: String,
    pub wda_ipa_or_app: PathBuf,
    pub output_ipa: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignResult {
    pub udid: String,
    pub signed_ipa: PathBuf,
    pub expires_at: DateTime<Utc>,
    pub message: String,
}

#[derive(Clone)]
pub struct SigningService {
    sidecar: PathBuf,
}

impl SigningService {
    pub fn new(sidecar_dir: PathBuf) -> Self {
        Self {
            sidecar: sidecar_dir.join("riviu_signer.py"),
        }
    }

    pub fn save_apple_id(&self, email: &str, password: &str) -> anyhow::Result<()> {
        let user = keyring::Entry::new(SERVICE, ACCOUNT_USER)?;
        let pass = keyring::Entry::new(SERVICE, ACCOUNT_PASS)?;
        user.set_password(email)?;
        pass.set_password(password)?;
        Ok(())
    }

    pub fn clear_apple_id(&self) -> anyhow::Result<()> {
        let user = keyring::Entry::new(SERVICE, ACCOUNT_USER)?;
        let pass = keyring::Entry::new(SERVICE, ACCOUNT_PASS)?;
        let _ = user.delete_credential();
        let _ = pass.delete_credential();
        Ok(())
    }

    pub fn apple_id_config(&self) -> AppleIdConfig {
        let email = keyring::Entry::new(SERVICE, ACCOUNT_USER)
            .ok()
            .and_then(|e| e.get_password().ok())
            .unwrap_or_default();
        let has_password = keyring::Entry::new(SERVICE, ACCOUNT_PASS)
            .ok()
            .and_then(|e| e.get_password().ok())
            .map(|p| !p.is_empty())
            .unwrap_or(false);
        AppleIdConfig {
            email,
            has_password,
        }
    }

    pub async fn sign_and_install_wda(
        &self,
        udid: &str,
        wda_source: &Path,
    ) -> anyhow::Result<SignResult> {
        let cfg = self.apple_id_config();
        let email = if cfg.email.is_empty() {
            "xcode-account@local".to_string()
        } else {
            cfg.email
        };
        let password = keyring::Entry::new(SERVICE, ACCOUNT_PASS)
            .ok()
            .and_then(|e| e.get_password().ok())
            .unwrap_or_else(|| "xcode-managed".into());

        if !self.sidecar.exists() {
            anyhow::bail!(
                "Thiếu sidecar signer. Kiểm tra sidecars/signer/riviu_signer.py"
            );
        }

        let output = Command::new("python3")
            .arg(&self.sidecar)
            .arg("sign-install-wda")
            .arg("--udid")
            .arg(udid)
            .arg("--apple-id")
            .arg(&email)
            .arg("--password")
            .arg(&password)
            .arg("--wda")
            .arg(wda_source)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let parsed: serde_json::Value = stdout
            .lines()
            .rev()
            .find_map(|line| {
                let line = line.trim();
                if line.starts_with('{') {
                    serde_json::from_str(line).ok()
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if !output.status.success() || parsed.get("ok") == Some(&serde_json::Value::Bool(false)) {
            let detail = parsed
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| {
                    if !stderr.trim().is_empty() {
                        stderr.trim()
                    } else {
                        stdout.trim()
                    }
                });
            anyhow::bail!("{detail}");
        }

        let expires_at = parsed
            .get("expiresAt")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|| Utc::now() + Duration::days(7));

        Ok(SignResult {
            udid: udid.to_string(),
            signed_ipa: wda_source.to_path_buf(),
            expires_at,
            message: parsed
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Riviumanagersphone agent installed")
                .to_string(),
        })
    }

    pub async fn bulk_resign(
        &self,
        udids: &[String],
        wda_source: &Path,
    ) -> anyhow::Result<Vec<SignResult>> {
        let mut results = Vec::new();
        for udid in udids {
            results.push(self.sign_and_install_wda(udid, wda_source).await?);
        }
        Ok(results)
    }

    pub fn wda_status(udid: &str, expires_at: Option<DateTime<Utc>>, running: bool) -> WdaStatus {
        let days_remaining = expires_at.map(|exp| (exp - Utc::now()).num_days());
        WdaStatus {
            udid: udid.to_string(),
            installed: expires_at.is_some(),
            running,
            expires_at,
            days_remaining,
        }
    }
}
