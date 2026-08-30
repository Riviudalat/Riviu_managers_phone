//! Driving TikTok's image picker and composer through the accessibility hierarchy.
//!
//! The sibling of [`crate::tiktok_drawer`], and deliberately shaped like it — but it
//! guards something the drawer does not. A comment that goes out wrong can be deleted;
//! this project has a measured path for that. **A carousel that goes out wrong cannot**:
//! `PostDeleteMenu` was looked for on a real own-post page and carries no `content-desc`
//! and no `text` at all, so there is no delete on Android for what this module publishes.
//!
//! That single fact decides the whole design.
//!
//! # Refusing is a type, not a check
//!
//! Everything the path needs is resolved **once, up front**, by [`ComposerPlan::resolve`],
//! and [`publish_carousel`] refuses before its first tap unless the plan can reach a post.
//! A [`Composer`] cannot be built any other way, so "refuse before opening anything" is not
//! a rule a later edit can forget — there is no code path that opens the composer without a
//! complete plan in hand.
//!
//! The one way to touch the phone with an incomplete plan is [`drive_to_edit_step`], which
//! is named for exactly what it is: the instrument a measuring session uses to reach the
//! screens whose labels are still missing. Everything it can reach is reversible.
//!
//! # What is measured, and what this therefore still refuses
//!
//! Measured 29/08/2026 on SM-G950F `98895a3355424e484f`, `com.ss.android.ugc.trill`
//! 38.3.2, `en-US`, 1080x2220 — the build sixteen of the twenty phones run:
//!
//! | | |
//! |---|---|
//! | [`TikTokControl::ComposerOpen`] | `Create`, unique in all 184 elements |
//! | [`TikTokControl::ComposerShutter`] | `Record video` — the geometry anchor |
//! | [`TikTokControl::PickerTabPhotos`] | `Photos` — the grid's anchor |
//! | [`TikTokControl::PickerMultiSelect`] | `Select multiple` |
//! | [`TikTokControl::PickerNext`] | `Next`, armed via `clickable` — see below |
//! | [`TikTokControl::PickerAlbumMenu`] | **on screen, not locatable** — see below |
//! | [`TikTokControl::ComposerNext`] | never measured, on any build |
//! | [`TikTokControl::PostButton`] | never measured, on any build |
//!
//! So on today's fleet [`ComposerPlan::resolve`] refuses, and names what is missing.
//!
//! # The album menu, and why "just use All" is not the shortcut it looks like
//!
//! The album pill reads the album *currently showing* — `All` now, and the campaign's own
//! `importId` once chosen — so a text locator names a value that changes the moment it is
//! used. Worse, `All` **also** belongs to the media-type tab one row below it.
//!
//! The tempting shortcut is to skip the album entirely: the campaign's images were imported
//! seconds ago, so they are the newest things in `All` and sit at the head of the grid. That
//! is true right up until the phone acquires one other image — a screenshot, a chat photo,
//! anything a background app saves — at which point the grid shifts by one and the carousel
//! goes out with a stranger's picture in it, published, with no delete. The album is what
//! makes the selection *addressed* rather than *guessed*.
//!
//! # The armed flag is `clickable`, not `enabled`
//!
//! Measured both ways: `Next` reads `clickable=false enabled=true` with nothing selected and
//! `clickable=true enabled=true` with one image selected. `enabled` is constant across the
//! transition and proves nothing here, while the comment drawer's Send button on another
//! build moves `enabled` and not this.
//!
//! # What the hierarchy cannot tell us, stated plainly
//!
//! Selecting an image renders **no per-cell numeral** on this build. So:
//!
//! * "**enough** images are selected" is **not checkable**. `Next` arming proves that *at
//!   least one* cell took, and nothing more. [`Selection::Armed`] says exactly that and no
//!   caller should read more into it.
//! * slide *order* cannot be read back, and a scrolled grid cannot be re-identified — which
//!   is why [`PhotoGrid`] refuses past the rows that are on screen instead of flicking.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// `tokio`'s clock for the same reason `tiktok_drawer` uses it: a
// `#[tokio::test(start_paused = true)]` moves these deadlines along with the sleeps.
use tokio::time::Instant;

use crate::driver::{ElementBox, ElementQuery, UiSession};
use crate::tiktok_drawer::TapPlanner;
use crate::tiktok_labels::{TikTokControl, TikTokControls};

/// How long the composer may take to appear after its tab is tapped.
pub const COMPOSER_WINDOW: Duration = Duration::from_millis(8_000);
/// How long the picker may take after the gallery entry is tapped.
///
/// Longer than the composer's own window on purpose: the picker enumerates the device's
/// media store, and a phone this project has just pushed a carousel onto has a store that
/// was written to seconds ago.
pub const PICKER_WINDOW: Duration = Duration::from_millis(10_000);
/// How long `Next` may take to arm after the last cell is tapped.
pub const ARM_WINDOW: Duration = Duration::from_millis(4_000);
/// How long to wait for the feed to come back after Post is tapped.
///
/// The upload runs after the screen returns, so this waits for the *navigation*, not the
/// upload — but TikTok does its own checks first, so it is not instant either.
pub const POST_CONFIRM_WINDOW: Duration = Duration::from_millis(20_000);
pub const POLL: Duration = Duration::from_millis(350);

/// The device screen, validated once so the geometry below cannot be built on nonsense.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Screen {
    width: f64,
    height: f64,
}

impl Screen {
    /// `None` for anything that is not a real screen.
    ///
    /// The check exists because every rectangle in this module is arithmetic on these two
    /// numbers, and arithmetic on a `NaN` or a zero produces a tap point that looks like a
    /// number and lands wherever the OS rounds it to. A refusal here happens before the
    /// composer opens; a bad number is a tap on a live screen.
    pub fn new(width: f64, height: f64) -> Option<Self> {
        (width.is_finite() && height.is_finite() && width >= 320.0 && height >= 480.0)
            .then_some(Self { width, height })
    }

    pub fn width(&self) -> f64 {
        self.width
    }

    pub fn height(&self) -> f64 {
        self.height
    }
}

/// What a publish attempt actually achieved, named for the step that failed.
///
/// Every variant except [`Self::Posted`] and [`Self::PostNotConfirmed`] means **nothing was
/// published**. Those two mean the carousel is — or may be — on a real account, and
/// [`Self::may_retry`] is the single question a caller must ask before dispatching again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerVerdict {
    /// The Post button was tapped and the **bottom tab bar came back**.
    ///
    /// Read that as what it is. It proves the composer let go of the screen, which is the
    /// strongest signal this path has and is *not* the same as "the carousel is on the
    /// account": TikTok returns to the feed and uploads in the background, so a post can still
    /// fail on the network or be rejected after this. What it does rule out is the composer
    /// still sitting there with the images in it.
    ///
    /// Closing the remaining gap means reading the post back from the account, which needs the
    /// route to our own post page — unmeasured on every build, and the same thing blocking the
    /// link capture.
    Posted,
    /// Post was tapped, or may have been, and the result could not be read. **Never
    /// retried.**
    ///
    /// Covers the transport failures too: once the tap has been handed to the agent, an
    /// error is an *unknown outcome*, not a failure. Returning `Err` there would let a
    /// caller that retries transport errors publish a duplicate.
    PostNotConfirmed,
    /// The composer tab was tapped and the composer never came up.
    ComposerDidNotOpen,
    /// The composer opened and the shutter — the gallery entry's anchor — was not on it.
    ///
    /// Refuses rather than falling back to remembered coordinates: the two controls beside
    /// the entry open the effects panel.
    NoShutterToAnchorTo,
    /// The gallery entry was tapped and the picker never came up.
    ///
    /// Also what a *mis-tap* looks like, which is why it is checked rather than assumed.
    PickerDidNotOpen,
    /// The album menu opened and the campaign's own album was not in it, or was in it twice.
    AlbumNotFound,
    /// The album row was tapped and the picker did not switch to that album.
    ///
    /// Distinct from [`Self::AlbumNotFound`] on purpose: the row existed, so the import
    /// landed — the tap is what did not take, most likely because the list reflowed while
    /// thumbnails loaded. Publishing anyway would publish **another album's images**.
    AlbumNotConfirmed,
    /// The picker's tab row — the grid's anchor — was not on screen.
    NoTabsToAnchorTo,
    /// More images were asked for than the grid shows without scrolling, or none.
    MoreCellsThanTheGridShows,
    /// The cells were tapped and `Next` never armed, so **no** cell took.
    NeverArmed,
    /// The first cell was tapped and `Next` did not arm, so `Select multiple` never engaged.
    ///
    /// Measured 30/08/2026 (§9.132), two of four walks: the toggle tap does not take, and the
    /// first cell tap then opens the **single-photo editor** — a screen that also renders a
    /// `Next` whose text node is not clickable, so the armed check reads it honestly as
    /// unarmed. Distinct from [`Self::NeverArmed`] because the remaining cells were **not**
    /// tapped: before this verdict existed they landed blind on the editor's own controls,
    /// which is a real tap on an unknown screen for every image past the first.
    MultiSelectDidNotEngage,
    /// The picker states how many images are selected, and it is not how many were asked for.
    ///
    /// Only reachable on a build whose `Next` renders a count. Where none is rendered the
    /// hierarchy cannot answer at all, and the run proceeds on the taps it sent — see
    /// [`Selection::Armed`].
    NotEnoughSelected,
    /// `Next` was tapped and the edit step never appeared.
    EditStepDidNotOpen,
    /// The edit step's Next was tapped and the post screen never appeared.
    PostScreenDidNotOpen,
    /// This build's Post button has never been measured, so nothing was tapped.
    PostUnmeasured,
    /// The post screen opened and the measured Post button was not on it.
    NoPostButton,
    /// The post screen opened and the caption field was not on it.
    NoCaptionField,
    /// The caption was typed and could not be read back out of the field.
    ///
    /// Refuses rather than posting, and the asymmetry with the rest of the module is
    /// deliberate: **an unverified caption is worse than an unverified tap**, because the tap
    /// either worked or is caught one step later, while a caption that silently did not take
    /// publishes a carousel with somebody else's words — or none.
    ///
    /// The most likely cause is a catalogue entry that names the field's *placeholder*: it
    /// stops matching the moment a character is typed, so the readback finds nothing. See
    /// [`TikTokControl::ComposerCaption`].
    CaptionNotConfirmed,
    /// The caller asked to stop, and it was still safe to obey.
    ///
    /// Only ever returned from before the Post tap. Once that tap is dispatched, stopping is
    /// no longer a thing that can be done — the outcome is [`Self::PostNotConfirmed`].
    ///
    /// Also what [`drive_to_edit_step`] returns when it finishes, because finishing *is*
    /// stopping short: it reached the screen it came to read and published nothing.
    Stopped,
}

impl ComposerVerdict {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Posted => "đã đăng",
            Self::PostNotConfirmed => {
                "đã bấm Đăng (hoặc có thể đã bấm) nhưng không xác nhận được; KHÔNG đăng lại — \
                 bài có thể đã lên và Android không có đường xoá"
            }
            Self::ComposerDidNotOpen => "bấm nút Tạo mà composer không mở",
            Self::NoShutterToAnchorTo => "không thấy nút chụp — không có mốc để tìm ô thư viện",
            Self::PickerDidNotOpen => "bấm ô thư viện mà picker không mở (có thể đã bấm nhầm)",
            Self::AlbumNotFound => "không thấy album của chiến dịch, hoặc thấy nhiều hơn một",
            Self::AlbumNotConfirmed => "bấm album mà picker không chuyển sang đúng album đó",
            Self::NoTabsToAnchorTo => "không thấy hàng tab của picker — không có mốc cho lưới ảnh",
            Self::MoreCellsThanTheGridShows => "số ảnh cần nhiều hơn số ô lưới hiện ra",
            Self::NeverArmed => "đã bấm các ô ảnh nhưng nút Tiếp không sáng — không ô nào ăn",
            Self::MultiSelectDidNotEngage => {
                "bấm 'Chọn nhiều' không ăn — ô đầu tiên mở thẳng trình sửa đơn, nên không bấm \
                 thêm ô nào vào màn hình lạ. Nguyên nhân đã đo: nút đó là công tắc HAI CHIỀU và \
                 TikTok nhớ trạng thái giữa các lượt, nên một cú bấm mù sẽ TẮT chế độ nhiều ảnh \
                 khi lượt trước đã bật nó — chạy lại lượt nữa là vào đúng"
            }
            Self::NotEnoughSelected => "picker báo số ảnh đã chọn khác số ảnh bài này cần",
            Self::EditStepDidNotOpen => "bấm Tiếp mà bước chỉnh sửa không mở",
            Self::PostScreenDidNotOpen => "bấm Tiếp ở bước chỉnh sửa mà màn đăng không mở",
            Self::PostUnmeasured => "chưa đo nút Đăng trên bản build này — không bấm gì cả",
            Self::NoPostButton => "không thấy nút Đăng ở màn cuối",
            Self::NoCaptionField => "không thấy ô caption ở màn cuối",
            Self::CaptionNotConfirmed => {
                "gõ caption xong mà đọc lại không thấy — KHÔNG đăng, vì bài sẽ lên với caption \
                 rỗng hoặc caption của bài khác"
            }
            Self::Stopped => "dừng lại trước khi bấm Đăng",
        }
    }

    /// Whether the composer completed its side of the post — see [`Self::Posted`] for the
    /// gap between that and the carousel being live on the account.
    pub fn is_posted(self) -> bool {
        self == Self::Posted
    }

    /// Whether the caller may dispatch this assignment again.
    ///
    /// **The one question this enum exists to answer.** `false` for
    /// [`Self::PostNotConfirmed`] as well as for [`Self::Posted`], because an unconfirmed
    /// post may be live and a second attempt would publish a duplicate that nothing here can
    /// take down.
    pub fn may_retry(self) -> bool {
        !matches!(self, Self::Posted | Self::PostNotConfirmed)
    }
}

/// The controls needed to **reach the edit step**, navigation and geometry anchors alike.
///
/// The two anchors are here for a reason that is easy to miss: this module reaches two
/// controls by arithmetic rather than by label, and arithmetic is only trustworthy when it
/// starts from an element located on the screen in front of us. So
/// [`TikTokControl::ComposerShutter`] and [`TikTokControl::PickerTabPhotos`] are required
/// even though neither is ever tapped. Without them the gallery entry and the image grid
/// would be reached from numbers copied off another phone — and next to the gallery entry is
/// the effects panel.
///
/// [`TikTokControl::PickerTabAll`] is **not** here: the album addresses the campaign's own
/// directory, which holds nothing but its images, so filtering by media type inside it
/// changes nothing.
///
/// The publish tail lives in [`REQUIRED_TO_PUBLISH`] instead — see there for why.
pub const REQUIRED: [TikTokControl; 6] = [
    TikTokControl::ComposerOpen,
    TikTokControl::ComposerShutter,
    TikTokControl::PickerAlbumMenu,
    TikTokControl::PickerTabPhotos,
    TikTokControl::PickerMultiSelect,
    TikTokControl::PickerNext,
];

