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
//! Everything the path needs is resolved **once, up front**, by
//! [`ComposerPlan::resolve`]. A [`Composer`] cannot be constructed any other way, so
//! "refuse before opening anything" is not a rule a later edit can forget to follow —
//! there is no code path that opens the composer without a complete plan in hand. The
//! catalogue's own note says it: *a post that went out and cannot be taken down is a
//! promise the session cannot keep*, so the refusal has to happen before anything is
//! published, and the only point where refusing is free is before the first tap.
//!
//! # What is measured, and what this therefore still refuses
//!
//! Measured 29/08/2026 on SM-G950F `98895a3355424e484f`, `com.ss.android.ugc.trill`
//! 38.3.2, `en-US`, 1080x2220 — the build sixteen of the twenty phones run:
//!
//! | | |
//! |---|---|
//! | [`TikTokControl::ComposerOpen`] | `Create`, and unique in all 184 elements |
//! | [`TikTokControl::PickerMultiSelect`] | `Select multiple` |
//! | [`TikTokControl::PickerNext`] | `Next`, armed via `clickable` — see below |
//! | [`TikTokControl::PickerAlbumMenu`] | **on screen, not locatable** — see below |
//! | [`TikTokControl::ComposerNext`] | never measured, on any build |
//! | [`TikTokControl::PostButton`] | never measured, on any build |
//!
//! So on today's fleet [`ComposerPlan::resolve`] refuses, and names the three controls
//! that are missing. That is the correct state of the world, not a gap to route around.
//!
//! # The album menu, and why "just use All" is not the shortcut it looks like
//!
//! The album pill reads the album *currently showing* — `All` now, and the campaign's
//! own `importId` once chosen — so a text locator names a value that changes the moment
//! it is used. Worse, `All` **also** belongs to the media-type tab one row below it, so
//! the string is ambiguous on screen before anything is selected at all.
//!
//! The tempting shortcut is to skip the album entirely: the campaign's images were
//! imported seconds ago, so they are the newest things in `All` and sit at the head of
//! the grid. That is true right up until the phone acquires one other image — a
//! screenshot, a chat photo, anything a background app saves — at which point the grid
//! shifts by one and the carousel goes out with a stranger's picture in it, published,
//! on a real account, with no delete. The album is what makes the selection *addressed*
//! rather than *guessed*, so it is required, and its absence is a refusal.
//!
//! # The armed flag is `clickable`, not `enabled`
//!
//! Measured both ways on the phone: `Next` reads `clickable=false enabled=true` with
//! nothing selected and `clickable=true enabled=true` with one image selected. So
//! `enabled` is constant across the transition and proves nothing here, while the
//! comment drawer's Send button on another build moves `enabled` and not this. Two
//! different ideas of "armed" in one app; this module asks for the one its own screen
//! was measured to move.

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
/// Longer than the composer's own window on purpose: the picker enumerates the
/// device's media store, and a phone this project has just pushed a carousel onto has
/// a store that was written to seconds ago.
pub const PICKER_WINDOW: Duration = Duration::from_millis(10_000);
/// How long `Next` may take to arm after the last cell is tapped.
pub const ARM_WINDOW: Duration = Duration::from_millis(4_000);
pub const POLL: Duration = Duration::from_millis(350);

/// What a publish attempt actually achieved, named for the step that failed.
///
/// Every variant except [`Self::Posted`] means **nothing was published**, with one
/// deliberate exception — [`Self::PostNotConfirmed`], which means a tap went out and
/// the result is unknown. That distinction is the same one
/// [`crate::db::Database::interrupt_orphaned_publish_campaigns`] draws between
/// `uncertain` and `failed_before_dispatch`, and for the same reason: one of them may
/// be live on a real account and must never be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerVerdict {
    /// Published, and the composer confirmed it by leaving the post screen.
    Posted,
    /// Post was tapped and the result could not be read back. **Never retried.**
    ///
    /// The carousel may be live. Retrying would publish a second copy, and there is no
    /// delete path on Android to undo either of them.
    PostNotConfirmed,
    /// The composer tab was tapped and the composer never came up.
    ComposerDidNotOpen,
    /// The gallery entry was tapped and the picker never came up.
    ///
    /// Also what a *mis-tap* looks like, which is why it is checked rather than
    /// assumed: the two circles left of the shutter open the effects panel, and on the
    /// build before this one they were the likeliest-looking candidates.
    PickerDidNotOpen,
    /// The album menu opened and the campaign's own album was not in it.
    ///
    /// Means the import did not land, or landed under another name. Refuses rather
    /// than falling back to `All` — see the module docs.
    AlbumNotFound,
    /// Fewer cells could be selected than the carousel needs.
    NotEnoughCells,
    /// The cells were tapped and `Next` never armed.
    ///
    /// The one verdict that proves the taps did **not** take: `clickable` is the
    /// hierarchy's own answer to "is anything selected", so this is evidence rather
    /// than a timeout.
    NeverArmed,
    /// `Next` was tapped and the edit step never appeared.
    EditStepDidNotOpen,
    /// This build's Post button has never been measured, so nothing was tapped.
    ///
    /// Distinct from [`Self::NoPostButton`], and the difference is who can fix it: this
    /// one is a gap in the catalogue that one dump on one phone closes, while the other
    /// means the control **was** measured and is not on the screen in front of us.
    PostUnmeasured,
    /// The edit step opened and the measured Post button was not on it.
    NoPostButton,
}

