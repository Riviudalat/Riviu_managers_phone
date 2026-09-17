//! Debug-only, explicitly isolated native UI checks. Not deployment smoke or a driver mode.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Context;

use crate::{command_error::CommandError, state::AppState};

// WebView2 can override the explicitly supplied data directory/browser options.
// Refuse these, never clear them process-wide or accidentally reuse an operator profile.
const WEBVIEW_ENV_OVERRIDES: &[&str] = &[
    "WEBVIEW2_USER_DATA_FOLDER",
    "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
    "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
    "WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER",
];

#[derive(Default)]
pub(crate) enum StartupPolicy {
    #[default]
    Normal,
    #[cfg(debug_assertions)]
    Smoke(SmokeSession),
}

#[cfg(debug_assertions)]
pub(crate) struct SmokeSession {
    root: PathBuf,
    next_attempt: std::sync::atomic::AtomicU64,
}

/// Only the policy can mint this after atomically claiming a fresh attempt directory.
#[cfg(debug_assertions)]
pub(crate) struct SmokeAttempt {
    pub(crate) data: PathBuf,
}

impl StartupPolicy {
    pub(crate) fn from_environment() -> anyhow::Result<Self> {
        Self::from_lookup(cfg!(debug_assertions), |name| std::env::var_os(name))
    }

    fn from_lookup(
        debug: bool,
        mut lookup: impl FnMut(&str) -> Option<OsString>,
    ) -> anyhow::Result<Self> {
        // Do not enumerate the environment or even request runtime secret values.
        let requested = lookup("RIVIU_UI_SMOKE");
        let directory = lookup("RIVIU_UI_SMOKE_DIR");
        if requested.is_none() && directory.is_none() {
            return Ok(Self::Normal);
        }
        anyhow::ensure!(debug && cfg!(debug_assertions), "UI smoke is debug-only");
        anyhow::ensure!(
            requested.as_deref() == Some(std::ffi::OsStr::new("1")),
            "UI smoke requires RIVIU_UI_SMOKE=1"
        );
        let mock = lookup("RIVIU_MOCK_DEVICES");
        anyhow::ensure!(
            mock.as_deref()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
            "UI smoke requires RIVIU_MOCK_DEVICES=1"
        );
        let directory = PathBuf::from(directory.context("UI smoke requires RIVIU_UI_SMOKE_DIR")?);
        for name in WEBVIEW_ENV_OVERRIDES {
            anyhow::ensure!(
                lookup(name).is_none(),
                "UI smoke refuses WebView environment override {name}"
            );
        }
        #[cfg(debug_assertions)]
        {
            let root = claim_scratch(&directory)?;
            Ok(Self::Smoke(SmokeSession {
                root,
                next_attempt: std::sync::atomic::AtomicU64::new(0),
            }))
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = directory;
            anyhow::bail!("UI smoke is unavailable in release")
        }
    }

    pub(crate) fn is_smoke(&self) -> bool {
        match self {
            Self::Normal => false,
            #[cfg(debug_assertions)]
            Self::Smoke(_) => true,
        }
    }

    pub(crate) fn log_directory(&self) -> Option<PathBuf> {
        match self {
            Self::Normal => None,
            #[cfg(debug_assertions)]
            Self::Smoke(session) => Some(session.root.join("logs")),
        }
    }

    pub(crate) fn webview_directory(&self) -> Option<PathBuf> {
        match self {
            Self::Normal => None,
            #[cfg(debug_assertions)]
            Self::Smoke(session) => Some(session.root.join("webview")),
        }
    }

    pub(crate) async fn bootstrap(
        &self,
        resource_dir: Option<PathBuf>,
    ) -> anyhow::Result<AppState> {
        match self {
            Self::Normal => AppState::bootstrap(resource_dir).await,
            #[cfg(debug_assertions)]
            Self::Smoke(session) => {
                // Each retry gets a new DB, never recovery/import from a failed attempt.
                check_plain_directory(&session.root)?;
                let index = session
                    .next_attempt
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let data = session.root.join(format!("attempt-{index}"));
                std::fs::create_dir(&data).context("UI smoke attempt must be fresh")?;
                AppState::bootstrap_ui_smoke(SmokeAttempt { data }).await
            }
        }
    }

