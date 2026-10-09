//! "Cài lại TikTok 45.7.3 + đăng nhập": one phone, one Repair lease, eight recorded stages.
//!
//! Kept apart from the generic library uninstall (`farm_commands::app_removal`) on purpose:
//! that path removes whatever library app an operator picks; this one knows it is TikTok
//! Global, needs a saved login and ends with a login. It reuses the removal path's own
//! uninstall, artifact snapshot and install internals under the same lease kind.
//!
//! Stage rules:
//!
//! - Every stage outcome is written to the op log (`tiktok.relogin.<stage>`) when it is
//!   decided. A stage that cannot be recorded stops the run before the next effect.
//! - Logging out is best effort: uninstalling removes the session from the phone anyway, so
//!   a logout that cannot be done through the UI is recorded and the run continues.
//! - Uninstall, install and the login submit are each sent at most once. An unproven result
//!   is `uncertain`, ends the run, and is never retried here.
//! - The login submit is preceded by its intent: an op log row and the credential index
//!   status `loginSubmitted`. Until the operator saves the password again, a later run on the
//!   same account refuses at preflight if that submit, or a wrong password, is unresolved.
//! - The session is opened without a GUI scope, so no hierarchy trace (which would include
//!   the typed username) is recorded, and no screenshot is taken.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use riviu_core::tiktok_credentials::{canonical_tiktok_handle, TikTokIdentifierKind};
use riviu_core::tiktok_labels::{controls_for, TikTokControls};
use riviu_core::tiktok_login::{
    self, FirstRunOutcome, LoginOutcome, LogoutOutcome, GLOBAL_PACKAGE, GLOBAL_SIGNER_SHA256,
    GLOBAL_VERSION, GLOBAL_VERSION_CODE,
};
use riviu_core::{
    AppInstallStatus, AppLibraryItem, AppLibraryPlatform, DeviceControlError, DevicePlatform,
    DeviceWorkOwner, UiSession, UiSessionContext,
};
use serde::Serialize;
use tokio::time::Instant;

use crate::command_error::CommandError;
use crate::farm_commands::app_removal::{
    install_prepared, prepare_reinstall, snapshot_reinstall_artifact, uninstall_confirmed,
    AppRemovalOutcome, UninstallStep,
};
use crate::state::AppState;

/// Index statuses that a new run must not paper over: the last submit is unresolved, or
/// TikTok refused the saved password. Saving the password again clears them.
const BLOCKING_STATUSES: &[&str] = &[
    "loginSubmitted",
    "loginUncertain",
    "loginStopped:wrongPassword",
    "loginStopped:tooManyAttempts",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReloginStage {
    Preflight,
    Logout,
    Uninstall,
    Install,
    FirstRun,
    Login,
    SecuritySheet,
    ProveHandle,
}

impl ReloginStage {
    fn key(self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Logout => "logout",
            Self::Uninstall => "uninstall",
            Self::Install => "install",
            Self::FirstRun => "firstRun",
            Self::Login => "login",
            Self::SecuritySheet => "securitySheet",
            Self::ProveHandle => "proveHandle",
        }
    }
}

/// `failed`: the stage did not complete and sent nothing it could not prove.
/// `uncertain`: the stage's effect was sent and its result is not proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StageOutcome {
    Done,
    Skipped,
    Failed,
    Uncertain,
}

