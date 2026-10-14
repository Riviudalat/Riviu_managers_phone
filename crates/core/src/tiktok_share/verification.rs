//! Future publication verification: one tuple, immutable observations and bounded navigation.
use super::*;
use anyhow::Context;
use hierarchy::Tree;
use serde::Serialize;
use sha2::Digest;
use std::collections::HashSet;
use tokio::time::Instant;

const CONTRACT_VERSION: u32 = 1;
const RECOVERY_WINDOW: Duration = Duration::from_secs(60);
const CAPTURE_WINDOW: Duration = Duration::from_secs(180);
/// Extra time, once one tile is proven, to finish ruling out a second match in that
/// viewport. Global 45.7.3/en, phones 39 ce11171beb405c2701 and 46 ce0417143cef30280d,
/// 10/10/2026: one tile cost 4-26 s, with one 74 s outlier under adb-slot contention;
/// 46 needed 154 s for nine tiles, and 39 ran past 180 s on its sixth.
const AMBIGUITY_SCAN_EXTENSION: Duration = Duration::from_secs(120);
/// Bound on leaving the viewer a pass opened itself after its search window ran out.
const LEAVE_VIEWER_WINDOW: Duration = Duration::from_secs(20);
const MAX_LEAVE_VIEWER_BACKS: u32 = 2;
const MAX_RECOVERY_ACTIONS: u32 = 5;
const MAX_CANDIDATES: u32 = 12;
const MAX_GLOBAL_45_7_3_TAPS: u32 = 24;
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
    MetadataUnavailable,
    StaleSnapshot,
    ProfileUnavailable,
    NavigationBudgetExhausted,
    ComposerOrUpload,
    LoginRequired,
    SecurityPrompt,
    UnrecognizedDialog,
    UnknownScreen,
    AccountMismatch,
    AccountUnreadable,
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
    CopyBudgetExhausted,
    ClipboardUnwritable,
    ClipboardUnreadable,
    ClipboardUnchanged,
    Processing,
    ClipboardNotPostLink,
    RedirectFailed,
    MultipleMatchingPosts,
    OtherPublication,
    AmbiguityScanIncomplete,
}

impl VerificationReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::IdentityMissing => "submissionIdentityMissing",
            Self::WrongApp => "wrongApp",
            Self::ReadFailed => "readFailed",
            Self::MetadataUnavailable => "metadataUnavailable",
            Self::StaleSnapshot => "staleSnapshot",
            Self::ProfileUnavailable => "profileUnavailable",
            Self::NavigationBudgetExhausted => "navigationBudgetExhausted",
            Self::ComposerOrUpload => "composerOrUpload",
            Self::LoginRequired => "loginRequired",
            Self::SecurityPrompt => "securityPrompt",
            Self::UnrecognizedDialog => "unrecognizedDialog",
            Self::UnknownScreen => "unknownScreen",
            Self::AccountMismatch => "accountMismatch",
            Self::AccountUnreadable => "accountUnreadable",
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
            Self::CopyBudgetExhausted => "copyBudgetExhausted",
            Self::ClipboardUnwritable => "clipboardUnwritable",
            Self::ClipboardUnreadable => "clipboardUnreadable",
            Self::ClipboardUnchanged => "clipboardUnchanged",
            Self::Processing => "tiktokProcessing",
            Self::ClipboardNotPostLink => "clipboardNotPostLink",
            Self::RedirectFailed => "redirectFailed",
            Self::MultipleMatchingPosts => "multipleMatchingPosts",
            Self::OtherPublication => "otherPublication",
            Self::AmbiguityScanIncomplete => "ambiguityScanIncomplete",
        }
    }

    pub fn message(&self) -> &'static str {
        match self {
            Self::OtherPublication => "Liên kết đã thuộc lượt đăng khác; chưa xác minh được bài mới trong giới hạn tìm kiếm.",
            Self::Verified => "Đã xác minh tài khoản, nội dung, thời gian và liên kết bài đăng.",
            Self::IdentityMissing => "Thiếu bằng chứng tài khoản hoặc thời điểm trước Đăng; chưa xác minh liên kết.",
            Self::WrongApp => "Ứng dụng đang mở khác bản TikTok của lượt đăng; chưa điều hướng để lấy liên kết.",
            Self::ReadFailed => "Đọc màn hình xác minh thất bại; giữ nguyên bài và chờ lần kiểm tra tiếp theo.",
            Self::MetadataUnavailable => "Đã đọc được liên kết nhưng chưa đủ dữ liệu công khai của TikTok để xác minh bài; giữ nguyên bài và chờ lượt xác minh tiếp theo.",
            Self::StaleSnapshot => "Ảnh chụp cây giao diện không mới hơn lần đọc trước; chưa dùng dữ liệu này để thao tác.",
            Self::ProfileUnavailable => "Hết thời gian chờ tab Hồ sơ xuất hiện; chưa tìm được đường đến bài đã gửi.",
            Self::NavigationBudgetExhausted => "Đã dùng hết ba bước phục hồi màn hình; chưa đến được hồ sơ để lấy liên kết.",
            Self::ComposerOrUpload => "TikTok đang ở màn soạn bài hoặc gửi nội dung; giữ nguyên màn hình và chờ kiểm tra lại.",
            Self::LoginRequired => "TikTok đang yêu cầu đăng nhập; cần mở lại đúng tài khoản trước khi kiểm tra liên kết.",
            Self::SecurityPrompt => "TikTok đang hiện nhắc bảo mật; chưa xác minh tài khoản và không bấm Tiếp tục.",
            Self::UnrecognizedDialog => "Hộp thoại chưa được nhận diện; chưa thao tác hoặc xác minh tài khoản.",
            Self::UnknownScreen => "Màn hình hiện tại chưa được nhận diện; chưa điều hướng hoặc bấm Back.",
            Self::AccountMismatch => "Tài khoản trên hồ sơ hoặc trong liên kết chưa khớp tài khoản ghi nhận trước Đăng.",
            Self::AccountUnreadable => "Chưa đọc được tên tài khoản trên đầu trang Hồ sơ; chưa kết luận tài khoản bị đổi.",
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
            Self::CopyBudgetExhausted => "Đã dùng hết ngân sách sao chép của lượt kiểm tra; chưa xác minh liên kết, không sao chép thêm trong lượt này.",
            Self::ClipboardUnwritable => "Chưa ghi và đọc lại được giá trị kiểm chứng clipboard; chưa bấm Sao chép liên kết.",
            Self::ClipboardUnreadable => "Không đọc được clipboard sau thao tác; chưa xác nhận đã sao chép liên kết.",
            Self::ClipboardUnchanged => "Đã mở đúng nội dung và bấm Sao chép liên kết nhưng TikTok chưa trả link; tự kiểm tra lại sau 2 phút.",
            Self::Processing => "TikTok báo bài đang được xử lý; chưa trả liên kết. Tự kiểm tra lại sau 2 phút.",
            Self::ClipboardNotPostLink => "Clipboard đã đổi nhưng nội dung không phải liên kết bài TikTok hợp lệ.",
            Self::RedirectFailed => "Đã sao chép liên kết nhưng chưa chuẩn hóa được đường dẫn đến bài TikTok.",
            Self::MultipleMatchingPosts => "Có nhiều bài cùng khớp caption và thời gian trong lưới hồ sơ; chưa chọn liên kết để tránh gán nhầm lượt đăng.",
            Self::AmbiguityScanIncomplete => "Đã thấy một bài khớp caption và thời gian nhưng hết thời gian rà các bài còn lại để loại trừ bài trùng; chưa xác minh liên kết, tự kiểm tra lại sau 2 phút, không đăng lại.",
        }
    }
}

/// Partial target evidence, never sufficient to settle publication or select a link.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationPublicationEvidence {
    pub caption_matched: bool,
    pub submission_time_matched: bool,
    /// First candidate this pass proved, kept as a lead for the next pass. It is never a
    /// verified link: only a completed ambiguity scan of its viewport selects one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proven_candidate_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationDiagnostic {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captured_metadata_candidate: Option<crate::publish_submission::CapturedMetadataCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publication_evidence: Option<VerificationPublicationEvidence>,
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
    /// Taps of TikTok's `Copy link` on the just-published post. TikTok very likely counts
    /// each one as a public share of that post (Global 45.7.3, 09/10/2026: the share counter
    /// went 0 -> 1 about a minute after one copy), so they are not free reads.
    pub copy_attempts: u32,
    /// Always true: marks `copy_attempts` in the stored evidence as share-counting actions,
    /// so a share count on our own post is never read as organic.
    pub copy_link_counts_as_share: bool,
    pub elapsed_ms: u64,
    pub navigation_matches: usize,
    pub navigation_enabled: usize,
    pub navigation_clickable: usize,
    pub screen_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    unknown_screen_shape: Option<UnknownScreenShape>,
    pub expanded_photo_error: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) candidate_trace: Vec<VerificationCandidateTrace>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct UnknownScreenShape {
    total_nodes: usize,
    target_visible_nodes: usize,
    target_clickable_nodes: usize,
    foreign_visible_nodes: usize,
    has_profile_tab: bool,
    has_feed_tab: bool,
    has_post_tile: bool,
    has_post_caption: bool,
    has_share_control: bool,
    has_comments_control: bool,
    has_copy_control: bool,
    has_back_control: bool,
}