/// The controls that turn a reachable edit step into a published post.
///
/// **Kept out of [`REQUIRED`], and the split is not a convenience.** The first version put
/// `ComposerNext` in the required set, which made [`drive_to_edit_step`] — the instrument
/// written to *measure* `ComposerNext` — refuse to run without it. A tool that demands the
/// reading it exists to take is no tool at all, and the plan for this work had named that
/// exact trap in `probe`'s two composer commands before reproducing it here.
///
/// Everything [`REQUIRED`] unlocks is reversible: Back walks out of it and nothing has left
/// the phone. Everything this list unlocks is not.
pub const REQUIRED_TO_PUBLISH: [TikTokControl; 3] = [
    TikTokControl::ComposerNext,
    TikTokControl::ComposerCaption,
    TikTokControl::PostButton,
];

/// Why this build cannot be driven, named control by control.
///
/// Carries the list rather than a bare `false` because the list is *actionable*: every entry
/// is one dump on one phone away from being closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerUnready {
    pub missing: Vec<TikTokControl>,
}

impl std::fmt::Display for ComposerUnready {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "bản build này chưa đo {} nhãn cần cho việc đăng: {}",
            self.missing.len(),
            self.missing
                .iter()
                .map(|control| format!("{control:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for ComposerUnready {}

/// Every locator the publish path needs, resolved before anything is opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposerPlan {
    open: ElementQuery<'static>,
    shutter: ElementQuery<'static>,
    album_menu: ElementQuery<'static>,
    tabs: ElementQuery<'static>,
    multi_select: ElementQuery<'static>,
    picker_next: ElementQuery<'static>,
    /// The edit step's own control, when it is measured, used **only to prove arrival**.
    ///
    /// Separate from the publishing tail below, and a review is why: keying the arrival proof
    /// on `publish` meant a build with a measured `ComposerNext` but no measured Post button
    /// fell back to the weak proof — and that build is exactly the one a measuring run is on.
    /// The strong evidence was available and discarded, so the tool could dump an error screen
    /// and call it the edit step.
    ///
    /// Navigation and publication are different questions; this answers the first.
    edit_step_marker: Option<ElementQuery<'static>>,
    /// The publishing tail, resolved **all or nothing**.
    ///
    /// One `Option` around three locators rather than three `Option`s, and the difference is
    /// the point: with three, "two of the three are measured" is a state the code can be in,
    /// and every place that asks "can this build publish?" has to remember to check all three.
    /// A reversal proved that was not hypothetical — dropping one of the three conjuncts from
    /// `can_publish` left every test green, and the build it then claimed could publish would
    /// have reached the post screen and typed nothing into a caption field it could not find.
    ///
    /// With one `Option` the partial state does not exist.
    publish: Option<PublishTail>,
}

/// The three locators that turn a reachable edit step into a published post.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PublishTail {
    edit_next: ElementQuery<'static>,
    caption: ElementQuery<'static>,
    post_button: ElementQuery<'static>,
}

impl ComposerPlan {
    /// Resolve every required control, or refuse and say which ones are missing.
    pub fn resolve(labels: &TikTokControls) -> Result<Self, ComposerUnready> {
        let missing: Vec<TikTokControl> = REQUIRED
            .into_iter()
            .filter(|control| labels.label(*control).is_none())
            .collect();
        if !missing.is_empty() {
            return Err(ComposerUnready { missing });
        }
        let query =
            |control: TikTokControl| labels.label(control).expect("checked above").to_query();
        let optional = |control: TikTokControl| labels.label(control).map(|label| label.to_query());
        Ok(Self {
            open: query(TikTokControl::ComposerOpen),
            shutter: query(TikTokControl::ComposerShutter),
            album_menu: query(TikTokControl::PickerAlbumMenu),
            tabs: query(TikTokControl::PickerTabPhotos),
            multi_select: query(TikTokControl::PickerMultiSelect),
            picker_next: query(TikTokControl::PickerNext),
            edit_step_marker: optional(TikTokControl::ComposerNext),
            publish: match (
                optional(TikTokControl::ComposerNext),
                optional(TikTokControl::ComposerCaption),
                optional(TikTokControl::PostButton),
            ) {
                (Some(edit_next), Some(caption), Some(post_button)) => Some(PublishTail {
                    edit_next,
                    caption,
                    post_button,
                }),
                _ => None,
            },
        })
    }

    /// The Post control, or `None` on a build that cannot publish.
    pub fn post_button(&self) -> Option<ElementQuery<'static>> {
        self.publish.map(|tail| tail.post_button)
    }

    /// Whether this build can be driven all the way to a published post.
    ///
    /// [`publish_carousel`] refuses **before its first tap** when this is false. These are not
    /// in [`REQUIRED`] because a plan without them is still useful — see
    /// [`drive_to_edit_step`], which is how the missing measurements get taken — but that use
    /// has its own entry point and its own name, rather than being what happens by default.
    pub fn can_publish(&self) -> bool {
        self.publish.is_some()
    }

    /// Which of [`REQUIRED_TO_PUBLISH`] this build is still missing.
    ///
    /// For the operator, and for the measuring session: every entry is one dump away.
    pub fn missing_to_publish(labels: &TikTokControls) -> Vec<TikTokControl> {
        REQUIRED_TO_PUBLISH
            .into_iter()
            .filter(|control| labels.label(*control).is_none())
            .collect()
    }
}

/// The unlabelled control that opens the gallery from inside the composer.
///
/// # The most expensive measurement in this module
///
/// It carries **no `content-desc`, no `text`**, so like [`PhotoGrid`] it is geometry rather
/// than a label. What makes it worse than the grid is that the wrong guess is not a miss but
/// a *different feature*: measured 29/08/2026 on the fleet's build, the two circles left of
/// the shutter are `resource-id=…:id/egr` and open the **effects panel**. An earlier note put
/// the gallery entry at the bottom-left, which is exactly where those sit. On this build it
/// is on the **right**:
///
/// ```text
///   shutter        Record video   375,1545  330x330   centre 540,1710
///   gallery entry  (unlabelled)   765,1590  240x240   centre 885,1710   id/bos
/// ```
///
/// Found by looking at a screenshot — the entry renders a 2x2 montage of real photos, which
/// no hierarchy dump says.
///
/// # Anchored to a located shutter, and refused when it lands off screen
///
/// The two share a **vertical centre exactly**, measured, so the shutter fixes `y` outright;
/// horizontally the entry sits one measured margin in from the right edge. The shutter is a
/// real located element ([`TikTokControl::ComposerShutter`], required), so this is arithmetic
/// on the screen in front of us rather than on remembered numbers.
///
/// A tap here **must** still be verified afterwards — see [`Composer::await_picker`]. An
/// overlay from another app can sit inside an unlabelled rectangle: a Messenger bubble once
/// landed *within* a gallery entry, and tapping its centre tapped the bubble.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GalleryEntry {
    x: f64,
    y: f64,
    size: f64,
}

impl GalleryEntry {
    /// Place the entry from the screen and the located shutter.
    ///
    /// `None` when the result would not be fully on screen. That is not defensive tidying: a
    /// tap at a computed off-screen point is not a no-op, it lands on whatever the OS clamps
    /// it to — which along the bottom of this screen is a row of controls.
    pub fn beside_shutter(screen: Screen, shutter: &ElementBox) -> Option<Self> {
        // 75/1080 and 240/1080 at the measured resolution.
        let margin = screen.width() * (75.0 / 1080.0);
        let size = screen.width() * (240.0 / 1080.0);
        // The anchor itself has to be a real rectangle on this screen. Validating only the
        // *derived* entry let a shutter reported at `height = -100` produce a plausible,
        // fully on-screen tap point — arithmetic from nonsense, which is the thing anchoring
        // was supposed to replace.
        if !on_screen(shutter, screen) {
            return None;
        }
        let entry = Self {
            x: screen.width() - margin - size,
            y: shutter.y + shutter.height / 2.0 - size / 2.0,
            size,
        };
        let on_screen = entry.x >= 0.0
            && entry.y >= 0.0
            && entry.x + entry.size <= screen.width()
            && entry.y + entry.size <= screen.height();
        on_screen.then_some(entry)
    }

    pub fn rect(&self) -> ElementBox {
        ElementBox {
            x: self.x,
            y: self.y,
            width: self.size,
            height: self.size,
            description: None,
            enabled: true,
            // Arithmetic, not a query: nothing read an attribute off this.
            clickable: false,
        }
    }
}

/// Whether a located element is a real rectangle on this screen.
///
/// Both geometric anchors go through this. The point of anchoring is that the numbers come
/// from the screen in front of us rather than from another phone — and an anchor with a
/// negative height or one lying off screen is not that, however finite its arithmetic is.
fn on_screen(element: &ElementBox, screen: Screen) -> bool {
    element.x.is_finite()
        && element.y.is_finite()
        && element.width.is_finite()
        && element.height.is_finite()
        && element.width > 0.0
        && element.height > 0.0
        && element.x >= 0.0
        && element.y >= 0.0
        && element.x + element.width <= screen.width()
        && element.y + element.height <= screen.height()
}

/// The picker's unlabelled image grid, in device pixels.
///
/// The cells carry **no `content-desc`, no `text` and no `resource-id`** — they are bare
/// `FrameLayout`s — so they cannot be addressed by the label catalogue at all.
///
/// # Anchored, not hard-coded
///
/// Measured on 1080x2220. Writing those numbers down as constants would break on the first
/// phone with a different status bar, so the vertical origin is taken from the **media-type
/// tab row**, a real located element, and the horizontal layout is derived from the screen
/// width. What is fixed is the *pattern* — three columns, one gap-width of margin each side:
///
/// ```text
///   6 │ 352 │ 6 │ 352 │ 6 │ 352 │ 6   = 1080   columns at x = 6, 364, 722
///   rows at y = 357, 719, 1081, 1443             pitch 362, height 356
/// ```
///
/// # There is no numeral to check the result against
///
/// Selecting an image renders **no per-cell numeral** anywhere on screen. So a tap's effect
/// cannot be read back cell by cell, and the only evidence any selection took is `Next`
/// arming — one signal for the whole set, which is why [`Composer::select`] taps them all and
/// checks once. It is also why scrolling is not supported: after a flick nothing on screen
/// identifies which row is which.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoGrid {
    origin_x: f64,
    origin_y: f64,
    cell_width: f64,
    cell_height: f64,
    gap: f64,
    screen: Screen,
}

/// Columns in the picker grid. Measured, and the same on both builds looked at.
pub const GRID_COLUMNS: usize = 3;
/// Rows measured as visible without scrolling on 1080x2220.
///
/// A ceiling, not a promise: [`PhotoGrid::capacity`] also drops rows that do not fit the
/// actual screen, so a shorter phone offers fewer.
pub const GRID_MEASURED_ROWS: usize = 4;

impl PhotoGrid {
    /// Build the grid from the screen and the located media-type tab row.
    ///
    /// `tabs_bottom` is the bottom edge of the tab row (`Photos` and its neighbours), which
    /// is a control the catalogue can find and [`REQUIRED`] insists on. The 45px between it
    /// and the first row of cells is measured; expressing it as a fraction of screen height
    /// would be worse, not better — it is a fixed layout margin, not a proportion.
    ///
    /// `None` when the anchor is not a sane place for a tab row to be, which is what a stale
    /// or invented `tabs_bottom` looks like.
    pub fn below_tabs(screen: Screen, tabs_bottom: f64) -> Option<Self> {
        if !tabs_bottom.is_finite() || tabs_bottom < 0.0 || tabs_bottom >= screen.height() {
            return None;
        }
        let gap = screen.width() / 180.0;
        let cell_width = (screen.width() - 4.0 * gap) / GRID_COLUMNS as f64;
        Some(Self {
            origin_x: gap,
            origin_y: tabs_bottom + 45.0,
            cell_width,
            // Cells are very slightly taller than wide — 356 against 352 at 1080 — which is
            // measured rather than assumed square. It only shifts the tap point by two
            // pixels, but writing `cell_width` here would record a number nobody read.
            cell_height: cell_width * (356.0 / 352.0),
            gap,
            screen,
        })
    }

    /// The rectangle of the cell at `index`, counting left to right then down.
    ///
    /// `None` once the cell would not be **fully on screen**, which is the bound the doc
    /// always claimed and the index alone never enforced: four rows is what a 2220-pixel
    /// screen shows, and a shorter one shows three.
    pub fn cell(&self, index: usize) -> Option<ElementBox> {
        if index >= GRID_COLUMNS * GRID_MEASURED_ROWS {
            return None;
        }
        let column = (index % GRID_COLUMNS) as f64;
        let row = (index / GRID_COLUMNS) as f64;
        let rect = ElementBox {
            x: self.origin_x + column * (self.cell_width + self.gap),
            y: self.origin_y + row * (self.cell_height + self.gap),
            width: self.cell_width,
            height: self.cell_height,
            description: None,
            // Unknowable and unused: these are `FrameLayout`s located by arithmetic, not by a
            // query, so nothing read an attribute off them. `false` is the refusing
            // direction, consistent with `ElementBox::clickable`'s default.
            enabled: true,
            clickable: false,
        };
        (rect.x >= 0.0
            && rect.y >= 0.0
            && rect.x + rect.width <= self.screen.width()
            && rect.y + rect.height <= self.screen.height())
        .then_some(rect)
    }

    /// How many images this grid can select without scrolling, on **this** screen.
    pub fn capacity(&self) -> usize {
        (0..GRID_COLUMNS * GRID_MEASURED_ROWS)
            .take_while(|index| self.cell(*index).is_some())
            .count()
    }
}

/// What tapping the grid achieved.
///
/// Separate from [`ComposerVerdict`] because selecting images is not an outcome of the
/// publish attempt — nothing has left the phone yet — and a function that could hand back
/// `Posted` from the middle of the picker is one refactor away from a caller believing it.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    /// `Select multiple` never engaged: the very first cell tap failed to arm `Next`.
    ///
    /// Checked after cell one and before cells two onward, because the measured failure
    /// (§9.132) is the first tap leaving the picker for the single-photo editor — every
    /// further "cell" tap would land on that editor's own controls.
    MultiSelectDidNotEngage,
    /// `Next` armed, which proves **at least one** cell took.
    ///
    /// Read that literally. There is no per-cell numeral on the measured build, so the
    /// hierarchy cannot say *which* cells are selected; a run that asked for five and landed
    /// four arms exactly the same way as one that landed five.
    ///
    /// `counted` is the one thing that can sometimes be recovered: some builds render the
    /// number of selected images in the `Next` control's own text. When it is `Some`, it is
    /// authoritative and [`ComposerVerdict::NotEnoughSelected`] is what a mismatch becomes.
    /// When it is `None` the build states no count, and the run proceeds on the count of taps
    /// it *sent* — which is weaker, and is why this variant names the difference instead of
    /// hiding it.
    Armed {
        next: ElementBox,
        counted: Option<usize>,
    },
    /// Asked for more cells than this screen's grid shows, or for none.
    MoreCellsThanTheGridShows,
    /// The cells were tapped and `Next` stayed unarmed — so **no** tap landed.
    NeverArmed,
    /// The caller asked to stop partway through.
    Stopped,
}

