//! Observations for the operator timeline. These never authorize a device action.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishProgress {
    RehearsalReady,
    WaitingTransfer,
    WaitingControl,
    CheckingDevice,
    DeviceReady,
    TransferringMedia { count: usize, video: bool },
    MediaTransferred,
    OpeningApp,
    AppReady,
    OpeningComposer,
    OpeningGallery,
    SelectingAlbum,
    SelectingMedia { count: usize, video: bool },
    MediaSelected { count: usize, video: bool },
    OpeningEditor,
    OpeningSounds,
    SelectingSound { title: String },
    SoundConfirmed { title: String },
    OpeningCaption,
    EnteringCaption,
    CaptionConfirmed,
    CheckingBeforePost,
    SubmittingPost,
    AwaitingPost,
    PostSubmitted,
    PostConfirmed,
    CapturingLink,
    LinkCaptured,
    LinkPending { reason: String },
    Finishing,
    ReleasingPendingUpload,
    Finished,
    FailedBeforePost { reason: String },
    PostUncertain { reason: String },
}

pub type PublishProgressObserver<'a> = dyn Fn(PublishProgress) + Send + Sync + 'a;

impl PublishProgress {
    pub fn state(&self) -> &'static str {
        match self {
            Self::RehearsalReady => "rehearsal_ready",
            Self::WaitingTransfer => "waiting_transfer",
            Self::WaitingControl => "waiting_control",
            Self::CheckingDevice => "checking_device",
            Self::DeviceReady => "device_ready",
            Self::TransferringMedia { .. } => "transferring_media",
            Self::MediaTransferred => "media_transferred",
            Self::OpeningApp => "opening_app",
            Self::AppReady => "app_ready",
            Self::OpeningComposer => "opening_composer",
            Self::OpeningGallery => "opening_gallery",
            Self::SelectingAlbum => "selecting_album",
            Self::SelectingMedia { .. } => "selecting_media",
            Self::MediaSelected { .. } => "media_selected",
            Self::OpeningEditor => "opening_editor",
            Self::OpeningSounds => "opening_sounds",
            Self::SelectingSound { .. } => "selecting_sound",
            Self::SoundConfirmed { .. } => "sound_confirmed",
            Self::OpeningCaption => "opening_caption",
            Self::EnteringCaption => "entering_caption",
            Self::CaptionConfirmed => "caption_confirmed",
            Self::CheckingBeforePost => "checking_before_post",
            Self::SubmittingPost => "submitting_post",
            Self::AwaitingPost => "awaiting_post",
            Self::PostSubmitted => "post_submitted",
            Self::PostConfirmed => "post_confirmed",
            Self::CapturingLink => "capturing_link",
            Self::LinkCaptured => "link_captured",
            Self::LinkPending { .. } => "link_pending",
            Self::Finishing => "finishing",
            Self::ReleasingPendingUpload => "releasing_pending_upload",
            Self::Finished => "finished",
            Self::FailedBeforePost { .. } => "failed_before_post",
            Self::PostUncertain { .. } => "post_uncertain",
        }
    }

    pub fn message(&self, device: &str) -> String {
        match self {
            Self::RehearsalReady => "Đã kiểm tra tới trước nút Đăng".into(),
            Self::WaitingTransfer => "Đang chờ lượt tải nội dung vào máy".into(),
            Self::WaitingControl => "Đã tải xong; chờ lượt điều khiển để đăng".into(),
            Self::CheckingDevice => format!("Kiểm tra kết nối và khả năng đăng bài của {device}"),
            Self::DeviceReady => format!("Đã xác nhận {device} sẵn sàng"),
            Self::TransferringMedia { count, video } => format!(
                "Đang tải {count} {} vào điện thoại",
                if *video { "video" } else { "ảnh" }
            ),
            Self::MediaTransferred => {
                "Đã tải và xác nhận nội dung trong thư viện điện thoại".into()
            }
            Self::OpeningApp => "Đang mở TikTok và chờ màn hình sẵn sàng".into(),
            Self::AppReady => "TikTok đã mở, phiên điều khiển sẵn sàng".into(),
            Self::OpeningComposer => "Bấm dấu + để tạo bài đăng".into(),
            Self::OpeningGallery => "Mở thư viện ảnh/video".into(),
            Self::SelectingAlbum => "Tìm và chọn album nội dung của bài này".into(),
            Self::SelectingMedia { count, video } => format!(
                "Đang chọn {count} {} theo thứ tự",
                if *video { "video" } else { "ảnh" }
            ),
            Self::MediaSelected { count, video } => format!(
                "Đã xác nhận chọn đủ {count} {}",
                if *video { "video" } else { "ảnh" }
            ),
            Self::OpeningEditor => "Bấm Tiếp để mở màn chỉnh sửa".into(),
            Self::OpeningSounds => "Mở bảng nhạc và đọc danh sách nhạc đề xuất".into(),
            Self::SelectingSound { title } => format!("Đang chọn nhạc: {title}"),
            Self::SoundConfirmed { title } => format!("Đã xác nhận nhạc: {title}"),
            Self::OpeningCaption => "Bấm Tiếp để mở màn nhập nội dung bài đăng".into(),
            Self::EnteringCaption => "Đang nhập nội dung chữ và hashtag".into(),
            Self::CaptionConfirmed => "Đã đọc lại và xác nhận nội dung chữ".into(),
            Self::CheckingBeforePost => "Kiểm tra lại nhạc và nội dung trước khi đăng".into(),
            Self::SubmittingPost => "Bấm Đăng bài".into(),
            Self::AwaitingPost => "Đã bấm Đăng, đang chờ TikTok xác nhận".into(),
            Self::PostSubmitted => {
                "Đã gửi bài; TikTok có thể đang tải hoặc xử lý, đang chờ xác minh".into()
            }
            Self::PostConfirmed => {
                "Đã xác minh bài đăng bằng liên kết đúng bài trên tài khoản".into()
            }
            Self::CapturingLink => "Đang lấy liên kết của bài vừa đăng".into(),
            Self::LinkCaptured => "Đã lấy liên kết bài đăng".into(),
            Self::LinkPending { .. } => {
                "Đã gửi bài; hệ thống sẽ kiểm tra liên kết định kỳ, xem chi tiết".into()
            }
            Self::Finishing => "Đang kết thúc phiên và xử lý nội dung tạm".into(),
            Self::ReleasingPendingUpload => {
                "Đang trả quyền điều khiển; giữ TikTok và nội dung để tiếp tục xác minh".into()
            }
            Self::Finished => format!("Thành công — {device} đã đăng bài"),
            Self::FailedBeforePost { .. } => {
                "Dừng ở bước trên — chưa bấm Đăng, xem chi tiết lỗi".into()
            }
            Self::PostUncertain { .. } => {
                "Chưa xác nhận được kết quả sau Đăng — cần kiểm tra bài trên TikTok".into()
            }
        }
    }

    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::LinkPending { reason }
            | Self::FailedBeforePost { reason }
            | Self::PostUncertain { reason } => Some(reason),
            _ => None,
        }
    }
}