impl ComposerVerdict {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Posted => "đã đăng",
            Self::PostNotConfirmed => {
                "đã bấm Đăng nhưng không xác nhận được; KHÔNG đăng lại — bài có thể đã lên và \
                 Android không có đường xoá"
            }
            Self::ComposerDidNotOpen => "bấm nút Tạo mà composer không mở",
            Self::PickerDidNotOpen => "bấm ô thư viện mà picker không mở (có thể đã bấm nhầm)",
            Self::AlbumNotFound => "không thấy album của chiến dịch trong danh sách album",
            Self::NotEnoughCells => "không chọn đủ số ảnh mà bài cần",
            Self::NeverArmed => "đã bấm các ô ảnh nhưng nút Tiếp không sáng",
            Self::EditStepDidNotOpen => "bấm Tiếp mà bước chỉnh sửa không mở",
            Self::PostUnmeasured => "chưa đo nút Đăng trên bản build này — không bấm gì cả",
            Self::NoPostButton => "không thấy nút Đăng ở bước cuối",
        }
    }

    /// Whether the caller should treat this as "the carousel is on the account".
    pub fn is_posted(self) -> bool {
        self == Self::Posted
    }

    /// Whether the caller may dispatch this assignment again.
    ///
    /// **The one question this enum exists to answer.** `false` for
    /// [`Self::PostNotConfirmed`] as well as for [`Self::Posted`], because an
    /// unconfirmed post may be live and a second attempt would publish a duplicate
    /// that nothing here can take down.
    pub fn may_retry(self) -> bool {
        !matches!(self, Self::Posted | Self::PostNotConfirmed)
    }
}

/// The controls this path cannot run without.
///
/// [`TikTokControl::PickerTabAll`] and [`TikTokControl::PickerTabPhotos`] are
/// deliberately **not** here: the album menu addresses the campaign's own directory,
/// which holds nothing but its images, so filtering by media type inside it changes
/// nothing. Requiring a control the path does not use would refuse builds that can
/// publish perfectly well.
pub const REQUIRED: [TikTokControl; 5] = [
    TikTokControl::ComposerOpen,
    TikTokControl::PickerAlbumMenu,
    TikTokControl::PickerMultiSelect,
    TikTokControl::PickerNext,
    TikTokControl::ComposerNext,
];

/// Why this build cannot be driven, named control by control.
///
/// Carries the list rather than a bare `false` because the list is *actionable*: every
/// entry is one dump on one phone away from being closed, and an operator reading
/// "cannot publish on this build" with no names cannot act on it.
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
///
/// Holding the queries rather than the labels is the point: a [`Composer`] can only be
/// built from one of these, so there is no way to reach the first tap without having
/// already proved every later step is reachable. The [`TikTokControl::PostButton`] is
/// **not** in here — see [`ComposerPlan::post_button`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComposerPlan {
    open: ElementQuery<'static>,
    album_menu: ElementQuery<'static>,
    multi_select: ElementQuery<'static>,
    picker_next: ElementQuery<'static>,
    edit_next: ElementQuery<'static>,
    post_button: Option<ElementQuery<'static>>,
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
        Ok(Self {
            open: query(TikTokControl::ComposerOpen),
            album_menu: query(TikTokControl::PickerAlbumMenu),
            multi_select: query(TikTokControl::PickerMultiSelect),
            picker_next: query(TikTokControl::PickerNext),
            edit_next: query(TikTokControl::ComposerNext),
            post_button: labels
                .label(TikTokControl::PostButton)
                .map(|label| label.to_query()),
        })
    }

    /// The Post control, or `None` on a build where it has never been measured.
    ///
    /// **Not in [`REQUIRED`], and that is not an oversight.** Everything up to the edit
    /// step is reversible — a Back walks out of it and nothing has left the phone — so a
    /// build with a measured picker and no measured Post button can still be *driven as
    /// far as it was measured*, which is exactly what a measuring session needs to do to
    /// close the gap. What it must not do is publish, and it cannot: [`Composer::post`]
    /// is the only thing that reads this, and it refuses when it is `None`.
    ///
    /// The split is what lets the next measuring run happen at all without either
    /// weakening the gate or hand-driving the phone.
    pub fn post_button(&self) -> Option<ElementQuery<'static>> {
        self.post_button
    }

    /// Whether this build can be driven all the way to a published post.
    pub fn can_publish(&self) -> bool {
        self.post_button.is_some()
    }
}