/// What typing the caption achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptionOutcome {
    /// The caption field holds **exactly** the caption, and only one node matched.
    Typed,
    /// The bundle's caption file was empty, so nothing was typed.
    NothingToSay,
    /// This build's caption field was never measured.
    Unmeasured,
    /// The field was not on the post screen.
    NoField,
    /// The text was set and could not be read back.
    NotConfirmed,
}

/// What choosing the campaign's album achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumChoice {
    /// The pill now names the requested album.
    Confirmed,
    /// No row matched the name, or more than one did.
    NotFound,
    /// A row matched and was tapped, and the pill did not follow.
    NotConfirmed,
}

/// A tap planner that lands inside the control rather than dead centre every time.
///
/// Offered here because [`crate::nurture::touch::TouchPointPlanner`] is crate-private, and the
/// alternative was to make it public so one caller outside this crate could build its own — which
/// would let two callers build *different* ones. A publish run that tapped exact centres would
/// stand out from every other session this project drives, and the jitter policy is not the
/// publish path's to choose.
pub fn human_taps(screen: Screen) -> impl TapPlanner {
    let mut planner =
        crate::nurture::touch::TouchPointPlanner::new((screen.width(), screen.height()));
    move |element: &ElementBox| planner.next(element.centre(), element.jitter_radius())
}

/// One composer session, driven a step at a time.
///
/// # The steps are private, and that is the safety
///
/// Only [`Composer::new`] and [`Composer::leave`] are public; every step between them is
/// reachable only through [`publish_carousel`] and [`reach_edit_step`], which walk them in
/// order and check each one.
///
/// A review found what the public surface admitted: `post` took nothing but a stop flag, so a
/// caller standing on the post screen could publish without a caption ever being typed — and
/// `advance_to_edit_step` took a caller-supplied rectangle and tapped it before checking where
/// it led, so handing it the Post button published too. Both made the claim on
/// [`Composer::post`] that it is the only function here that can publish simply false.
pub struct Composer<'a, P: TapPlanner> {
    session: &'a dyn UiSession,
    plan: ComposerPlan,
    plan_tap: P,
}

impl<'a, P: TapPlanner> Composer<'a, P> {
    pub fn new(session: &'a dyn UiSession, plan: ComposerPlan, plan_tap: P) -> Self {
        Self {
            session,
            plan,
            plan_tap,
        }
    }

    pub fn plan(&self) -> ComposerPlan {
        self.plan
    }

    async fn tap_inside(&mut self, element: &ElementBox) -> anyhow::Result<()> {
        let point = (self.plan_tap)(element);
        self.session.tap(point).await
    }

    /// Tap the composer tab and **wait until the bottom bar is gone**.
    ///
    /// The wait is the point, and an earlier version skipped it: it tapped and returned
    /// `true`, so a Create tap that the phone accepted and ignored — an overlay eating it,
    /// most often — was followed straight away by a blind arithmetic tap on the *feed*,
    /// around where the gallery entry would have been. That lands on the action rail.
    ///
    /// The signal is the composer opener's own disappearance: it lives on the bottom tab bar,
    /// which the composer replaces with its own mode row. A control this plan already
    /// carries, and one no other screen can fake.
    async fn open(&mut self, stop: &AtomicBool) -> anyhow::Result<bool> {
        let Some(opener) = self.session.locate(self.plan.open).await? else {
            return Ok(false);
        };
        self.tap_inside(&opener).await?;
        self.await_absent(COMPOSER_WINDOW, self.plan.open, stop)
            .await
    }

    /// Locate the shutter and tap the gallery entry anchored to it.
    ///
    /// Two steps in one call because they are one decision: without the anchor there is no
    /// entry to tap, and the alternative — remembered coordinates — is what puts a tap on the
    /// effects panel.
    async fn tap_gallery_entry(
        &mut self,
        screen: Screen,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        let Some(shutter) = self
            .await_condition(COMPOSER_WINDOW, self.plan.shutter, stop, |_| true)
            .await?
        else {
            return Ok(false);
        };
        let Some(entry) = GalleryEntry::beside_shutter(screen, &shutter) else {
            return Ok(false);
        };
        self.tap_inside(&entry.rect()).await?;
        Ok(true)
    }

    /// Wait for the picker, proved by a control only the picker has.
    ///
    /// **Not by the camera controls' absence.** Measured on this build: a hierarchy dump
    /// taken while the picker is open still contains the camera screen's nodes underneath it,
    /// so "the shutter is gone" is never true and "the shutter is present" never means the
    /// picker is closed. The only sound test is a node the picker alone contributes.
    async fn await_picker(&self, stop: &AtomicBool) -> anyhow::Result<bool> {
        Ok(self
            .await_condition(PICKER_WINDOW, self.plan.multi_select, stop, |_| true)
            .await?
            .is_some())
    }

    /// Open the album menu, pick the campaign's own album by name, **and check it took**.
    ///
    /// `album` is the `importId` — a string **this project wrote itself** when it created the
    /// import directory, which is the whole reason the album can be addressed at all.
    ///
    /// # Exactly one match, or refuse
    ///
    /// A row is chosen only when the name matches once. More than one match means the phone
    /// holds two directories whose names both contain ours, and picking the first would
    /// publish from whichever the list sorted higher.
    ///
    /// # And then the pill has to agree
    ///
    /// Matching a row proves what was on screen in **one snapshot**; it does not prove the
    /// coordinate tap that followed selected that row. The album list reflows as thumbnails
    /// load, so a tap dispatched against a stale rectangle can land on the neighbour — and
    /// the picker that comes up afterwards looks entirely normal, selects fine, arms `Next`
    /// fine, and publishes **another album's images**. So the pill is read back: it shows the
    /// current album, which is the one thing on this screen that says which album we are
    /// actually in.
    async fn select_album(
        &mut self,
        album: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<AlbumChoice> {
        let Some(menu) = self
            .await_condition(PICKER_WINDOW, self.plan.album_menu, stop, |_| true)
            .await?
        else {
            return Ok(AlbumChoice::NotFound);
        };
        self.tap_inside(&menu).await?;
        sleep(POLL, stop).await;
        let rows = self
            .session
            .locate_all_described(ElementQuery::Text {
                value: album,
                exact: true,
            })
            .await?;
        let [row] = rows.as_slice() else {
            return Ok(AlbumChoice::NotFound);
        };
        let row = row.clone();
        self.tap_inside(&row).await?;
        Ok(if self.pill_reads(album, stop).await? {
            AlbumChoice::Confirmed
        } else {
            AlbumChoice::NotConfirmed
        })
    }

    /// Whether the album pill now names `album`.
    ///
    /// # This has to read `text`, and reading the wrong attribute made it always fail
    ///
    /// [`UiSession::locate`] fills `ElementBox::description` from **`content-desc`**, whatever
    /// the query matched on — the query decides which node, not which attribute comes back.
    /// The pill is found by its resource id and its name lives in `text`; measured on
    /// `com.ss.android.ugc.trill` 38.3.2, *no* control in the picker carries a `content-desc`
    /// at all. So comparing `description` compared `None` against the album name, on every
    /// build, and every walk stopped at [`AlbumChoice::NotConfirmed`] before selecting an
    /// image.
    ///
    /// [`UiSession::locate_all_described`] is the one that reads `text`, which is why the
    /// readback goes through it and the tap above does not.
    async fn pill_reads(&self, album: &str, stop: &AtomicBool) -> anyhow::Result<bool> {
        let deadline = Instant::now() + PICKER_WINDOW;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            let pills = self
                .session
                .locate_all_described(self.plan.album_menu)
                .await
                .unwrap_or_default();
            if pills.iter().any(|pill| {
                pill.description
                    .as_deref()
                    .is_some_and(|text| text.trim() == album)
            }) {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            sleep(POLL, stop).await;
        }
    }

    /// Turn on multi-selection, which a carousel needs.
    async fn enable_multi_select(&mut self, stop: &AtomicBool) -> anyhow::Result<bool> {
        let Some(control) = self
            .await_condition(PICKER_WINDOW, self.plan.multi_select, stop, |_| true)
            .await?
        else {
            return Ok(false);
        };
        self.tap_inside(&control).await?;
        Ok(true)
    }

    /// Build the image grid from the picker's located tab row.
    ///
    /// `None` when the tab row is not on screen — the anchor is required rather than
    /// defaulted, so a picker that did not render cannot be tapped at remembered coordinates.
    async fn grid(&self, screen: Screen, stop: &AtomicBool) -> anyhow::Result<Option<PhotoGrid>> {
        let Some(tabs) = self
            .await_condition(PICKER_WINDOW, self.plan.tabs, stop, |_| true)
            .await?
        else {
            return Ok(None);
        };
        // Same rule as the gallery entry's anchor: a tab row that is not a rectangle on this
        // screen cannot fix where the grid starts, even when the arithmetic happens to land
        // somewhere plausible.
        if !on_screen(&tabs, screen) {
            return Ok(None);
        }
        Ok(PhotoGrid::below_tabs(screen, tabs.y + tabs.height))
    }

    /// Tap `count` cells — proving the **first** one took before any more are sent — then
    /// check once at the end that the whole set armed.
    ///
    /// The first-cell proof exists because of a measured failure, not a preference
    /// (§9.132, two of four walks): the `Select multiple` tap sometimes does not take, and
    /// in single mode the first cell tap opens the single-photo **editor**. Every further
    /// "cell" tap then lands blind on that editor's own controls — a real tap on an unknown
    /// screen per image. `Next` arming after exactly one cell is the same signal
    /// [`Self::await_armed`] already trusts (the editor's own `Next` renders its text on a
    /// non-clickable node, so it reads honestly as unarmed there).
    ///
    /// Past the first cell there is nothing per cell to check — see [`Selection::Armed`]
    /// for exactly how weak the end-of-set evidence is, and why it is still the strongest
    /// available.
    ///
    /// Refuses a `count` past this screen's visible capacity instead of scrolling.
    async fn select(
        &mut self,
        grid: &PhotoGrid,
        count: usize,
        stop: &AtomicBool,
    ) -> anyhow::Result<Selection> {
        if count == 0 || count > grid.capacity() {
            return Ok(Selection::MoreCellsThanTheGridShows);
        }
        for index in 0..count {
            // Checked **before** each tap, not only in the sleep. An earlier version let the
            // stop flag skip the pacing while still sending every remaining tap, so asking
            // the run to stop made it tap faster.
            if stop.load(Ordering::Relaxed) {
                return Ok(Selection::Stopped);
            }
            let Some(cell) = grid.cell(index) else {
                return Ok(Selection::MoreCellsThanTheGridShows);
            };
            self.tap_inside(&cell).await?;
            sleep(POLL, stop).await;
            if index == 0 && self.await_armed(stop).await?.is_none() {
                return Ok(if stop.load(Ordering::Relaxed) {
                    Selection::Stopped
                } else {
                    Selection::MultiSelectDidNotEngage
                });
            }
        }
        match self.await_armed(stop).await? {
            Some(next) => {
                let counted = self.counted_selection().await;
                Ok(Selection::Armed { next, counted })
            }
            None if stop.load(Ordering::Relaxed) => Ok(Selection::Stopped),
            None => Ok(Selection::NeverArmed),
        }
    }

    /// How many images the picker says are selected, when it says so at all.
    ///
    /// Read out of the `Next` control's own rendered text, because that is where a build that
    /// states a count puts it (`Next (5)`, `Tiếp 5`). Measured on
    /// `com.ss.android.ugc.trill` 38.3.2 the text is a bare `Next` and this returns `None` —
    /// which is a fact about that build, not a failure.
    ///
    /// Through `locate_all_described` rather than `locate` for the reason the album pill
    /// needed the same: `locate` returns `content-desc`, and the picker's controls carry only
    /// `text`.
    async fn counted_selection(&self) -> Option<usize> {
        let rows = self
            .session
            .locate_all_described(self.plan.picker_next)
            .await
            .ok()?;
        rows.iter()
            .filter_map(|row| row.description.as_deref())
            .filter_map(|text| {
                let digits: String = text
                    .chars()
                    .skip_while(|character| !character.is_ascii_digit())
                    .take_while(char::is_ascii_digit)
                    .collect();
                digits.parse::<usize>().ok()
            })
            .next()
    }

    /// Wait for `Next` to arm, and return it so the caller can tap it.
    ///
    /// Reads `clickable`, **not** `enabled`. On this build `enabled` is `true` both before
    /// and after a selection, so a version of this that read it would return immediately with
    /// nothing selected — and the next step would advance out of the picker empty-handed.
    async fn await_armed(&self, stop: &AtomicBool) -> anyhow::Result<Option<ElementBox>> {
        self.await_condition(ARM_WINDOW, self.plan.picker_next, stop, |element| {
            element.clickable
        })
        .await
    }

    /// Tap the picker's `Next` and wait for the edit step.
    ///
    /// # Two proofs, and the weaker one is what makes the measuring trip possible
    ///
    /// When `ComposerNext` is measured, arrival is proved by that control appearing — a
    /// positive signal about the screen we wanted.
    ///
    /// When it is not, arrival is proved by the **picker's** multi-select control going away.
    /// That is weaker: it says we left the picker, not that we arrived at the edit step. It is
    /// also the only thing available, and refusing to accept it is what made the first version
    /// of this module unable to take its own measurement — `drive_to_edit_step` waited for
    /// `ComposerNext`, which is the label the trip exists to read.
    ///
    /// The weaker proof is never enough to publish: [`ComposerPlan::can_publish`] is false on
    /// exactly the builds that fall back to it.
    async fn advance_to_edit_step(
        &mut self,
        next: &ElementBox,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        self.tap_inside(next).await?;
        match self.plan.edit_step_marker {
            Some(edit_next) => Ok(self
                .await_condition(COMPOSER_WINDOW, edit_next, stop, |_| true)
                .await?
                .is_some()),
            None => {
                self.await_absent(COMPOSER_WINDOW, self.plan.multi_select, stop)
                    .await
            }
        }
    }

    /// Tap the **edit step's** Next and wait for the post screen.
    ///
    /// This step existed only as a screen marker before, which is a bug worth naming: the
    /// path required `ComposerNext` to be measured, waited for it to appear, and then never
    /// tapped it — so a fully measured build would have reached the edit screen, looked for
    /// the Post button that lives on the *next* screen, timed out, and reported "no Post
    /// button" on a run where everything worked.
    ///
    /// Proved by the Post button arriving, so a build without it measured cannot call this —
    /// which is correct: that build has nothing to advance toward.
    async fn advance_to_post_screen(&mut self, stop: &AtomicBool) -> anyhow::Result<bool> {
        let Some(tail) = self.plan.publish else {
            return Ok(false);
        };
        let (post, edit_next) = (tail.post_button, tail.edit_next);
        let Some(next) = self
            .await_condition(COMPOSER_WINDOW, edit_next, stop, |_| true)
            .await?
        else {
            return Ok(false);
        };
        self.tap_inside(&next).await?;
        Ok(self
            .await_condition(COMPOSER_WINDOW, post, stop, |_| true)
            .await?
            .is_some())
    }

    /// Type the caption into the post screen's field, and read it back.
    ///
    /// # Why the readback, when nothing else in this module verifies a keystroke
    ///
    /// Because the failure is silent and the result is permanent. `type_text` targets the
    /// **focused** text field, and the post screen has more than one — the comment drawer's
    /// equivalent mistake wrote into the collapsed bar behind the real one, succeeding at the
    /// API level while the screen stayed empty. Here that would publish a carousel with an
    /// empty caption, and there is no delete path on Android to correct it.
    ///
    /// So the field is tapped to focus it, the text is set, and the same locator is asked what
    /// the field now holds. That last part is why
    /// [`TikTokControl::ComposerCaption`] must be a class or a resource id rather than the
    /// placeholder string: a placeholder stops matching as soon as a character arrives, and a
    /// placeholder-based entry would type correctly and then report that it had not.
    ///
    /// # An empty caption types nothing and is not a failure
    ///
    /// The scan requires exactly one `caption*.txt` per bundle; it does not require the file
    /// to have anything in it. An operator who wants a bare carousel gets one.
    async fn type_caption(
        &mut self,
        caption: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<CaptionOutcome> {
        let Some(query) = self.plan.publish.map(|tail| tail.caption) else {
            return Ok(CaptionOutcome::Unmeasured);
        };
        if caption.trim().is_empty() {
            return Ok(CaptionOutcome::NothingToSay);
        }
        let Some(field) = self
            .await_condition(COMPOSER_WINDOW, query, stop, |_| true)
            .await?
        else {
            return Ok(CaptionOutcome::NoField);
        };
        // Focusing is not optional: `type_text` writes into whichever field has focus.
        self.tap_inside(&field).await?;
        self.session.type_text(caption).await?;

        // **Equality on exactly one field, not a prefix on any field.**
        //
        // The first version compared `text.contains(first 24 characters)` across every match,
        // and a review took it apart in two directions at once. It accepted a field that had
        // kept only the first 24 characters — publishing a truncated caption that Android
        // cannot delete. And because the focus tap uses `locate` (the first match) while the
        // readback used `locate_all_described` (any match), a non-unique or placeholder
        // locator could focus one node and confirm a different one: a placeholder reading
        // `Describe your post…` confirms a caption that starts with those words even though
        // the write never landed.
        //
        // So: exactly one node must match the locator, and its text must equal the caption.
        // A build that reformats what it stores fails closed here — which is a measurement to
        // take, not a post to publish on a guess.
        let wanted = caption.trim();
        let deadline = Instant::now() + COMPOSER_WINDOW;
        loop {
            let rows = self
                .session
                .locate_all_described(query)
                .await
                .unwrap_or_default();
            if let [only] = rows.as_slice() {
                if only
                    .description
                    .as_deref()
                    .is_some_and(|text| text.trim() == wanted)
                {
                    return Ok(CaptionOutcome::Typed);
                }
            }
            if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                return Ok(CaptionOutcome::NotConfirmed);
            }
            sleep(POLL, stop).await;
        }
    }

    /// Publish, or refuse because this build's Post button was never measured.
    ///
    /// **The only function in this module that can put something on a real account.**
    ///
    /// # Nothing after the tap may return `Err`
    ///
    /// Once the tap has been handed to the agent the outcome is *unknown*, not failed — the
    /// phone may well have published. So every failure from that line onward becomes
    /// [`ComposerVerdict::PostNotConfirmed`], which [`ComposerVerdict::may_retry`] refuses.
    /// Propagating an error instead would hand a caller that retries transport errors a clean
    /// path to publishing a duplicate.
    ///
    /// # Confirmed by the feed coming back, not by the button going away
    ///
    /// An earlier version waited for the Post label to be absent, which is wrong in both
    /// directions: a button still rendered on the first poll was reported unconfirmed even
    /// though the publish was seconds from finishing, and *any* screen without a Post label —
    /// an error dialog, an account check — was reported as published. The positive signal is
    /// the bottom tab bar returning, which only happens once the composer has let go of the
    /// screen.
    ///
    /// What is still not measured is the public/private confirmation sheet this build is
    /// expected to raise between the tap and the feed. If it appears, the feed does not come
    /// back, and this returns `PostNotConfirmed` — the safe answer, not a correct one.
    /// Closing that is a measurement, not a code change.
    async fn post(&mut self, stop: &AtomicBool) -> anyhow::Result<ComposerVerdict> {
        let Some(query) = self.plan.publish.map(|tail| tail.post_button) else {
            return Ok(ComposerVerdict::PostUnmeasured);
        };
        if stop.load(Ordering::Relaxed) {
            return Ok(ComposerVerdict::Stopped);
        }
        let Some(button) = self
            .await_condition(COMPOSER_WINDOW, query, stop, |_| true)
            .await?
        else {
            return Ok(ComposerVerdict::NoPostButton);
        };
        // The last point at which stopping is still free.
        if stop.load(Ordering::Relaxed) {
            return Ok(ComposerVerdict::Stopped);
        }
        if self.tap_inside(&button).await.is_err() {
            // The tap may have reached the phone before the transport died.
            return Ok(ComposerVerdict::PostNotConfirmed);
        }
        // **Deliberately not passing `stop`.** Cancelling cannot un-publish, and a stop set
        // here would end the wait early and downgrade a good post to `PostNotConfirmed`,
        // which is permanently unclaimable.
        //
        // **And one dropped hierarchy read must not end it either.** `await_condition`
        // propagates the first error, which this used to swallow with `.ok().flatten()` — so a
        // single transient agent failure on the first poll turned a live post into
        // `PostNotConfirmed`, permanently unclaimable, twenty seconds early.
        let back_on_the_feed = self.await_feed(POST_CONFIRM_WINDOW).await;
        Ok(if back_on_the_feed {
            ComposerVerdict::Posted
        } else {
            ComposerVerdict::PostNotConfirmed
        })
    }

    /// Back out until the bottom tab bar is visible again.
    ///
    /// Best effort by design, exactly like [`crate::tiktok_drawer::CommentDrawer::leave`]:
    /// this runs on failure paths, where returning an error would replace a precise verdict
    /// with a vague one. What it must not do is leave the phone standing inside a half-filled
    /// composer, because the next session's first gesture would land there.
    ///
    /// # Waits for a screen it *wants*, not for screens it recognises
    ///
    /// The first version tested the negative — press Back while the picker's controls are on
    /// screen — and was wrong in the case that matters most. The flow can stand on five
    /// screens, and the camera screen between the composer tab and the picker carries none of
    /// those controls, so an error there read as "already out" and left the phone sitting in
    /// the composer with the shutter up.
    ///
    /// # A failing Back is retried, not surrendered to
    ///
    /// One transport error used to end the whole attempt, leaving the composer open on the
    /// strength of a single dropped request. The budget is spent on attempts, not on
    /// successes, so a flaky link gets its retries and a dead one still terminates.
    ///
    /// Returns whether the phone is known to be out. A caller may log that; it must not turn
    /// it into the run's verdict, because a composer left open is a *housekeeping* failure
    /// and the verdict is about the post.
    ///
    /// **What this cannot cover:** dropping the future mid-`drive` — an aborted task — skips
    /// it entirely, because there is no async destructor to hang it on. A caller that aborts
    /// publish work has to reconcile the phone itself.
    pub async fn leave(&self) -> bool {
        for _ in 0..8 {
            if self
                .session
                .locate(self.plan.open)
                .await
                .ok()
                .flatten()
                .is_some()
            {
                return true;
            }
            // A failed Back costs an attempt, not the attempt: one dropped request used to
            // end this loop and leave the composer open.
            let _ = self.session.back().await;
            // A real sleep, deliberately not the stop-aware one: this is the pause that lets
            // the screen change between presses, and skipping it turns eight Backs into one
            // burst the app coalesces — so a cancelled run would be the one left inside the
            // composer.
            tokio::time::sleep(POLL).await;
        }
        self.session
            .locate(self.plan.open)
            .await
            .ok()
            .flatten()
            .is_some()
    }

    /// Wait for the bottom tab bar to come back, ignoring reads that fail on the way.
    ///
    /// The one wait in this module that must not give up on a transport error: it runs after
    /// the Post tap, where "I could not read the screen" and "the post did not go" are
    /// different facts and only the second is worth reporting.
    async fn await_feed(&self, window: Duration) -> bool {
        let deadline = Instant::now() + window;
        loop {
            if matches!(self.session.locate(self.plan.open).await, Ok(Some(_))) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(POLL).await;
        }
    }

    /// Wait for an element to be **absent**, which is a different question from
    /// [`Self::await_condition`] and cannot be expressed with it.
    async fn await_absent(
        &self,
        window: Duration,
        query: ElementQuery<'_>,
        stop: &AtomicBool,
    ) -> anyhow::Result<bool> {
        let deadline = Instant::now() + window;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            if self.session.locate(query).await?.is_none() {
                return Ok(true);
            }
            if Instant::now() >= deadline {
                return Ok(false);
            }
            sleep(POLL, stop).await;
        }
    }

