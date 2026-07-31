//! Free Apple ID signing helpers (anisette-based free signing via sidecar).

pub mod credentials;

pub use credentials::{CredentialBackend, CredentialStore};

use std::path::{Path, PathBuf};
use std::process::Stdio;

use chrono::{DateTime, Duration, Utc};
use riviu_core::{AppleIdConfig, WdaStatus};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use credentials::{APPLE_EMAIL_ACCOUNT, APPLE_PASSWORD_ACCOUNT};

fn background_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

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
    sidecar_root: PathBuf,
    credentials: CredentialStore,
}

impl SigningService {
    pub fn new(sidecar_dir: PathBuf) -> anyhow::Result<Self> {
        Ok(Self::with_credentials(
            sidecar_dir,
            CredentialStore::system()?,
        ))
    }

    pub fn with_credentials(sidecar_dir: PathBuf, credentials: CredentialStore) -> Self {
        let sidecar_root = sidecar_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| sidecar_dir.clone());
        Self {
            sidecar: sidecar_dir.join("riviu_signer.py"),
            sidecar_root,
            credentials,
        }
    }

    fn signer_command(&self) -> Command {
        let runtime = self
            .sidecar_root
            .join("pymobiledevice3")
            .join("runtime")
            .join(if cfg!(windows) {
                "riviu-pmd.exe"
            } else {
                "riviu-pmd"
            });
        if runtime.is_file() {
            let mut command = background_command(&runtime);
            command
                .arg("__script")
                .arg(&self.sidecar)
                .env("RIVIU_EMBEDDED_PYTHON_RUNTIME", runtime);
            return command;
        }

        let mut command = background_command(if cfg!(windows) { "python" } else { "python3" });
        command.arg(&self.sidecar);
        command
    }

    pub fn save_apple_id(&self, email: &str, password: &str) -> anyhow::Result<()> {
        self.credentials.set(APPLE_EMAIL_ACCOUNT, email)?;
        self.credentials.set(APPLE_PASSWORD_ACCOUNT, password)?;
        Ok(())
    }

    pub fn clear_apple_id(&self) -> anyhow::Result<()> {
        self.credentials.delete(APPLE_EMAIL_ACCOUNT)?;
        self.credentials.delete(APPLE_PASSWORD_ACCOUNT)?;
        Ok(())
    }

    pub fn apple_id_config(&self) -> anyhow::Result<AppleIdConfig> {
        let email = self
            .credentials
            .get(APPLE_EMAIL_ACCOUNT)?
            .unwrap_or_default();
        let has_password = self
            .credentials
            .get(APPLE_PASSWORD_ACCOUNT)?
            .map(|p| !p.is_empty())
            .unwrap_or(false);
        Ok(AppleIdConfig {
            email,
            has_password,
        })
    }

    pub async fn sign_and_install_wda(
        &self,
        udid: &str,
        wda_source: &Path,
    ) -> anyhow::Result<SignResult> {
        let cfg = self.apple_id_config()?;
        let email = if cfg.email.is_empty() {
            "xcode-account@local".to_string()
        } else {
            cfg.email
        };
        let password = self
            .credentials
            .get(APPLE_PASSWORD_ACCOUNT)?
            .unwrap_or_else(|| "xcode-managed".into());

        if !self.sidecar.exists() {
            anyhow::bail!("Thiếu sidecar signer. Kiểm tra sidecars/signer/riviu_signer.py");
        }

        let output = self
            .signer_command()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Arc;

    struct EmptyCredentials;

    impl CredentialBackend for EmptyCredentials {
        fn get(&self, _account: &str) -> anyhow::Result<Option<String>> {
            Ok(None)
        }

        fn set(&self, _account: &str, _value: &str) -> anyhow::Result<()> {
            Ok(())
        }

        fn delete(&self, _account: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn packaged_signer_prefers_the_embedded_python_runtime() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("fixture time after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "riviu-signing-runtime-{}-{nonce}",
            std::process::id()
        ));
        let signer_dir = root.join("signer");
        let runtime_dir = root.join("pymobiledevice3").join("runtime");
        std::fs::create_dir_all(&signer_dir).expect("create signer fixture");
        std::fs::create_dir_all(&runtime_dir).expect("create runtime fixture");
        let signer = signer_dir.join("riviu_signer.py");
        let runtime = runtime_dir.join(if cfg!(windows) {
            "riviu-pmd.exe"
        } else {
            "riviu-pmd"
        });
        std::fs::write(&signer, b"# fixture\n").expect("write signer fixture");
        std::fs::write(&runtime, b"fixture").expect("write runtime fixture");

        let service = SigningService::with_credentials(
            signer_dir,
            CredentialStore::new(Arc::new(EmptyCredentials)),
        );
        let command = service.signer_command();
        let std_command = command.as_std();
        let args: Vec<OsString> = std_command.get_args().map(OsString::from).collect();
        let embedded_environment = std_command
            .get_envs()
            .find(|(key, _)| *key == "RIVIU_EMBEDDED_PYTHON_RUNTIME")
            .and_then(|(_, value)| value)
            .map(PathBuf::from);

        std::fs::remove_dir_all(&root).expect("remove signing fixture");

        assert_eq!(std_command.get_program(), runtime.as_os_str());
        assert_eq!(
            args,
            vec![OsString::from("__script"), signer.into_os_string()]
        );
        assert_eq!(embedded_environment.as_ref(), Some(&runtime));
    }
}