/// The unlabelled control that opens the gallery from inside the composer.
///
/// # The most expensive measurement in this module
///
/// It carries **no `content-desc`, no `text`**, so like [`PhotoGrid`] it is geometry
/// rather than a label. What makes it worse than the grid is that the wrong guess is
/// not a miss but a *different feature*: measured 29/08/2026 on the fleet's build, the
/// two circles left of the shutter are `resource-id=…:id/egr` and open the **effects
/// panel**. An earlier note put the gallery entry at the bottom-left, which is exactly
/// where those sit. On this build it is on the **right**:
///
/// ```text
///   shutter        Record video   375,1545  330x330   centre 540,1710
///   gallery entry  (unlabelled)   765,1590  240x240   centre 885,1710   id/bos
/// ```
///
/// Found by looking at a screenshot — the entry renders a 2x2 montage of real photos,
/// which no hierarchy dump says.
///
/// # Anchored to the shutter, which is a real located element
///
/// The two share a **vertical centre exactly**, measured, so the shutter fixes `y`
/// outright. Horizontally the entry sits one measured margin in from the right screen
/// edge, which is how a bottom-bar side control is normally laid out and is the
/// parameterisation most likely to survive another resolution.
///
/// It is still one measurement on one screen size, and a tap here **must** be verified
/// afterwards rather than trusted — see [`Composer::await_picker`]. An overlay from
/// another app can sit inside an unlabelled rectangle: a Messenger bubble once landed
/// *within* a gallery entry, and tapping its centre tapped the bubble.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GalleryEntry {
    x: f64,
    y: f64,
    size: f64,
}

impl GalleryEntry {
    /// Place the entry from the screen width and the located shutter.
    pub fn beside_shutter(screen_width: f64, shutter: &ElementBox) -> Self {
        // 75/1080 and 240/1080 at the measured resolution.
        let margin = screen_width * (75.0 / 1080.0);
        let size = screen_width * (240.0 / 1080.0);
        Self {
            x: screen_width - margin - size,
            y: shutter.y + shutter.height / 2.0 - size / 2.0,
            size,
        }
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

/// The picker's unlabelled image grid, in device pixels.
///
/// The cells carry **no `content-desc`, no `text` and no `resource-id`** — they are bare
/// `FrameLayout`s — so they cannot be addressed by the label catalogue at all, and this
/// is the geometry that replaces it.
///
/// # Anchored, not hard-coded
///
/// Measured on 1080x2220. Writing those numbers down as constants would break on the
/// first phone with a different status bar, so the vertical origin is taken from the
/// **media-type tab row**, which is a real located element, and the horizontal layout
/// is derived from the screen width. What is fixed is the *pattern* — three columns,
/// one gap-width of margin on each side — which is what was actually measured:
///
/// ```text
///   6 │ 352 │ 6 │ 352 │ 6 │ 352 │ 6   = 1080   columns at x = 6, 364, 722
///   rows at y = 357, 719, 1081, 1443             pitch 362, height 356
/// ```
///
/// # There is no numeral to check the result against
///
/// Also measured: selecting an image renders **no per-cell numeral** anywhere on
/// screen. So a tap's effect cannot be read back cell by cell, and the only evidence
/// that a selection took is `Next` arming. That is why [`Composer::select`] taps the
/// whole set and then checks once, rather than pretending to verify each one — and why
/// scrolling this grid is not supported: after a flick there is nothing on screen that
/// identifies which row is which.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoGrid {
    origin_x: f64,
    origin_y: f64,
    cell_width: f64,
    cell_height: f64,
    gap: f64,
}

/// Columns in the picker grid. Measured, and the same on both builds looked at.
pub const GRID_COLUMNS: usize = 3;
/// Rows visible without scrolling, measured on 1080x2220.
///
/// The cap on how many images can be selected without a flick, and therefore — since
/// scrolling cannot be made safe, see [`PhotoGrid`] — the cap on a carousel this path
/// will attempt at all.
pub const GRID_VISIBLE_ROWS: usize = 4;

impl PhotoGrid {
    /// Build the grid from the screen width and the located media-type tab row.
    ///
    /// `tabs_bottom` is the bottom edge of the tab row (`Photos` and its neighbours),
    /// which is a control the catalogue can find. The 45px between it and the first row
    /// of cells is measured; expressing it as a fraction of the screen *height* would be
    /// worse, not better — it is a fixed layout margin, not a proportion.
    pub fn below_tabs(screen_width: f64, tabs_bottom: f64) -> Self {
        let gap = screen_width / 180.0;
        Self {
            origin_x: gap,
            origin_y: tabs_bottom + 45.0,
            cell_width: (screen_width - 4.0 * gap) / GRID_COLUMNS as f64,
            // Cells are very slightly taller than wide — 356 against 352 at 1080 —
            // which is measured rather than assumed square. It only shifts the tap
            // point by two pixels, but writing `cell_width` here would be recording a
            // number nobody read off a phone.
            cell_height: (screen_width - 4.0 * gap) / GRID_COLUMNS as f64 * (356.0 / 352.0),
            gap,
        }
    }

    /// The rectangle of the cell at `index`, counting left to right then down.
    ///
    /// `None` past the last visible row: past there the cell is off screen, and a tap
    /// at a computed off-screen point is a tap on whatever the OS decides is nearest.
    pub fn cell(&self, index: usize) -> Option<ElementBox> {
        if index >= GRID_COLUMNS * GRID_VISIBLE_ROWS {
            return None;
        }
        let column = (index % GRID_COLUMNS) as f64;
        let row = (index / GRID_COLUMNS) as f64;
        Some(ElementBox {
            x: self.origin_x + column * (self.cell_width + self.gap),
            y: self.origin_y + row * (self.cell_height + self.gap),
            width: self.cell_width,
            height: self.cell_height,
            description: None,
            // Unknowable and unused: these are `FrameLayout`s located by arithmetic,
            // not by a query, so nothing read an attribute off them. `false` is the
            // refusing direction, consistent with `ElementBox::clickable`'s default.
            enabled: true,
            clickable: false,
        })
    }