impl StageOutcome {
    fn key(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloginStageResult {
    pub stage: ReloginStage,
    pub outcome: StageOutcome,
    pub detail: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TikTokReloginReport {
    pub run_id: String,
    pub udid: String,
    /// The phone's assigned nick, without `@`; empty when none was assigned.
    pub handle: String,
    /// True only when the last stage proved the expected handle on the phone.
    pub completed: bool,
    pub observed_handle: Option<String>,
    pub stages: Vec<ReloginStageResult>,
}

struct Journal<'a> {
    state: &'a AppState,
    report: TikTokReloginReport,
}

impl Journal<'_> {
    /// Record one stage outcome durably. An unrecordable outcome stops the run.
    fn record(
        &mut self,
        stage: ReloginStage,
        outcome: StageOutcome,
        detail: impl Into<String>,
    ) -> Result<(), ()> {
        let detail = detail.into();
        let written = self.state.db.log_op(
            &format!("tiktok.relogin.{}", stage.key()),
            &format!(
                "{} {} @{} {}: {detail}",
                self.report.run_id,
                self.report.udid,
                self.report.handle,
                outcome.key()
            ),
        );
        self.report.stages.push(ReloginStageResult {
            stage,
            outcome,
            detail,
            at: chrono::Utc::now().to_rfc3339(),
        });
        written.map_err(|error| {
            self.report.stages.push(ReloginStageResult {
                stage,
                outcome: StageOutcome::Failed,
                detail: format!(
                    "Không ghi được kết quả bước vào nhật ký; dừng trước bước tiếp theo: {error:#}"
                ),
                at: chrono::Utc::now().to_rfc3339(),
            });
        })
    }

    fn status(&self, status: &str) {
        if let Err(error) = self
            .state
            .db
            .set_tiktok_credential_status(&self.report.handle, Some(status))
        {
            log::warn!(
                "không ghi được trạng thái đăng nhập TikTok của @{}: {error:#}",
                self.report.handle
            );
        }
    }
}

pub(super) async fn run(
    state: &AppState,
    udid: String,
    app_id: String,
) -> Result<TikTokReloginReport, CommandError> {
    if udid.trim().is_empty() || app_id.trim().is_empty() {
        return Err(CommandError::invalid_argument(
            "Cần chọn máy và gói TikTok 45.7.3 trong thư viện ứng dụng.",
        ));
    }
    let run_id = format!("relogin-{}", uuid::Uuid::new_v4());
    let assigned = state
        .db
        .get_device_meta(&udid)
        .map_err(CommandError::operation)?
        .handle;
    let handle = canonical_tiktok_handle(&assigned).unwrap_or_default();
    let mut journal = Journal {
        state,
        report: TikTokReloginReport {
            run_id: run_id.clone(),
            udid: udid.clone(),
            handle: handle.clone(),
            completed: false,
            observed_handle: None,
            stages: Vec::new(),
        },
    };
    let _ = stages(&mut journal, &udid, &app_id, &run_id).await;
    let report = journal.report;
    let done = report
        .stages
        .iter()
        .filter(|stage| stage.outcome == StageOutcome::Done)
        .count();
    let _ = state.db.log_op(
        "tiktok.relogin.finish",
        &format!(
            "{run_id} {udid} @{} completed={} {done}/{} bước xong",
            report.handle,
            report.completed,
            report.stages.len()
        ),
    );
    Ok(report)
}

/// Runs until the first stage that ends the run. `Err(())` means "stopped"; the reason is
/// already in the journal.
async fn stages(
    journal: &mut Journal<'_>,
    udid: &str,
    app_id: &str,
    run_id: &str,
) -> Result<(), ()> {
    use ReloginStage::*;
    use StageOutcome::*;
    let state = journal.state;
    let handle = journal.report.handle.clone();

    // ---- 0. Preflight: nothing below touches the phone until every check passed. ----
    if handle.is_empty() {
        return fail(
            journal,
            Preflight,
            "Máy chưa gán nick TikTok; gán nick cho máy trước khi cài lại và đăng nhập.".into(),
        );
    }
    let login = match state.secrets.tiktok_login(&handle) {
        Ok(Some(login)) => login,
        Ok(None) => {
            return fail(
                journal,
                Preflight,
                format!("Chưa lưu username/mật khẩu cho @{handle}; nhập trong mục Đăng nhập TikTok của máy."),
            )
        }
        Err(error) => {
            return fail(
                journal,
                Preflight,
                format!("Không đọc được mật khẩu đã lưu của @{handle}: {error:#}"),
            )
        }
    };
    if login.identifier_kind() == TikTokIdentifierKind::Phone {
        return fail(
            journal,
            Preflight,
            "Đăng nhập đã lưu là số điện thoại; Riviu chỉ đăng nhập bằng username hoặc email."
                .into(),
        );
    }
    match state.db.get_tiktok_credential_index(&handle) {
        Ok(Some(entry)) => {
            if let Some(status) = entry
                .last_status
                .as_deref()
                .filter(|status| BLOCKING_STATUSES.contains(status))
            {
                return fail(
                    journal,
                    Preflight,
                    format!(
                        "Lần đăng nhập trước của @{handle} chưa được giải quyết ({status}); \
                         kiểm tra máy bằng tay rồi lưu lại mật khẩu để chạy lại."
                    ),
                );
            }
        }
        Ok(None) => {
            return fail(
                journal,
                Preflight,
                format!("Mật khẩu của @{handle} chưa có trong danh mục; lưu lại mật khẩu."),
            )
        }
        Err(error) => {
            return fail(
                journal,
                Preflight,
                format!("Chưa đọc được danh mục đăng nhập: {error:#}"),
            )
        }
    }
    let item = match library_item(state, app_id) {
        Ok(item) => item,
        Err(detail) => return fail(journal, Preflight, detail),
    };
    match state.control.list_devices().await {
        Ok(devices) => match devices.iter().find(|device| device.udid == udid) {
            Some(device) if device.platform == DevicePlatform::Android => {}
            Some(_) => return fail(journal, Preflight, "Chỉ hỗ trợ điện thoại Android.".into()),
            None => return fail(journal, Preflight, "Máy chưa kết nối.".into()),
        },
        Err(error) => {
            return fail(
                journal,
                Preflight,
                format!("Chưa đọc được danh sách máy: {error}"),
            )
        }
    }
    if let Some(detail) = pending_effect(state, udid) {
        return fail(journal, Preflight, detail);
    }
    let exclusive = match state
        .control
        .try_acquire_exclusive(udid, DeviceWorkOwner::Repair)
        .await
    {
        Ok(context) => context,
        Err(DeviceControlError::Busy(busy)) => {
            return fail(
                journal,
                Preflight,
                format!(
                    "Máy đang bận ({:?}); không chờ và không chen ngang việc đang chạy.",
                    busy.current_owner
                ),
            )
        }
        Err(DeviceControlError::PendingPublication { reason, .. }) => {
            return fail(journal, Preflight, reason)
        }
        Err(error) => return fail(journal, Preflight, format!("Chưa giữ được máy: {error}")),
    };
    // Under the lease: the same re-check the removal path does.
    if let Some(detail) = pending_effect(state, udid) {
        let _ = state.control.close_exclusive_context(exclusive);
        return fail(journal, Preflight, detail);
    }
    let artifact = match snapshot_reinstall_artifact(state, item, run_id) {
        Ok(artifact) => artifact,
        Err(error) => {
            let _ = state.control.close_exclusive_context(exclusive);
            return fail(
                journal,
                Preflight,
                format!(
                    "Gói APK trong thư viện không còn khớp SHA-256 lúc nhập: {}",
                    error.message
                ),
            );
        }
    };
    let plan = artifact.plan(GLOBAL_PACKAGE);
    let prepared = match prepare_reinstall(&state.control, (&exclusive).into(), &plan).await {
        Ok(prepared) => prepared,
        Err(detail) => {
            let _ = state.control.close_exclusive_context(exclusive);
            return fail(
                journal,
                Preflight,
                format!("Chưa chuẩn bị được gói cài: {detail}"),
            );
        }
    };
    let context = match state.control.start_owned_ui_session(exclusive).await {
        Ok(context) => context,
        Err(error) => {
            return fail(
                journal,
                Preflight,
                format!("Chưa mở được phiên điều khiển máy: {error}"),
            )
        }
    };
    let recorded = journal.record(
        Preflight,
        Done,
        format!(
            "APK {} {GLOBAL_VERSION} ({GLOBAL_VERSION_CODE}) sha256 {} khớp; đã có mật khẩu của @{handle}; không có việc đăng bài/tương tác đang chờ.",
            artifact.item.name, artifact.item.sha256
        ),
    );
    let result = match recorded {
        Ok(()) => with_session(journal, udid, &context, &login, prepared, &artifact.item).await,
        Err(()) => Err(()),
    };
    if let Err(error) = state.control.close_session_context(context) {
        log::warn!("relogin {udid}: không đóng được phiên điều khiển: {error}");
    }
    drop(artifact);
    result
}

/// Stages 1–7, all under the lease the session context holds.
async fn with_session(
    journal: &mut Journal<'_>,
    udid: &str,
    context: &UiSessionContext,
    login: &riviu_signing::TikTokLogin,
    prepared: crate::farm_commands::app_removal::PreparedInstall,
    item: &AppLibraryItem,
) -> Result<(), ()> {
    use ReloginStage::*;
    use StageOutcome::*;
    let state = journal.state;
    let control = &state.control;
    let handle = journal.report.handle.clone();
    let session = match control.session(context) {
        Ok(session) => session,
        Err(error) => {
            let _ = journal.record(
                Logout,
                Failed,
                format!("Phiên điều khiển không dùng được: {error}"),
            );
            return Err(());
        }
    };
    let session: &dyn UiSession = session.as_ref();

    // ---- 1. Logout (best effort; uninstall clears the session anyway). ----
    let installed = control
        .list_installed_apps(udid)
        .await
        .map(|apps| apps.iter().any(|app| app.bundle_id == GLOBAL_PACKAGE));
    match installed {
        Ok(false) => journal.record(Logout, Skipped, "TikTok Global chưa cài trên máy.")?,
        Err(error) => journal.record(
            Logout,
            Skipped,
            format!("Chưa đọc được danh sách ứng dụng ({error}); bỏ qua đăng xuất, bước gỡ sẽ đọc lại."),
        )?,
        Ok(true) => match labels(state, udid).await {
            Err(detail) => journal.record(
                Logout,
                Skipped,
                format!("{detail}; không đăng xuất qua giao diện, gỡ cài đặt sẽ xóa phiên đăng nhập trên máy."),
            )?,
            Ok(labels) => {
                let deadline = Instant::now() + Duration::from_secs(120);
                match tiktok_login::log_out(session, labels, deadline).await {
                    Ok(LogoutOutcome::LoggedOut { previous_handle }) => journal.record(
                        Logout,
                        Done,
                        format!(
                            "Đã đăng xuất{}; thấy màn hình đăng nhập.",
                            previous_handle
                                .map(|seen| format!(" @{seen}"))
                                .unwrap_or_default()
                        ),
                    )?,
                    Ok(LogoutOutcome::AlreadyLoggedOut) => journal.record(
                        Logout,
                        Skipped,
                        "TikTok đang ở trạng thái chưa đăng nhập.",
                    )?,
                    Ok(LogoutOutcome::Unverified {
                        previous_handle,
                        reason,
                    }) => journal.record(
                        Logout,
                        Uncertain,
                        format!(
                            "Đã bấm Log out{} nhưng chưa xác nhận được: {reason}. Vẫn gỡ cài đặt vì gỡ sẽ xóa phiên.",
                            previous_handle
                                .map(|seen| format!(" (đang là @{seen})"))
                                .unwrap_or_default()
                        ),
                    )?,
                    Err(error) => journal.record(
                        Logout,
                        Skipped,
                        format!("Không đăng xuất được qua giao diện: {error:#}. Vẫn gỡ cài đặt vì gỡ sẽ xóa phiên."),
                    )?,
                }
            }
        },
    }

    // ---- 2. Uninstall. ----
    match uninstall_confirmed(control, &state.db, context.into(), udid, GLOBAL_PACKAGE).await {
        UninstallStep::Removed => journal.record(
            Uninstall,
            Done,
            "Đã gỡ TikTok Global; danh sách ứng dụng không còn gói.",
        )?,
        UninstallStep::NotPresent => journal.record(
            Uninstall,
            Skipped,
            "TikTok Global không có trên máy; không cần gỡ.",
        )?,
        UninstallStep::Failed { outcome, detail } => {
            let outcome = if outcome == AppRemovalOutcome::UnknownAfterDispatch {
                Uncertain
            } else {
                Failed
            };
            let _ = journal.record(Uninstall, outcome, detail);
            return Err(());
        }
    }

    // ---- 3. Install, once. ----
    if let Err(error) = state.db.log_op(
        "app.reinstall.dispatch",
        &format!("{udid} {GLOBAL_PACKAGE} {}", journal.report.run_id),
    ) {
        let _ = journal.record(
            Install,
            Failed,
            format!("Đã gỡ nhưng chưa ghi được ý định cài nên chưa cài; cần cài thủ công: {error}"),
        );
        return Err(());
    }
    let install = install_prepared(control, context.into(), item, prepared, udid).await;
    match install.status {
        AppInstallStatus::Succeeded => journal.record(
            Install,
            Done,
            format!(
                "Đã cài TikTok Global {}.",
                install
                    .observed_version_name
                    .as_deref()
                    .unwrap_or(GLOBAL_VERSION)
            ),
        )?,
        AppInstallStatus::BeforeEffect | AppInstallStatus::CancelledBeforeDispatch => {
            let _ = journal.record(
                Install,
                Failed,
                format!(
                    "Chưa gửi lệnh cài; máy đang không có TikTok, cần cài lại: {}",
                    install.detail.as_deref().unwrap_or("không có chi tiết")
                ),
            );
            return Err(());
        }
        AppInstallStatus::FailedVerified | AppInstallStatus::Uncertain => {
            let _ = journal.record(
                Install,
                Uncertain,
                format!(
                    "Cài chưa được xác nhận ({:?}); cần kiểm lại trên máy, không tự cài lại: {}",
                    install.status,
                    install.detail.as_deref().unwrap_or("không có chi tiết")
                ),
            );
            return Err(());
        }
    }

    // ---- 4. First-run screens of the fresh install. ----
    let labels = match labels(state, udid).await {
        Ok(labels) => labels,
        Err(detail) => {
            let _ = journal.record(FirstRun, Failed, detail);
            return Err(());
        }
    };
    let deadline = Instant::now() + Duration::from_secs(150);
    match tiktok_login::pass_first_run(session, labels, deadline).await {
        Ok(FirstRunOutcome::FeedReached) => journal.record(
            FirstRun,
            Done,
            "Đã qua các màn hình lần đầu; thấy trang chủ.",
        )?,
        Ok(FirstRunOutcome::Blocked(reason)) => {
            let _ = journal.record(FirstRun, Failed, reason);
            return Err(());
        }
        Err(error) => {
            let _ = journal.record(FirstRun, Failed, format!("{error:#}"));
            return Err(());
        }
    }

    // ---- 5. Login: one submit. ----
    let submitted = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(240);
    let outcome = {
        let report = &journal.report;
        let db = &state.db;
        let submitted = &submitted;
        tiktok_login::log_in(
            session,
            labels,
            login.identifier(),
            login.password().expose(),
            deadline,
            move || {
                db.log_op(
                    "tiktok.relogin.login.submit",
                    &format!(
                        "{} {} @{} gửi đăng nhập một lần",
                        report.run_id, report.udid, report.handle
                    ),
                )?;
                db.set_tiktok_credential_status(&report.handle, Some("loginSubmitted"))?;
                submitted.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
    };
    match outcome {
        Ok(LoginOutcome::LoggedIn) => {
            journal.status("loggedIn");
            journal.record(Login, Done, "Đã đăng nhập; TikTok vào trang chủ.")?;
        }
        Ok(LoginOutcome::StoppedBeforeSubmit(reason)) => {
            let _ = journal.record(Login, Failed, format!("Chưa gửi đăng nhập: {reason}"));
            return Err(());
        }
        Ok(LoginOutcome::Rejected(stop)) => {
            journal.status(&format!("loginStopped:{}", stop.as_str()));
            let _ = journal.record(Login, Failed, stop.operator_reason());
            return Err(());
        }
        Ok(LoginOutcome::Uncertain(reason)) => {
            journal.status("loginUncertain");
            let _ = journal.record(Login, Uncertain, reason);
            return Err(());
        }
        Err(error) => {
            let outcome = if submitted.load(Ordering::SeqCst) {
                journal.status("loginUncertain");
                Uncertain
            } else {
                Failed
            };
            let _ = journal.record(Login, outcome, format!("{error:#}"));
            return Err(());
        }
    }

    // ---- 6. Security sheet: Close only. ----
    let deadline = Instant::now() + Duration::from_secs(60);
    match tiktok_login::close_post_login_sheets(session, labels, deadline).await {
        Ok(FirstRunOutcome::FeedReached) => journal.record(
            SecuritySheet,
            Done,
            "Trang chủ không còn bảng hay hộp thoại nào che.",
        )?,
        Ok(FirstRunOutcome::Blocked(reason)) => {
            let _ = journal.record(SecuritySheet, Failed, reason);
            return Err(());
        }
        Err(error) => {
            let _ = journal.record(SecuritySheet, Failed, format!("{error:#}"));
            return Err(());
        }
    }

    // ---- 7. Prove the handle through this lease. ----
    let reading = crate::interaction_commands::inspection::read_account_from_session(
        control, &state.db, udid, session,
    )
    .await;
    match reading {
        Ok(reading) => {
            journal.report.observed_handle = reading.observed_handle.clone();
            match reading.status {
                "matched" => {
                    journal.record(
                        ProveHandle,
                        Done,
                        format!("Hồ sơ hiện @{handle}, đúng nick gán cho máy."),
                    )?;
                    journal.report.completed = true;
                }
                "mismatch" => {
                    journal.status("handleMismatch");
                    let _ = journal.record(
                        ProveHandle,
                        Failed,
                        format!(
                            "Hồ sơ hiện @{}, không phải @{handle}.",
                            reading.observed_handle.unwrap_or_default()
                        ),
                    );
                    return Err(());
                }
                other => {
                    let _ = journal.record(
                        ProveHandle,
                        Uncertain,
                        format!("Chưa đọc được nick trên trang Hồ sơ ({other})."),
                    );
                    return Err(());
                }
            }
        }
        Err(error) => {
            let _ = journal.record(ProveHandle, Uncertain, format!("{error:#}"));
            return Err(());
        }
    }
    Ok(())
}

fn fail(journal: &mut Journal<'_>, stage: ReloginStage, detail: String) -> Result<(), ()> {
    let _ = journal.record(stage, StageOutcome::Failed, detail);
    Err(())
}

/// The library row must be the verified Global 45.7.3 APK, identified the way
/// `tools/tiktok-apk/verify_apk.py` identifies it. Its SHA-256 is re-checked when the
/// artifact is snapshotted and its APK metadata when the install set is prepared.
fn library_item(state: &AppState, app_id: &str) -> Result<AppLibraryItem, String> {
    let item = state
        .db
        .list_apps_library()
        .map_err(|error| format!("Chưa đọc được thư viện ứng dụng: {error:#}"))?
        .into_iter()
        .find(|item| item.id == app_id)
        .ok_or_else(|| "Gói đã chọn không còn trong thư viện ứng dụng.".to_string())?;
    let matches = item.platform == AppLibraryPlatform::Android
        && item.application_id == GLOBAL_PACKAGE
        && item.version_name == GLOBAL_VERSION
        && item.version_code.as_deref() == Some(GLOBAL_VERSION_CODE)
        && item
            .signer_sha256
            .eq_ignore_ascii_case(GLOBAL_SIGNER_SHA256)
        && item.sha256.len() == 64;
    if !matches {
        return Err(format!(
            "Gói {} không phải APK TikTok Global {GLOBAL_VERSION} gốc đã xác minh \
             (cần {GLOBAL_PACKAGE}, versionCode {GLOBAL_VERSION_CODE}, chữ ký gốc của TikTok).",
            item.name
        ));
    }
    Ok(item)
}

/// A post still uploading or awaiting proof, or an interaction quarantined on this phone,
/// would lose its evidence with the app.
fn pending_effect(state: &AppState, udid: &str) -> Option<String> {
    match state.db.has_pending_publish_for_device(udid) {
        Ok(false) => {}
        Ok(true) => {
            return Some(
                "Máy còn bài đang tải hoặc chờ xác minh; chưa gỡ để không mất bằng chứng bài đăng."
                    .into(),
            )
        }
        Err(error) => {
            return Some(format!(
                "Chưa đọc được trạng thái bài đang xử lý: {error:#}"
            ))
        }
    }
    if !state
        .control
        .interaction_quarantine_snapshot(udid)
        .is_empty()
    {
        return Some(
            "Máy còn phiên tương tác bị cách ly chưa xử lý; xử lý xong mới cài lại.".into(),
        );
    }
    None
}

async fn labels(state: &AppState, udid: &str) -> Result<TikTokControls, String> {
    let (package, version, language) = state
        .control
        .tiktok_build(udid)
        .await
        .map_err(|error| format!("Chưa đọc được bản TikTok trên máy: {error}"))?;
    let labels = controls_for(&package, &language, &version)
        .filter(|labels| tiktok_login::supported(*labels))
        .ok_or_else(|| {
            format!(
                "Máy đang dùng {package} {version} ngôn ngữ {language}; chỉ hỗ trợ TikTok Global {GLOBAL_VERSION} tiếng Anh"
            )
        })?;
    Ok(labels)
}