    pub(crate) fn check_command(&self, command: &str) -> Result<(), CommandError> {
        if !self.is_smoke() || smoke_read_command(command) {
            return Ok(());
        }
        Err(CommandError::code("UiSmokeUnavailable", format!(
            "UI smoke: {command} không khả dụng; chỉ đọc dữ liệu cô lập, không thiết bị/Google/network hay tác vụ thật"
        )))
    }

    pub(crate) fn configure_context<R: tauri::Runtime>(
        &self,
        #[cfg_attr(not(debug_assertions), allow(unused_variables))] context: &mut tauri::Context<R>,
    ) -> anyhow::Result<()> {
        if !self.is_smoke() {
            return Ok(());
        }
        #[cfg(debug_assertions)]
        {
            // Plugins bypass invoke_handler. Replace (never augment) the compiled ACL
            // before Builder::build can create a WebView. Locked Tauri API, tested below.
            let mut resolved = tauri::utils::acl::resolved::Resolved::default();
            for command in ["plugin:event|listen", "plugin:event|unlisten"] {
                resolved.allowed_commands.insert(
                    command.into(),
                    vec![tauri::utils::acl::resolved::ResolvedCommand {
                        context: tauri::utils::acl::ExecutionContext::Local,
                        windows: vec!["main".parse()?],
                        ..Default::default()
                    }],
                );
            }
            *context.runtime_authority_mut() =
                tauri::runtime_authority!(Default::default(), resolved);
            // No declarative window may open with the default operational profile first.
            context.config_mut().app.windows.clear();
        }
        Ok(())
    }
}

// Explicit reviewed reads only; no prefix matches and no shared "read-only" exemptions
// (those include update_check, network redirects, driver probes and credential import).
fn smoke_read_command(command: &str) -> bool {
    matches!(
        command,
        "startup_error"
            | "retry_startup"
            | "app_log_directory"
            | "log_frontend_error"
            | "list_devices"
            | "list_device_work_states"
            | "list_jobs"
            | "driver_mode"
            | "driver_degraded_reason"
            | "android_unavailable_reason"
            | "android_tool_problems"
            | "get_stream_settings"
            | "agent_get_settings"
            | "agent_list_statuses"
            | "list_groups"
            | "list_device_metas"
            | "list_materials"
            | "list_apps_library"
            | "list_schedules"
            | "list_op_logs"
            | "analytics_summary"
            | "api_docs"
            | "automation_list"
            | "automation_get"
            | "automation_schedule_list"
            | "flow_action_catalog"
            | "flow_list"
            | "flow_get"
            | "flow_list_runs"
            | "flow_get_run"
            | "orchestration_list"
            | "orchestration_get"
            | "orchestration_list_runs"
            | "orchestration_get_run"
            | "interaction_list"
            | "interaction_get"
            | "interaction_list_target_notes"
            | "interaction_list_artifacts"
            | "nurture_get_settings"
            | "nurture_list_comment_attempts"
            | "nurture_cost_summary"
            | "nurture_session_status"
            | "nurture_session_log"
            | "nurture_session_log_summary"
            | "publish_list"
            | "publish_get"
            | "publish_device_guards"
            | "publish_get_limits"
    )
}

#[cfg(debug_assertions)]
fn claim_scratch(path: &std::path::Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(path.is_absolute(), "UI smoke scratch must be absolute");
    anyhow::ensure!(
        !path.components().any(|part| matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )),
        "UI smoke scratch must not contain relative components"
    );
    #[cfg(windows)]
    anyhow::ensure!(
        matches!(path.components().next(), Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_))),
        "UI smoke scratch must be on a local disk, not a network/device path"
    );
    let parent = path.parent().context("UI smoke scratch needs a parent")?;
    // Reject symlinks/junctions before canonicalization, not only the final component.
    for ancestor in parent.ancestors() {
        check_plain_directory(ancestor)?;
    }
    let root = parent.canonicalize()?.join(
        path.file_name()
            .context("UI smoke scratch needs a fresh directory name")?,
    );
    // Atomic create, not exists + create_dir_all: existing empty dirs are unsafe too.
    std::fs::create_dir(&root).context("UI smoke scratch must not already exist")?;
    Ok(root)
}

