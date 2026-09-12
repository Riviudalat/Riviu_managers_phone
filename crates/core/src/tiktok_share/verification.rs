//! Future publication verification: one tuple, immutable observations and bounded navigation.
use super::*;
use anyhow::Context;
use hierarchy::Tree;
use serde::Serialize;
use std::collections::HashSet;
use tokio::time::Instant;

const CONTRACT_VERSION: u32 = 1;
const RECOVERY_WINDOW: Duration = Duration::from_secs(60);
const CAPTURE_WINDOW: Duration = Duration::from_secs(180);
const MAX_RECOVERY_ACTIONS: u32 = 3;
const MAX_CANDIDATES: u32 = 12;
const MAX_VIEWPORTS: u32 = 3;

#[derive(Debug, Clone, Copy)]
pub struct PublishVerificationPlan {
    pub(super) labels: TikTokControls,
    caption_id: &'static str,
    time_id: &'static str,
}

impl PublishVerificationPlan {
    pub fn for_runtime(package: &str, locale: &str, version: &str) -> anyhow::Result<Self> {
        if let Ok(plan) = Self::for_build(package, locale, version) {
            return Ok(plan);
        }
        let labels = crate::tiktok_labels::controls_for_runtime(package, locale, version)
            .context("Chưa hỗ trợ ngôn ngữ giao diện")?;
        anyhow::ensure!(labels.adaptive(), "verification_contract_unavailable");
        Ok(Self {
            labels,
            caption_id: ":id/desc",
            time_id: ":id/tv_post_time",
        })
    }
    pub fn for_build(package: &str, locale: &str, version: &str) -> anyhow::Result<Self> {
        let package = package.trim();
        let version = version.trim();
        let language = crate::tiktok_labels::normalise_language(locale);
        let locale = language.as_str();
        let labels =
            crate::tiktok_labels::controls_for(package, locale, version).ok_or_else(|| {
                anyhow::anyhow!("Chưa đo giao diện xác minh bài cho phiên bản TikTok này")
            })?;
        anyhow::ensure!(
            crate::tiktok_account::account_read_supported(labels),
            "Chưa đo phép đọc tài khoản của phiên bản TikTok này"
        );
        anyhow::ensure!(
            labels.resource_version() == Some(version),
            "Không có tuple xác minh chính xác"
        );
        let (caption_id, time_id) = match (package, locale, version) {
            // Existing retained own-post fixtures, AGENTS §9.209/9.210.
            ("com.ss.android.ugc.trill", "en", "38.3.2") => (":id/dmk", ":id/qrp"),
            ("com.zhiliaoapp.musically", "en", "45.4.3") => (":id/desc", ":id/zj1"),
            ("com.zhiliaoapp.musically", "en", "45.7.3") => (":id/desc", ":id/zwj"),
            (
                "com.zhiliaoapp.musically",
                "en",
                "46.0.41" | "46.1.3" | "46.2.1" | "46.2.42" | "46.4.3",
            ) => (":id/desc", ":id/tv_post_time"),
            _ => anyhow::bail!("Chưa đo caption và thời gian bài cho tuple TikTok này"),
        };
        anyhow::ensure!(
            [TikTokControl::ProfileTab, TikTokControl::Share]
                .iter()
                .all(|control| labels.label(*control).is_some())
                && labels.post_tile_id().is_some(),
            "Thiếu nhận diện Hồ sơ, bài hoặc Chia sẻ để xác minh"
        );
        Ok(Self {
            labels,
            caption_id,
            time_id,
        })
    }

    pub fn contract_version(&self) -> u32 {
        CONTRACT_VERSION
    }
    pub fn provenance(&self) -> &'static str {
        "android-own-profile-snapshot-v1"
    }
}