    /// How many images this grid can select without scrolling.
    pub fn capacity(&self) -> usize {
        GRID_COLUMNS * GRID_VISIBLE_ROWS
    }
}

/// What tapping the grid achieved.
///
/// Separate from [`ComposerVerdict`] because selecting images is not an outcome of the
/// publish attempt — nothing has left the phone yet — and a function that could hand
/// back `Posted` from the middle of the picker is one refactor away from a caller
/// believing it.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    /// The taps took, and `Next` armed. Carries the armed control, ready to tap.
    Armed(ElementBox),
    /// Asked for more cells than the grid shows without scrolling, or for none.
    NotEnoughCells,
    /// The cells were tapped and `Next` stayed unarmed — so the taps did not land.
    NeverArmed,
}

/// One composer session, driven a step at a time.
///
/// Constructed only from a [`ComposerPlan`], which is what makes the refusal
/// unforgettable rather than merely documented.
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

    /// Tap the composer tab and wait for the composer to actually come up.
    ///
    /// Proved by the gallery entry's *neighbour* rather than by a sleep: the entry
    /// itself is unlabelled, so what is waited for is the multi-select control's
    /// screen, one step later. `Ok(false)` is a real observation — the composer did not
    /// open — and the caller decides whether to back out.
    pub async fn open(&mut self) -> anyhow::Result<bool> {
        let Some(opener) = self.session.locate(self.plan.open).await? else {
            return Ok(false);
        };
        self.tap_inside(&opener).await?;
        Ok(true)
    }

    /// Tap the unlabelled gallery entry.
    ///
    /// Separate from [`Self::await_picker`] rather than folded into it, because the tap
    /// and the proof are two different things and the proof is the important one — see
    /// [`GalleryEntry`] for what tapping the wrong rectangle opens.
    pub async fn tap_gallery_entry(&mut self, entry: &GalleryEntry) -> anyhow::Result<()> {
        let rect = entry.rect();
        self.tap_inside(&rect).await
    }

    /// Wait for the picker, proved by a control only the picker has.
    ///
    /// **Not by the presence of camera controls' absence.** Measured on this build: a
    /// hierarchy dump taken while the picker is open still contains the camera screen's
    /// nodes underneath it, so "the shutter is gone" is never true and "the shutter is
    /// present" never means the picker is closed. The only sound test is a node the
    /// picker alone contributes, which is what [`TikTokControl::PickerMultiSelect`] is
    /// used for here.
    pub async fn await_picker(&self, stop: &AtomicBool) -> anyhow::Result<bool> {
        Ok(self
            .await_condition(PICKER_WINDOW, self.plan.multi_select, stop, |_| true)
            .await?
            .is_some())
    }

    /// Open the album menu and pick the campaign's own album by name.
    ///
    /// `album` is the `importId` — a string **this project wrote itself** when it
    /// created the import directory, which is the whole reason the album can be
    /// addressed at all. Nothing else on this screen is a value we control.
    ///
    /// # Exactly one match, or refuse
    ///
    /// A row is chosen only when the name matches once. More than one match means the
    /// phone holds two directories whose names both contain ours, and picking the first
    /// would publish from whichever the list happened to sort higher. That is the same
    /// discipline the catalogue records for the delete row, where a `Contains` locator
    /// would have tapped the favourites toggle.
    pub async fn select_album(&mut self, album: &str, stop: &AtomicBool) -> anyhow::Result<bool> {
        let Some(menu) = self
            .await_condition(PICKER_WINDOW, self.plan.album_menu, stop, |_| true)
            .await?
        else {
            return Ok(false);
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
            return Ok(false);
        };
        let row = row.clone();
        self.tap_inside(&row).await?;
        Ok(true)
    }

    /// Turn on multi-selection, which a carousel needs.
    pub async fn enable_multi_select(&mut self, stop: &AtomicBool) -> anyhow::Result<bool> {
        let Some(control) = self
            .await_condition(PICKER_WINDOW, self.plan.multi_select, stop, |_| true)
            .await?
        else {
            return Ok(false);
        };
        self.tap_inside(&control).await?;
        Ok(true)
    }

    /// Tap `count` cells, then check **once** that the selection took.
    ///
    /// One check rather than one per cell because there is nothing per cell to check:
    /// the grid renders no selection numeral on this build, so the only evidence any of
    /// the taps landed is `Next` arming, and that is a single signal for the whole set.
    ///
    /// Refuses a `count` past the grid's visible capacity instead of scrolling — see
    /// [`PhotoGrid`] for why a flick cannot be made safe here.
    pub async fn select(
        &mut self,
        grid: &PhotoGrid,
        count: usize,
        stop: &AtomicBool,
    ) -> anyhow::Result<Selection> {
        if count == 0 || count > grid.capacity() {
            return Ok(Selection::NotEnoughCells);
        }
        for index in 0..count {
            let Some(cell) = grid.cell(index) else {
                return Ok(Selection::NotEnoughCells);
            };
            self.tap_inside(&cell).await?;
            sleep(POLL, stop).await;
        }
        match self.await_armed(stop).await? {
            Some(next) => Ok(Selection::Armed(next)),
            None => Ok(Selection::NeverArmed),
        }
    }

    /// Wait for `Next` to arm, and return it so the caller can tap it.
    ///
    /// Reads `clickable`, **not** `enabled`. On this build `enabled` is `true` both
    /// before and after a selection, so a version of this that read it would return
    /// immediately with nothing selected — and the next step would advance out of the
    /// picker empty-handed.
    pub async fn await_armed(&self, stop: &AtomicBool) -> anyhow::Result<Option<ElementBox>> {
        self.await_condition(ARM_WINDOW, self.plan.picker_next, stop, |element| {
            element.clickable
        })
        .await
    }

    /// Tap `Next` and wait for the edit step.
    pub async fn advance(&mut self, next: &ElementBox, stop: &AtomicBool) -> anyhow::Result<bool> {
        self.tap_inside(next).await?;
        Ok(self
            .await_condition(COMPOSER_WINDOW, self.plan.edit_next, stop, |_| true)
            .await?
            .is_some())
    }

    /// Publish, or refuse because this build's Post button was never measured.
    ///
    /// **The only function in this module that can put something on a real account**,
    /// and the only reader of [`ComposerPlan::post_button`]. `None` there means no
    /// phone has ever had this control read off it — on *any* build, as of 29/08/2026 —
    /// so this returns [`ComposerVerdict::PostNotConfirmed`]'s sober cousin: it does not
    /// tap at all, and the caller learns nothing was published.
    ///
    /// # Confirmed by the button going away, and the honest gap in that
    ///
    /// The proof of publication is the Post control no longer being on screen, the same
    /// shape as the comment drawer's disarm. What has **not** been measured is what
    /// TikTok puts on screen *between* the tap and the feed: this build is expected to
    /// raise a public/private confirmation sheet, and nobody has dumped it. If it
    /// appears, the Post control stays visible behind it, this returns
    /// [`ComposerVerdict::PostNotConfirmed`], and the assignment becomes permanently
    /// unclaimable — which is the safe failure, not a correct one. Closing that gap is a
    /// measurement, not a code change.
    pub async fn post(&mut self, stop: &AtomicBool) -> anyhow::Result<ComposerVerdict> {
        let Some(query) = self.plan.post_button else {
            return Ok(ComposerVerdict::PostUnmeasured);
        };
        let Some(button) = self
            .await_condition(COMPOSER_WINDOW, query, stop, |_| true)
            .await?
        else {
            return Ok(ComposerVerdict::NoPostButton);
        };
        self.tap_inside(&button).await?;
        let gone = self
            .await_condition(COMPOSER_WINDOW, query, stop, |_| true)
            .await?
            .is_none();
        Ok(if gone {
            ComposerVerdict::Posted
        } else {
            ComposerVerdict::PostNotConfirmed
        })
    }

    /// Back out until the composer is gone.
    ///
    /// Best effort by design, exactly like [`crate::tiktok_drawer::CommentDrawer::leave`]:
    /// this runs on failure paths, where returning an error would replace a precise
    /// verdict with a vague one. What it must not do is leave the phone standing inside
    /// a half-filled composer, because the next session's first gesture would then land
    /// somewhere nobody planned for.
    ///
    /// Bounded at six presses rather than three: the picker is two screens deep inside
    /// the composer, so a drawer's budget would stop one screen short of the feed.
    ///
    /// # Waits for a screen it *wants*, not for screens it recognises
    ///
    /// The first version tested the negative — press Back while the picker's or the edit
    /// step's controls are on screen — and it was wrong in the case that matters most. The
    /// flow can stand on **four** screens, and the camera screen between the composer tab
    /// and the picker carries neither of those controls, so an error there read as "already
    /// out" and left the phone sitting in the composer with the shutter up.
    ///
    /// The positive signal is the composer opener itself. It lives on the bottom tab bar,
    /// which the feed has and the composer replaces with its own mode row — so seeing it
    /// means the bar is back, which means we are out. A control the plan already carries,
    /// and one screen cannot fake.
    pub async fn leave(&self, stop: &AtomicBool) {
        for _ in 0..6 {
            if self
                .session
                .locate(self.plan.open)
                .await
                .ok()
                .flatten()
                .is_some()
            {
                return;
            }
            if self.session.back().await.is_err() {
                return;
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
            if let Some(element) = self.session.locate(query).await? {
                if ready(&element) {
                    return Ok(Some(element));
                }
            }
            if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            sleep(POLL, stop).await;
        }
    }
}