#[cfg(debug_assertions)]
fn check_plain_directory(path: &std::path::Path) -> anyhow::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "UI smoke directory must not be a symlink"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        anyhow::ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "UI smoke directory must not be a junction/reparse point"
        );
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn memory_credentials() -> riviu_signing::CredentialStore {
    #[derive(Default)]
    struct MemoryCredentials(parking_lot::Mutex<std::collections::HashMap<String, String>>);
    impl riviu_signing::CredentialBackend for MemoryCredentials {
        fn get(&self, account: &str) -> anyhow::Result<Option<String>> {
            Ok(self.0.lock().get(account).cloned())
        }
        fn set(&self, account: &str, value: &str) -> anyhow::Result<()> {
            self.0.lock().insert(account.into(), value.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> anyhow::Result<()> {
            self.0.lock().remove(account);
            Ok(())
        }
    }
    riviu_signing::CredentialStore::new(std::sync::Arc::new(MemoryCredentials::default()))
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    fn fresh_path() -> PathBuf {
        std::env::temp_dir().join(format!("riviu-ui-smoke-test-{}", uuid::Uuid::new_v4()))
    }

    fn requested(root: &std::path::Path) -> StartupPolicy {
        StartupPolicy::from_lookup(true, |name| match name {
            "RIVIU_UI_SMOKE" | "RIVIU_MOCK_DEVICES" => Some("1".into()),
            "RIVIU_UI_SMOKE_DIR" => Some(root.as_os_str().to_owned()),
            key if WEBVIEW_ENV_OVERRIDES.contains(&key) => None,
            // Synthetic hostile environment. Policy must never ask for these values.
            _ => panic!("smoke consulted operational environment: {name}"),
        })
        .unwrap()
    }

    #[test]
    fn invalid_requests_fail_before_creating_any_scratch() {
        let root = fresh_path();
        for (debug, opt_in, mock, directory) in [
            (true, Some("1"), None, Some(root.clone())),
            (true, Some("1"), Some("0"), Some(root.clone())),
            (true, Some("1"), Some("1"), None),
            (true, None, Some("1"), Some(root.clone())),
            (true, Some("true"), Some("1"), Some(root.clone())),
            (false, Some("1"), Some("1"), Some(root.clone())),
            (true, Some("1"), Some("1"), Some("relative-smoke".into())),
        ] {
            let result = StartupPolicy::from_lookup(debug, |name| match name {
                "RIVIU_UI_SMOKE" => opt_in.map(Into::into),
                "RIVIU_MOCK_DEVICES" => mock.map(Into::into),
                "RIVIU_UI_SMOKE_DIR" => directory.clone().map(Into::into),
                key if WEBVIEW_ENV_OVERRIDES.contains(&key) => None,
                _ => panic!("unexpected environment read: {name}"),
            });
            assert!(result.is_err());
            assert!(!root.exists());
        }
    }

    #[test]
    fn a_normal_start_does_not_read_mock_or_secret_environment() {
        let policy = StartupPolicy::from_lookup(true, |name| {
            assert!(matches!(name, "RIVIU_UI_SMOKE" | "RIVIU_UI_SMOKE_DIR"));
            None
        })
        .unwrap();
        assert!(!policy.is_smoke());
        assert!(policy.log_directory().is_none());
        assert!(policy.check_command("google_login_start").is_ok());
    }

    #[test]
    fn webview_environment_overrides_cannot_escape_the_scratch_profile() {
        for override_name in [
            "WEBVIEW2_USER_DATA_FOLDER",
            "WEBVIEW2_BROWSER_EXECUTABLE_FOLDER",
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS",
            "WEBVIEW2_PIPE_FOR_SCRIPT_DEBUGGER",
        ] {
            let root = fresh_path();
            let result = StartupPolicy::from_lookup(true, |name| match name {
                "RIVIU_UI_SMOKE" | "RIVIU_MOCK_DEVICES" => Some("1".into()),
                "RIVIU_UI_SMOKE_DIR" => Some(root.clone().into()),
                key if key == override_name => Some("synthetic-override-must-not-be-used".into()),
                _ => None,
            });
            let refused = result.is_err();
            drop(result);
            if root.exists() {
                std::fs::remove_dir_all(&root).unwrap();
            }
            assert!(
                refused,
                "{override_name} must be refused before WebView creation"
            );
        }
    }

    #[test]
    fn normal_context_keeps_the_compiled_acl_and_windows() {
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        let windows = serde_json::to_value(&context.config().app.windows).unwrap();
        let before = context.runtime_authority_mut().resolve_access(
            "plugin:dialog|open",
            "main",
            "main",
            &tauri::ipc::Origin::Local,
        );
        assert!(before.is_some());
        StartupPolicy::Normal
            .configure_context(&mut context)
            .unwrap();
        assert_eq!(
            before,
            context.runtime_authority_mut().resolve_access(
                "plugin:dialog|open",
                "main",
                "main",
                &tauri::ipc::Origin::Local
            )
        );
        assert_eq!(
            windows,
            serde_json::to_value(&context.config().app.windows).unwrap()
        );
    }

    #[test]
    fn scratch_must_be_fresh_even_when_empty() {
        let root = fresh_path();
        std::fs::create_dir(&root).unwrap();
        assert!(StartupPolicy::from_lookup(true, |name| match name {
            "RIVIU_UI_SMOKE" | "RIVIU_MOCK_DEVICES" => Some("1".into()),
            "RIVIU_UI_SMOKE_DIR" => Some(root.clone().into()),
            _ => None,
        })
        .is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn credentials_are_memory_only_without_any_system_fallback() {
        let store = memory_credentials();
        assert!(!store.has_agent_token().unwrap());
        assert!(store.app_secret("google-oauth-tokens").unwrap().is_none());
        store
            .set_app_secret("test-secret", "synthetic-not-operational")
            .unwrap();
        assert_eq!(
            store.app_secret("test-secret").unwrap().as_deref(),
            Some("synthetic-not-operational")
        );
        assert!(memory_credentials()
            .app_secret("test-secret")
            .unwrap()
            .is_none());
        store.set_app_secret("test-secret", "").unwrap();
        assert!(store.app_secret("test-secret").unwrap().is_none());
    }

    #[test]
    fn smoke_denies_google_effects_and_unknown_commands_before_dispatch() {
        let root = fresh_path();
        let policy = requested(&root);
        for command in [
            "google_connection_status",
            "google_login_start",
            "google_app_config",
            "publish_sheet_check",
            "publish_execute",
            "update_check",
            "refresh_devices",
            "gui_service_status",
            "interaction_resolve_links",
            "device_tap",
            "new_unreviewed_command",
            "plugin:updater|check",
        ] {
            let error = policy.check_command(command).unwrap_err();
            assert_eq!(error.code, "UiSmokeUnavailable", "{command}");
            assert!(error.message.contains("UI smoke"));
        }
        for command in [
            "startup_error",
            "retry_startup",
            "list_devices",
            "list_groups",
            "list_jobs",
            "automation_list",
            "flow_list",
            "publish_list",
            "app_log_directory",
        ] {
            assert!(policy.check_command(command).is_ok(), "{command}");
        }
        drop(policy);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn actual_tauri_acl_denies_plugin_effects_before_plugin_dispatch() {
        let root = fresh_path();
        let policy = requested(&root);
        let mut context: tauri::Context<tauri::Wry> = tauri::generate_context!();
        policy.configure_context(&mut context).unwrap();
        let authority = context.runtime_authority_mut();
        for command in [
            "plugin:updater|check",
            "plugin:updater|download_and_install",
            "plugin:dialog|open",
            "plugin:webview|create_webview",
            "plugin:window|create",
            "plugin:app|exit",
            "plugin:path|resolve_directory",
            "plugin:event|emit",
            "plugin:unreviewed|effect",
        ] {
            assert!(
                authority
                    .resolve_access(command, "main", "main", &tauri::ipc::Origin::Local)
                    .is_none(),
                "{command}"
            );
        }
        for command in ["plugin:event|listen", "plugin:event|unlisten"] {
            assert!(authority
                .resolve_access(command, "main", "main", &tauri::ipc::Origin::Local)
                .is_some());
            assert!(authority
                .resolve_access(command, "other", "other", &tauri::ipc::Origin::Local)
                .is_none());
            assert!(authority
                .resolve_access(
                    command,
                    "main",
                    "main",
                    &tauri::ipc::Origin::Remote {
                        url: "https://example.invalid".parse().unwrap()
                    }
                )
                .is_none());
        }
        assert!(
            context.config().app.windows.is_empty(),
            "no pre-created WebView may use the operator profile"
        );
        drop(policy);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn bootstrap_retry_and_teardown_stay_inside_the_claimed_scratch() {
        let root = fresh_path();
        let policy = requested(&root);
        let first = policy
            .bootstrap(Some("untrusted-sidecar-path".into()))
            .await
            .unwrap();
        assert!(first.is_ui_smoke());
        assert_eq!(first.driver_mode, riviu_ios_driver::DriverMode::Mock);
        assert!(first.android.is_none());
        assert!(first.comment_verifications.is_none());
        assert!(first
            .registry
            .list()
            .iter()
            .all(|d| d.udid.starts_with("MOCK-")));
        assert!(!first.agent_token_configured);
        assert!(first.ensure_accepting_work().is_err());
        let lease = first
            .control
            .open_manual_session("MOCK-IPHONE-01", riviu_core::DeviceWorkOwner::ManualControl)
            .await;
        let plane_was_stopped = matches!(
            lease,
            Err(riviu_core::DeviceControlError::ControlPlaneStopped)
        );
        if let Ok(context) = lease {
            first.control.close_manual_session(context).unwrap();
        }
        if !plane_was_stopped {
            first.control.shutdown_cleanup().await.unwrap();
        }
        assert!(
            plane_was_stopped,
            "the mock control-plane cleanup task must already be joined during bootstrap"
        );
        first
            .db
            .set_setting("smoke-first-attempt-only", "do not import")
            .unwrap();
        first
            .secrets
            .set_app_secret("probe", "synthetic-only")
            .unwrap();
        let second = policy.bootstrap(None).await.unwrap();
        assert_ne!(first.artifacts_dir, second.artifacts_dir);
        assert!(second
            .db
            .get_setting("smoke-first-attempt-only")
            .unwrap()
            .is_none());
        assert!(second.secrets.app_secret("probe").unwrap().is_none());
        assert!(first
            .artifacts_dir
            .starts_with(policy.log_directory().unwrap().parent().unwrap()));
        for state in [&first, &second] {
            tokio::time::timeout(std::time::Duration::from_secs(1), state.shutdown_ui_smoke())
                .await
                .unwrap()
                .unwrap();
            // The two Tauri exit events may both call cleanup.
            state.shutdown_ui_smoke().await.unwrap();
        }
        drop((first, second, policy));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn failed_attempt_does_not_import_a_database_or_fall_back_to_live() {
        let root = fresh_path();
        let policy = requested(&root);
        let attempt = root.join("attempt-0");
        std::fs::create_dir(&attempt).unwrap();
        std::fs::write(attempt.join("riviu.db"), b"foreign db must not be opened").unwrap();
        assert!(policy.bootstrap(None).await.is_err());
        assert_eq!(
            std::fs::read(attempt.join("riviu.db")).unwrap(),
            b"foreign db must not be opened"
        );
        let retry = policy.bootstrap(None).await.unwrap();
        assert!(retry.is_ui_smoke());
        retry.shutdown_ui_smoke().await.unwrap();
        drop((retry, policy));
        std::fs::remove_dir_all(root).unwrap();
    }

    // The unsafe baseline cannot be executed: it opens the operator's credential store.
    // These boundary regressions pin the ordering until the isolated path can be exercised.
    #[test]
    fn ui_smoke_policy_precedes_startup_side_effects() {
        let source = include_str!("lib.rs");
        let run = source.split("pub fn run() {").nth(1).unwrap();
        let policy = run
            .find("ui_smoke::StartupPolicy::from_environment()")
            .expect("parse and validate UI smoke before starting the desktop");
        for effect in [
            "install_panic_logging()",
            "install_process_tree_guard()",
            "tauri::Builder::default()",
        ] {
            assert!(
                policy < run.find(effect).unwrap(),
                "policy must precede {effect}"
            );
        }
    }

    #[test]
    fn ui_smoke_retry_uses_the_pinned_startup_policy() {
        let source = include_str!("lib.rs");
        let retry = source
            .split("async fn retry_startup(")
            .nth(1)
            .unwrap()
            .split("/// The message a panic")
            .next()
            .unwrap();
        assert!(
            retry.contains("policy.bootstrap(resource_dir).await"),
            "retry must not re-read operational env or fall back to live bootstrap"
        );
    }
}