    async fn await_condition(
        &self,
        window: Duration,
        query: ElementQuery<'_>,
        stop: &AtomicBool,
        ready: impl Fn(&ElementBox) -> bool,
    ) -> anyhow::Result<Option<ElementBox>> {
        let deadline = Instant::now() + window;
        loop {
            // **Stop first.** With the order reversed a control that happened to be on screen
            // won over an already-set stop flag, so asking the run to stop while the Post
            // button was rendered handed that button straight back to the caller.
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            if let Some(element) = self.session.locate(query).await? {
                if ready(&element) {
                    return Ok(Some(element));
                }
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            sleep(POLL, stop).await;
        }
    }
}

/// What one carousel needs: which album, how many images, and the screen it is on.
///
/// Carries no geometry. An earlier version took a caller-built `PhotoGrid` and
/// `GalleryEntry`, which let a caller hand in numbers measured on a different phone — the
/// grid would then tap a header row, arm `Next` on whatever it hit, and publish a wrong
/// subset. Both are now built inside the driver from located anchors.
pub struct CarouselRequest<'a> {
    /// The `importId` the media path used to name the album.
    pub album: &'a str,
    /// How many images to select.
    pub images: usize,
    /// The bundle's caption, verbatim. Empty is allowed and types nothing.
    pub caption: &'a str,
    pub screen: Screen,
}