impl UnknownScreenShape {
    fn from_tree(tree: &Tree, plan: &PublishVerificationPlan) -> Self {
        let package = plan.labels.package();
        let has = |control| {
            plan.labels
                .label(control)
                .is_some_and(|label| !tree.matching(package, label.to_query()).is_empty())
        };
        let mut target_visible_nodes = 0;
        let mut target_clickable_nodes = 0;
        let mut foreign_visible_nodes = 0;
        for (index, node) in tree.nodes.iter().enumerate() {
            if !tree.ancestors_visible(index) {
                continue;
            }
            if node.visible(package) {
                target_visible_nodes += 1;
                if node
                    .rect()
                    .is_some_and(|rect| rect.enabled && rect.clickable)
                {
                    target_clickable_nodes += 1;
                }
            } else if !node.attr("package").is_empty()
                && node.attr("displayed") != "false"
                && node.attr("visible-to-user") != "false"
            {
                foreign_visible_nodes += 1;
            }
        }
        Self {
            total_nodes: tree.nodes.len(),
            target_visible_nodes,
            target_clickable_nodes,
            foreign_visible_nodes,
            has_profile_tab: has(TikTokControl::ProfileTab),
            has_feed_tab: has(TikTokControl::FeedTab),
            has_post_tile: plan
                .labels
                .post_tile_id()
                .is_some_and(|label| !tree.matching(package, label.to_query()).is_empty()),
            has_post_caption: !tree
                .matching(package, ElementQuery::ResourceIdSuffix(plan.caption_id))
                .is_empty(),
            has_share_control: has(TikTokControl::Share),
            has_comments_control: has(TikTokControl::Comments),
            has_copy_control: tree.copy_control(package).is_ok_and(|row| row.is_some()),
            has_back_control: plan.labels.back_descriptions().iter().any(|value| {
                !tree
                    .matching(package, ElementQuery::Description { value, exact: true })
                    .is_empty()
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerificationCandidateTrace {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) canonical_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) post_id_allocated_at: Option<String>,
    pub(crate) viewport: u32,
    pub(crate) index: u32,
    pub(crate) profile_snapshot_generation: u64,
    pub(crate) post_snapshot_generation: Option<u64>,
    pub(crate) tile_bounds: [f64; 4],
    pub(crate) caption_digest: Option<String>,
    pub(crate) time_digest: Option<String>,
    pub(crate) reason_code: Option<VerificationReason>,
    pub(crate) started_at_ms: u64,
    pub(crate) elapsed_ms: Option<u64>,
}

fn candidate_budget_exhausted(
    plan: &PublishVerificationPlan,
    trace: &[VerificationCandidateTrace],
    attempts: u32,
) -> bool {
    if plan.labels.package() != "com.zhiliaoapp.musically"
        || plan.labels.resource_version() != Some("45.7.3")
    {
        return attempts >= MAX_CANDIDATES;
    }
    if attempts >= MAX_GLOBAL_45_7_3_TAPS {
        return true;
    }
    let mut rejected = HashSet::new();
    let mut distinct = attempts.saturating_sub(trace.len() as u32);
    for candidate in trace {
        if candidate.reason_code == Some(VerificationReason::CaptionMismatch) {
            if let (Some(caption), Some(time)) = (
                candidate.caption_digest.as_deref(),
                candidate.time_digest.as_deref(),
            ) {
                if !rejected.insert((caption, time)) {
                    continue;
                }
            }
        }
        distinct += 1;
    }
    distinct >= MAX_CANDIDATES
}

#[derive(Debug)]
pub struct VerificationCapture {
    pub outcome: OwnPostLink,
    pub diagnostic: VerificationDiagnostic,
}

impl VerificationCapture {
    /// Reuse the phone verifier's metadata contract without opening a phone session.
    pub async fn verify_metadata_candidate(
        captured: &crate::publish_submission::CapturedMetadataCandidate,
        caption: &str,
        other_publication_urls: &[String],
    ) -> anyhow::Result<crate::publish_submission::MetadataAttempt> {
        use crate::publish_submission::*;
        let identity = SubmissionIdentity {
            account: captured.expected_account.clone(),
            prepared_at: captured.prepared_at.clone(),
            submitted_at: captured.submitted_at.clone(),
        };
        let parsed = url::Url::parse(&captured.canonical_url)?;
        anyhow::ensure!(parsed.scheme() == "https"
            && super::canonical_post_path(&parsed).as_deref() == Some(captured.canonical_url.as_str()),
            "candidate is not canonical");
        anyhow::ensure!(!other_publication_urls.contains(&captured.canonical_url), "candidate belongs to another publication");
        anyhow::ensure!(super::photo_proof::validate_photo_identity(&captured.canonical_url, &identity)? == captured.post_id,
            "candidate post ID changed");
        anyhow::ensure!(normalized_caption_sha256(caption) == captured.normalized_caption_sha256,
            "candidate caption changed");
        let captured_at = chrono::DateTime::parse_from_rfc3339(&captured.captured_at)?;
        let submitted_at = chrono::DateTime::parse_from_rfc3339(&captured.submitted_at)?;
        anyhow::ensure!(captured_at >= submitted_at && captured_at <= chrono::Utc::now(), "candidate capture time invalid");
        let result = super::photo_proof::validate_public_link(&captured.canonical_url, caption, &identity).await;
        let (state, stage, http_status) = match result {
            Ok(()) => (MetadataAttemptState::Verified, MetadataDiagnostic::PublicMetadata, None),
            Err(error) => match error.downcast_ref::<super::photo_proof::PublicMetadataUnavailable>() {
                Some(error) => (MetadataAttemptState::Unavailable, error.stage, error.http_status),
                None => (MetadataAttemptState::Rejected, MetadataDiagnostic::MetadataIdentity, None),
            },
        };
        Ok(MetadataAttempt { state, stage, http_status,
            attempts: captured.metadata.attempts.saturating_add(1), checked_at: chrono::Utc::now().to_rfc3339() })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Screen {
    Profile,
    Feed,
    Post,
    Share,
    Comments,
    Dialog,
    Composer,
    Login,
    SecurityPrompt,
    UnrecognizedDialog,
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
    if let Some(blocker) = crate::app_automation::dialogs::account_blocker(tree, plan.labels) {
        use crate::app_automation::dialogs::AccountBlocker;
        match blocker {
            AccountBlocker::LoginRequired => return Screen::Login,
            AccountBlocker::SecurityPrompt => return Screen::SecurityPrompt,
            AccountBlocker::UnrecognizedDialog
                if decline_facebook_permission(tree, plan).is_none()
                    && tree.copy_control(package).ok().flatten().is_none()
                    && tree.share_sheet_close(package).is_none() =>
            {
                return Screen::UnrecognizedDialog
            }
            AccountBlocker::UnrecognizedDialog => {}
        }
    }
    if has(TikTokControl::ComposerCaption)
        || has(TikTokControl::PostButton)
        || (has(TikTokControl::ComposerShutter)
            && !has(TikTokControl::ProfileTab)
            && !has(TikTokControl::FeedTab))
    {
        return Screen::Composer;
    }
    if has(TikTokControl::DialogDismiss)
        || decline_facebook_permission(tree, plan).is_some()
        || crate::app_automation::dialogs::decline_contacts(tree, plan.labels).is_some()
    {
        return Screen::Dialog;
    }
    // Trill 38.3.2/en, phone 4, 15/09/2026: tapping a video caption
    // opens a Comments/Likes drawer above :id/cnd Add comment. The
    // underlying video's nodes can remain visible, so classify the drawer
    // before Post. This authorizes Back only, never writing to the input.
    if package == "com.ss.android.ugc.trill"
        && plan.labels.resource_version() == Some("38.3.2")
        && plan.labels.language() == "en"
    {
        let counted_header = |prefix: &str| {
            tree.nodes.iter().enumerate().any(|(index, node)| {
                node.visible(package)
                    && tree.ancestors_visible(index)
                    && node.attr("class") == "android.widget.TextView"
                    && node.rect().is_some()
                    && node.attr("text").strip_prefix(prefix).is_some_and(|count| {
                        !count.is_empty() && count.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
        };
        let inputs = tree.matching(package, ElementQuery::ResourceIdSuffix(":id/cnd"));
        if counted_header("Comments ")
            && counted_header("Likes ")
            && inputs.len() == 1
            && tree.nodes[inputs[0]].attr("class") == "android.widget.EditText"
            && tree.nodes[inputs[0]]
                .rect()
                .is_some_and(|input| input.enabled && input.clickable)
        {
            return Screen::Comments;
        }
    }
    if package == "com.ss.android.ugc.trill"
        && has(TikTokControl::Comments)
        && has(TikTokControl::Share)
        && !tree
            .matching(package, ElementQuery::ResourceIdSuffix(":id/desc"))
            .is_empty()
        && plan.labels.back_descriptions().iter().any(|value| {
            !tree
                .matching(package, ElementQuery::Description { value, exact: true })
                .is_empty()
        })
    {
        return Screen::Post;
    }
    if tree.copy_control(package).is_err()
        || tree.copy_control(package).ok().flatten().is_some()
        || tree.share_sheet_close(package).is_some()
    {
        return Screen::Share;
    }
    if plan
        .labels
        .resource_version()
        .is_some_and(|version| super::photo_proof::expanded_photo_surface(tree, package, version))
    {
        return Screen::Post;
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
    // Global 46.2.1 own photo viewer, 14/09/2026, machine 12: :id/rs_ caption,
    // bottom Comments + Share controls, no profile tab. It is a post surface from
    // which Back is valid, even though full caption/time proof is not visible yet.
    if has(TikTokControl::Share)
        && !tree
            .matching(package, ElementQuery::ResourceIdSuffix(plan.caption_id))
            .is_empty()
    {
        return Screen::Post;
    }
    if tree
        .nodes
        .iter()
        .any(|n| n.visible(package) && n.attr("content-desc") == "Share")
        && tree
            .nodes
            .iter()
            .any(|n| n.visible(package) && n.attr("content-desc") == "Comments")
        && tree
            .nodes
            .iter()
            .any(|n| n.visible(package) && n.attr("resource-id").ends_with(":id/rs_"))
    {
        return Screen::Post;
    }
    if tree
        .nodes
        .iter()
        .any(|n| n.visible(package) && n.attr("resource-id").ends_with(":id/m3q"))
        && tree
            .nodes
            .iter()
            .any(|n| n.visible(package) && n.attr("resource-id").ends_with(":id/m3h"))
    {
        return Screen::Post;
    }
    Screen::Unknown
}

/// Navigation-only recovery for the measured caption-hidden post viewers.
/// Copy and publication proof still require their unchanged independent checks.
fn hidden_post_restore_surface(
    tree: &Tree,
    plan: &PublishVerificationPlan,
) -> Option<Vec<ElementBox>> {
    let package = plan.labels.package();
    let tuple = (
        package,
        plan.labels.resource_version(),
        plan.labels.language(),
    );
    if !matches!(
        tuple,
        ("com.ss.android.ugc.trill", Some("38.3.2"), "en")
            | ("com.zhiliaoapp.musically", Some("45.7.3"), "en")
    ) || classify(tree, plan) != Screen::Unknown
    {
        return None;
    }
    let shape = UnknownScreenShape::from_tree(tree, plan);
    let editing = |focused_only: bool| {
        tree.nodes.iter().enumerate().any(|(index, node)| {
            node.visible(package)
                && tree.ancestors_visible(index)
                && node.attr("class") == "android.widget.EditText"
                && (!focused_only || node.attr("focused") == "true")
        })
    };
    if shape.foreign_visible_nodes != 0
        || shape.has_profile_tab
        || shape.has_feed_tab
        || shape.has_post_tile
        || shape.has_post_caption
        || shape.has_copy_control
    {
        return None;
    }
    if package == "com.zhiliaoapp.musically" {
        // Global 45.7.3/en, phone 39 ce11171beb405c2701, 10/10/2026 19:17Z: the
        // own-post viewer the previous pass left open read Back, Comments and Share,
        // no tab, tile, :id/desc caption, copy row or foreign node, and stayed Unknown.
        // Its "Add comment..." field is allowed only while it is not being typed into.
        if editing(true) {
            return None;
        }
        let actionable = |query| {
            tree.matching(package, query)
                .into_iter()
                .filter(|index| tree.ancestors_visible(*index))
                .find_map(|index| {
                    tree.nodes[index]
                        .rect()
                        .filter(|rect| rect.enabled && rect.clickable)
                })
        };
        let back = plan
            .labels
            .back_descriptions()
            .iter()
            .find_map(|value| actionable(ElementQuery::Description { value, exact: true }))?;
        return Some(vec![
            back,
            actionable(plan.labels.label(TikTokControl::Comments)?.to_query())?,
            actionable(plan.labels.label(TikTokControl::Share)?.to_query())?,
        ]);
    }
    if editing(false) {
        return None;
    }
    // 06/10/2026 machine2-post-observe: exact measured native control tuple.
    let actionable = |query, id: &str, class: &str| {
        let matches = tree.matching(package, query);
        let [index] = matches.as_slice() else { return None; };
        let node = &tree.nodes[*index];
        if !node.attr("resource-id").ends_with(id) || node.attr("class") != class {
            return None;
        }
        node.rect().filter(|rect| rect.enabled && rect.clickable)
    };
    Some(vec![
        actionable(ElementQuery::Description { value: "Back", exact: true }, ":id/aur", "android.widget.ImageView")?,
        actionable(plan.labels.label(TikTokControl::Comments)?.to_query(), ":id/cqn", "android.widget.Button")?,
        actionable(plan.labels.label(TikTokControl::Share)?.to_query(), ":id/drt", "android.widget.Button")?,
        actionable(ElementQuery::Description { value: "Video", exact: true }, ":id/long_press_layout", "android.view.View")?,
    ])
}

fn decline_facebook_permission(tree: &Tree, plan: &PublishVerificationPlan) -> Option<ElementBox> {
    crate::app_automation::dialogs::decline_facebook_permission(tree, plan.labels)
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

/// Every successful grid proof, including public photo/video metadata, shares
/// this ambiguity boundary. A second candidate cannot silently replace the first.
fn remember_candidate(
    candidate: &mut Option<(String, VerificationDiagnostic)>,
    proof: (String, VerificationDiagnostic),
) -> Result<(), VerificationReason> {
    if let Some((canonical, prior_proof)) = candidate.as_ref() {
        // A shifted grid can expose the same post twice. Coalesce only exact
        // canonical identities after both observations passed public metadata proof.
        if canonical == &proof.0
            && matches!(
                prior_proof.stage,
                "videoPublicProof" | "expandedPhotoPublicProof"
            )
            && matches!(proof.1.stage, "videoPublicProof" | "expandedPhotoPublicProof")
        {
            return Ok(());
        }
        return Err(VerificationReason::MultipleMatchingPosts);
    }
    *candidate = Some(proof);
    Ok(())
}

struct Capture<'a> {
    session: &'a dyn UiSession,
    plan: &'a PublishVerificationPlan,
    caption: &'a str,
    identity: &'a SubmissionIdentity,
    other_publication_urls: &'a [String],
    started: Instant,
    diagnostic: VerificationDiagnostic,
    caption_expanded: bool,
    public_link: Option<String>,
    trace_nonce: [u8; 16],
    metadata: MetadataRoute,
    device: DeviceProof,
}

/// Own-Profile proof state of the tile this pass opened, for the two measured cases where
/// one snapshot cannot hold caption and time, both closed by the copied post ID.
///
/// Folded caption, Global45.7.3/en, measured r117 14/10/2026 on phone 47
/// (`ce051715ab9fee2002`, SM-G955F): `desc` shows 150 of the 207 characters and tapping it
/// opens the expanded photo page, which carries the complete caption (`rey`) and Share
/// (`red`) but no time label. The proof then spans the two snapshots joined by that tap: the
/// tile viewer's folded caption and time, then the expanded page's exact caption.
///
/// Coarse time, r119 14/10/2026 on phone 46: an hour or day label ("· 1h ago") cannot place
/// its rounded interval after the submission, so it only must not exclude it.
///
/// Either way Copy link must give the own account and a post ID allocated inside the
/// recorded preparation window (`photo_proof::validate_photo_identity`).
#[derive(Debug, Default)]
struct DeviceProof {
    /// The tile viewer whose own caption this pass tapped showed a folded prefix of the
    /// submitted caption and a time label that matches or does not exclude the submission.
    tile_time_matched: bool,
    /// The current post surface is that tap's expanded photo page with the exact caption.
    expanded_page: bool,
    /// The time label did not exclude the submission without proving it; the copied post
    /// ID must.
    id_time_required: bool,
}

/// What this pass knows about TikTok's public metadata for the submission.
#[derive(Debug, Default)]
struct MetadataRoute {
    /// Post ID of the copied candidate an earlier observation retained because public
    /// metadata did not answer. A proof of any other post never verifies over it.
    retained_post_id: Option<String>,
    /// Public metadata did not answer. Another public proof would only Copy and wait on it
    /// again, so the own-Profile device proof is the remaining route.
    skip_public_proof: bool,
    /// This pass copied a candidate whose public metadata did not answer. When the own-Profile
    /// proof settles nothing, that stays the outcome so the candidate keeps its route.
    unavailable_here: bool,
}

/// Longest wait for the reloaded Profile grid to differ from the cached one. A cap: a pass
/// that still reads the old grid tries again on the next 2-minute check.
const PROFILE_RELOAD_WINDOW: Duration = Duration::from_secs(5);

/// The visible grid: each text inside a post tile (view counts, badges) with its position.
/// A reload that brings a new post or fresh counters changes it.
fn grid_signature(tree: &Tree, plan: &PublishVerificationPlan) -> Vec<String> {
    let package = plan.labels.package();
    let tiles = tree.grid(plan);
    tree.nodes
        .iter()
        .filter(|node| node.visible(package) && !node.attr("text").is_empty())
        .filter_map(|node| {
            let rect = node.rect()?;
            tiles
                .iter()
                .any(|tile| {
                    rect.x >= tile.x
                        && rect.y >= tile.y
                        && rect.x + rect.width <= tile.x + tile.width
                        && rect.y + rect.height <= tile.y + tile.height
                })
                .then(|| format!("{}:{}:{}", rect.x, rect.y, node.attr("text")))
        })
        .collect()
}

fn post_id(canonical: &str) -> Option<&str> {
    canonical.rsplit('/').next().filter(|id| !id.is_empty())
}

impl Capture<'_> {
    fn defer_public_metadata(&mut self) {
        self.metadata.skip_public_proof = true;
        self.metadata.unavailable_here = true;
    }

    /// A proof of a different post than a copied candidate still awaiting public metadata
    /// is a second match, never a replacement for that candidate.
    fn conflicts_with_candidate(&self, canonical: &str) -> bool {
        self.metadata
            .retained_post_id
            .iter()
            .chain(
                self.diagnostic
                    .captured_metadata_candidate
                    .iter()
                    .map(|candidate| &candidate.post_id),
            )
            .any(|candidate| Some(candidate.as_str()) != post_id(canonical))
    }

    fn remember_proven_candidate(
        &mut self,
        candidate: &mut Option<(String, VerificationDiagnostic)>,
        proof: (String, VerificationDiagnostic),
    ) -> Result<(), VerificationReason> {
        // Preserve both proven URLs before the unchanged ambiguity refusal.
        // ID allocation is diagnostic, not publication time or a newest-post rule.
        if let Some(last) = self.diagnostic.candidate_trace.last_mut() {
            last.canonical_url = Some(proof.0.clone());
            last.post_id_allocated_at = proof
                .0
                .rsplit('/')
                .next()
                .and_then(|id| id.parse::<u64>().ok())
                .and_then(|id| chrono::DateTime::from_timestamp((id >> 32) as i64, 0))
                .map(|at| at.to_rfc3339());
        }
        // Every proof reaching here matched the full caption, the account and a post
        // allocated after this submission, including the public video/photo proofs that
        // never pass `post_proof`. Record that as the restart frontier (stage 2) and keep
        // the first URL as a lead; neither selects a link.
        let evidence = self
            .diagnostic
            .publication_evidence
            .get_or_insert_with(Default::default);
        evidence.caption_matched = true;
        evidence.submission_time_matched = true;
        evidence
            .proven_candidate_url
            .get_or_insert_with(|| proof.0.clone());
        if self.conflicts_with_candidate(&proof.0) {
            return Err(VerificationReason::MultipleMatchingPosts);
        }
        remember_candidate(candidate, proof)
    }

    fn trace_enabled(&self) -> bool {
        matches!(
            (
                self.plan.labels.package(),
                self.plan.labels.resource_version()
            ),
            ("com.zhiliaoapp.musically", Some("45.7.3"))
                | ("com.ss.android.ugc.trill", Some("38.3.2"))
        )
    }

    fn trace_digest(&self, value: &str) -> String {
        let mut digest = sha2::Sha256::new();
        digest.update(self.trace_nonce);
        digest.update(value.as_bytes());
        format!("{:x}", digest.finalize())
    }

    fn mark_candidate(&mut self, reason: VerificationReason) {
        if let Some(last) = self.diagnostic.candidate_trace.last_mut() {
            last.reason_code = Some(reason);
            last.elapsed_ms = Some(self.started.elapsed().as_millis() as u64 - last.started_at_ms);
        }
    }

    fn matched_photo_failure(&mut self, error: &anyhow::Error) -> Option<VerificationReason> {
        if let Some(candidate) = error.downcast_ref::<super::photo_proof::CapturedMetadataFailure>() {
            self.diagnostic.captured_metadata_candidate = Some(candidate.0.clone());
        }
        if error.is::<super::photo_proof::OtherPublication>() {
            return Some(VerificationReason::OtherPublication);
        }
        if error.is::<super::photo_proof::EarlierPublication>()
            || error.is::<super::photo_proof::VisibleEarlierPublication>()
        {
            self.diagnostic.expanded_photo_error = Some(error.to_string());
            return Some(VerificationReason::SubmissionTooOld);
        }
        self.diagnostic.expanded_photo_error = Some(error.to_string());
        let failure = error.downcast_ref::<super::photo_proof::MatchedPhotoCopyFailure>()?;
        Some(match &failure.0 {
            LinkCapture::Processing(_) => VerificationReason::Processing,
            LinkCapture::CopyBudgetExhausted => VerificationReason::CopyBudgetExhausted,
            LinkCapture::CopyDidNotLand => VerificationReason::ClipboardUnchanged,
            LinkCapture::NoCopyRow => VerificationReason::CopyUnavailable,
            LinkCapture::AmbiguousCopyRow => VerificationReason::CopyAmbiguous,
            LinkCapture::ClipboardUnwritable(_) => VerificationReason::ClipboardUnwritable,
            LinkCapture::NotAPostLink(_) => VerificationReason::ClipboardNotPostLink,
            LinkCapture::ReadFailed(_) if error.is::<super::photo_proof::PublicMetadataUnavailable>() => {
                VerificationReason::MetadataUnavailable
            }
            LinkCapture::ReadFailed(_) => VerificationReason::ReadFailed,
            _ => VerificationReason::ShareUnavailable,
        })
    }
    fn proven(&self) -> bool {
        self.diagnostic
            .publication_evidence
            .as_ref()
            .is_some_and(|evidence| evidence.proven_candidate_url.is_some())
    }
    /// The search window, extended once a tile is proven so the ambiguity scan of
    /// its viewport can finish. Both bounds are finite.
    fn window(&self) -> Duration {
        if self.proven() {
            CAPTURE_WINDOW + AMBIGUITY_SCAN_EXTENSION
        } else {
            CAPTURE_WINDOW
        }
    }
    fn expired(&self) -> bool {
        self.started.elapsed() >= self.window()
    }

    /// The pass ran out of time on a viewer it opened itself. Leave it with at most
    /// two Backs (Share sheet, then viewer), each from a freshly read post, share or
    /// comments surface, so the next pass starts from the profile. Never a tap; any
    /// other screen, a stale read or the deadline stops it.
    async fn leave_own_viewer(&mut self) {
        let deadline = Instant::now() + LEAVE_VIEWER_WINDOW;
        for _ in 0..MAX_LEAVE_VIEWER_BACKS {
            if !self.viewer_proved_before(deadline).await || self.session.back().await.is_err() {
                return;
            }
            self.diagnostic.navigation_actions += 1;
            tokio::time::sleep(POLL).await;
        }
    }

    /// A fresh read past the search window, used only to authorize one Back.
    async fn fresh_tree_before(&mut self, deadline: Instant) -> Option<Tree> {
        if Instant::now() >= deadline
            || self.session.active_app_bundle().await.ok().as_deref()
                != Some(self.plan.labels.package())
        {
            return None;
        }
        let tree = Tree::parse(self.session.hierarchy_source_snapshot().await.ok()?).ok()?;
        if Instant::now() >= deadline || tree.generation <= self.diagnostic.snapshot_generation {
            return None;
        }
        self.diagnostic.snapshot_generation = tree.generation;
        Some(tree)
    }

    async fn viewer_proved_before(&mut self, deadline: Instant) -> bool {
        let Some(tree) = self.fresh_tree_before(deadline).await else {
            return false;
        };
        if matches!(
            classify(&tree, self.plan),
            Screen::Post | Screen::Share | Screen::Comments
        ) {
            return true;
        }
        // The unlabelled viewer surface needs the same two-read proof as `profile`.
        let Some(surface) = hidden_post_restore_surface(&tree, self.plan) else {
            return false;
        };
        let epoch = self.session.gui_session_epoch();
        let Some(fresh) = self.fresh_tree_before(deadline).await else {
            return false;
        };
        self.session.gui_session_epoch() == epoch
            && hidden_post_restore_surface(&fresh, self.plan).as_ref() == Some(&surface)
    }
    async fn read(&mut self) -> Result<Tree, VerificationReason> {
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        let foreground = self
            .session
            .active_app_bundle()
            .await
            .map_err(|_| VerificationReason::ReadFailed)?;
        if foreground != self.plan.labels.package()
            && foreground != "com.google.android.packageinstaller"
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
        // The OS package is accepted only under a measured OS question the popup step in
        // `profile` answers: Trill location, or Global contacts (box 31-50, 10/10/2026).
        if foreground != self.plan.labels.package()
            && crate::app_automation::dialogs::optional_decline(&tree, self.plan.labels).is_none()
        {
            return Err(VerificationReason::WrongApp);
        }
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        if tree.generation <= self.diagnostic.snapshot_generation {
            return Err(VerificationReason::StaleSnapshot);
        }
        self.diagnostic.snapshot_generation = tree.generation;
        Ok(tree)
    }

    async fn account(&self, restoring: bool) -> Result<(), VerificationReason> {
        let observed = if restoring {
            crate::tiktok_account::observe_own_account(self.session, self.plan.labels).await
        } else {
            crate::tiktok_account::restore_own_profile_header(self.session, self.plan.labels).await
        }
        .map_err(|error| {
            use crate::tiktok_account::{AccountDiagnostic, AccountState};
            match error.downcast_ref::<AccountDiagnostic>().map(|d| d.state) {
                Some(AccountState::LoginRequired) => VerificationReason::LoginRequired,
                Some(AccountState::SecurityPrompt) => VerificationReason::SecurityPrompt,
                Some(AccountState::UnrecognizedDialog) => VerificationReason::UnrecognizedDialog,
                Some(AccountState::Unreadable) => VerificationReason::AccountUnreadable,
                _ => VerificationReason::ReadFailed,
            }
        })?;
        // Initial header proof is mandatory. Later grid scrolls may hide that
        // header; preserve their position. Canonical URL still must match account.
        if observed.is_none() && restoring {
            return Ok(());
        }
        if observed.is_none() {
            return Err(VerificationReason::AccountUnreadable);
        }
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
        let mut popup = crate::app_automation::dialogs::PopupBudget::default();
        let popup_stop = std::sync::atomic::AtomicBool::new(false);
        let mut last_screen = None;
        let mut last_action_at: Option<Instant> = None;
        loop {
            let mut tree = self.read().await?;
            if crate::app_automation::dialogs::optional_decline(&tree, self.plan.labels).is_some() {
                use crate::app_automation::dialogs::{step_before_deadline, PopupStep};
                use crate::ui_automation::runtime::ReadWaitResult;
                match step_before_deadline(
                    self.session,
                    self.plan.labels,
                    &mut popup,
                    (start + RECOVERY_WINDOW).min(self.started + self.window()),
                    &popup_stop,
                    &popup_stop,
                )
                .await
                .map_err(|_| VerificationReason::ReadFailed)?
                {
                    ReadWaitResult::Ready(PopupStep::Clear) => continue,
                    ReadWaitResult::Ready(PopupStep::Dismissed) => {
                        actions += 1;
                        self.diagnostic.navigation_actions += 1;
                        if actions >= MAX_RECOVERY_ACTIONS {
                            return Err(VerificationReason::NavigationBudgetExhausted);
                        }
                        continue;
                    }
                    ReadWaitResult::Ready(PopupStep::Blocked) => {
                        return Err(VerificationReason::UnrecognizedDialog)
                    }
                    _ => return Err(VerificationReason::SearchBudgetExhausted),
                }
            }
            let mut screen = classify(&tree, self.plan);
            // Also on the first profile read: a pass that ran out of time can leave its
            // own viewer open, and the next pass must be able to Back out of it.
            if screen == Screen::Unknown {
                if let Some(surface) = hidden_post_restore_surface(&tree, self.plan) {
                    let epoch = self.session.gui_session_epoch();
                    let fresh = self.read().await?;
                    if self.session.gui_session_epoch() != epoch {
                        return Err(VerificationReason::StaleSnapshot);
                    }
                    screen = if hidden_post_restore_surface(&fresh, self.plan).as_ref() == Some(&surface) {
                        Screen::Post
                    } else {
                        classify(&fresh, self.plan)
                    };
                    tree = fresh;
                }
            }
            self.diagnostic.screen_state = format!("{screen:?}");
            if screen == Screen::Unknown && self.diagnostic.unknown_screen_shape.is_none() {
                self.diagnostic.unknown_screen_shape =
                    Some(UnknownScreenShape::from_tree(&tree, self.plan));
            }
            if screen == Screen::Profile {
                self.account(restoring).await?;
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
            if screen == Screen::SecurityPrompt {
                return Err(VerificationReason::SecurityPrompt);
            }
            if screen == Screen::UnrecognizedDialog {
                return Err(VerificationReason::UnrecognizedDialog);
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
            } else if matches!(screen, Screen::Post | Screen::Share | Screen::Comments)
                && (last_screen.as_ref() != Some(&screen)
                    || last_action_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(5)))
            {
                // Back only from a proved post detail/share/comments surface, never a
                // composer, arbitrary activity, or unlabelled screen. A measured share
                // sheet, left open by an earlier pass, answers only its own Close.
                let close = (screen == Screen::Share)
                    .then(|| tree.share_sheet_close(self.plan.labels.package()))
                    .flatten();
                match close {
                    Some(close) => self.session.tap(close.centre()).await,
                    None => self.session.back().await,
                }
                .map_err(|_| VerificationReason::ReadFailed)?;
                last_screen = Some(screen);
                last_action_at = Some(Instant::now());
                actions += 1;
                self.diagnostic.navigation_actions += 1;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    fn post_proof(&mut self, tree: &Tree) -> Result<(), VerificationReason> {
        if tree.publication_removed(self.plan.labels.package()) {
            return Err(VerificationReason::CaptionMismatch);
        }
        let captions = tree.matching(
            self.plan.labels.package(),
            ElementQuery::ResourceIdSuffix(self.plan.caption_id),
        );
        let times = self.time_nodes(tree);
        self.diagnostic.caption_candidates = captions.len();
        self.diagnostic.time_candidates = times.len();
        if self.trace_enabled() {
            let caption_digest = captions
                .first()
                .filter(|_| captions.len() == 1)
                .map(|index| self.trace_digest(tree.nodes[*index].attr("text")));
            let time_digest = times
                .first()
                .filter(|_| times.len() == 1)
                .map(|index| self.trace_digest(tree.nodes[*index].attr("text")));
            if let Some(last) = self.diagnostic.candidate_trace.last_mut() {
                last.post_snapshot_generation = Some(tree.generation);
                last.caption_digest = caption_digest;
                last.time_digest = time_digest;
            }
        }
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
            // A short ellipsized prefix authorizes ONLY opening the caption.
            // Trill S8 38.3.2 (14/09) truncates at ~55 chars, below the legacy
            // 64-char matching threshold. Full text equality is still required.
            // Global45.7.3/en, measured own-photo desc: TikTok places one
            // WORD JOINER after the ellipsis. Remove it only for expansion
            // admission; full caption equality above retains every character.
            let expansion_visible = if self.plan.labels.package() == "com.zhiliaoapp.musically"
                && self.plan.labels.resource_version() == Some("45.7.3")
                && self.plan.labels.language() == "en"
                && visible.ends_with("…\u{2060}")
            {
                visible.strip_suffix('\u{2060}').unwrap_or(visible)
            } else {
                visible
            };
            let expansion_prefix = normalize(expansion_visible);
            let short_prefix = ["...", "…"]
                .iter()
                .find_map(|suffix| expansion_prefix.strip_suffix(suffix))
                .map(str::trim_end)
                .is_some_and(|prefix| {
                    prefix.chars().count() >= 20 && normalize(self.caption).starts_with(prefix)
                });
            if !(visible_caption_matches(expansion_visible, self.caption) || short_prefix) {
                return Err(VerificationReason::CaptionMismatch);
            }
            if !self.caption_expanded {
                // The folded tile viewer is the only snapshot with this post's time label.
                if let [time] = times.as_slice() {
                    let (text, now) = (tree.nodes[*time].attr("text"), chrono::Utc::now());
                    let submitted = &self.identity.submitted_at;
                    let exact = relative_post_time_matches(text, submitted, now);
                    let coarse = relative_post_time_consistent(text, submitted, now);
                    self.device.tile_time_matched = exact || coarse;
                    self.device.id_time_required = !exact && coarse;
                }
            }
            return Err(VerificationReason::CaptionTruncated);
        }
        self.diagnostic
            .publication_evidence
            .get_or_insert_with(Default::default)
            .caption_matched = true;
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
            self.diagnostic
                .publication_evidence
                .get_or_insert_with(Default::default)
                .submission_time_matched = true;
            return Ok(());
        }
        if relative_post_age(text).is_none() {
            return Err(VerificationReason::TimestampUnmeasured);
        }
        if submission_time_recheck_delay(text, &self.identity.submitted_at, now).is_some() {
            return Err(VerificationReason::SubmissionTimeAmbiguous);
        }
        if relative_post_time_consistent(text, &self.identity.submitted_at, now) {
            self.device.id_time_required = true;
            return Ok(());
        }
        Err(VerificationReason::SubmissionTooOld)
    }

    fn time_nodes(&self, tree: &Tree) -> Vec<usize> {
        let times = tree.matching(
            self.plan.labels.package(),
            ElementQuery::ResourceIdSuffix(self.plan.time_id),
        );
        if !times.is_empty() || !self.plan.labels.adaptive() {
            return times;
        }
        tree.nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| {
                n.visible(self.plan.labels.package())
                    && tree.ancestors_visible(*i)
                    && n.rect().is_some()
                    && relative_post_age(n.attr("text")).is_some()
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The second snapshot of the folded-caption proof (`DeviceProof`): this pass tapped
    /// the folded caption of a tile viewer whose time matched, and this is the measured
    /// expanded photo page with the exact submitted caption.
    fn expanded_caption_proof(&mut self, tree: &Tree) -> Result<(), VerificationReason> {
        let labels = &self.plan.labels;
        let measured = labels.package() == "com.zhiliaoapp.musically"
            && labels.resource_version() == Some("45.7.3")
            && labels.language() == "en";
        if !measured
            || !self.caption_expanded
            || !self.device.tile_time_matched
            || !super::photo_proof::expanded_photo_surface(tree, labels.package(), "45.7.3")
        {
            return Err(VerificationReason::CaptionMissing);
        }
        let (caption_id, _) = super::photo_proof::viewer_controls(labels.package(), "45.7.3")
            .map_err(|_| VerificationReason::CaptionMissing)?;
        let captions = tree.matching(labels.package(), ElementQuery::ResourceIdSuffix(caption_id));
        let [caption] = captions.as_slice() else {
            return Err(VerificationReason::CaptionAmbiguous);
        };
        let normalize = |value: &str| {
            value
                .trim_matches(['\u{200e}', '\u{200f}'])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        if normalize(tree.nodes[*caption].attr("text")) != normalize(self.caption) {
            return Err(VerificationReason::CaptionMismatch);
        }
        let evidence = self
            .diagnostic
            .publication_evidence
            .get_or_insert_with(Default::default);
        evidence.caption_matched = true;
        evidence.submission_time_matched = !self.device.id_time_required;
        self.device.expanded_page = true;
        Ok(())
    }

    async fn prove_post(&mut self) -> Result<Tree, VerificationReason> {
        let start = Instant::now();
        loop {
            let tree = self.read().await?;
            let mut proof = self.post_proof(&tree);
            if matches!(proof, Err(VerificationReason::CaptionMissing)) && self.caption_expanded {
                match self.expanded_caption_proof(&tree) {
                    Err(VerificationReason::CaptionMissing) => {}
                    other => proof = other,
                }
            }
            if let Err(reason) = proof {
                self.mark_candidate(reason);
            }
            // A tile tap can first return the profile while the viewer opens.
            // Recheck the settled video before expanding its caption into the
            // comments drawer. Only the full public proof may populate this link.
            // A measured truncated Unicode placeholder looks like a mismatch
            // here; the candidate filter and exact public metadata decide it.
            if !self.metadata.skip_public_proof
                && matches!(
                    proof,
                    Err(VerificationReason::CaptionTruncated
                        | VerificationReason::CaptionMismatch
                        | VerificationReason::TimestampMissing)
                )
                && tree.nodes.iter().any(|node| {
                    node.visible(self.plan.labels.package()) && node.attr("content-desc") == "Video"
                })
            {
                match self.visible_video().await {
                    Ok(Some(link)) => {
                        self.public_link = Some(link);
                        return Ok(tree);
                    }
                    Ok(None)
                    | Err(
                        VerificationReason::ClipboardUnchanged
                        | VerificationReason::ClipboardNotPostLink,
                    ) => {}
                    // This viewer was opened from the own Profile: its caption and time
                    // can still prove it on the device below.
                    Err(VerificationReason::MetadataUnavailable) => self.defer_public_metadata(),
                    Err(reason) => return Err(reason),
                }
            }
            match proof {
                Ok(()) => {
                    self.mark_candidate(VerificationReason::Verified);
                    return Ok(tree);
                }
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
                        .activate_element(ElementQuery::ResourceIdSuffix(self.plan.caption_id))
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
                        || self.started.elapsed() + delay >= self.window()
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
            // The "Send to" variant has no Copy row; its measured Close is the only tap.
            let close = tree
                .copy_control(self.plan.labels.package())
                .is_ok_and(|row| row.is_none())
                .then(|| tree.share_sheet_close(self.plan.labels.package()))
                .flatten();
            match close {
                Some(close) => self.session.tap(close.centre()).await,
                None => self.session.back().await,
            }
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
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        if self.diagnostic.copy_attempts >= MAX_COPY_ATTEMPTS {
            return Err(VerificationReason::CopyBudgetExhausted);
        }
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
        let share = if self.device.expanded_page {
            self.expanded_caption_proof(&fresh)?;
            let (_, share_id) =
                super::photo_proof::viewer_controls(self.plan.labels.package(), "45.7.3")
                    .map_err(|_| VerificationReason::ShareUnavailable)?;
            let shares = fresh.matching(
                self.plan.labels.package(),
                ElementQuery::ResourceIdSuffix(share_id),
            );
            let [share] = shares.as_slice() else {
                return Err(VerificationReason::ShareUnavailable);
            };
            fresh.nodes[*share]
                .rect()
                .filter(|rect| rect.enabled && rect.clickable)
                .ok_or(VerificationReason::ShareUnavailable)?
        } else {
            self.post_proof(&fresh)?;
            fresh
                .control(
                    self.plan.labels.package(),
                    self.plan.labels.label(TikTokControl::Share),
                )
                .ok_or(VerificationReason::ShareUnavailable)?
        };
        let mut winning_proof = self.diagnostic.clone();
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
                    if self.other_publication_urls.contains(&canonical) {
                        return Err(VerificationReason::OtherPublication);
                    }
                    // A folded caption read its time on another snapshot and a coarse label
                    // did not prove it: the post ID must have been allocated inside this
                    // submission's window (`DeviceProof`).
                    if self.device.expanded_page || self.device.id_time_required {
                        if super::photo_proof::validate_photo_identity(&canonical, self.identity)
                            .is_err()
                        {
                            return Err(VerificationReason::SubmissionTooOld);
                        }
                        winning_proof
                            .publication_evidence
                            .get_or_insert_with(Default::default)
                            .submission_time_matched = true;
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

    async fn visible_video(&mut self) -> Result<Option<String>, VerificationReason> {
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        let result = super::photo_proof::capture_visible_video_link_counted(
            self.session,
            self.plan.labels.package(),
            self.caption,
            self.identity,
            &mut self.diagnostic.copy_attempts,
            self.other_publication_urls,
        )
        .await;
        // Retain a copied candidate even if the metadata request consumed the
        // remaining phone-search window. The next lookup needs no phone session.
        if let Err(error) = &result {
            if error.is::<super::photo_proof::CapturedMetadataFailure>() {
                self.matched_photo_failure(error);
            }
        }
        if self.expired() {
            return Err(VerificationReason::SearchBudgetExhausted);
        }
        match result {
            Ok(link) => {
                self.diagnostic.stage = "videoPublicProof";
                Ok(Some(link))
            }
            Err(error) => {
                if let Some(viewer) =
                    error.downcast_ref::<super::photo_proof::PhotoCarouselViewer>()
                {
                    // Photo carousels copy before `post_proof`. Only this complete
                    // one-snapshot read may advance the restart frontier; it never
                    // selects a link or proves publication.
                    if viewer.caption_and_time_matched {
                        let evidence = self
                            .diagnostic
                            .publication_evidence
                            .get_or_insert_with(Default::default);
                        evidence.caption_matched = true;
                        evidence.submission_time_matched = true;
                    }
                    let photo = super::photo_proof::capture_expanded_photo_link_counted(
                        self.session,
                        self.plan.labels.package(),
                        self.caption,
                        self.identity,
                        &mut self.diagnostic.copy_attempts,
                        self.other_publication_urls,
                    )
                    .await;
                    if let Err(error) = &photo {
                        if error.is::<super::photo_proof::CapturedMetadataFailure>() {
                            self.matched_photo_failure(error);
                        }
                    }
                    if self.expired() {
                        return Err(VerificationReason::SearchBudgetExhausted);
                    }
                    return match photo {
                        Ok(link) => {
                            self.diagnostic.stage = "expandedPhotoPublicProof";
                            Ok(Some(link))
                        }
                        Err(error) => {
                            self.diagnostic.expanded_photo_error = Some(error.to_string());
                            if error.is::<super::photo_proof::EarlierPublication>() {
                                return Err(VerificationReason::SubmissionTooOld);
                            }
                            match self.matched_photo_failure(&error) {
                                Some(reason) => Err(reason),
                                None => Ok(None),
                            }
                        }
                    };
                }
                if self.plan.labels.package() == "com.zhiliaoapp.musically"
                    && self.plan.labels.resource_version() == Some("45.7.3")
                {
                    if let Some(mismatch) =
                        error.downcast_ref::<super::photo_proof::FullVideoCaptionMismatch>()
                    {
                        let caption_digest = self.trace_digest(&mismatch.caption);
                        let time_digest =
                            mismatch.time.as_deref().map(|time| self.trace_digest(time));
                        if let Some(last) = self.diagnostic.candidate_trace.last_mut() {
                            last.post_snapshot_generation = Some(mismatch.generation);
                            last.caption_digest = Some(caption_digest);
                            last.time_digest = time_digest;
                        }
                        return Err(VerificationReason::CaptionMismatch);
                    }
                }
                self.diagnostic.expanded_photo_error = Some(format!("video: {error:#}"));
                match self.matched_photo_failure(&error) {
                    Some(reason) => Err(reason),
                    None => Ok(None),
                }
            }
        }
    }

    async fn capture(&mut self) -> Result<String, VerificationReason> {
        // 45.7.3/en (box 31-50, 10/10): the open viewer can say "Post is being
        // processed" while the Profile grid tile already carries the caption and
        // time. Keep searching the grid; report Processing only if that fails too.
        // A viewer whose Share sheet had no Copy link (the "Send to" variant, phone
        // 44, 14/10/2026) is kept the same way.
        let mut processing = None;
        // (1) and (2) prove a post only through TikTok's public metadata, since a viewer
        // opened from the feed cannot prove ownership. When that metadata does not answer,
        // the own-Profile grid (3) proves the post on the device instead of waiting.
        if !self.metadata.skip_public_proof {
            match self.visible_video().await {
                Ok(Some(link)) => return Ok(link),
                Ok(None)
                | Err(
                    VerificationReason::OtherPublication
                    | VerificationReason::CaptionMismatch
                    | VerificationReason::SubmissionTooOld,
                ) => {}
                Err(
                    reason @ (VerificationReason::Processing
                    | VerificationReason::ClipboardUnchanged
                    | VerificationReason::ClipboardNotPostLink
                    | VerificationReason::CopyUnavailable
                    | VerificationReason::ShareUnavailable),
                ) => {
                    processing = Some(reason);
                }
                Err(VerificationReason::MetadataUnavailable) => self.defer_public_metadata(),
                Err(reason) => return Err(reason),
            }
        }
        if !self.metadata.skip_public_proof {
            match super::photo_proof::capture_expanded_photo_link_counted(
                self.session,
                self.plan.labels.package(),
                self.caption,
                self.identity,
                &mut self.diagnostic.copy_attempts,
                self.other_publication_urls,
            )
            .await
            {
                Ok(link) => {
                    self.diagnostic.stage = "expandedPhotoPublicProof";
                    return Ok(link);
                }
                Err(error) => match self.matched_photo_failure(&error) {
                    Some(
                        reason @ (VerificationReason::Processing
                        | VerificationReason::ClipboardUnchanged
                        | VerificationReason::ClipboardNotPostLink
                        | VerificationReason::CopyUnavailable
                        | VerificationReason::ShareUnavailable),
                    ) => processing = Some(reason),
                    Some(VerificationReason::MetadataUnavailable) => self.defer_public_metadata(),
                    Some(
                        VerificationReason::OtherPublication | VerificationReason::SubmissionTooOld,
                    )
                    | None => {}
                    Some(reason) => return Err(reason),
                },
            }
        }
        self.diagnostic.stage = "profile";
        let mut tree = self.profile(false).await?;
        // TikTok keeps the Profile grid it last loaded. r118 14/10/2026, phone 47
        // (ce051715ab9fee2002, Global45.7.3/en): 90 minutes after Post the newest tile was a
        // 12-hour-old post, so every own-Profile proof ran out of tiles. Re-selecting the
        // Profile tab reloads it (measured by hand on phone 36: counters 0→3, the new post
        // first). Once per pass, the bottom-navigation Profile control only, then the
        // account is proven again on the reloaded grid.
        let before = grid_signature(&tree, self.plan);
        if let Some(tab) = tree.control(
            self.plan.labels.package(),
            self.plan.labels.label(TikTokControl::ProfileTab),
        ) {
            if self.expired() {
                return Err(VerificationReason::SearchBudgetExhausted);
            }
            self.diagnostic.stage = "profileReload";
            self.session
                .tap(tab.centre())
                .await
                .map_err(|_| VerificationReason::ReadFailed)?;
            self.diagnostic.navigation_actions += 1;
            let start = Instant::now();
            while start.elapsed() < PROFILE_RELOAD_WINDOW {
                tokio::time::sleep(POLL).await;
                if grid_signature(&self.read().await?, self.plan) != before {
                    break;
                }
            }
            self.diagnostic.stage = "profile";
            tree = self.profile(true).await?;
        }
        // Trill38.3.2/en, SM-G955U1 (19/09): suggestions can push the entire
        // Videos grid under bottom navigation. Account proof above is mandatory.
        // Collapse only the measured suggestions group, once, then prove the
        // profile again. This never grants publication proof or a new Post.
        if tree.grid(self.plan).is_empty() {
            if let Some(button) = tree.profile_suggestions_hide(self.plan) {
                if self.expired() {
                    return Err(VerificationReason::SearchBudgetExhausted);
                }
                self.diagnostic.stage = "profileSuggestionsHide";
                self.session
                    .tap(button.centre())
                    .await
                    .map_err(|_| VerificationReason::ReadFailed)?;
                self.diagnostic.navigation_actions += 1;
                tokio::time::sleep(POLL).await;
                tree = self.profile(true).await?;
                if tree.profile_suggestions_hide(self.plan).is_some() {
                    return Err(VerificationReason::PostNotVisible);
                }
            }
        }
        let mut last_reason = processing.unwrap_or(VerificationReason::PostNotVisible);
        for page in 0..MAX_VIEWPORTS {
            self.diagnostic.viewports_visited = page + 1;
            let mut visited = HashSet::new();
            let mut candidate = None;
            let mut unresolved_candidate = None;
            let mut page_index = 0;
            loop {
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
                if self.expired()
                    || candidate_budget_exhausted(
                        self.plan,
                        &self.diagnostic.candidate_trace,
                        self.diagnostic.candidates_visited,
                    )
                {
                    return Err(VerificationReason::SearchBudgetExhausted);
                }
                visited.insert((
                    tile.x.to_bits(),
                    tile.y.to_bits(),
                    tile.width.to_bits(),
                    tile.height.to_bits(),
                ));
                self.diagnostic.candidates_visited += 1;
                page_index += 1;
                if self.trace_enabled() {
                    self.diagnostic
                        .candidate_trace
                        .push(VerificationCandidateTrace {
                            canonical_url: None,
                            post_id_allocated_at: None,
                            viewport: page + 1,
                            index: page_index,
                            profile_snapshot_generation: tree.generation,
                            post_snapshot_generation: None,
                            tile_bounds: [tile.x, tile.y, tile.width, tile.height],
                            caption_digest: None,
                            time_digest: None,
                            reason_code: None,
                            started_at_ms: self.started.elapsed().as_millis() as u64,
                            elapsed_ms: None,
                        });
                }
                self.diagnostic.stage = "postProof";
                self.caption_expanded = false;
                self.device = DeviceProof::default();
                self.public_link = None;
                self.session
                    .tap(tile.centre())
                    .await
                    .map_err(|_| VerificationReason::ReadFailed)?;
                // A video caption opens Comments on measured Trill builds. Copy
                // the visible video only through exact public metadata proof,
                // before any caption expansion hides its Share control.
                'candidate_proof: {
                    let public = if self.metadata.skip_public_proof {
                        Ok(None)
                    } else {
                        self.visible_video().await
                    };
                    match public {
                        Ok(Some(link)) => {
                            self.mark_candidate(VerificationReason::Verified);
                            self.remember_proven_candidate(
                                &mut candidate,
                                (link, self.diagnostic.clone()),
                            )?;
                            break 'candidate_proof;
                        }
                        Ok(None) => {}
                        Err(VerificationReason::OtherPublication) => {
                            self.mark_candidate(VerificationReason::OtherPublication);
                            last_reason = VerificationReason::OtherPublication;
                            break 'candidate_proof;
                        }
                        Err(VerificationReason::CaptionMismatch) => {
                            self.mark_candidate(VerificationReason::CaptionMismatch);
                            last_reason = VerificationReason::CaptionMismatch;
                            break 'candidate_proof;
                        }
                        Err(VerificationReason::SubmissionTooOld) => {
                            self.mark_candidate(VerificationReason::SubmissionTooOld);
                            last_reason = VerificationReason::SubmissionTooOld;
                            break 'candidate_proof;
                        }
                        Err(
                            VerificationReason::ClipboardUnchanged
                            | VerificationReason::ClipboardNotPostLink,
                        ) => {}
                        // A tile opened from the own Profile proves itself on the device
                        // below: account, full caption, submission time, then Copy link.
                        Err(VerificationReason::MetadataUnavailable) => {
                            self.defer_public_metadata()
                        }
                        Err(reason) => {
                            self.mark_candidate(reason);
                            return Err(reason);
                        }
                    }
                    match self.prove_post().await {
                        Ok(mut post) => {
                            if let Some(link) = self.public_link.take() {
                                self.mark_candidate(VerificationReason::Verified);
                                self.diagnostic.stage = "videoPublicProof";
                                self.remember_proven_candidate(
                                    &mut candidate,
                                    (link, self.diagnostic.clone()),
                                )?;
                                break 'candidate_proof;
                            }
                            if candidate.is_some() && self.other_publication_urls.is_empty() {
                                return Err(VerificationReason::MultipleMatchingPosts);
                            }
                            self.diagnostic.stage = "copy";
                            for attempt in 0..2 {
                                let copied = self.copy_once(post).await;
                                let closed = self.close_share().await;
                                match copied {
                                    Ok(link) => {
                                        closed?;
                                        self.mark_candidate(VerificationReason::Verified);
                                        self.remember_proven_candidate(&mut candidate, link)?;
                                        break;
                                    }
                                    Err(VerificationReason::OtherPublication) => {
                                        closed?;
                                        self.mark_candidate(VerificationReason::OtherPublication);
                                        last_reason = VerificationReason::OtherPublication;
                                        break;
                                    }
                                    Err(VerificationReason::ClipboardUnchanged) if attempt == 0 => {
                                        closed?;
                                        post = match self.prove_post().await {
                                            Err(VerificationReason::OtherPublication) => {
                                                self.mark_candidate(
                                                    VerificationReason::OtherPublication,
                                                );
                                                last_reason = VerificationReason::OtherPublication;
                                                break;
                                            }
                                            result => result?,
                                        };
                                        if let Some(link) = self.public_link.take() {
                                            self.diagnostic.stage = "videoPublicProof";
                                            self.remember_proven_candidate(
                                                &mut candidate,
                                                (link, self.diagnostic.clone()),
                                            )?;
                                            break 'candidate_proof;
                                        }
                                    }
                                    Err(reason) => {
                                        self.mark_candidate(reason);
                                        return Err(reason);
                                    }
                                }
                            }
                        }
                        Err(reason) => {
                            self.mark_candidate(reason);
                            // The delayed-viewer path may already have attempted
                            // Copy through public video proof. Preserve that exact
                            // failure instead of searching older tiles and replacing
                            // its diagnosis with a caption/navigation mismatch.
                            if matches!(
                                reason,
                                VerificationReason::Processing
                                    | VerificationReason::ClipboardUnchanged
                                    | VerificationReason::ClipboardUnreadable
                                    | VerificationReason::ClipboardUnwritable
                                    | VerificationReason::ClipboardNotPostLink
                                    | VerificationReason::CopyUnavailable
                                    | VerificationReason::CopyAmbiguous
                                    | VerificationReason::CopyBudgetExhausted
                                    | VerificationReason::ShareUnavailable
                                    | VerificationReason::ReadFailed
                                    | VerificationReason::MetadataUnavailable
                            ) {
                                return Err(reason);
                            }
                            let mut reason = reason;
                            if !self.metadata.skip_public_proof
                                && matches!(
                                    reason,
                                    VerificationReason::CaptionMissing
                                        | VerificationReason::TimestampMissing
                                        | VerificationReason::CaptionTruncated
                                )
                            {
                                match super::photo_proof::capture_expanded_photo_link_counted(
                                    self.session,
                                    self.plan.labels.package(),
                                    self.caption,
                                    self.identity,
                                    &mut self.diagnostic.copy_attempts,
                                    self.other_publication_urls,
                                )
                                .await
                                {
                                    Ok(link) => {
                                        self.mark_candidate(VerificationReason::Verified);
                                        self.diagnostic.stage = "expandedPhotoPublicProof";
                                        self.remember_proven_candidate(
                                            &mut candidate,
                                            (link, self.diagnostic.clone()),
                                        )?;
                                        break 'candidate_proof;
                                    }
                                    Err(error) => {
                                        if let Some(failure) = self.matched_photo_failure(&error) {
                                            if !matches!(
                                                failure,
                                                VerificationReason::OtherPublication
                                                    | VerificationReason::SubmissionTooOld
                                            ) {
                                                return Err(failure);
                                            }
                                            reason = failure;
                                            self.mark_candidate(reason);
                                        }
                                    }
                                }
                            }
                            if !matches!(
                                reason,
                                VerificationReason::CaptionMismatch
                                    | VerificationReason::SubmissionTooOld
                                    | VerificationReason::OtherPublication
                            ) {
                                unresolved_candidate = Some(reason);
                            }
                            last_reason = unresolved_candidate.unwrap_or(reason);
                        }
                    }
                }
                self.diagnostic.stage = "restoreProfile";
                tree = match self.profile(true).await {
                    Ok(tree) => tree,
                    Err(reason) => {
                        self.mark_candidate(reason);
                        return Err(reason);
                    }
                };
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
                        processing.unwrap_or(VerificationReason::PostNotVisible)
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

/// Compatibility entry for callers without a persisted assignment owner.
pub async fn capture_submission_link(
    session: &dyn UiSession,
    plan: &PublishVerificationPlan,
    caption: &str,
    identity: &SubmissionIdentity,
) -> VerificationCapture {
    capture_submission_link_excluding(session, plan, caption, identity, &[]).await
}

/// Exclude durable other-assignment links while retaining all identity proofs.
/// No future is force-cancelled while it may own an IME transition. Budgets
/// prevent the next action; an in-flight primitive completes its own restore.
pub async fn capture_submission_link_excluding(
    session: &dyn UiSession,
    plan: &PublishVerificationPlan,
    caption: &str,
    identity: &SubmissionIdentity,
    other_publication_urls: &[String],
) -> VerificationCapture {
    capture_with(
        session,
        plan,
        caption,
        identity,
        other_publication_urls,
        None,
    )
    .await
}

/// A candidate retained while TikTok's public metadata did not answer, proven on the own
/// Profile instead: account, full caption, submission time, Copy link and canonical
/// redirect. Only a link to the candidate's own post ID verifies; public metadata is not
/// asked again in this pass, and nothing here reaches the composer or Post.
pub async fn capture_retained_candidate_link(
    session: &dyn UiSession,
    plan: &PublishVerificationPlan,
    caption: &str,
    identity: &SubmissionIdentity,
    other_publication_urls: &[String],
    candidate_canonical_url: &str,
) -> VerificationCapture {
    let candidate = post_id(candidate_canonical_url)
        .unwrap_or_default()
        .to_owned();
    capture_with(
        session,
        plan,
        caption,
        identity,
        other_publication_urls,
        Some(candidate),
    )
    .await
}

async fn capture_with(
    session: &dyn UiSession,
    plan: &PublishVerificationPlan,
    caption: &str,
    identity: &SubmissionIdentity,
    other_publication_urls: &[String],
    retained_post_id: Option<String>,
) -> VerificationCapture {
    let mut capture = Capture {
        session,
        plan,
        caption,
        identity,
        other_publication_urls,
        started: Instant::now(),
        caption_expanded: false,
        public_link: None,
        trace_nonce: *uuid::Uuid::new_v4().as_bytes(),
        metadata: MetadataRoute {
            skip_public_proof: retained_post_id.is_some(),
            retained_post_id,
            unavailable_here: false,
        },
        device: DeviceProof::default(),
        diagnostic: VerificationDiagnostic {
            captured_metadata_candidate: None,
            publication_evidence: None,
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
            copy_link_counts_as_share: true,
            expanded_photo_error: None,
            candidate_trace: Vec::new(),
            elapsed_ms: 0,
            navigation_matches: 0,
            navigation_enabled: 0,
            navigation_clickable: 0,
            screen_state: String::new(),
            unknown_screen_shape: None,
        },
    };
    let result = if !submission_identity_valid(identity) || caption.trim().is_empty() {
        Err(VerificationReason::IdentityMissing)
    } else {
        capture.capture().await
    };
    // A proven tile whose viewport could not be fully ruled out stays unverified (the
    // ambiguity guard holds), but it is not an empty search.
    let result = match result {
        Err(VerificationReason::SearchBudgetExhausted) if capture.proven() => {
            Err(VerificationReason::AmbiguityScanIncomplete)
        }
        result => result,
    };
    if matches!(
        result,
        Err(VerificationReason::SearchBudgetExhausted | VerificationReason::AmbiguityScanIncomplete)
    ) && matches!(
        capture.diagnostic.stage,
        "postProof" | "copy" | "videoPublicProof" | "expandedPhotoPublicProof" | "restoreProfile"
    ) {
        capture.leave_own_viewer().await;
    }
    // The own-Profile proof did not settle a candidate whose public metadata did not answer:
    // keep that outcome so the retained candidate keeps its route. A second matching post
    // still refuses.
    let result = match result {
        Err(reason)
            if capture.metadata.unavailable_here
                && reason != VerificationReason::MultipleMatchingPosts =>
        {
            Err(VerificationReason::MetadataUnavailable)
        }
        result => result,
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
                VerificationReason::Processing => OwnPostLink::ReadFailed(reason.message().into()),
                VerificationReason::MetadataUnavailable => OwnPostLink::SubmissionUnverified,
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
                | VerificationReason::AmbiguityScanIncomplete
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
