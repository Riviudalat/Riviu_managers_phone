//! Global 45.7.3/en popups on the feed, profile, post detail, comments, composer, editor,
//! post screen and sound picker that the measured detectors above do not already cover.
//! Each `key` is a row of the git-ignored research catalog
//! target/popup-catalog-45.7.3/CATALOG.md (APK strings `nxstring_default`, layouts, real
//! dumps). Strings are the 45.7.3 English table values, copied exactly.
//!
//! `mode` is the promotion switch: an entry acts only once a real 45.7.3/en dump proved the
//! whole popup. APK-only entries stay [`PopupMode::Shadow`]: recognised, logged and captured
//! for review, never pressed. Popups that do not cover the screen (the feed reward pendant,
//! the profile "expired Stories" banner) are deliberately absent: nothing needs dismissing.
use super::*;

const fn part(
    id: Option<&'static str>,
    class: Option<&'static str>,
    text: Option<&'static str>,
    desc: Option<&'static str>,
) -> PopupPart {
    PopupPart {
        id,
        class,
        text,
        desc,
    }
}

const fn id(id: &'static str) -> PopupPart {
    part(Some(id), None, None, None)
}

const fn id_text(id: &'static str, text: &'static str) -> PopupPart {
    part(Some(id), None, Some(text), None)
}

const fn text(text: &'static str) -> PopupPart {
    part(None, None, Some(text), None)
}

const fn button(text: &'static str) -> PopupPart {
    part(None, Some(BUTTON), Some(text), None)
}

/// Tux dialog frame: `visual_area` described "Dialog" (see the measured Facebook and Terms
/// dialogs above). Buttons in its `x4` bar are Buttons without an id.
const DIALOG: PopupPart = part(Some("visual_area"), None, None, Some("Dialog"));
/// Tux bottom-sheet frame. The sound picker uses the same `fl4`, so it is never evidence
/// on its own; every sheet entry adds its own anchors.
const SHEET: PopupPart = id("fl4");