/// Drive one carousel from the feed to published, and close the composer behind it.
///
/// The whole flow, for callers that just want a post out. Every early return goes
/// through [`Composer::leave`], including the ones an error would take: a `?` that
/// skipped it would leave the phone standing in a half-filled composer with a
/// campaign's images selected, and the next session's first gesture would land there.
///
/// **`grid` and `entry` are the caller's**, because both are geometry measured against
/// a screen size this function cannot see. The caller locates the shutter and the tab
/// row, builds them, and hands them in — which also means a caller on an unmeasured
/// resolution simply cannot call this.
pub async fn publish_carousel(
    session: &dyn UiSession,
    plan: ComposerPlan,
    plan_tap: impl TapPlanner,
    request: &CarouselRequest<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<ComposerVerdict> {
    let mut composer = Composer::new(session, plan, plan_tap);
    let outcome = drive(&mut composer, request, stop).await;
    composer.leave(stop).await;
    outcome
}

/// What one carousel needs: which album, how many images, and where they are.
pub struct CarouselRequest<'a> {
    /// The `importId` the media path used to name the album.
    pub album: &'a str,
    /// How many images to select.
    pub images: usize,
    pub entry: GalleryEntry,
    pub grid: PhotoGrid,
}

/// The steps that need an open composer, split out so [`publish_carousel`] can close it
/// on every exit without repeating the call at each early return.
async fn drive<P: TapPlanner>(
    composer: &mut Composer<'_, P>,
    request: &CarouselRequest<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<ComposerVerdict> {
    if !composer.open().await? {
        return Ok(ComposerVerdict::ComposerDidNotOpen);
    }
    composer.tap_gallery_entry(&request.entry).await?;
    if !composer.await_picker(stop).await? {
        return Ok(ComposerVerdict::PickerDidNotOpen);
    }
    if !composer.select_album(request.album, stop).await? {
        return Ok(ComposerVerdict::AlbumNotFound);
    }
    if !composer.enable_multi_select(stop).await? {
        return Ok(ComposerVerdict::PickerDidNotOpen);
    }
    let next = match composer.select(&request.grid, request.images, stop).await? {
        Selection::Armed(next) => next,
        Selection::NotEnoughCells => return Ok(ComposerVerdict::NotEnoughCells),
        Selection::NeverArmed => return Ok(ComposerVerdict::NeverArmed),
    };
    if !composer.advance(&next, stop).await? {
        return Ok(ComposerVerdict::EditStepDidNotOpen);
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
        every_publish_control_but_post_measured, every_publish_control_measured, nothing_measured,
        TIKTOK_LABEL_SETS,
    };
    use crate::types::TapPoint;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    fn plan() -> ComposerPlan {
        ComposerPlan::resolve(&every_publish_control_measured()).expect("the fixture is complete")
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

    /// A phone that shows one screen at a time and advances when tapped.
    ///
    /// Modelled as a *stack of screens* rather than a queue of answers because the thing
    /// under test is navigation: the composer is three screens deep, and the failure this
    /// fake has to be able to express is "the tap went out and the next screen never
    /// came", which a per-query queue cannot say.
    #[derive(Default)]
    struct FakeSession {
        screens: Vec<HashMap<String, ElementBox>>,
        at: Mutex<usize>,
        taps: Mutex<Vec<TapPoint>>,
        backs: Mutex<usize>,
        rows: Mutex<HashMap<String, Vec<ElementBox>>>,
        /// Fail every tap from this one onward, to exercise the error path — which is
        /// the one that used to skip the close-behind-you step in the drawer this module
        /// is modelled on. Counting rather than a plain flag because the interesting
        /// failure is **mid-flow**: a run that dies before the first tap never opened
        /// anything, so it has nothing to close and proves nothing about closing.
        fail_taps_after: Option<usize>,
        /// Stop advancing at this screen however many taps arrive.
        stuck_at: Option<usize>,
    }

    impl FakeSession {
        fn with(screens: Vec<Vec<(&str, ElementBox)>>) -> Self {
            Self {
                screens: screens
                    .into_iter()
                    .map(|screen| {
                        screen
                            .into_iter()
                            .map(|(key, value)| (key.to_string(), value))
                            .collect()
                    })
                    .collect(),
                ..Default::default()
            }
        }

        fn rows(self, key: &str, rows: Vec<ElementBox>) -> Self {
            self.rows.lock().insert(key.to_string(), rows);
            self
        }

        fn current(&self) -> HashMap<String, ElementBox> {
            let at = *self.at.lock();
            self.screens.get(at).cloned().unwrap_or_default()
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
            self.taps.lock().push(point);
            let mut at = self.at.lock();
            let ceiling = self
                .stuck_at
                .unwrap_or(self.screens.len().saturating_sub(1));
            *at = (*at + 1).min(ceiling);
            Ok(())
        }
        async fn swipe(&self, _gesture: crate::types::SwipeGesture) -> anyhow::Result<()> {
            Ok(())
        }
        async fn type_text(&self, _text: &str) -> anyhow::Result<()> {
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            *self.backs.lock() += 1;
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
            Ok(self.rows.lock().get(wanted).cloned().unwrap_or_default())
        }
    }

    /// **Not one shipped build can be driven to a post today, and the refusal names why.**
    ///
    /// The single most important assertion in this module. Every set in the catalogue is
    /// missing at least `composer_next` and `post_button` — nobody has read either off a
    /// phone, on any build — so `resolve` must refuse for all of them. If a later edit
    /// fills a label in without measuring it, or relaxes `REQUIRED`, this is what fails,
    /// and it fails *before* anything could open a composer on a live account.
    #[test]
    fn no_build_in_the_catalogue_can_reach_a_post_yet_and_each_refusal_names_its_gaps() {
        let mut checked = 0;
        for set in TIKTOK_LABEL_SETS {
            let controls = crate::tiktok_labels::controls_for(set.package, set.language, "")
                .expect("every catalogued set resolves to controls");
            let refusal = ComposerPlan::resolve(&controls).expect_err(&format!(
                "{} / {} claims it can publish; measure the controls first, then this \
                 assertion",
                set.package, set.language
            ));
            assert!(
                refusal.missing.contains(&TikTokControl::ComposerNext),
                "{} / {} resolves everything up to the edit step; if that is now measured, \
                 this assertion is what should change: {:?}",
                set.package,
                set.language,
                refusal.missing
            );
            // The message has to be actionable, because an operator reading it is the
            // person who would go and take the measurement.
            let rendered = refusal.to_string();
            assert!(rendered.contains("ComposerNext"), "{rendered}");
            // And the Post button separately, because it is deliberately **not** in
            // `REQUIRED` — a build can resolve a plan and still be unable to publish, so
            // its absence never shows up in `missing`. Nobody has read it off any phone.
            assert!(
                controls.label(TikTokControl::PostButton).is_none(),
                "{} / {} claims a measured Post button; the catalogue records none",
                set.package,
                set.language
            );
            checked += 1;
        }
        assert!(
            checked >= 3,
            "only {checked} sets scanned; the sweep is broken"
        );
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

    /// **A missing Post button stops publishing without stopping measurement.**
    ///
    /// The distinction the split between `REQUIRED` and `post_button` exists for: this
    /// build resolves, so the next measuring run can drive the phone as far as the edit
    /// step — which is how `post_button` gets measured at all — and `post` still refuses
    /// to tap anything.
    #[tokio::test(start_paused = true)]
    async fn a_build_without_a_measured_post_button_drives_but_never_publishes() {
        let plan = ComposerPlan::resolve(&every_publish_control_but_post_measured())
            .expect("the pre-publish path is measured");
        assert!(!plan.can_publish());
        assert!(plan.post_button().is_none());

        let session = FakeSession::with(vec![vec![("fixture-post", box_at(0.0, 0.0))]]);
        let mut composer = Composer::new(&session, plan, |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer.post(&stop).await.expect("no transport error"),
            ComposerVerdict::PostUnmeasured
        );
        assert!(
            session.taps.lock().is_empty(),
            "refusing to publish must not tap anything at all"
        );
    }

    /// **`Next` arms on `clickable`, and a version reading `enabled` would fire early.**
    ///
    /// The fixture is exactly the measured state: `enabled` true throughout, `clickable`
    /// false with nothing selected. If `await_armed` ever reads `enabled`, it returns
    /// immediately here and the flow advances out of the picker empty-handed.
    #[tokio::test(start_paused = true)]
    async fn nothing_selected_is_not_armed_even_though_enabled_says_true() {
        let unarmed = ElementBox {
            enabled: true,
            clickable: false,
            ..box_at(552.0, 1896.0)
        };
        let session = FakeSession::with(vec![vec![("fixture-picker-next", unarmed)]]);
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

        // And the same control, armed, is found.
        let armed = ElementBox {
            enabled: true,
            clickable: true,
            ..box_at(552.0, 1896.0)
        };
        let session = FakeSession::with(vec![vec![("fixture-picker-next", armed)]]);
        let composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        assert!(composer
            .await_armed(&stop)
            .await
            .expect("no error")
            .is_some());
    }

    /// The grid reproduces what was read off the phone, to the pixel.
    #[test]
    fn the_photo_grid_lands_on_the_measured_cells() {
        // Tabs measured at `[824,255][976,312]`, so their bottom edge is 312.
        let grid = PhotoGrid::below_tabs(1080.0, 312.0);
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

    /// **Past the last visible row there is no cell, and no scroll to reach one.**
    ///
    /// Not a bounds check for tidiness. With no per-cell numeral on this build there is
    /// nothing on screen that says which row is which after a flick, so a scrolled grid
    /// cannot be re-identified — and a computed tap at an off-screen point lands on
    /// whatever the OS decides is nearest.
    #[test]
    fn the_grid_stops_at_the_visible_rows_rather_than_scrolling() {
        let grid = PhotoGrid::below_tabs(1080.0, 312.0);
        assert_eq!(grid.capacity(), 12);
        assert!(grid.cell(11).is_some());
        assert!(grid.cell(12).is_none());
    }

    /// **The gallery entry is on the right of the shutter, and shares its vertical centre.**
    ///
    /// Pinned because the previous note put it bottom-*left*, which on this build is the
    /// effects panel. A regression here does not fail — it opens a different feature and
    /// the run reports "the picker did not open".
    #[test]
    fn the_gallery_entry_sits_where_the_screenshot_showed_it() {
        let shutter = ElementBox {
            x: 375.0,
            y: 1545.0,
            width: 330.0,
            height: 330.0,
            description: Some("Record video".into()),
            enabled: true,
            clickable: true,
        };
        let entry = GalleryEntry::beside_shutter(1080.0, &shutter).rect();
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

    /// Asking for more images than fit refuses instead of tapping what it can.
    #[tokio::test(start_paused = true)]
    async fn a_carousel_bigger_than_the_visible_grid_is_refused_before_any_tap() {
        let session = FakeSession::with(vec![vec![]]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let grid = PhotoGrid::below_tabs(1080.0, 312.0);
        let stop = AtomicBool::new(false);
        assert_eq!(
            composer.select(&grid, 13, &stop).await.expect("no error"),
            Selection::NotEnoughCells
        );
        assert_eq!(
            composer.select(&grid, 0, &stop).await.expect("no error"),
            Selection::NotEnoughCells
        );
        assert!(session.taps.lock().is_empty(), "refusing must not tap");
    }

    /// **Two albums matching the campaign's name is a refusal, not a coin flip.**
    #[tokio::test(start_paused = true)]
    async fn an_ambiguous_album_name_refuses_rather_than_taking_the_first() {
        let screens = vec![
            vec![("fixture-album-menu", box_at(483.0, 115.0))],
            vec![("fixture-album-menu", box_at(483.0, 115.0))],
        ];
        let session = FakeSession::with(screens)
            .rows("riviu-abc", vec![box_at(0.0, 400.0), box_at(0.0, 500.0)]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert!(
            !composer
                .select_album("riviu-abc", &stop)
                .await
                .expect("no error"),
            "two directories matched and one was chosen anyway"
        );
        // One tap: the menu opened. The second — choosing a row — must not have happened.
        assert_eq!(session.taps.lock().len(), 1);
    }

    /// One match is taken, and it is the row rather than the menu.
    #[tokio::test(start_paused = true)]
    async fn the_campaigns_own_album_is_chosen_when_it_is_unambiguous() {
        let screens = vec![
            vec![("fixture-album-menu", box_at(483.0, 115.0))],
            vec![("fixture-album-menu", box_at(483.0, 115.0))],
        ];
        let row = box_at(0.0, 400.0);
        let session = FakeSession::with(screens).rows("riviu-abc", vec![row.clone()]);
        let mut composer = Composer::new(&session, plan(), |element: &ElementBox| element.centre());
        let stop = AtomicBool::new(false);
        assert!(composer
            .select_album("riviu-abc", &stop)
            .await
            .expect("no error"));
        let taps = session.taps.lock();
        assert_eq!(taps.len(), 2);
        // `TapPoint` is not `PartialEq`, so compare the coordinates it carries.
        assert_eq!(
            (taps[1].x, taps[1].y),
            (row.centre().x, row.centre().y),
            "the row was tapped, not the menu again"
        );
    }

    /// **The composer is closed behind every exit, including the error one.**
    ///
    /// The failure this guards is the one `tiktok_drawer` records from experience: each
    /// `?` in the flow used to skip the close, leaving the phone standing inside an open
    /// screen with the campaign's selection still made. Here that screen is a composer
    /// holding images, which is worse than a comment field holding text.
    #[tokio::test(start_paused = true)]
    async fn a_transport_error_still_closes_the_composer() {
        // Two screens: the feed, which carries the composer tab, and the camera screen
        // inside the composer, which carries **none** of the plan's controls. The first
        // tap opens the composer; the second — the unlabelled gallery entry — dies. That
        // is the case an earlier `leave` read as "already out".
        let session = FakeSession {
            fail_taps_after: Some(1),
            ..FakeSession::with(vec![
                vec![("fixture-composer-open", box_at(432.0, 1929.0))],
                vec![],
            ])
        };
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            entry: GalleryEntry::beside_shutter(1080.0, &box_at(375.0, 1545.0)),
            grid: PhotoGrid::below_tabs(1080.0, 312.0),
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
        assert!(
            *session.backs.lock() > 0,
            "the error path walked away leaving the composer open"
        );
    }

    /// A composer that never opens is reported as that, and nothing further is tried.
    #[tokio::test(start_paused = true)]
    async fn a_composer_that_does_not_open_is_named_rather_than_timed_out_later() {
        let session = FakeSession::with(vec![vec![]]);
        let request = CarouselRequest {
            album: "riviu-abc",
            images: 3,
            entry: GalleryEntry::beside_shutter(1080.0, &box_at(375.0, 1545.0)),
            grid: PhotoGrid::below_tabs(1080.0, 312.0),
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
    }

    /// **An unconfirmed post is never retried, and neither is a successful one.**
    ///
    /// The one question the verdict enum exists to answer. `PostNotConfirmed` may be live
    /// on a real account, so treating it as retryable would publish a duplicate that
    /// nothing in this project can take down.
    #[test]
    fn only_the_verdicts_that_published_nothing_may_be_dispatched_again() {
        assert!(!ComposerVerdict::Posted.may_retry());
        assert!(!ComposerVerdict::PostNotConfirmed.may_retry());
        for verdict in [
            ComposerVerdict::ComposerDidNotOpen,
            ComposerVerdict::PickerDidNotOpen,
            ComposerVerdict::AlbumNotFound,
            ComposerVerdict::NotEnoughCells,
            ComposerVerdict::NeverArmed,
            ComposerVerdict::EditStepDidNotOpen,
            ComposerVerdict::PostUnmeasured,
            ComposerVerdict::NoPostButton,
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