/// Exercise the same Helper clipboard primitive as Copy, restoring both content
/// and the keyboard through that primitive on every attempted mutation.
pub async fn probe_clipboard_restore(session: &dyn UiSession) -> anyhow::Result<()> {
    let (kind, prior) = session
        .get_clipboard(CLIPBOARD_LIMIT)
        .await
        .map_err(|_| anyhow::anyhow!("Không đọc được clipboard trước kiểm tra"))?;
    anyhow::ensure!(
        is_text_kind(&kind),
        "Clipboard hiện tại không phải văn bản; chưa thay đổi clipboard"
    );
    let mark = sentinel();
    let probe = async {
        session
            .set_clipboard("plaintext", mark.as_bytes())
            .await
            .map_err(|_| anyhow::anyhow!("Không ghi được clipboard kiểm tra"))?;
        let (kind, bytes) = session
            .get_clipboard(CLIPBOARD_LIMIT)
            .await
            .map_err(|_| anyhow::anyhow!("Không đọc lại được clipboard kiểm tra"))?;
        anyhow::ensure!(
            is_text_kind(&kind) && bytes == mark.as_bytes(),
            "Clipboard kiểm tra không khớp giá trị vừa ghi"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let restored = async {
        session
            .set_clipboard("plaintext", &prior)
            .await
            .map_err(|_| anyhow::anyhow!("Chưa khôi phục được clipboard sau kiểm tra"))?;
        let (kind, bytes) = session
            .get_clipboard(CLIPBOARD_LIMIT)
            .await
            .map_err(|_| anyhow::anyhow!("Không xác nhận được clipboard đã khôi phục"))?;
        anyhow::ensure!(
            is_text_kind(&kind) && bytes == prior,
            "Clipboard sau khôi phục không khớp ban đầu"
        );
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match (probe, restored) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(probe), Err(restore)) => Err(anyhow::anyhow!("{probe}; {restore}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VerificationReason {
    Verified,
    IdentityMissing,
    WrongApp,
    ReadFailed,
    StaleSnapshot,
    ProfileUnavailable,
    NavigationBudgetExhausted,
    ComposerOrUpload,
    LoginRequired,
    UnknownScreen,
    AccountMismatch,
    CaptionMissing,
    CaptionAmbiguous,
    CaptionMismatch,
    CaptionTruncated,
    TimestampMissing,
    TimestampAmbiguous,
    TimestampUnmeasured,
    SubmissionTooOld,
    SubmissionTimeAmbiguous,
    DraftsObserved,
    PostNotVisible,
    SearchBudgetExhausted,
    PaginationUnmeasured,
    GridNotRestored,
    ShareUnavailable,
    CopyUnavailable,
    CopyAmbiguous,
    ClipboardUnwritable,
    ClipboardUnreadable,
    ClipboardUnchanged,
    ClipboardNotPostLink,
    RedirectFailed,
    MultipleMatchingPosts,
}

impl VerificationReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::IdentityMissing => "submissionIdentityMissing",
            Self::WrongApp => "wrongApp",
            Self::ReadFailed => "readFailed",
            Self::StaleSnapshot => "staleSnapshot",
            Self::ProfileUnavailable => "profileUnavailable",
            Self::NavigationBudgetExhausted => "navigationBudgetExhausted",
            Self::ComposerOrUpload => "composerOrUpload",
            Self::LoginRequired => "loginRequired",
            Self::UnknownScreen => "unknownScreen",
            Self::AccountMismatch => "accountMismatch",
            Self::CaptionMissing => "captionMissing",
            Self::CaptionAmbiguous => "captionAmbiguous",
            Self::CaptionMismatch => "captionMismatch",
            Self::CaptionTruncated => "captionTruncated",
            Self::TimestampMissing => "timestampMissing",
            Self::TimestampAmbiguous => "timestampAmbiguous",
            Self::TimestampUnmeasured => "timestampUnmeasured",
            Self::SubmissionTooOld => "submissionTooOld",
            Self::SubmissionTimeAmbiguous => "submissionTimeAmbiguous",
            Self::DraftsObserved => "draftsObserved",
            Self::PostNotVisible => "postNotVisible",
            Self::SearchBudgetExhausted => "searchBudgetExhausted",
            Self::PaginationUnmeasured => "paginationUnmeasured",
            Self::GridNotRestored => "gridNotRestored",
            Self::ShareUnavailable => "shareUnavailable",
            Self::CopyUnavailable => "copyUnavailable",
            Self::CopyAmbiguous => "copyAmbiguous",
            Self::ClipboardUnwritable => "clipboardUnwritable",
            Self::ClipboardUnreadable => "clipboardUnreadable",
            Self::ClipboardUnchanged => "clipboardUnchanged",
            Self::ClipboardNotPostLink => "clipboardNotPostLink",
            Self::RedirectFailed => "redirectFailed",
            Self::MultipleMatchingPosts => "multipleMatchingPosts",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::Verified => "Đã xác minh tài khoản, nội dung, thời gian và liên kết bài đăng.",
            Self::IdentityMissing => "Thiếu bằng chứng tài khoản hoặc thời điểm trước Đăng; chưa xác minh liên kết.",
            Self::WrongApp => "Ứng dụng đang mở khác bản TikTok của lượt đăng; chưa điều hướng để lấy liên kết.",
            Self::ReadFailed => "Đọc màn hình xác minh thất bại; giữ nguyên bài và chờ lần kiểm tra tiếp theo.",
            Self::StaleSnapshot => "Ảnh chụp cây giao diện không mới hơn lần đọc trước; chưa dùng dữ liệu này để thao tác.",
            Self::ProfileUnavailable => "Hết thời gian chờ tab Hồ sơ xuất hiện; chưa tìm được đường đến bài đã gửi.",
            Self::NavigationBudgetExhausted => "Đã dùng hết ba bước phục hồi màn hình; chưa đến được hồ sơ để lấy liên kết.",
            Self::ComposerOrUpload => "TikTok đang ở màn soạn bài hoặc gửi nội dung; giữ nguyên màn hình và chờ kiểm tra lại.",
            Self::LoginRequired => "TikTok đang yêu cầu đăng nhập; cần mở lại đúng tài khoản trước khi kiểm tra liên kết.",
            Self::UnknownScreen => "Màn hình hiện tại chưa được nhận diện; chưa điều hướng hoặc bấm Back.",
            Self::AccountMismatch => "Tài khoản trên hồ sơ hoặc trong liên kết chưa khớp tài khoản ghi nhận trước Đăng.",
            Self::CaptionMissing => "Không đọc được caption của bài đang mở; chưa đủ bằng chứng chọn bài này.",
            Self::CaptionAmbiguous => "Cây giao diện có nhiều caption cùng lúc; chưa xác định được nội dung thuộc bài đang mở.",
            Self::CaptionMismatch => "Caption của bài đang mở khác nội dung đã duyệt cho lượt đăng.",
            Self::CaptionTruncated => "Caption vẫn bị rút gọn hoặc không có nút mở rộng được xác nhận; chưa đọc đủ nội dung để phân biệt đúng bài.",
            Self::TimestampMissing => "Không thấy nhãn thời gian của bài đang mở; chưa chứng minh được bài thuộc lượt vừa gửi.",
            Self::TimestampAmbiguous => "Cây giao diện có nhiều nhãn thời gian; chưa xác định được thời gian của bài đang mở.",
            Self::TimestampUnmeasured => "Định dạng thời gian trên bài chưa được nhận diện; cần bổ sung phép đọc cho phiên bản này.",
            Self::SubmissionTooOld => "Thời gian của bài đang mở không chứng minh được bài được tạo sau lần Đăng này.",
            Self::SubmissionTimeAmbiguous => "Thời gian TikTok đang làm tròn chưa phân biệt được bài mới với bài cũ; chờ kiểm tra lại.",
            Self::DraftsObserved => "Hồ sơ có bản nháp nhưng chưa tìm thấy bài khớp lượt gửi; chưa kết luận bản nháp thuộc lượt này.",
            Self::PostNotVisible => "Hồ sơ chưa hiện bài đăng có thể kiểm tra; chờ TikTok xử lý rồi thử lại.",
            Self::SearchBudgetExhausted => "Đã hết giới hạn tìm bài của lượt kiểm tra; chưa có liên kết được xác minh.",
            Self::PaginationUnmeasured => "Chưa xác định được vùng cuộn của lưới hồ sơ; chưa tìm tiếp các bài phía dưới.",
            Self::GridNotRestored => "Sau khi xem bài, chưa xác nhận đã trở lại đúng lưới hồ sơ; dừng tìm để tránh chạm nhầm.",
            Self::ShareUnavailable => "Bài đã mở nhưng không có nút Chia sẻ khả dụng đúng nhận diện.",
            Self::CopyUnavailable => "Bảng Chia sẻ chưa có nút Sao chép liên kết khả dụng.",
            Self::CopyAmbiguous => "Có nhiều nút Sao chép liên kết cùng khớp; chưa chọn nút để tránh lấy nhầm liên kết.",
            Self::ClipboardUnwritable => "Chưa ghi và đọc lại được giá trị kiểm chứng clipboard; chưa bấm Sao chép liên kết.",
            Self::ClipboardUnreadable => "Không đọc được clipboard sau thao tác; chưa xác nhận đã sao chép liên kết.",
            Self::ClipboardUnchanged => "Đã thử Sao chép liên kết hai lần nhưng clipboard vẫn giữ giá trị kiểm chứng.",
            Self::ClipboardNotPostLink => "Clipboard đã đổi nhưng nội dung không phải liên kết bài TikTok hợp lệ.",
            Self::RedirectFailed => "Đã sao chép liên kết nhưng chưa chuẩn hóa được đường dẫn đến bài TikTok.",
            Self::MultipleMatchingPosts => "Có nhiều bài cùng khớp caption và thời gian trong lưới hồ sơ; chưa chọn liên kết để tránh gán nhầm lượt đăng.",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationDiagnostic {
    pub contract_version: u32,
    pub package: String,
    pub locale: String,
    pub version: String,
    pub stage: &'static str,
    pub reason_code: VerificationReason,
    pub snapshot_generation: u64,
    pub caption_candidates: usize,
    pub time_candidates: usize,
    pub time_label: Option<String>,
    pub navigation_actions: u32,
    pub candidates_visited: u32,
    pub viewports_visited: u32,
    pub copy_attempts: u32,
    pub elapsed_ms: u64,
    pub navigation_matches: usize,
    pub navigation_enabled: usize,
    pub navigation_clickable: usize,
    pub screen_state: String,
}

#[derive(Debug)]
pub struct VerificationCapture {
    pub outcome: OwnPostLink,
    pub diagnostic: VerificationDiagnostic,
}

#[derive(Debug, PartialEq, Eq)]
enum Screen {
    Profile,
    Feed,
    Post,
    Share,
    Dialog,
    Composer,
    Login,
    Unknown,
}

fn classify(tree: &Tree, plan: &PublishVerificationPlan) -> Screen {
    let package = plan.labels.package();
    let has = |control| {
        plan.labels
            .label(control)
            .is_some_and(|label| match label.to_query() {
                ElementQuery::Semantic(role) => {
                    !crate::app_automation::tiktok_roles::indices(tree, package, role).is_empty()
                }
                query => !tree.matching(package, query).is_empty(),
            })
    };
    if has(TikTokControl::ComposerCaption)
        || has(TikTokControl::PostButton)
        || (has(TikTokControl::ComposerShutter)
            && !has(TikTokControl::ProfileTab)
            && !has(TikTokControl::FeedTab))
    {
        return Screen::Composer;
    }
    if tree.nodes.iter().any(|node| {
        node.visible(package)
            && ["Log in", "Sign up for TikTok", "Đăng nhập"].contains(&node.attr("text"))
    }) {
        return Screen::Login;
    }
    if has(TikTokControl::DialogDismiss) {
        return Screen::Dialog;
    }
    if tree.copy_control(package).is_err() || tree.copy_control(package).ok().flatten().is_some() {
        return Screen::Share;
    }
    let own_profile = if package == "com.zhiliaoapp.musically" {
        !tree
            .matching(
                package,
                ElementQuery::Text {
                    value: "Edit",
                    exact: true,
                },
            )
            .is_empty()
            && !tree
                .matching(
                    package,
                    ElementQuery::Description {
                        value: "Profile menu",
                        exact: true,
                    },
                )
                .is_empty()
    } else {
        !tree
            .matching(
                package,
                ElementQuery::Text {
                    value: "Edit profile",
                    exact: true,
                },
            )
            .is_empty()
    };
    if own_profile {
        return Screen::Profile;
    }
    if plan
        .labels
        .post_tile_id()
        .is_some_and(|label| !tree.matching(package, label.to_query()).is_empty())
    {
        return Screen::Profile;
    }
    if has(TikTokControl::ProfileTab) || has(TikTokControl::FeedTab) {
        return Screen::Feed;
    }
    if has(TikTokControl::Share)
        && !tree
            .matching(package, ElementQuery::ResourceIdSuffix(plan.caption_id))
            .is_empty()
    {
        return Screen::Post;
    }
    Screen::Unknown
}

fn submission_identity_valid(identity: &SubmissionIdentity) -> bool {
    let handle = identity.account.trim().trim_start_matches('@');
    !handle.is_empty()
        && handle.len() <= 24
        && !handle.ends_with('.')
        && handle
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.')
        && chrono::DateTime::parse_from_rfc3339(&identity.submitted_at).is_ok()
}

struct Capture<'a> {
    session: &'a dyn UiSession,
    plan: &'a PublishVerificationPlan,
    caption: &'a str,
    identity: &'a SubmissionIdentity,
    started: Instant,
    diagnostic: VerificationDiagnostic,
    caption_expanded: bool,
}

impl Capture<'_> {
    fn expired(&self) -> bool {
        self.started.elapsed() >= CAPTURE_WINDOW
    }
    async fn read(&mut self) -> Result<Tree, VerificationReason> {
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        if self
            .session
            .active_app_bundle()
            .await
            .map_err(|_| VerificationReason::ReadFailed)?
            != self.plan.labels.package()
        {
            return Err(VerificationReason::WrongApp);
        }
        let tree = Tree::parse(
            self.session
                .hierarchy_source_snapshot()
                .await
                .map_err(|_| VerificationReason::ReadFailed)?,
        )
        .map_err(|_| VerificationReason::ReadFailed)?;
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        if tree.generation <= self.diagnostic.snapshot_generation {
            return Err(VerificationReason::StaleSnapshot);
        }
        self.diagnostic.snapshot_generation = tree.generation;
        Ok(tree)
    }

    async fn account(&self) -> Result<(), VerificationReason> {
        let observed = crate::tiktok_account::observe_own_account(self.session, self.plan.labels)
            .await
            .map_err(|_| VerificationReason::ReadFailed)?;
        if !observed.as_deref().is_some_and(|account| {
            account.eq_ignore_ascii_case(self.identity.account.trim_start_matches('@'))
        }) {
            return Err(VerificationReason::AccountMismatch);
        }
        Ok(())
    }

    async fn profile(&mut self, restoring: bool) -> Result<Tree, VerificationReason> {
        let start = Instant::now();
        let mut actions = 0;
        let mut last_screen = None;
        let mut last_action_at: Option<Instant> = None;
        loop {
            let tree = self.read().await?;
            let screen = classify(&tree, self.plan);
            self.diagnostic.screen_state = format!("{screen:?}");
            if screen == Screen::Profile {
                self.account().await?;
                let fresh = self.read().await?;
                if classify(&fresh, self.plan) == Screen::Profile {
                    return Ok(fresh);
                }
                continue;
            }
            if screen == Screen::Composer {
                return Err(VerificationReason::ComposerOrUpload);
            }
            if screen == Screen::Login {
                return Err(VerificationReason::LoginRequired);
            }
            if start.elapsed() >= RECOVERY_WINDOW {
                return Err(if restoring {
                    VerificationReason::GridNotRestored
                } else {
                    VerificationReason::ProfileUnavailable
                });
            }
            if actions >= MAX_RECOVERY_ACTIONS {
                return Err(VerificationReason::NavigationBudgetExhausted);
            }
            let control = match screen {
                Screen::Dialog => Some(TikTokControl::DialogDismiss),
                Screen::Feed => Some(TikTokControl::ProfileTab),
                _ => None,
            };
            if let Some(control) = control {
                if let Some(label) = self.plan.labels.label(control) {
                    let matched = tree.matching(self.plan.labels.package(), label.to_query());
                    self.diagnostic.navigation_matches = matched.len();
                    self.diagnostic.navigation_enabled = matched
                        .iter()
                        .filter(|i| tree.nodes[**i].rect().is_some_and(|r| r.enabled))
                        .count();
                    self.diagnostic.navigation_clickable = matched
                        .iter()
                        .filter(|i| tree.nodes[**i].rect().is_some_and(|r| r.clickable))
                        .count();
                }
                if last_screen.as_ref() != Some(&screen)
                    || last_action_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5))
                {
                    let mut button =
                        tree.control(self.plan.labels.package(), self.plan.labels.label(control));
                    if button.is_none() && control == TikTokControl::ProfileTab {
                        let budget = RECOVERY_WINDOW
                            .saturating_sub(start.elapsed())
                            .min(Duration::from_secs(30));
                        button = crate::ui_automation::runtime::resolve_navigation(
                            self.session,
                            "profile",
                            budget,
                        )
                        .await
                        .unwrap_or(None);
                    }
                    if let Some(button) = button {
                        self.session
                            .tap(button.centre())
                            .await
                            .map_err(|_| VerificationReason::ReadFailed)?;
                        last_screen = Some(screen);
                        last_action_at = Some(Instant::now());
                        actions += 1;
                        self.diagnostic.navigation_actions += 1;
                    }
                }
            } else if screen == Screen::Unknown
                && self.session.gui_reasoner().is_some()
                && last_screen.as_ref() != Some(&screen)
            {
                let remaining = RECOVERY_WINDOW
                    .saturating_sub(start.elapsed())
                    .min(Duration::from_secs(30));
                if let Ok(Some(button)) = crate::ui_automation::runtime::resolve_navigation(
                    self.session,
                    "profile",
                    remaining,
                )
                .await
                {
                    self.session
                        .tap(button.centre())
                        .await
                        .map_err(|_| VerificationReason::ReadFailed)?;
                    actions += 1;
                    self.diagnostic.navigation_actions += 1;
                }
                last_screen = Some(screen);
                last_action_at = Some(Instant::now());
            } else if matches!(screen, Screen::Post | Screen::Share)
                && last_screen.as_ref() != Some(&screen)
            {
                // Back only from a proved post detail/share surface, never a
                // composer, arbitrary activity, or unlabelled screen.
                self.session
                    .back()
                    .await
                    .map_err(|_| VerificationReason::ReadFailed)?;
                last_screen = Some(screen);
                actions += 1;
                self.diagnostic.navigation_actions += 1;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    fn post_proof(&mut self, tree: &Tree) -> Result<(), VerificationReason> {
        let captions = tree.matching(
            self.plan.labels.package(),
            ElementQuery::ResourceIdSuffix(self.plan.caption_id),
        );
        let mut times = tree.matching(
            self.plan.labels.package(),
            ElementQuery::ResourceIdSuffix(self.plan.time_id),
        );
        if times.is_empty() && self.plan.labels.adaptive() {
            times = tree
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, n)| {
                    n.visible(self.plan.labels.package())
                        && tree.ancestors_visible(*i)
                        && n.rect().is_some()
                        && relative_post_age(n.attr("text")).is_some()
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.diagnostic.caption_candidates = captions.len();
        self.diagnostic.time_candidates = times.len();
        let [caption] = captions.as_slice() else {
            return Err(if captions.is_empty() {
                VerificationReason::CaptionMissing
            } else {
                VerificationReason::CaptionAmbiguous
            });
        };
        let visible = tree.nodes[*caption].attr("text");
        let normalize = |value: &str| {
            value
                .trim_matches(['\u{200e}', '\u{200f}'])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        if normalize(visible) != normalize(self.caption) {
            // A matching prefix is only permission to inspect a caption's own
            // expansion control, never proof of this publication's identity.
            return Err(if visible_caption_matches(visible, self.caption) {
                VerificationReason::CaptionTruncated
            } else {
                VerificationReason::CaptionMismatch
            });
        }
        let [time] = times.as_slice() else {
            return Err(if times.is_empty() {
                VerificationReason::TimestampMissing
            } else {
                VerificationReason::TimestampAmbiguous
            });
        };
        let text = tree.nodes[*time].attr("text");
        self.diagnostic.time_label = Some(text.chars().take(80).collect());
        let now = chrono::Utc::now();
        if relative_post_time_matches(text, &self.identity.submitted_at, now) {
            return Ok(());
        }
        if relative_post_age(text).is_none() {
            return Err(VerificationReason::TimestampUnmeasured);
        }
        if submission_time_recheck_delay(text, &self.identity.submitted_at, now).is_some() {
            return Err(VerificationReason::SubmissionTimeAmbiguous);
        }
        Err(VerificationReason::SubmissionTooOld)
    }

    async fn prove_post(&mut self) -> Result<Tree, VerificationReason> {
        let start = Instant::now();
        loop {
            let tree = self.read().await?;
            match self.post_proof(&tree) {
                Ok(()) => return Ok(tree),
                Err(VerificationReason::CaptionTruncated) if !self.caption_expanded => {
                    let controls = tree.matching(
                        self.plan.labels.package(),
                        ElementQuery::ResourceIdSuffix(self.plan.caption_id),
                    );
                    let [index] = controls.as_slice() else {
                        return Err(VerificationReason::CaptionAmbiguous);
                    };
                    let button = tree.nodes[*index]
                        .rect()
                        .ok_or(VerificationReason::CaptionTruncated)?;
                    // Raw retained Global45.7.3/en links/2/post-1.xml (09/09)
                    // shows :id/desc itself clickable with a …more suffix.
                    // Other known tuples must present the same live semantic
                    // proof. Never borrow a parent or tap a text neighbour.
                    if !button.enabled || !button.clickable || self.expired() {
                        return Err(VerificationReason::CaptionTruncated);
                    }
                    self.caption_expanded = true;
                    self.session
                        .tap(button.centre())
                        .await
                        .map_err(|_| VerificationReason::ReadFailed)?;
                    tokio::time::sleep(POLL).await;
                }
                Err(VerificationReason::CaptionMissing) if start.elapsed() < POST_PAGE_WINDOW => {
                    tokio::time::sleep(POLL).await
                }
                Err(VerificationReason::SubmissionTimeAmbiguous) => {
                    let delay = submission_time_recheck_delay(
                        self.diagnostic.time_label.as_deref().unwrap_or_default(),
                        &self.identity.submitted_at,
                        chrono::Utc::now(),
                    )
                    .ok_or(VerificationReason::SubmissionTimeAmbiguous)?;
                    if start.elapsed() + delay >= Duration::from_secs(65)
                        || self.started.elapsed() + delay >= CAPTURE_WINDOW
                    {
                        return Err(VerificationReason::SubmissionTimeAmbiguous);
                    }
                    tokio::time::sleep(delay).await;
                }
                Err(reason) => return Err(reason),
            }
        }
    }

    async fn close_share(&mut self) -> Result<(), VerificationReason> {
        let tree = self.read().await?;
        if classify(&tree, self.plan) == Screen::Share {
            self.session
                .back()
                .await
                .map_err(|_| VerificationReason::ReadFailed)?;
            tokio::time::sleep(POLL).await;
            if classify(&self.read().await?, self.plan) == Screen::Share {
                return Err(VerificationReason::UnknownScreen);
            }
        }
        Ok(())
    }

    async fn copy_once(
        &mut self,
        _post: Tree,
    ) -> Result<(String, VerificationDiagnostic), VerificationReason> {
        let mark = sentinel();
        self.session
            .set_clipboard("plaintext", mark.as_bytes())
            .await
            .map_err(|_| VerificationReason::ClipboardUnwritable)?;
        let (kind, bytes) = self
            .session
            .get_clipboard(CLIPBOARD_LIMIT)
            .await
            .map_err(|_| VerificationReason::ClipboardUnreadable)?;
        if !is_text_kind(&kind) || bytes != mark.as_bytes() {
            return Err(VerificationReason::ClipboardUnwritable);
        }
        let fresh = self.read().await?;
        self.post_proof(&fresh)?;
        let winning_proof = self.diagnostic.clone();
        let share = fresh
            .control(
                self.plan.labels.package(),
                self.plan.labels.label(TikTokControl::Share),
            )
            .ok_or(VerificationReason::ShareUnavailable)?;
        self.session
            .tap(share.centre())
            .await
            .map_err(|_| VerificationReason::ReadFailed)?;
        let start = Instant::now();
        let copy = loop {
            let tree = self.read().await?;
            match tree.copy_control(self.plan.labels.package()) {
                Ok(Some(copy)) => break copy,
                Err(()) => return Err(VerificationReason::CopyAmbiguous),
                Ok(None) if start.elapsed() < SHEET_WINDOW => tokio::time::sleep(POLL).await,
                Ok(None) => return Err(VerificationReason::CopyUnavailable),
            }
        };
        self.diagnostic.copy_attempts += 1;
        self.session
            .tap(copy.centre())
            .await
            .map_err(|_| VerificationReason::ReadFailed)?;
        let start = Instant::now();
        let mut readable = false;
        loop {
            if self.expired() {
                return Err(VerificationReason::SearchBudgetExhausted);
            }
            if let Ok((kind, bytes)) = self.session.get_clipboard(CLIPBOARD_LIMIT).await {
                readable = true;
                if bytes != mark.as_bytes() && !bytes.is_empty() {
                    let value = std::str::from_utf8(&bytes)
                        .map_err(|_| VerificationReason::ClipboardNotPostLink)?;
                    if !is_text_kind(&kind) || !looks_like_a_post_link(value) {
                        return Err(VerificationReason::ClipboardNotPostLink);
                    }
                    let canonical = resolve_canonical_post_link(value)
                        .await
                        .map_err(|_| VerificationReason::RedirectFailed)?;
                    if !canonical_account_matches(&canonical, &self.identity.account) {
                        return Err(VerificationReason::AccountMismatch);
                    }
                    return Ok((canonical, winning_proof));
                }
            }
            if start.elapsed() >= CLIPBOARD_WINDOW {
                return Err(if readable {
                    VerificationReason::ClipboardUnchanged
                } else {
                    VerificationReason::ClipboardUnreadable
                });
            }
            tokio::time::sleep(POLL).await;
        }
    }

    async fn capture(&mut self) -> Result<String, VerificationReason> {
        self.diagnostic.stage = "profile";
        let mut tree = self.profile(false).await?;
        let mut last_reason = VerificationReason::PostNotVisible;
        for page in 0..MAX_VIEWPORTS {
            self.diagnostic.viewports_visited = page + 1;
            let mut visited = HashSet::new();
            let mut candidate = None;
            let mut unresolved_candidate = None;
            loop {
                if self.expired() || self.diagnostic.candidates_visited >= MAX_CANDIDATES {
                    return Err(VerificationReason::SearchBudgetExhausted);
                }
                let tiles = tree.grid(self.plan);
                let next = tiles.into_iter().find(|tile| {
                    !visited.contains(&(
                        tile.x.to_bits(),
                        tile.y.to_bits(),
                        tile.width.to_bits(),
                        tile.height.to_bits(),
                    ))
                });
                let Some(tile) = next else {
                    if self.diagnostic.candidates_visited == 0 && tree.drafts_observed(self.plan) {
                        last_reason = VerificationReason::DraftsObserved;
                    }
                    break;
                };
                visited.insert((
                    tile.x.to_bits(),
                    tile.y.to_bits(),
                    tile.width.to_bits(),
                    tile.height.to_bits(),
                ));
                self.diagnostic.candidates_visited += 1;
                self.diagnostic.stage = "postProof";
                self.caption_expanded = false;
                self.session
                    .tap(tile.centre())
                    .await
                    .map_err(|_| VerificationReason::ReadFailed)?;
                match self.prove_post().await {
                    Ok(mut post) => {
                        if candidate.is_some() {
                            return Err(VerificationReason::MultipleMatchingPosts);
                        }
                        self.diagnostic.stage = "copy";
                        for attempt in 0..2 {
                            let copied = self.copy_once(post).await;
                            let closed = self.close_share().await;
                            match copied {
                                Ok(link) => {
                                    closed?;
                                    candidate = Some(link);
                                    break;
                                }
                                Err(VerificationReason::ClipboardUnchanged) if attempt == 0 => {
                                    closed?;
                                    post = self.prove_post().await?;
                                }
                                Err(reason) => return Err(reason),
                            }
                        }
                    }
                    Err(reason) => {
                        if !matches!(
                            reason,
                            VerificationReason::CaptionMismatch
                                | VerificationReason::SubmissionTooOld
                        ) {
                            unresolved_candidate = Some(reason);
                        }
                        last_reason = unresolved_candidate.unwrap_or(reason);
                    }
                }
                self.diagnostic.stage = "restoreProfile";
                tree = self.profile(true).await?;
            }
            if let Some((link, winning_proof)) = candidate {
                if let Some(reason) = unresolved_candidate {
                    return Err(reason);
                }
                self.diagnostic.stage = "verified";
                self.diagnostic.snapshot_generation = winning_proof.snapshot_generation;
                self.diagnostic.caption_candidates = winning_proof.caption_candidates;
                self.diagnostic.time_candidates = winning_proof.time_candidates;
                self.diagnostic.time_label = winning_proof.time_label;
                return Ok(link);
            }
            if page + 1 == MAX_VIEWPORTS {
                break;
            }
            let Some(gesture) = tree.grid_scroll(self.plan) else {
                return Err(
                    if self.diagnostic.candidates_visited == 0
                        && last_reason == VerificationReason::DraftsObserved
                    {
                        last_reason
                    } else if self.diagnostic.candidates_visited == 0 {
                        VerificationReason::PostNotVisible
                    } else if last_reason == VerificationReason::PostNotVisible {
                        VerificationReason::PaginationUnmeasured
                    } else {
                        last_reason
                    },
                );
            };
            self.diagnostic.stage = "profileScroll";
            self.session
                .swipe(gesture)
                .await
                .map_err(|_| VerificationReason::ReadFailed)?;
            tokio::time::sleep(POLL).await;
            tree = self.profile(true).await?;
        }
        Err(if last_reason == VerificationReason::PostNotVisible {
            VerificationReason::SearchBudgetExhausted
        } else {
            last_reason
        })
    }
}

#[cfg(test)]
mod tests;

/// No future is force-cancelled while it may own an IME transition. Budgets
/// prevent the next action; an in-flight primitive completes its own restore.
pub async fn capture_submission_link(
    session: &dyn UiSession,
    plan: &PublishVerificationPlan,
    caption: &str,
    identity: &SubmissionIdentity,
) -> VerificationCapture {
    let mut capture = Capture {
        session,
        plan,
        caption,
        identity,
        started: Instant::now(),
        caption_expanded: false,
        diagnostic: VerificationDiagnostic {
            contract_version: CONTRACT_VERSION,
            package: plan.labels.package().into(),
            locale: plan.labels.language().into(),
            version: plan.labels.resource_version().unwrap_or_default().into(),
            stage: "identity",
            reason_code: VerificationReason::IdentityMissing,
            snapshot_generation: 0,
            caption_candidates: 0,
            time_candidates: 0,
            time_label: None,
            navigation_actions: 0,
            candidates_visited: 0,
            viewports_visited: 0,
            copy_attempts: 0,
            elapsed_ms: 0,
            navigation_matches: 0,
            navigation_enabled: 0,
            navigation_clickable: 0,
            screen_state: String::new(),
        },
    };
    let result = if !submission_identity_valid(identity) || caption.trim().is_empty() {
        Err(VerificationReason::IdentityMissing)
    } else {
        capture.capture().await
    };
    let outcome = match result {
        Ok(link) => {
            capture.diagnostic.reason_code = VerificationReason::Verified;
            OwnPostLink::Captured(link)
        }
        Err(reason) => {
            capture.diagnostic.reason_code = reason;
            match reason {
                VerificationReason::AccountMismatch => OwnPostLink::AccountUnverified,
                VerificationReason::ProfileUnavailable
                | VerificationReason::NavigationBudgetExhausted
                | VerificationReason::UnknownScreen
                | VerificationReason::GridNotRestored => OwnPostLink::ProfileTabMissing,
                VerificationReason::ReadFailed
                | VerificationReason::StaleSnapshot
                | VerificationReason::WrongApp => OwnPostLink::ReadFailed(format!("{:?}", reason)),
                VerificationReason::ClipboardUnchanged => {
                    OwnPostLink::Sheet(LinkCapture::CopyDidNotLand)
                }
                VerificationReason::ClipboardUnreadable => {
                    OwnPostLink::Sheet(LinkCapture::ReadFailed("Không đọc được clipboard".into()))
                }
                VerificationReason::ClipboardUnwritable => {
                    OwnPostLink::Sheet(LinkCapture::ClipboardUnwritable(
                        "Không xác minh được sentinel clipboard".into(),
                    ))
                }
                VerificationReason::CopyAmbiguous => {
                    OwnPostLink::Sheet(LinkCapture::AmbiguousCopyRow)
                }
                VerificationReason::CopyUnavailable => OwnPostLink::Sheet(LinkCapture::NoCopyRow),
                VerificationReason::ShareUnavailable => {
                    OwnPostLink::Sheet(LinkCapture::NoShareControl)
                }
                VerificationReason::PostNotVisible
                | VerificationReason::SearchBudgetExhausted
                | VerificationReason::PaginationUnmeasured => OwnPostLink::CaptionNotFound,
                VerificationReason::DraftsObserved => OwnPostLink::DraftsPresent(1),
                _ => OwnPostLink::SubmissionUnverified,
            }
        }
    };
    capture.diagnostic.elapsed_ms =
        capture.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    VerificationCapture {
        outcome,
        diagnostic: capture.diagnostic,
    }
}