pub(super) const CATALOG: &[CatalogPopup] = &[
    // Proven: crates/core/fixtures/tiktok-search/global45.7.3-location-dialog.fixture, a
    // real Global 45.7.3/en capture (nurture search). Cancel only closes the prompt; Open
    // settings leaves TikTok. nurture/search.rs answers it the same way while searching.
    CatalogPopup {
        key: "search_location_settings_dialog",
        mode: PopupMode::Act,
        class: PopupClass::Dismissable,
        container: part(Some("pwe"), None, None, None),
        anchors: &[
            id_text("ase", "See relevant content and places nearby"),
            id_text(
                "android:id/message",
                "Open your device settings and go to Locations > While Using the App. You can turn this off at any time.",
            ),
            part(Some("android:id/button1"), Some(BUTTON), Some("Open settings"), None),
        ],
        dismiss: Some(part(Some("android:id/button3"), Some(BUTTON), Some("Cancel"), None)),
        acted: "đóng hộp thoại xin vị trí của TikTok (bấm Cancel, không mở Cài đặt); chờ đọc lại màn hình",
        spent: "hộp thoại xin vị trí vẫn còn sau một lần bấm Cancel — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra hộp thoại xin vị trí — khi được bật sẽ bấm Cancel; lần này Riviu không bấm, cần người vận hành",
    },
    // Proven: crates/core/fixtures/tiktok-publish/musically-45.7.3-en/
    // real-20261009-precise-location-dialog.xml, a real Global 45.7.3/en capture over the
    // composer post screen (Riviu #31). Title zp_ and body zp9 sit in d93, the id-less
    // Buttons in the x4 bar. Not now keeps the approximate location; Allow is never pressed.
    // decline_precise_location in dialogs.rs owns the different OS-style "Don't allow" form.
    CatalogPopup {
        key: "precise_location_dialog",
        mode: PopupMode::Act,
        class: PopupClass::Dismissable,
        container: DIALOG,
        anchors: &[
            id("d93"),
            id("x4"),
            part(Some("zp_"), Some(TEXT_VIEW), Some("Turn on precise location"), None),
            part(
                Some("zp9"),
                Some(TEXT_VIEW),
                Some("For more relevant content and ads, and location-based features, allow TikTok to use your device's precise location. Otherwise, TikTok will continue to use your device's approximate location."),
                None,
            ),
            part(Some(""), Some(BUTTON), Some("Allow"), None),
        ],
        dismiss: Some(part(Some(""), Some(BUTTON), Some("Not now"), None)),
        acted: "đóng hộp thoại \"Turn on precise location\" của TikTok (bấm Not now, giữ vị trí gần đúng, không bấm Allow); chờ đọc lại màn hình",
        spent: "hộp thoại \"Turn on precise location\" vẫn còn sau một lần bấm Not now — không bấm lại, không bấm Allow; cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra hộp thoại \"Turn on precise location\" — khi được bật sẽ bấm Not now; lần này Riviu không bấm, cần người vận hành",
    },
    // Proven: a real Global 45.7.3/en capture over the For You feed (box 31-50 round 6,
    // ce071827715f321b05, 10/10/2026 15:50:57, artifacts/popups/20261010/
    // 155057.999-unknown-modal-ce071827715f321b05). Title xwe and body esv (in tx3) sit in
    // d93, the id-less Buttons in the x4 bar. APK: X.0Nct ("global_email_auth_dialog")
    // builds it; its button result (AwS122S1100000_9.invoke$17) maps Not now to the same
    // decline as an outside dismissal, while OK links the device's email addresses, so OK
    // is never pressed. The body is matched by id only: it carries a newline before
    // "Learn more", a link out of TikTok.
    CatalogPopup {
        key: "link_email_dialog",
        mode: PopupMode::Act,
        class: PopupClass::Dismissable,
        container: DIALOG,
        anchors: &[
            id("d93"),
            id("x4"),
            part(Some("xwe"), Some(TEXT_VIEW), Some("Link email"), None),
            part(Some("esv"), Some(TEXT_VIEW), None, None),
            part(Some(""), Some(BUTTON), Some("OK"), None),
        ],
        dismiss: Some(part(Some(""), Some(BUTTON), Some("Not now"), None)),
        acted: "đóng hộp thoại \"Link email\" của TikTok (bấm Not now, không liên kết email, không bấm OK); chờ đọc lại màn hình",
        spent: "hộp thoại \"Link email\" vẫn còn sau một lần bấm Not now — không bấm lại, không bấm OK; cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra hộp thoại \"Link email\" — khi được bật sẽ bấm Not now; lần này Riviu không bấm, cần người vận hành",
    },
    // Proven: target/sound-picker-probe-20261009/raw/nav_exit.xml (real 45.7.3/en). Kept in
    // shadow: the composer walk-back owns this menu and taps its Discard row itself, so the
    // shared step must not turn it into a block. Discard acts at once, with no confirmation.
    CatalogPopup {
        key: "editor_exit_menu",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: part(Some("tk8"), Some("android.widget.FrameLayout"), None, None),
        anchors: &[
            id("esz"),
            part(None, Some(TEXT_VIEW), Some("Discard"), None),
            part(None, Some(TEXT_VIEW), Some("Save draft"), None),
            part(None, Some(TEXT_VIEW), Some("Send to friends"), None),
        ],
        dismiss: None,
        acted: "",
        spent: "menu thoát trình chỉnh sửa đang mở (Discard / Save draft) — Riviu không chọn hộ; cần người vận hành",
        shadow: "[chạy bóng] nhận ra menu thoát trình chỉnh sửa (Discard / Save draft / Send to friends) — chỉ bước dọn nháp của Riviu hoặc người vận hành được chọn; bước chung không bấm gì, cần người vận hành nếu nó chặn bước này",
    },
    // APK only: rating prompt; Not now (0x7f1160ee) only logs rating_cancel. Rate opens Play.
    CatalogPopup {
        key: "rate_tiktok_dialog",
        mode: PopupMode::Shadow,
        class: PopupClass::Dismissable,
        container: DIALOG,
        anchors: &[text("Rate TikTok"), text("Give us a rating in Google Play")],
        dismiss: Some(button("Not now")),
        acted: "đóng hộp thoại mời chấm điểm TikTok (bấm Not now, không chấm điểm); chờ đọc lại màn hình",
        spent: "hộp thoại chấm điểm vẫn còn sau một lần bấm Not now — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra hộp thoại \"Rate TikTok\" (chỉ có trong APK 45.7.3, chưa có bản dump thật) — khi được bật sẽ bấm \"Not now\"; lần này Riviu không bấm, cần người vận hành",
    },
    // APK only: after login. Not now (0x7f1146b6) closes; Save login turns on one-click login.
    CatalogPopup {
        key: "save_login_info_dialog",
        mode: PopupMode::Shadow,
        class: PopupClass::Dismissable,
        container: DIALOG,
        anchors: &[text("Save login for next time?"), button("Save login")],
        dismiss: Some(button("Not now")),
        acted: "đóng hộp thoại lưu thông tin đăng nhập (bấm Not now, không lưu); chờ đọc lại màn hình",
        spent: "hộp thoại lưu đăng nhập vẫn còn sau một lần bấm Not now — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra hộp thoại \"Save login for next time?\" (chỉ có trong APK) — khi được bật sẽ bấm \"Not now\"; lần này Riviu không bấm, cần người vận hành",
    },
    // APK only (layout a4f in a sheet): cny is "Not now" here; the auto-pop variant reuses
    // cny as "Turn off", a privacy change, so the exact text is part of the match.
    CatalogPopup {
        key: "post_view_history_prompt",
        mode: PopupMode::Shadow,
        class: PopupClass::Dismissable,
        container: SHEET,
        anchors: &[id_text("tv_title", "See who viewed your posts"), id_text("crx", "Turn on")],
        dismiss: Some(id_text("cny", "Not now")),
        acted: "đóng bảng \"See who viewed your posts\" (bấm Not now, không bật lịch sử xem); chờ đọc lại màn hình",
        spent: "bảng lịch sử xem bài vẫn còn sau một lần bấm Not now — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra bảng \"See who viewed your posts\" (chỉ có trong APK) — khi được bật sẽ bấm \"Not now\"; lần này Riviu không bấm, cần người vận hành",
    },
    // APK only: email opt-in. No thanks (0x7f110400) declines; the other button subscribes.
    CatalogPopup {
        key: "email_updates_dialog",
        mode: PopupMode::Shadow,
        class: PopupClass::Dismissable,
        container: DIALOG,
        anchors: &[text("Get updates sent to your email?")],
        dismiss: Some(button("No thanks")),
        acted: "đóng hộp thoại nhận email cập nhật (bấm No thanks); chờ đọc lại màn hình",
        spent: "hộp thoại nhận email vẫn còn sau một lần bấm No thanks — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra hộp thoại \"Get updates sent to your email?\" (chỉ có trong APK) — khi được bật sẽ bấm \"No thanks\"; lần này Riviu không bấm, cần người vận hành",
    },
    // APK only: in-app location ask before the OS prompt. Continue requests the permission.
    CatalogPopup {
        key: "location_access_prompt",
        mode: PopupMode::Shadow,
        class: PopupClass::Dismissable,
        container: DIALOG,
        anchors: &[text("Allow location access to discover more nearby")],
        dismiss: Some(button("Not now")),
        acted: "đóng lời mời bật vị trí (bấm Not now); chờ đọc lại màn hình",
        spent: "lời mời bật vị trí vẫn còn sau một lần bấm Not now — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra lời mời \"Allow location access to discover more nearby\" (chỉ có trong APK) — khi được bật sẽ bấm \"Not now\"; lần này Riviu không bấm, cần người vận hành",
    },
    // APK only (layout bmh): reached only after choosing "Not interested", which Riviu never
    // does. l86 closes without Submit (cpf).
    CatalogPopup {
        key: "not_interested_reason_sheet",
        mode: PopupMode::Shadow,
        class: PopupClass::Dismissable,
        container: id("n8o"),
        anchors: &[id_text("tv_title", "Not interested"), id_text("tv_select_hint", "Select a reason")],
        dismiss: Some(part(Some("l86"), None, None, Some("Close dislike reason dialog"))),
        acted: "đóng bảng chọn lý do \"Not interested\" (bấm nút đóng, không gửi lý do); chờ đọc lại màn hình",
        spent: "bảng lý do \"Not interested\" vẫn còn sau một lần đóng — không bấm lại, cần người vận hành kiểm tra",
        shadow: "[chạy bóng] nhận ra bảng lý do \"Not interested\" (chỉ có trong APK) — khi được bật sẽ bấm nút đóng; lần này Riviu không bấm, cần người vận hành",
    },
    // APK only (layout a15): shown after Send. Every button decides whether the comment is
    // published, so the Send outcome stays uncertain until observed; never retried.
    CatalogPopup {
        key: "comment_reconsider_prompt",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: id("kj5"),
        anchors: &[id_text("z0l", "Would you like to reconsider posting this?")],
        dismiss: None,
        acted: "",
        spent: "TikTok hỏi có muốn sửa bình luận trước khi đăng — Riviu không chọn hộ và không gửi lại; cần người vận hành",
        shadow: "[chạy bóng] nhận ra hỏi \"Would you like to reconsider posting this?\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    // APK only: expired login; OK leads to the login chooser.
    CatalogPopup {
        key: "account_logged_out_dialog",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: DIALOG,
        anchors: &[text("Your account was logged out. Try logging in again.")],
        dismiss: None,
        acted: "",
        spent: "TikTok báo tài khoản đã bị đăng xuất — Riviu không tự đăng nhập; cần người vận hành",
        shadow: "[chạy bóng] nhận ra thông báo \"Your account was logged out\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    // APK only: editor sub-tool after Back. Keep has no listener, but deciding about the
    // operator's edits stays with the operator.
    CatalogPopup {
        key: "editor_discard_edits_dialog",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: DIALOG,
        anchors: &[text("Discard your edits?")],
        dismiss: None,
        acted: "",
        spent: "TikTok hỏi có bỏ các chỉnh sửa video không — Riviu không chọn hộ; cần người vận hành",
        shadow: "[chạy bóng] nhận ra hỏi \"Discard your edits?\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    CatalogPopup {
        key: "editor_discard_edits_dialog",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: DIALOG,
        anchors: &[text("Discard edits?")],
        dismiss: None,
        acted: "",
        spent: "TikTok hỏi có bỏ các chỉnh sửa video không — Riviu không chọn hộ; cần người vận hành",
        shadow: "[chạy bóng] nhận ra hỏi \"Discard edits?\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    // APK only (layout c_u): leads to a contacts upload.
    CatalogPopup {
        key: "find_contacts_prompt",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: id("rbr"),
        anchors: &[id_text("nwt", "Find contacts")],
        dismiss: None,
        acted: "",
        spent: "TikTok mời đồng bộ danh bạ — Riviu không đồng bộ; cần người vận hành",
        shadow: "[chạy bóng] nhận ra lời mời \"Find contacts\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    // APK only (layout czd): no decline control in the layout; fail closed.
    CatalogPopup {
        key: "notification_inapp_prompt",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: id("tk8"),
        anchors: &[
            id_text("sd_", "Allow permissions"),
            id_text(
                "sd9",
                "To stay on top of likes, comments, the latest videos, and more, allow TikTok to send you notifications.",
            ),
        ],
        dismiss: None,
        acted: "",
        spent: "TikTok xin quyền thông báo trong ứng dụng — Riviu không cấp quyền; cần người vận hành",
        shadow: "[chạy bóng] nhận ra lời xin \"Allow permissions\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    // APK only (layout i4r): first Repost; its OK may complete a public repost.
    CatalogPopup {
        key: "repost_intro_sheet",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: SHEET,
        anchors: &[id_text("tv_title", "Repost this video")],
        dismiss: None,
        acted: "",
        spent: "TikTok giới thiệu Repost — có thể đăng lại công khai; Riviu không bấm, cần người vận hành",
        shadow: "[chạy bóng] nhận ra bảng \"Repost this video\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
    // APK only: cancelling an upload; the Post outcome is uncertain while it is up.
    CatalogPopup {
        key: "cancel_upload_confirm",
        mode: PopupMode::Shadow,
        class: PopupClass::Operator,
        container: DIALOG,
        anchors: &[text("Cancel upload")],
        dismiss: None,
        acted: "",
        spent: "TikTok hỏi có hủy bài đang tải lên không — Riviu không hủy và không đăng lại; cần người vận hành",
        shadow: "[chạy bóng] nhận ra hỏi \"Cancel upload\" (chỉ có trong APK) — loại chỉ người vận hành xử lý; Riviu không bấm gì, cần người vận hành",
    },
];