/// Drive one carousel from the feed to published, and close the composer behind it.
///
/// **Refuses before the first tap** on a build whose Post button was never measured. That
/// refusal is the module's whole promise, and an earlier version broke it: it drove the phone
/// through the composer, the album, and every grid tap before discovering at the last step
/// that it could not publish — leaving a phone parked in an armed composer for a condition
/// that was knowable before anything opened.
///
/// A measuring session that *wants* to reach those screens calls [`drive_to_edit_step`]
/// instead, which says so in its name.
pub async fn publish_carousel(
    session: &dyn UiSession,
    plan: ComposerPlan,
    plan_tap: impl TapPlanner,
    request: &CarouselRequest<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<ComposerVerdict> {
    if !plan.can_publish() {
        // Nothing was opened, so there is nothing to close.
        return Ok(ComposerVerdict::PostUnmeasured);
    }
    let mut composer = Composer::new(session, plan, plan_tap);
    let outcome = drive(&mut composer, request, stop, true).await;
    // Closed on the way out whatever happened, including on an error: every step can fail with
    // `?`, and each of those failures would otherwise leave the phone standing in a composer
    // with a campaign's images selected.
    //
    // **Except when the post is unconfirmed.** That verdict means a screen we do not recognise
    // is up after the Post tap, and the one this build is expected to raise is a
    // public/private confirmation sheet — which is *the thing that commits the post*. Pressing
    // Back there dismisses it: the carousel definitely does not go out, and the assignment is
    // still marked permanently unclaimable, so the run sacrifices a post it could have had.
    // Leaving the phone as it is hands an operator a screen they can act on.
    if !matches!(outcome, Ok(ComposerVerdict::PostNotConfirmed)) {
        composer.leave().await;
    }
    outcome
}

/// Drive as far as the edit step and stop there, publishing nothing.
///
/// **The instrument for the measurement that is still missing.** `ComposerNext` and
/// `PostButton` have never been read off any phone, and they cannot be by hand: they live on
/// screens nobody can reach and dump fast enough while the picker state is right. This gets a
/// phone to them with a real campaign album selected, so `label_scout` can read them — and
/// then backs out.
///
/// Everything it touches is reversible. It never taps the edit step's Next and never looks
/// for a Post button, so there is no sequence of failures inside it that publishes anything.
pub async fn drive_to_edit_step(
    session: &dyn UiSession,
    plan: ComposerPlan,
    plan_tap: impl TapPlanner,
    request: &CarouselRequest<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<ComposerVerdict> {
    let mut composer = Composer::new(session, plan, plan_tap);
    let outcome = reach_edit_step(&mut composer, request, stop).await;
    composer.leave().await;
    outcome
}

/// The same walk, **without backing out at the end**.
///
/// Exists for one caller: the measuring tool, which has to dump the screen it arrived at. The
/// convenience above closes the composer immediately, which is right for every other use and
/// exactly wrong for the one that came to look at it.
///
/// The caller owns the [`Composer`] and **must** call [`Composer::leave`] itself — including on
/// the error paths, which is why this is not the default shape.
///
/// Publishes nothing: it never taps the edit step's Next and never looks for a Post button, so
/// there is no sequence of failures inside it that puts a carousel on an account.
pub async fn reach_edit_step<P: TapPlanner>(
    composer: &mut Composer<'_, P>,
    request: &CarouselRequest<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<ComposerVerdict> {
    drive(composer, request, stop, false).await
}

/// The steps that need an open composer, split out so both entry points close it on every
/// exit without repeating the call at each early return.
///
/// `publish` decides whether the last two steps happen at all. A parameter rather than two
/// copies of the walk, because the walk is where the verification lives and two copies would
/// drift — the measuring path would quietly lose a check the publishing path has.
async fn drive<P: TapPlanner>(
    composer: &mut Composer<'_, P>,
    request: &CarouselRequest<'_>,
    stop: &AtomicBool,
    publish: bool,
) -> anyhow::Result<ComposerVerdict> {
    if !composer.open(stop).await? {
        return Ok(ComposerVerdict::ComposerDidNotOpen);
    }
    if !composer.tap_gallery_entry(request.screen, stop).await? {
        return Ok(ComposerVerdict::NoShutterToAnchorTo);
    }
    if !composer.await_picker(stop).await? {
        return Ok(ComposerVerdict::PickerDidNotOpen);
    }
    match composer.select_album(request.album, stop).await? {
        AlbumChoice::Confirmed => {}
        AlbumChoice::NotFound => return Ok(ComposerVerdict::AlbumNotFound),
        AlbumChoice::NotConfirmed => return Ok(ComposerVerdict::AlbumNotConfirmed),
    }
    if !composer.enable_multi_select(stop).await? {
        return Ok(ComposerVerdict::PickerDidNotOpen);
    }
    // Built after the album is chosen, because that is when the grid this taps exists.
    let Some(grid) = composer.grid(request.screen, stop).await? else {
        return Ok(ComposerVerdict::NoTabsToAnchorTo);
    };
    let next = match composer.select(&grid, request.images, stop).await? {
        // **A stated count is believed, and a mismatch stops the run.** `Next` arming proves
        // only that *something* is selected; a build that also renders the number is the one
        // chance to prove the rest, and taking it is the difference between publishing five
        // images and publishing whichever of the five taps happened to land.
        Selection::Armed {
            counted: Some(counted),
            ..
        } if counted != request.images => return Ok(ComposerVerdict::NotEnoughSelected),
        Selection::Armed { next, .. } => next,
        Selection::MoreCellsThanTheGridShows => {
            return Ok(ComposerVerdict::MoreCellsThanTheGridShows)
        }
        Selection::NeverArmed => return Ok(ComposerVerdict::NeverArmed),
        Selection::MultiSelectDidNotEngage => return Ok(ComposerVerdict::MultiSelectDidNotEngage),
        Selection::Stopped => return Ok(ComposerVerdict::Stopped),
    };
    if !composer.advance_to_edit_step(&next, stop).await? {
        return Ok(ComposerVerdict::EditStepDidNotOpen);
    }
    if !publish {
        // The measuring path stops here, on the screen it came to read.
        return Ok(ComposerVerdict::Stopped);
    }
    if !composer.advance_to_post_screen(stop).await? {
        return Ok(ComposerVerdict::PostScreenDidNotOpen);
    }
    // **Before Post, and its failures stop the run.** A carousel that goes out with the wrong
    // caption — or none — cannot be corrected from here.
    match composer.type_caption(request.caption, stop).await? {
        CaptionOutcome::Typed | CaptionOutcome::NothingToSay => {}
        CaptionOutcome::Unmeasured => return Ok(ComposerVerdict::PostUnmeasured),
        CaptionOutcome::NoField => return Ok(ComposerVerdict::NoCaptionField),
        CaptionOutcome::NotConfirmed => return Ok(ComposerVerdict::CaptionNotConfirmed),
    }
    composer.post(stop).await
}

/// Sleep unless the caller has asked to stop.
async fn sleep(duration: Duration, stop: &AtomicBool) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(left.min(Duration::from_millis(120))).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::ElementBox;
    use crate::tiktok_labels::{
        controls_for, every_publish_control_but_caption_measured,
        every_publish_control_but_post_measured, every_publish_control_measured, nothing_measured,
        TIKTOK_LABEL_SETS,
    };
    use crate::types::TapPoint;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    fn plan() -> ComposerPlan {
        ComposerPlan::resolve(&every_publish_control_measured()).expect("the fixture is complete")
    }

    fn measuring_plan() -> ComposerPlan {
        ComposerPlan::resolve(&every_publish_control_but_post_measured())
            .expect("everything up to the edit step is measured")
    }

    fn screen() -> Screen {
        Screen::new(1080.0, 2220.0).expect("the measured screen")
    }

    fn labelled(label: &str, x: f64, y: f64, width: f64, height: f64) -> ElementBox {
        ElementBox {
            x,
            y,
            width,
            height,
            description: Some(label.into()),
            enabled: true,
            clickable: true,
        }
    }

    fn box_at(x: f64, y: f64) -> ElementBox {
        ElementBox {
            x,
            y,
            width: 100.0,
            height: 50.0,
            description: None,
            enabled: true,
            clickable: true,
        }
    }

    /// The five screens the flow walks, as the fake sees them.
    ///
    /// Named rather than numbered because every navigation assertion in this module is about
    /// *which* screen the phone ended on, and `screens[2]` says nothing.
    /// One screen, plus the control whose tap navigates off it.
    ///
    /// `exit: None` means *any* tap navigates, which is how the camera screen behaves here:
    /// what gets tapped there is the unlabelled gallery entry, so there is no key to name.
    struct Scene {
        elements: HashMap<String, ElementBox>,
        /// The **rendered `text`** of a node, which is a different attribute from the
        /// `content-desc` `elements` carries.
        ///
        /// Modelled separately because Android's two read paths return different attributes
        /// for the same node, and a fake that conflated them hid a blocking bug: the album
        /// pill's name lives in `text`, `UiSession::locate` returns `content-desc`, and the
        /// confirmation compared the wrong one — so every walk stopped before selecting an
        /// image while every test passed.
        texts: HashMap<String, String>,
        /// Which element's tap navigates off this screen.
        exit: Option<String>,
        /// A rectangle a tap must land **inside** to navigate, for screens whose exit is
        /// unlabelled geometry.
        ///
        /// `exit: None` used to mean "any tap navigates", which erased the wrong-control
        /// failure this module treats as consequential: tapping the shutter, or the effects
        /// circles beside it, counted as opening the gallery.
        exit_rect: Option<ElementBox>,
    }

    fn scene(elements: Vec<(&str, ElementBox)>, exit: Option<&str>) -> Scene {
        Scene {
            elements: elements
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
            texts: HashMap::new(),
            exit: exit.map(str::to_string),
            exit_rect: None,
        }
    }

    impl Scene {
        /// Give a node a rendered `text`, distinct from its `content-desc`.
        fn texted(mut self, key: &str, text: &str) -> Self {
            self.texts.insert(key.to_string(), text.to_string());
            self
        }

        /// Navigate only when a tap lands inside this rectangle.
        fn leaving_by(mut self, rect: ElementBox) -> Self {
            self.exit_rect = Some(rect);
            self
        }
    }

    fn feed() -> Scene {
        scene(
            vec![(
                "fixture-composer-open",
                labelled("Create", 432.0, 1929.0, 216.0, 147.0),
            )],
            Some("fixture-composer-open"),
        )
    }
    fn camera() -> Scene {
        let shutter = labelled("Record video", 375.0, 1545.0, 330.0, 330.0);
        let entry = GalleryEntry::beside_shutter(screen(), &shutter)
            .expect("the measured entry is on screen")
            .rect();
        // **Only a tap inside the gallery entry opens the picker.** The shutter records video
        // and the circles beside it open the effects panel, which is the failure this module
        // exists to avoid — a fake where any tap navigates cannot see it.
        scene(vec![("fixture-shutter", shutter)], None).leaving_by(entry)
    }
    fn picker_elements() -> Vec<(&'static str, ElementBox)> {
        vec![
            ("fixture-multi-select", box_at(126.0, 1937.0)),
            (
                "fixture-tab-photos",
                labelled("Photos", 824.0, 255.0, 152.0, 57.0),
            ),
            // **No `content-desc`.** Measured: not one control in this picker has one, which
            // is why the album pill has to be read through `locate_all_described`.
            (
                "fixture-album-menu",
                ElementBox {
                    description: None,
                    ..labelled("", 483.0, 115.0, 60.0, 57.0)
                },
            ),
            (
                "fixture-picker-next",
                ElementBox {
                    clickable: true,
                    ..box_at(552.0, 1896.0)
                },
            ),
        ]
    }
    /// The picker, with the album its pill currently names and the control that leaves it.
    fn picker(album: &str, exit: Option<&str>) -> Scene {
        scene(picker_elements(), exit).texted("fixture-album-menu", album)
    }
    fn edit_step() -> Scene {
        scene(
            vec![("fixture-edit-next", box_at(900.0, 200.0))],
            Some("fixture-edit-next"),
        )
    }
    fn post_screen() -> Scene {
        scene(
            vec![
                ("fixture-caption", box_at(100.0, 300.0)),
                ("fixture-post", box_at(900.0, 2000.0)),
            ],
            // Only Post leaves this screen; tapping the caption field focuses it.
            Some("fixture-post"),
        )
    }

    /// A phone that shows one screen at a time and advances when tapped.
    ///
    /// Modelled as a *stack of screens* rather than a queue of answers because the thing
    /// under test is navigation: the composer is four screens deep, and the failure this fake
    /// has to be able to express is "the tap went out and the next screen never came", which
    /// a per-query queue cannot say.
    #[derive(Default)]
    struct FakeSession {
        screens: Vec<Scene>,
        at: Mutex<usize>,
        taps: Mutex<Vec<TapPoint>>,
        backs: Mutex<usize>,
        rows: Mutex<HashMap<String, Vec<ElementBox>>>,
        /// What `type_text` last wrote, which is what the caption field then holds.
        ///
        /// Modelled rather than ignored: the readback in `type_caption` is the whole reason
        /// that step exists, and a fake whose field never changes cannot tell a caption that
        /// took from one that silently did not.
        typed: Mutex<Option<String>>,
        /// Make the caption field keep whatever `caption_placeholder` holds, i.e. never show
        /// the typed text. `None` there models a field that vanishes; `Some` models the far
        /// more likely case — a placeholder that is still on screen.
        caption_never_takes: bool,
        caption_placeholder: Option<String>,
        /// Put a second node in front of the caption locator.
        ///
        /// A locator that is not unique can focus one node and confirm another; only a fixture
        /// with two matches can tell "read back the field" from "read back something".
        caption_has_a_twin: bool,
        /// Fail every tap from this one onward. Counting rather than a flag because the
        /// interesting failure is **mid-flow**: a run that dies before its first tap never
        /// opened anything, so it proves nothing about closing what it opened.
        fail_taps_after: Option<usize>,
        /// Stop advancing at this screen however many taps arrive.
        stuck_at: Option<usize>,
        /// Refuse every Back, so the give-up path is exercised.
        backs_fail: bool,
        /// Fail the `locate` at this call index, then answer normally.
        ///
        /// An index rather than a flag because *where* the read drops decides what is correct:
        /// before the Post tap, propagating is right — nothing has been published. After it,
        /// "I could not read the screen" and "the post did not go" are different facts.
        locate_fails_at: Option<usize>,
        locates: Mutex<usize>,
    }

    impl FakeSession {
        fn with(screens: Vec<Scene>) -> Self {
            Self {
                screens,
                ..Default::default()
            }
        }

        /// The whole happy walk, one scene per screen the phone really stands on.
        ///
        /// Spelled out rather than generated because the *shape* is the fixture: the picker
        /// appears four times because four different taps happen on it, and a grid tap
        /// happens on the fourth without leaving it. A fake that advanced on every tap — the
        /// first version here — could not express that, and made the grid taps navigate.
        fn full_walk(album: &'static str) -> Self {
            Self::with(vec![
                feed(),
                camera(),
                // Opening the album menu.
                picker("All", Some("fixture-album-menu")),
                // The menu is open. Only a tap **on the matching row** leaves it — an
                // `exit: None` that accepted any tap could not tell choosing the album from
                // missing it.
                picker("All", None).leaving_by(box_at(0.0, 400.0)),
                // The album took: the pill now names it. Multi-select is what leaves.
                picker(album, Some("fixture-multi-select")),
                // Where the grid taps land, and none of them navigates.
                picker(album, Some("fixture-picker-next")),
                edit_step(),
                post_screen(),
                feed(),
            ])
            .rows(album, vec![box_at(0.0, 400.0)])
        }

        fn rows(self, key: &str, rows: Vec<ElementBox>) -> Self {
            self.rows.lock().insert(key.to_string(), rows);
            self
        }

        fn current(&self) -> HashMap<String, ElementBox> {
            self.screens
                .get(*self.at.lock())
                .map(|scene| scene.elements.clone())
                .unwrap_or_default()
        }

        /// What `locate_all_described` sees: the same nodes, carrying their **`text`**.
        fn current_texts(&self) -> HashMap<String, ElementBox> {
            let at = *self.at.lock();
            let Some(scene) = self.screens.get(at) else {
                return HashMap::new();
            };
            scene
                .elements
                .iter()
                .map(|(key, element)| {
                    (
                        key.clone(),
                        ElementBox {
                            description: scene.texts.get(key).cloned(),
                            ..element.clone()
                        },
                    )
                })
                .collect()
        }

        fn on_screen(&self) -> usize {
            *self.at.lock()
        }
    }

    #[async_trait::async_trait]
    impl UiSession for FakeSession {
        async fn tap(&self, point: TapPoint) -> anyhow::Result<()> {
            if self
                .fail_taps_after
                .is_some_and(|limit| self.taps.lock().len() >= limit)
            {
                anyhow::bail!("agent went away mid-gesture");
            }
            let (x, y) = (point.x, point.y);
            self.taps.lock().push(point);
            let mut at = self.at.lock();
            // **Only a tap on this screen's exit control navigates.** A grid cell is tapped
            // where no element is registered, so it changes nothing — which is what the real
            // picker does, and what a counter-based fake got wrong.
            let inside = |element: &ElementBox| {
                x >= element.x
                    && x <= element.x + element.width
                    && y >= element.y
                    && y <= element.y + element.height
            };
            let navigates = match self.screens.get(*at) {
                None => false,
                Some(scene) => match (&scene.exit, &scene.exit_rect) {
                    (Some(key), _) => scene.elements.get(key).is_some_and(inside),
                    (None, Some(rect)) => inside(rect),
                    // No exit at all: this screen does not lead anywhere in the fixture.
                    (None, None) => false,
                },
            };
            if navigates {
                let ceiling = self
                    .stuck_at
                    .unwrap_or(self.screens.len().saturating_sub(1));
                *at = (*at + 1).min(ceiling);
            }
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            *self.typed.lock() = Some(text.to_string());
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            *self.backs.lock() += 1;
            if self.backs_fail {
                anyhow::bail!("the agent is not answering");
            }
            let mut at = self.at.lock();
            *at = at.saturating_sub(1);
            Ok(())
        }
        async fn find_and_tap(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn assert_visible(&self, _accessibility_id: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        fn supports_element_bounds(&self) -> bool {
            true
        }
        async fn locate(&self, query: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            {
                let mut seen = self.locates.lock();
                let at = *seen;
                *seen += 1;
                if self.locate_fails_at == Some(at) {
                    anyhow::bail!("the agent dropped one hierarchy read");
                }
            }
            let wanted = match query {
                ElementQuery::Description { value, .. }
                | ElementQuery::Text { value, .. }
                | ElementQuery::ClassName(value)
                | ElementQuery::ResourceIdSuffix(value) => value,
            };
            Ok(self.current().get(wanted).cloned())
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            let wanted = match query {
                ElementQuery::Description { value, .. }
                | ElementQuery::Text { value, .. }
                | ElementQuery::ClassName(value)
                | ElementQuery::ResourceIdSuffix(value) => value,
            };
            // The caption field reports the text it holds, the way a real one does — or the
            // placeholder it kept, when the write did not land.
            if wanted == "fixture-caption" {
                let held = match (self.caption_never_takes, self.typed.lock().clone()) {
                    (true, _) => self.caption_placeholder.clone(),
                    (false, typed) => typed,
                };
                let mut rows: Vec<ElementBox> = held
                    .map(|text| {
                        vec![ElementBox {
                            description: Some(text),
                            ..box_at(100.0, 300.0)
                        }]
                    })
                    .unwrap_or_default();
                if self.caption_has_a_twin {
                    rows.push(ElementBox {
                        description: Some("một ô khác cũng khớp".into()),
                        ..box_at(100.0, 900.0)
                    });
                }
                return Ok(rows);
            }
            if let Some(rows) = self.rows.lock().get(wanted) {
                return Ok(rows.clone());
            }
            // Everything else answers with its rendered `text`, which is the attribute this
            // call reads and the one `locate` does not.
            Ok(self
                .current_texts()
                .get(wanted)
                .filter(|element| element.description.is_some())
                .cloned()
                .into_iter()
                .collect())
        }
    }

    // ------------------------------------------------------------------ readiness

    /// **Exactly one shipped build can publish, and it is the one that made the trip.**
    ///
    /// The single most important assertion in this module, in both directions. Until
    /// 30/08/2026 it said *no* build publishes, because `ComposerNext`, `ComposerCaption`
    /// and `PostButton` had never been read off a phone. Two `composer_scout` trips on
    /// `trill` 38.3.2 `en` closed that (AGENTS.md §9.132) — and only there: every other
    /// catalogued set, and the same set on a version whose caption id nobody read, still
    /// refuses before the first tap. A second build showing up publishable here without its
    /// own trip is a copied id, which is exactly what this table exists to forbid.
    #[test]
    fn exactly_one_catalogued_build_can_publish_and_the_rest_still_refuse() {
        let mut checked = 0;
        let mut publishable = Vec::new();
        for set in TIKTOK_LABEL_SETS {
            for version in ["", set.measured_app_version] {
                let Some(controls) = controls_for(set.package, set.language, version) else {
                    continue;
                };
                let missing = ComposerPlan::missing_to_publish(&controls);
                let resolves_publishable = ComposerPlan::resolve(&controls)
                    .map(|plan| plan.can_publish())
                    .unwrap_or(false);
                assert_eq!(
                    missing.is_empty(),
                    resolves_publishable,
                    "{} / {} (app {version:?}): the two publish gates disagree",
                    set.package,
                    set.language
                );
                if resolves_publishable {
                    publishable.push(format!(
                        "{} / {} (app {version:?})",
                        set.package, set.language
                    ));
                }
                checked += 1;
            }
        }
        assert_eq!(
            publishable,
            vec![r#"com.ss.android.ugc.trill / en (app "38.3.2")"#.to_string()],
            "the measured trip covers exactly this build; anything else publishing here \
             claims a measurement AGENTS.md does not record"
        );
        assert!(
            checked >= 4,
            "only {checked} sets scanned; the sweep is broken"
        );
    }

    /// **The fleet's own build carries the whole publish tail, measured, and can publish.**
    ///
    /// `com.ss.android.ugc.trill` 38.3.2 runs on sixteen of the twenty phones. The
    /// reach-the-edit-step half was measured 29/08/2026 (the album pill's version-keyed id
    /// was the unlock), and the tail — `ComposerNext`, then `ComposerCaption` and
    /// `PostButton` from the caption screen — came back on 30/08/2026's two `composer_scout`
    /// trips (AGENTS.md §9.132). This assertion has flipped twice on purpose: it pinned
    /// "cannot publish" while the trip was owed, and it pins "can publish" now that the
    /// readings exist — either drift is a lie about what was measured.
    #[test]
    fn the_build_sixteen_phones_run_has_its_whole_publish_tail_measured() {
        let controls = controls_for("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("the fleet's build is catalogued");
        let plan = ComposerPlan::resolve(&controls).unwrap_or_else(|refusal| {
            panic!("the measuring trip is blocked on {:?}", refusal.missing)
        });
        assert!(
            plan.can_publish(),
            "every tail control is measured; a plan that still refuses is dropping one"
        );
        assert_eq!(
            ComposerPlan::missing_to_publish(&controls),
            Vec::<TikTokControl>::new(),
            "nothing is owed on this build any more"
        );

        // **And the album pill is keyed to the version, not to the language.** Resource ids are
        // reassigned on every app rebuild, so a phone whose `versionName` was not read must not
        // borrow another build's id — it refuses instead.
        let unknown_version =
            controls_for("com.ss.android.ugc.trill", "en", "").expect("the language set exists");
        assert_eq!(
            unknown_version.label(TikTokControl::PickerAlbumMenu),
            None,
            "an unread app version borrowed another build's resource id"
        );
        assert!(ComposerPlan::resolve(&unknown_version).is_err());
    }

    /// A set with nothing measured refuses every required control, not just the first.
    #[test]
    fn an_unmeasured_build_is_refused_for_all_of_it_at_once() {
        let refusal = ComposerPlan::resolve(&nothing_measured()).expect_err("refuses");
        assert_eq!(refusal.missing.len(), REQUIRED.len());
        for control in REQUIRED {
            assert!(refusal.missing.contains(&control), "{control:?} not named");
        }
    }

    /// **The geometry anchors are required even though nothing taps them.**
    ///
    /// The gallery entry and the image grid are reached by arithmetic. Arithmetic from a
    /// located element survives a screen the numbers were not taken on; arithmetic from
    /// remembered numbers taps the effects panel. So a build missing either anchor must
    /// refuse, and this is what says so.
    #[test]
    fn a_build_without_its_geometry_anchors_cannot_resolve_a_plan() {
        for anchor in [
            TikTokControl::ComposerShutter,
            TikTokControl::PickerTabPhotos,
        ] {
            assert!(
                REQUIRED.contains(&anchor),
                "{anchor:?} anchors a coordinate tap and must be required"
            );
        }
    }

    // ------------------------------------------------- refusing before the first tap

    /// **A build with no measured Post button never opens the composer at all.**
    ///
    /// The module's whole promise, and the thing an earlier version got wrong: it drove the
    /// phone through the composer, the album and every grid tap, and only then discovered it
    /// could not publish — leaving a phone parked in an armed composer for a condition that
    /// was knowable before anything opened. This drives the **whole** entry point, not just
    /// `post`, because only that can see the difference.
    #[tokio::test(start_paused = true)]
    async fn publishing_on_a_build_without_a_post_button_taps_nothing_whatsoever() {
        let session = FakeSession::full_walk("riviu-abc");
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                measuring_plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::PostUnmeasured
        );
        assert!(
            session.taps.lock().is_empty(),
            "refusing to publish must not tap anything at all — not even Create"
        );
        assert_eq!(session.on_screen(), 0, "the phone never left the feed");
    }

    /// The measuring entry point *does* drive, and stops on the edit step having published
    /// nothing.
    ///
    /// The other half of the split: without it, closing the `post_button` gap would mean
    /// either weakening the gate or driving the phone by hand.
    #[tokio::test(start_paused = true)]
    async fn the_measuring_entry_point_reaches_the_edit_step_and_stops_there() {
        let session = FakeSession::full_walk("riviu-abc");
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            drive_to_edit_step(
                &session,
                measuring_plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::Stopped
        );
        assert!(
            !session.taps.lock().is_empty(),
            "the measuring path is supposed to drive the phone"
        );
    }

    /// **The fleet's real labels walk to the edit step and STOP, tail measured or not.**
    ///
    /// Every other walk test runs on a fixture set. This one runs on
    /// `com.ss.android.ugc.trill` 38.3.2 exactly as catalogued — sixteen of the twenty
    /// phones — and it is the case the measuring trip actually takes.
    ///
    /// It began life proving the walk was possible with `ComposerNext` unmeasured (arrival
    /// was proved by the picker going away). Since 30/08/2026 the whole tail is measured, so
    /// the property it pins hardened: a plan that **can** publish, driven by the measuring
    /// entry point, still stops on the edit step — the marker is awaited, never tapped. The
    /// on-screen assertion at the bottom is what holds that: one tap past the stop would
    /// leave the fake on a later scene.
    #[tokio::test(start_paused = true)]
    async fn the_fleets_own_labels_reach_the_edit_step_and_stop_there() {
        let controls = controls_for("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("the fleet's build is catalogued");
        let plan = ComposerPlan::resolve(&controls).expect("the walk is reachable");
        assert!(
            plan.can_publish(),
            "the whole tail is measured now; see §9.132 — and the walk below must stop anyway"
        );

        // Scenes keyed by what the **catalogue** says, not by fixture strings — and carrying
        // the album name in `text`, because that is where the measured picker puts it and
        // nothing in it has a `content-desc` at all.
        let picker_real = |album: &str, exit: Option<&str>| {
            scene(
                vec![
                    ("Select multiple", box_at(126.0, 1937.0)),
                    ("Photos", labelled("Photos", 824.0, 255.0, 152.0, 57.0)),
                    (
                        ":id/snr",
                        ElementBox {
                            description: None,
                            ..labelled("", 483.0, 115.0, 60.0, 57.0)
                        },
                    ),
                    (
                        "Next",
                        ElementBox {
                            clickable: true,
                            ..box_at(552.0, 1896.0)
                        },
                    ),
                ],
                exit,
            )
            .texted(":id/snr", album)
        };
        let shutter = labelled("Record video", 375.0, 1545.0, 330.0, 330.0);
        let entry = GalleryEntry::beside_shutter(screen(), &shutter)
            .expect("the measured entry is on screen")
            .rect();
        let session = FakeSession::with(vec![
            scene(
                vec![("Create", labelled("Create", 432.0, 1929.0, 216.0, 147.0))],
                Some("Create"),
            ),
            // Only a tap inside the gallery entry leaves the camera.
            scene(vec![("Record video", shutter)], None).leaving_by(entry),
            picker_real("All", Some(":id/snr")),
            picker_real("All", None).leaving_by(box_at(0.0, 400.0)),
            picker_real("riviu-abc", Some("Select multiple")),
            picker_real("riviu-abc", Some("Next")),
            // The edit step, carrying exactly what the measured screen carries: its own
            // `Next` (`:id/kl7`, whose only text child reads `Next` — measured 30/08/2026).
            // With `composer_next` in the catalogue, `advance_to_edit_step` proves arrival
            // by this marker *appearing* — and the measuring walk still never taps it, which
            // is what the on-screen assertion below holds the fake to.
            scene(
                vec![(
                    "Next",
                    ElementBox {
                        clickable: true,
                        ..box_at(545.0, 1954.0)
                    },
                )],
                None,
            ),
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0)]);

        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        let mut composer = Composer::new(&session, plan, |element: &ElementBox| element.centre());
        assert_eq!(
            reach_edit_step(&mut composer, &request, &stop)
                .await
                .expect("no transport error"),
            ComposerVerdict::Stopped,
            "the measuring walk must reach the edit step and stop there"
        );
        assert_eq!(
            session.on_screen(),
            6,
            "the phone did not end on the edit step"
        );
    }

    // ----------------------------------------------------------------- the walk

    /// The whole happy path, ending `Posted` because the feed came back.
    #[tokio::test(start_paused = true)]
    async fn a_full_walk_ends_posted_when_the_feed_returns() {
        let session = FakeSession::full_walk("riviu-abc");
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::Posted
        );
    }

    /// **The edit step's Next is actually tapped.**
    ///
    /// It used to be required, waited for, and never pressed — so a fully measured build
    /// would have sat on the edit screen looking for a Post button that lives on the next
    /// one, timed out, and reported `NoPostButton` on a run where everything worked. The
    /// fixture puts the post screen one tap past the edit step, so only a real tap reaches it.
    #[tokio::test(start_paused = true)]
    async fn the_edit_steps_next_is_pressed_rather_than_only_waited_for() {
        let session = FakeSession::full_walk("riviu-abc");
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        let verdict = publish_carousel(
            &session,
            plan(),
            |element: &ElementBox| element.centre(),
            &request,
            &stop,
        )
        .await
        .expect("no transport error");
        assert_ne!(
            verdict,
            ComposerVerdict::NoPostButton,
            "the run stalled on the edit screen, which is what forgetting to tap looks like"
        );
        assert_eq!(verdict, ComposerVerdict::Posted);
    }

    /// A composer that never opens is named, and **nothing further is tapped**.
    ///
    /// The tap assertion is the point. Without it the test passes just as well against a
    /// version that goes on to tap the gallery-entry rectangle on the feed, which is where
    /// the action rail is.
    #[tokio::test(start_paused = true)]
    async fn a_composer_that_does_not_open_is_named_and_stops_the_walk() {
        // One screen, and tapping does not leave it: the Create tap is accepted and ignored.
        let session = FakeSession {
            stuck_at: Some(0),
            ..FakeSession::with(vec![feed()])
        };
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::ComposerDidNotOpen
        );
        assert_eq!(
            session.taps.lock().len(),
            1,
            "only Create should have been tapped; a second tap is the blind one on the feed"
        );
    }

    // -------------------------------------------------------------- the album

    /// **Two albums matching the campaign's name is a refusal, not a coin flip.**
    #[tokio::test(start_paused = true)]
    async fn an_ambiguous_album_name_refuses_rather_than_taking_the_first() {
        let session = FakeSession::with(vec![
            picker("All", Some("fixture-album-menu")),
            picker("All", None).leaving_by(box_at(0.0, 400.0)),
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0), box_at(0.0, 500.0)]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .select_album("riviu-abc", &stop)
                .await
                .expect("no error"),
            AlbumChoice::NotFound
        );
        assert_eq!(
            session.taps.lock().len(),
            1,
            "the menu opened and no row was chosen"
        );
    }

    /// **The album row being tapped is not the album being selected.**
    ///
    /// A row matched in one snapshot; the list reflows while thumbnails load; the coordinate
    /// tap lands on the neighbour. Everything after that looks normal — the picker renders,
    /// cells select, `Next` arms — and the carousel publishes **another album's images**.
    /// Only the pill can tell, so only the pill is believed. Here the pill never changes.
    #[tokio::test(start_paused = true)]
    async fn an_album_tap_that_did_not_take_is_caught_by_the_pill() {
        let session = FakeSession::with(vec![
            picker("All", Some("fixture-album-menu")),
            picker("All", None),
            picker("All", None),
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0)]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .select_album("riviu-abc", &stop)
                .await
                .expect("no error"),
            AlbumChoice::NotConfirmed,
            "the pill still reads `All`, so the grid below is not the campaign's"
        );
    }

    /// And the whole walk refuses on it rather than publishing whatever the grid holds.
    #[tokio::test(start_paused = true)]
    async fn a_walk_whose_album_did_not_take_publishes_nothing() {
        let session = FakeSession::with(vec![
            feed(),
            camera(),
            picker("All", Some("fixture-album-menu")),
            picker("All", None).leaving_by(box_at(0.0, 400.0)),
            // The album tap did not take: the pill still reads `All`.
            picker("All", None),
            picker("All", None),
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0)]);
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::AlbumNotConfirmed
        );
    }

    /// One match is taken, and it is the row rather than the menu.
    #[tokio::test(start_paused = true)]
    async fn the_campaigns_own_album_is_chosen_when_it_is_unambiguous() {
        let row = box_at(0.0, 400.0);
        let session = FakeSession::with(vec![
            picker("All", Some("fixture-album-menu")),
            picker("riviu-abc", None).leaving_by(row.clone()),
        ])
        .rows("riviu-abc", vec![row.clone()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .select_album("riviu-abc", &stop)
                .await
                .expect("no error"),
            AlbumChoice::Confirmed
        );
        let taps = session.taps.lock();
        assert_eq!(taps.len(), 2);
        assert_eq!(
            (taps[1].x, taps[1].y),
            (row.centre().x, row.centre().y),
            "the row was tapped, not the menu again"
        );
    }

    // --------------------------------------------------------------- selection

    /// **`Next` arms on `clickable`, and a version reading `enabled` would fire early.**
    ///
    /// The fixture is exactly the measured state: `enabled` true throughout, `clickable`
    /// false with nothing selected.
    #[tokio::test(start_paused = true)]
    async fn nothing_selected_is_not_armed_even_though_enabled_says_true() {
        let unarmed = ElementBox {
            enabled: true,
            clickable: false,
            ..box_at(552.0, 1896.0)
        };
        let session = FakeSession::with(vec![scene(vec![("fixture-picker-next", unarmed)], None)]);
        let composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert!(
            composer
                .await_armed(&stop)
                .await
                .expect("no error")
                .is_none(),
            "an unarmed Next was read as armed; `enabled` is not the flag on this build"
        );

        let armed = ElementBox {
            enabled: true,
            clickable: true,
            ..box_at(552.0, 1896.0)
        };
        let session = FakeSession::with(vec![scene(vec![("fixture-picker-next", armed)], None)]);
        let composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        assert!(composer
            .await_armed(&stop)
            .await
            .expect("no error")
            .is_some());
    }

    /// Asking for more images than fit refuses instead of tapping what it can.
    #[tokio::test(start_paused = true)]
    async fn a_carousel_bigger_than_the_visible_grid_is_refused_before_any_tap() {
        let session = FakeSession::with(vec![scene(vec![], None)]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let grid = PhotoGrid::below_tabs(screen(), 312.0).expect("a sane anchor");
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer.select(&grid, 13, &stop).await.expect("no error"),
            Selection::MoreCellsThanTheGridShows
        );
        assert_eq!(
            composer.select(&grid, 0, &stop).await.expect("no error"),
            Selection::MoreCellsThanTheGridShows
        );
        assert!(session.taps.lock().is_empty(), "refusing must not tap");
    }

    /// **Asking the run to stop must not make it tap faster.**
    ///
    /// The stop flag used to reach only the inter-tap sleeps, so setting it removed the pacing
    /// and sent every remaining cell tap back to back — the opposite of stopping.
    #[tokio::test(start_paused = true)]
    async fn a_stop_during_selection_stops_the_taps_rather_than_hurrying_them() {
        let session = FakeSession::with(vec![picker("riviu-abc", None)]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let grid = PhotoGrid::below_tabs(screen(), 312.0).expect("a sane anchor");
        let stop = AtomicBool::new(true);
        assert_eq!(
            composer.select(&grid, 9, &stop).await.expect("no error"),
            Selection::Stopped
        );
        assert!(
            session.taps.lock().is_empty(),
            "an already-set stop sent {} taps",
            session.taps.lock().len()
        );
    }

    /// **The first cell is its own proof, and its failure stops the walk before more taps.**
    ///
    /// The fixture is the failure §9.132 measured on a real phone, two of four walks: the
    /// `Select multiple` tap does not take (the picker stays), and the first cell tap then
    /// leaves for the single-photo **editor** — a screen that also renders a `Next`, on a
    /// node that is not clickable, exactly like the real `id/kl_`. The old shape tapped the
    /// remaining cells blind onto that editor and reported `NeverArmed` only at the end;
    /// this pins that exactly one grid tap is sent and the verdict names the toggle.
    #[tokio::test(start_paused = true)]
    async fn a_toggle_that_did_not_take_stops_after_the_first_cell() {
        // Everything below the tab row navigates (the grid); the toggle at y=1937 does not.
        let grid_region = ElementBox {
            description: None,
            enabled: true,
            clickable: false,
            x: 0.0,
            y: 320.0,
            width: 1080.0,
            height: 1500.0,
        };
        let single_editor = scene(
            vec![
                (
                    "fixture-picker-next",
                    ElementBox {
                        clickable: false,
                        ..box_at(755.0, 1985.0)
                    },
                ),
                ("fixture-edit-next", box_at(545.0, 1954.0)),
            ],
            None,
        );
        let session = FakeSession::with(vec![
            feed(),
            camera(),
            picker("All", Some("fixture-album-menu")),
            picker("All", None).leaving_by(box_at(0.0, 400.0)),
            // The toggle tap lands and does nothing: this scene leaves only through the
            // grid region, which is what the first cell tap does — into the editor.
            picker("riviu-abc", None).leaving_by(grid_region),
            single_editor,
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0)]);
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        assert_eq!(
            reach_edit_step(&mut composer, &request, &stop)
                .await
                .expect("no transport error"),
            ComposerVerdict::MultiSelectDidNotEngage
        );
        // Counted as the whole journey, because a geometric filter also catches the album
        // row and the gallery entry: Create, the entry, the album menu, the album row, the
        // toggle, and exactly ONE cell — six. The old shape sent eight: two more "cells"
        // straight into the editor.
        assert_eq!(
            session.taps.lock().len(),
            6,
            "exactly one cell tap may be spent proving the toggle; the rest were landing on \
             an editor before this verdict existed"
        );
    }

    /// **A stop must never turn into a tap on Post.**
    ///
    /// `await_condition` used to check for a ready element *before* checking stop, so a Post
    /// button that happened to be rendered won over an already-set flag and was handed back
    /// to be tapped. The one place in this project where losing that race publishes.
    #[tokio::test(start_paused = true)]
    async fn a_stop_set_while_the_post_button_is_on_screen_does_not_publish() {
        let session = FakeSession::with(vec![post_screen()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(true);
        assert_eq!(
            composer.post(&stop).await.expect("no error"),
            ComposerVerdict::Stopped
        );
        assert!(session.taps.lock().is_empty(), "a stopped run tapped Post");
    }

    /// **A stop set mid-walk stops the walk, not just the pacing.**
    ///
    /// `await_condition` is where every step waits, and it used to look for its element
    /// *before* looking at the stop flag — so a control that happened to be rendered won the
    /// race against an already-set stop and was handed straight back to be tapped. `post` has
    /// its own guards, so the Post button was covered; every other step was not, and this is
    /// the one that walks toward it.
    ///
    /// The reversal that found this gap: deleting the stop check from `await_condition` left
    /// every test green.
    #[tokio::test(start_paused = true)]
    async fn a_stop_set_between_screens_does_not_advance_toward_the_post_screen() {
        let session = FakeSession::with(vec![edit_step(), post_screen()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(true);
        assert!(
            !composer
                .advance_to_post_screen(&stop)
                .await
                .expect("no error"),
            "a stopped run advanced to the screen that has the Post button on it"
        );
        assert!(
            session.taps.lock().is_empty(),
            "a stopped run tapped the edit step's Next"
        );
        assert_eq!(session.on_screen(), 0, "the phone moved after a stop");
    }

    // ------------------------------------------------------------------- post

    /// **A transport failure after the Post tap is `PostNotConfirmed`, never an `Err`.**
    ///
    /// Once the tap is handed to the agent the outcome is unknown, not failed. An `Err` here
    /// is indistinguishable from any other transport error, and a caller that retries those
    /// publishes a duplicate that cannot be taken down.
    #[tokio::test(start_paused = true)]
    async fn a_dead_link_at_the_moment_of_posting_is_unconfirmed_rather_than_an_error() {
        let session = FakeSession {
            fail_taps_after: Some(0),
            ..FakeSession::with(vec![post_screen()])
        };
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        let verdict = composer.post(&stop).await.expect("must not be an Err");
        assert_eq!(verdict, ComposerVerdict::PostNotConfirmed);
        assert!(
            !verdict.may_retry(),
            "an unknown outcome must not be retried"
        );
    }

    /// **A screen that is not the feed is not a published post.**
    ///
    /// The old check was "the Post label is gone", which reads an error dialog or an account
    /// check as success. The signal is the bottom tab bar returning.
    #[tokio::test(start_paused = true)]
    async fn a_post_that_lands_on_some_other_screen_is_not_reported_as_published() {
        // Tapping Post leads to a screen carrying neither Post nor the tab bar.
        let session = FakeSession::with(vec![
            post_screen(),
            scene(vec![("something-else", box_at(0.0, 0.0))], None),
        ]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer.post(&stop).await.expect("no error"),
            ComposerVerdict::PostNotConfirmed
        );
    }

    /// And the feed coming back **is** the confirmation.
    #[tokio::test(start_paused = true)]
    async fn the_feed_coming_back_is_what_proves_the_post_went() {
        let session = FakeSession::with(vec![post_screen(), feed()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer.post(&stop).await.expect("no error"),
            ComposerVerdict::Posted
        );
    }

    /// **A measured edit-step control is used even when the build cannot publish.**
    ///
    /// Arrival at the edit step and permission to publish are different questions, and keying
    /// the first on the second threw away the strong evidence on exactly the build a measuring
    /// run is made on: `ComposerNext` measured, Post button not. The weak proof — the picker
    /// disappearing — then accepted an error screen, a permission modal, or the feed as "the
    /// edit step", and the measurement written from that dump would be of the wrong screen.
    #[tokio::test(start_paused = true)]
    async fn a_measured_edit_control_is_believed_even_on_a_build_that_cannot_publish() {
        let plan = measuring_plan();
        assert!(!plan.can_publish(), "this test is about that being false");

        // The picker goes away and the edit step never arrives.
        let session = FakeSession::with(vec![
            picker("riviu-abc", Some("fixture-picker-next")),
            scene(vec![("something-else", box_at(0.0, 0.0))], None),
        ]);
        let mut composer = Composer::new(&session, plan, |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        let next = ElementBox {
            clickable: true,
            ..box_at(552.0, 1896.0)
        };
        assert!(
            !composer
                .advance_to_edit_step(&next, &stop)
                .await
                .expect("no error"),
            "the picker merely disappearing was accepted as arrival at the edit step"
        );
    }

    // ---------------------------------------------------------------- caption

    /// **A caption that silently did not take must not become a published post.**
    ///
    /// `type_text` writes into whichever field has focus, and the post screen has more than
    /// one — the comment drawer's version of this mistake wrote into the collapsed bar behind
    /// the real field, succeeding at the API level while the screen stayed empty. Here that
    /// publishes a carousel with no words on it, and there is no delete on Android.
    ///
    /// The fixture is the realistic cause: a catalogue entry naming the field's *placeholder*,
    /// which stops matching the moment a character arrives.
    #[tokio::test(start_paused = true)]
    async fn a_caption_that_cannot_be_read_back_stops_the_run_before_post() {
        let session = FakeSession {
            caption_never_takes: true,
            ..FakeSession::full_walk("riviu-abc")
        };
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::CaptionNotConfirmed
        );
        // The Post button's own centre, not a y threshold: the composer tab sits at y=2002 on
        // this fixture, so a threshold catches the wrong tap and passes for the wrong reason.
        let post = box_at(900.0, 2000.0).centre();
        assert!(
            !session
                .taps
                .lock()
                .iter()
                .any(|tap| (tap.x, tap.y) == (post.x, post.y)),
            "Post was tapped with an unconfirmed caption"
        );
    }

    /// **A field still showing its placeholder is not a typed caption.**
    ///
    /// The likely cause of a failed write, and the readback used to accept it: the check was
    /// `contains(first 24 characters)` across *any* matching node, so a placeholder reading
    /// `Describe your post…` confirmed a caption that begins with those words. The write had
    /// not landed and the post went out blank.
    #[tokio::test(start_paused = true)]
    async fn a_placeholder_that_survives_the_write_is_not_mistaken_for_the_caption() {
        let session = FakeSession {
            caption_never_takes: true,
            caption_placeholder: Some("Describe your post to viewers".into()),
            ..FakeSession::with(vec![post_screen()])
        };
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .type_caption("Describe your post to viewers: ngày ra mắt", &stop)
                .await
                .expect("no error"),
            CaptionOutcome::NotConfirmed,
            "the placeholder contains the caption's opening words and was accepted"
        );
    }

    /// **A field that kept only part of the caption is not a typed caption either.**
    ///
    /// The other half of the same defect: a prefix check accepts truncation, and a truncated
    /// caption on a live post cannot be edited from here.
    #[tokio::test(start_paused = true)]
    async fn a_truncated_caption_is_refused_rather_than_published() {
        let session = FakeSession {
            caption_never_takes: true,
            caption_placeholder: Some("đi Đà Lạt thật đã, lưu lại".into()),
            ..FakeSession::with(vec![post_screen()])
        };
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .type_caption("đi Đà Lạt thật đã, lưu lại thôi! #dalat #review", &stop)
                .await
                .expect("no error"),
            CaptionOutcome::NotConfirmed
        );
    }

    /// **A field holding more than the caption is not the caption.**
    ///
    /// The prefix check accepted it: a leftover draft, or a field that appends its own
    /// footer, contains the requested text and was confirmed. The published caption is then
    /// something the operator never wrote.
    #[tokio::test(start_paused = true)]
    async fn a_field_holding_more_than_the_caption_is_refused() {
        let session = FakeSession {
            caption_never_takes: true,
            caption_placeholder: Some(
                "đi Đà Lạt thật đã, lưu lại thôi! #dalat — bản nháp cũ".into(),
            ),
            ..FakeSession::with(vec![post_screen()])
        };
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .type_caption("đi Đà Lạt thật đã, lưu lại thôi! #dalat", &stop)
                .await
                .expect("no error"),
            CaptionOutcome::NotConfirmed,
            "the field contains the caption and is not the caption"
        );
    }

    /// **Two nodes matching the caption locator is a refusal, not a coin flip.**
    ///
    /// The focus tap uses the first match and the readback used any match, so a non-unique
    /// locator could type into one node and confirm another — publishing whatever the first
    /// node actually holds.
    #[tokio::test(start_paused = true)]
    async fn a_caption_locator_that_matches_twice_is_refused() {
        let session = FakeSession {
            caption_has_a_twin: true,
            ..FakeSession::with(vec![post_screen()])
        };
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .type_caption("đi Đà Lạt thật đã", &stop)
                .await
                .expect("no error"),
            CaptionOutcome::NotConfirmed,
            "one of two matching nodes was accepted as the caption field"
        );
    }

    /// The caption is typed into the field and read back through the same locator.
    #[tokio::test(start_paused = true)]
    async fn the_caption_reaches_the_field_and_is_proved_there() {
        let session = FakeSession::with(vec![post_screen()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer
                .type_caption("đi Đà Lạt thật đã #dalat", &stop)
                .await
                .expect("no error"),
            CaptionOutcome::Typed
        );
        assert_eq!(
            session.typed.lock().as_deref(),
            Some("đi Đà Lạt thật đã #dalat"),
            "the caption must go in verbatim — the hashtag is part of what the operator wrote"
        );
        assert!(
            !session.taps.lock().is_empty(),
            "the field must be focused before typing, or the text lands elsewhere"
        );
    }

    /// An empty caption file types nothing and is not a failure.
    #[tokio::test(start_paused = true)]
    async fn an_empty_caption_types_nothing_and_lets_the_post_through() {
        let session = FakeSession::with(vec![post_screen()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer.type_caption("   ", &stop).await.expect("no error"),
            CaptionOutcome::NothingToSay
        );
        assert!(
            session.typed.lock().is_none(),
            "nothing should have been typed"
        );
        assert!(
            session.taps.lock().is_empty(),
            "nothing should have been tapped"
        );
    }

    /// **A build with a Post button and no caption field cannot publish.**
    ///
    /// The state that could put a bare carousel on an account, and the test that claimed to
    /// cover it used the *no-Post* fixture — its assertion checked `PostButton`, so relaxing
    /// `resolve` to make the caption optional would have left the suite green.
    #[test]
    fn a_build_with_a_post_button_but_no_caption_field_cannot_publish() {
        let controls = every_publish_control_but_caption_measured();
        assert_eq!(
            ComposerPlan::missing_to_publish(&controls),
            vec![TikTokControl::ComposerCaption],
            "this fixture exists to be missing exactly the caption"
        );
        assert!(
            controls.label(TikTokControl::PostButton).is_some(),
            "and to have the Post button, or it proves nothing"
        );
        let plan =
            ComposerPlan::resolve(&controls).expect("everything up to the edit step is measured");
        assert!(
            !plan.can_publish(),
            "a build that cannot type a caption must not be able to publish"
        );
        assert!(plan.post_button().is_none());
    }

    /// And the no-Post build is still refused, for its own reason.
    #[test]
    fn a_build_without_a_measured_post_button_cannot_publish_either() {
        let controls = every_publish_control_but_post_measured();
        assert!(!ComposerPlan::resolve(&controls)
            .expect("the pre-publish path is measured")
            .can_publish());
        assert!(ComposerPlan::missing_to_publish(&controls).contains(&TikTokControl::PostButton));
    }

    /// **The picker's own count is believed when it states one.**
    ///
    /// `Next` arming proves at least one cell took and nothing about the rest; a build that
    /// also renders the number is the only chance to prove the count, and taking it is the
    /// difference between publishing five images and publishing whichever taps landed.
    #[tokio::test(start_paused = true)]
    async fn a_stated_count_that_disagrees_stops_the_run() {
        // The picker says two are selected; the run asked for three.
        let session = FakeSession::with(vec![
            feed(),
            camera(),
            picker("All", Some("fixture-album-menu")),
            picker("All", None).leaving_by(box_at(0.0, 400.0)),
            picker("riviu-abc", Some("fixture-multi-select")),
            picker("riviu-abc", Some("fixture-picker-next"))
                .texted("fixture-picker-next", "Next (2)"),
            edit_step(),
            post_screen(),
            feed(),
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0)]);
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::NotEnoughSelected
        );
    }

    /// A build that states no count runs on the taps it sent, which is the measured case.
    #[tokio::test(start_paused = true)]
    async fn a_build_that_states_no_count_still_publishes() {
        let session = FakeSession::full_walk("riviu-abc");
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::Posted
        );
    }

    /// **One dropped hierarchy read must not shorten the post-confirmation window.**
    ///
    /// After the Post tap, "I could not read the screen" and "the post did not go" are
    /// different facts and only the second is worth reporting. Propagating the first error
    /// turned a live post into `PostNotConfirmed` — permanently unclaimable — on the first
    /// poll of a twenty-second wait.
    #[tokio::test(start_paused = true)]
    async fn a_transient_read_after_the_post_tap_does_not_end_the_wait() {
        // Call 0 finds the Post button; call 1 is the first confirmation read, which is the
        // one that must survive a drop.
        let session = FakeSession {
            locate_fails_at: Some(1),
            ..FakeSession::with(vec![post_screen(), feed()])
        };
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        // The first read fails; the Post button is found on the retry, tapped, and the feed
        // comes back.
        assert_eq!(
            composer.post(&stop).await.expect("no error"),
            ComposerVerdict::Posted
        );
    }

    /// **An unconfirmed post is left on screen, not backed out of.**
    ///
    /// The verdict means an unrecognised screen is up after the Post tap, and the one this
    /// build is expected to raise is the confirmation sheet that *commits* the post. Pressing
    /// Back there cancels it — and the assignment is still permanently unclaimable, so the run
    /// throws away a post it could have had.
    #[tokio::test(start_paused = true)]
    async fn an_unconfirmed_post_is_not_dismissed_on_the_way_out() {
        let session = FakeSession::with(vec![
            feed(),
            camera(),
            picker("All", Some("fixture-album-menu")),
            picker("All", None).leaving_by(box_at(0.0, 400.0)),
            picker("riviu-abc", Some("fixture-multi-select")),
            picker("riviu-abc", Some("fixture-picker-next")),
            edit_step(),
            post_screen(),
            // Whatever is up after Post is not the feed.
            scene(vec![("an-unmeasured-sheet", box_at(0.0, 0.0))], None),
        ])
        .rows("riviu-abc", vec![box_at(0.0, 400.0)]);
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        assert_eq!(
            publish_carousel(
                &session,
                plan(),
                |element: &ElementBox| element.centre(),
                &request,
                &stop
            )
            .await
            .expect("no transport error"),
            ComposerVerdict::PostNotConfirmed
        );
        assert_eq!(
            *session.backs.lock(),
            0,
            "Back was pressed on a screen that may be the one committing the post"
        );
    }

    // ------------------------------------------------------------------ leave

    /// **The composer is closed behind every exit, and the phone reaches the feed.**
    ///
    /// Asserting `backs > 0` was not enough: it stayed green against a `leave` that pressed
    /// Back once from four screens deep. This checks where the phone **ended**.
    #[tokio::test(start_paused = true)]
    async fn a_transport_error_deep_in_the_walk_still_gets_the_phone_back_to_the_feed() {
        let session = FakeSession {
            // Feed, camera, picker — then the grid tap dies.
            fail_taps_after: Some(4),
            ..FakeSession::full_walk("riviu-abc")
        };
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            caption: "đi Đà Lạt thật đã",
            screen: screen(),
        };
        let stop = AtomicBool::new(false);
        let outcome = publish_carousel(
            &session,
            plan(),
            |element: &ElementBox| element.centre(),
            &request,
            &stop,
        )
        .await;
        assert!(outcome.is_err(), "the tap failure must reach the caller");
        assert_eq!(
            session.on_screen(),
            0,
            "the run walked away leaving the phone {} screens inside the composer",
            session.on_screen()
        );
    }

    /// **One failed Back does not end the attempt.**
    ///
    /// It used to: `leave` returned on the first error, leaving the composer open on the
    /// strength of a single dropped request. The budget is spent on attempts, so a flaky link
    /// gets its retries — and a dead one still terminates rather than looping.
    #[tokio::test(start_paused = true)]
    async fn a_back_that_fails_is_retried_and_a_dead_link_still_terminates() {
        let session = FakeSession {
            backs_fail: true,
            ..FakeSession::with(vec![feed(), camera(), picker("All", None)])
        };
        *session.at.lock() = 2;
        let composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        assert!(!composer.leave().await, "a dead link cannot reach the feed");
        assert!(
            *session.backs.lock() > 1,
            "gave up after {} Back(s); one dropped request must not end the attempt",
            session.backs.lock()
        );
    }

    // ---------------------------------------------------------------- geometry

    /// The grid reproduces what was read off the phone, to the pixel.
    #[test]
    fn the_photo_grid_lands_on_the_measured_cells() {
        // Tabs measured at `[824,255][976,312]`, so their bottom edge is 312.
        let grid = PhotoGrid::below_tabs(screen(), 312.0).expect("a sane anchor");
        let cell = |index: usize| {
            let cell = grid.cell(index).expect("inside the visible grid");
            (
                cell.x.round(),
                cell.y.round(),
                cell.width.round(),
                cell.height.round(),
            )
        };
        assert_eq!(cell(0), (6.0, 357.0, 352.0, 356.0));
        assert_eq!(cell(1), (364.0, 357.0, 352.0, 356.0));
        assert_eq!(cell(2), (722.0, 357.0, 352.0, 356.0));
        assert_eq!(cell(3).1, 719.0);
        assert_eq!(cell(6).1, 1081.0);
        assert_eq!(cell(11), (722.0, 1443.0, 352.0, 356.0));
    }

    /// **A cell that would fall off the screen is not a cell.**
    ///
    /// The bound used to be the index alone, so the doc's promise — `None` when the cell is
    /// off screen — was not enforced at all: a grid anchored low on a short phone happily
    /// returned rectangles past the bottom edge, and a tap there lands on whatever the OS
    /// clamps it to. Earlier taps can already have armed `Next`, so nothing downstream
    /// notices.
    #[test]
    fn the_grid_stops_at_the_screen_rather_than_at_the_index() {
        let full = PhotoGrid::below_tabs(screen(), 312.0).expect("a sane anchor");
        assert_eq!(full.capacity(), 12);
        assert!(full.cell(11).is_some());
        assert!(full.cell(12).is_none());

        // The same 1080-wide layout anchored far down the screen: the last rows do not fit.
        let low = PhotoGrid::below_tabs(screen(), 1000.0).expect("a sane anchor");
        assert!(
            low.capacity() < 12,
            "a grid anchored at y=1000 on a 2220-high screen claims all 12 cells fit"
        );
        for index in low.capacity()..12 {
            let cell = low.cell(index);
            assert!(
                cell.is_none(),
                "cell {index} is off screen and was still offered: {cell:?}"
            );
        }
    }

    /// **An anchor that is not a rectangle on this screen is refused.**
    ///
    /// Validating only the *derived* rectangle let a shutter reported at `height = -100`
    /// produce a plausible, fully on-screen tap point — arithmetic from nonsense, which is the
    /// thing anchoring was supposed to replace.
    #[test]
    fn a_located_anchor_that_is_not_a_rectangle_anchors_nothing() {
        for broken in [
            ElementBox {
                height: -100.0,
                ..labelled("Record video", 375.0, 1000.0, 330.0, 330.0)
            },
            ElementBox {
                width: 0.0,
                ..labelled("Record video", 375.0, 1545.0, 330.0, 330.0)
            },
            // Off the bottom of the screen, but its centre still computes.
            labelled("Record video", 375.0, 3000.0, 330.0, 330.0),
            ElementBox {
                x: -50.0,
                ..labelled("Record video", 0.0, 1545.0, 330.0, 330.0)
            },
        ] {
            assert!(
                GalleryEntry::beside_shutter(screen(), &broken).is_none(),
                "anchored on {broken:?}"
            );
        }
        // The measured one still works, or the rule above is refusing everything.
        assert!(GalleryEntry::beside_shutter(
            screen(),
            &labelled("Record video", 375.0, 1545.0, 330.0, 330.0)
        )
        .is_some());
    }

    /// The same rule for the grid's anchor, through the step that locates it.
    #[tokio::test(start_paused = true)]
    async fn a_tab_row_that_is_not_a_rectangle_builds_no_grid() {
        let broken = ElementBox {
            height: -688.0,
            ..labelled("Photos", 824.0, 1000.0, 152.0, 57.0)
        };
        // `1000 + (-688)` is 312 — the exact anchor the measured screen has, from a node that
        // is not on it.
        let session = FakeSession::with(vec![scene(vec![("fixture-tab-photos", broken)], None)]);
        let composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert!(
            composer
                .grid(screen(), &stop)
                .await
                .expect("no error")
                .is_none(),
            "a tab row with a negative height anchored the grid anyway"
        );
    }

    /// A nonsense anchor is refused rather than turned into coordinates.
    #[test]
    fn an_anchor_that_is_not_on_the_screen_builds_no_grid() {
        assert!(PhotoGrid::below_tabs(screen(), -1.0).is_none());
        assert!(PhotoGrid::below_tabs(screen(), 2220.0).is_none());
        assert!(PhotoGrid::below_tabs(screen(), f64::NAN).is_none());
        assert!(Screen::new(f64::NAN, 2220.0).is_none());
        assert!(Screen::new(0.0, 0.0).is_none());
        assert!(Screen::new(1080.0, f64::INFINITY).is_none());
    }

    /// **The gallery entry is on the right of the shutter, and shares its vertical centre.**
    ///
    /// Pinned because the previous note put it bottom-*left*, which on this build is the
    /// effects panel. A regression here does not fail — it opens a different feature and the
    /// run reports "the picker did not open".
    #[test]
    fn the_gallery_entry_sits_where_the_screenshot_showed_it() {
        let shutter = labelled("Record video", 375.0, 1545.0, 330.0, 330.0);
        let entry = GalleryEntry::beside_shutter(screen(), &shutter)
            .expect("on screen")
            .rect();
        assert_eq!(
            (entry.x.round(), entry.y.round(), entry.width.round()),
            (765.0, 1590.0, 240.0)
        );
        assert!(
            entry.centre().x > shutter.centre().x,
            "the entry must be to the RIGHT of the shutter; to its left is the effects panel"
        );
        assert_eq!(
            entry.centre().y,
            shutter.centre().y,
            "measured: same centre"
        );
    }

    /// An entry that would land off screen is refused rather than clamped.
    #[test]
    fn a_shutter_near_the_bottom_edge_yields_no_entry_at_all() {
        let low = labelled("Record video", 375.0, 2200.0, 330.0, 330.0);
        assert!(
            GalleryEntry::beside_shutter(screen(), &low).is_none(),
            "an off-screen entry must not be offered; the OS clamps the tap onto real controls"
        );
    }

    // ------------------------------------------------------------------ verdicts

    /// **An unconfirmed post is never retried, and neither is a successful one.**
    #[test]
    fn only_the_verdicts_that_published_nothing_may_be_dispatched_again() {
        assert!(!ComposerVerdict::Posted.may_retry());
        assert!(!ComposerVerdict::PostNotConfirmed.may_retry());
        for verdict in [
            ComposerVerdict::ComposerDidNotOpen,
            ComposerVerdict::NoShutterToAnchorTo,
            ComposerVerdict::PickerDidNotOpen,
            ComposerVerdict::AlbumNotFound,
            ComposerVerdict::AlbumNotConfirmed,
            ComposerVerdict::NoTabsToAnchorTo,
            ComposerVerdict::MoreCellsThanTheGridShows,
            ComposerVerdict::NeverArmed,
            ComposerVerdict::MultiSelectDidNotEngage,
            ComposerVerdict::EditStepDidNotOpen,
            ComposerVerdict::PostScreenDidNotOpen,
            ComposerVerdict::PostUnmeasured,
            ComposerVerdict::NoPostButton,
            // Both caption failures happen **before** the Post tap, so both stay retryable.
            // The table omitted them, and making them permanently unclaimable kept it green.
            ComposerVerdict::NoCaptionField,
            ComposerVerdict::CaptionNotConfirmed,
            ComposerVerdict::NotEnoughSelected,
            ComposerVerdict::NoShutterToAnchorTo,
            ComposerVerdict::NoTabsToAnchorTo,
            ComposerVerdict::Stopped,
        ] {
            assert!(
                verdict.may_retry(),
                "{verdict:?} published nothing and must stay retryable"
            );
            assert!(!verdict.reason().is_empty());
        }
        assert!(ComposerVerdict::Posted.is_posted());
        assert!(!ComposerVerdict::PostNotConfirmed.is_posted());
    }
}
