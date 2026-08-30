//! Measured `content-desc` labels for TikTok's controls, per app build and UI
//! language.
//!
//! This exists because a documented assumption turned out to be false. AGENTS.md
//! §9 said `content-desc` is "English regardless of the UI language", measured on
//! the global build `com.zhiliaoapp.musically`. On the South-East Asian build
//! `com.ss.android.ugc.trill` with a Vietnamese UI, the labels are **translated**:
//! the like control is `Thích`, the feed tab is `Đề xuất`. Every English locator
//! silently found nothing — `find("Like")` absent, `assert_visible("For You")`
//! failed — while the rail was plainly on screen.
//!
//! So labels are data, not constants, and the data is *measured*. There is no rule
//! to derive them: the same Vietnamese build keeps `Follow <name>` in English while
//! translating `Like`. Anything not measured is [`None`] here, and `None` must mean
//! **refuse**, exactly as `screen::CALIBRATED_LAYOUTS` refuses a screen class
//! nobody has calibrated (AGENTS.md §10). Guessing a translation would produce a
//! locator that silently matches nothing, which is the failure this module exists
//! to stop.
//!
//! # Two kinds of label, two different keys
//!
//! Most labels are **translations**: language-dependent, and stable across app
//! updates. A few are **unresolved Android resource references** — `@2131823284` —
//! that the app never turned into a string. Those are the exact opposite:
//! language-independent, and *reassigned every time the app is rebuilt*.
//!
//! Measured, and this is why the split exists rather than being a tidiness argument.
//! Two phones, same package `com.ss.android.ugc.trill`, same Vietnamese UI, and the
//! comment drawer's Send button:
//!
//! | phone | app version | Send button |
//! |---|---|---|
//! | Redmi Note 12 | 46.3.3 | `@2131823284` |
//! | SM-N950F | 46.4.3 | **`@2131823293`** |
//!
//! On both, the button's `enabled` flag went false → true the moment the field held
//! text, so the armed-signal contract is identical; only the id moved. Keying that id
//! by language would have made the second phone refuse a control it plainly has, and a
//! reassignment that happened to land on *another* button would have made it tap the
//! wrong thing.
//!
//! So: [`TIKTOK_LABEL_SETS`] is keyed by (package, language) and holds translations;
//! [`TIKTOK_RESOURCE_SETS`] is keyed by (package, app version) and holds resource ids.
//! [`controls_for`] resolves both and is the **only** way to get a label — there is no
//! `label()` on either table alone, so no caller can accidentally read a version-keyed
//! id out of a language-keyed set.

/// A TikTok control, named for what it does rather than what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TikTokControl {
    /// The "For You" / `Đề xuất` feed tab.
    FeedTab,
    /// The button that declines a modal TikTok puts in the way — `Not now`.
    ///
    /// Only ever the *declining* option, which is what makes tapping it safe without
    /// knowing which dialog is up: declining changes no setting and no account, and the
    /// dialog simply comes back next time. The affirmative button is deliberately not
    /// catalogued — there is no dialog this project wants to say yes to on its own.
    ///
    /// Worth naming why it matters: a modal owns the whole accessibility tree. Measured
    /// 18/08/2026 on an SM-G955U1 sitting behind "Save login for next time?", the entire
    /// dump was one `content-desc` of `Dialog` — so the feed tab, the Home tab and the
    /// action rail were all equally invisible and the session could only report that it
    /// never saw a feed.
    DialogDismiss,
    /// The declining button on TikTok's new-user **journey** — `Skip`.
    ///
    /// Not the same thing as [`Self::DialogDismiss`], and giving it its own variant is the
    /// point. A journey page is not a modal over the feed: it is
    /// `com.ss.android.ugc.aweme.journey.NewUserJourneyActivity`, a full activity the app
    /// puts in front of the feed and keeps there. `Not now` is nowhere on it, so the
    /// modal decline could not clear it, and Back does not either — measured below.
    ///
    /// **Measured 23/08/2026 on five phones at once** — 9889db374744474635,
    /// 98895a3355424e484f, ce03171392f9390c01, ce031713dd735a1103, ce051715ac247a3f01, all
    /// `com.ss.android.ugc.trill` 38.3.2 in English. Six of the fourteen phones attached
    /// that day were sitting on this page, and the five dumped were identical in every
    /// label. The page reads *"TikTok is better with friends!"* over a Facebook
    /// friend-sync pitch, and its decline is `android.widget.Button` with `text="Skip"`
    /// and no `content-desc`, so this is a [`LabelMatch::Text`].
    ///
    /// **What makes it safe is what sits beside it.** `Skip` has a sibling `Sync` (the
    /// same page) and, one step later, `Find friends` (the confirmation dialog) — both
    /// affirmative, both of which would hand a real account's friend list to TikTok. So
    /// this must stay an *exact* match on the decline. Never widen it to a substring: a
    /// `Contains("Skip")` would also match the confirmation dialog's title
    /// *"Skip finding Facebook friends?"*, which is a `TextView` and not tappable, and the
    /// locator returns the first hit rather than the button.
    ///
    /// One label clears **two** consecutive steps, which is why there is no separate
    /// variant for the confirmation: tapping `Skip` on the page raises the dialog
    /// *"Skip finding Facebook friends?"*, whose decline is also a `Button` reading
    /// `Skip` — no resource id on that one. Same string, same attribute, so the same
    /// locator dismisses both in turn.
    JourneySkip,
    /// The way out of the journey's last page — `Done`.
    ///
    /// The suggested-follow list, *"Your friends on TikTok"*, reached after
    /// [`Self::JourneySkip`] has cleared the two friend-sync steps. It offers no decline
    /// at all: every row carries its own `Follow` / `Remove` pair and the only control
    /// that leaves is a full-width `Button` reading `Done`.
    ///
    /// **Back is not an alternative here, and that was measured rather than assumed.** One
    /// `KEYCODE_BACK` on this page changed nothing — the dump before and after was
    /// byte-identical at 47,509 bytes, same activity, same window id. An onboarding page
    /// that swallows Back is exactly the shape that strands a phone forever.
    ///
    /// **`Done` does not follow anybody, and this is the measurement that mattered.** The
    /// repo's own rule is that a measured label is not a safe label, and a button on a
    /// screen full of `Follow` buttons is precisely where that bites. Measured on
    /// 9889db374744474635, 23/08/2026: the page offered five accounts, `Done` was tapped,
    /// and the account's profile then read **Following 1** — `hương phạm`, a follow that
    /// predated the test — with all five suggestions still showing an untapped `Follow`
    /// in the Following screen's own suggestion strip. So `Done` dismisses; it does not
    /// accept on the account's behalf.
    JourneyDone,
    /// The control that reveals comments TikTok has hidden — `View folded comments`.
    ///
    /// Not a tidiness feature. TikTok folds comments it considers "similar to others
    /// flagged by our community", and it does this to these accounts progressively: the
    /// first few comments on a post are visible and later ones are not. A reply looking
    /// for its parent then scrolls to the end of a list the parent is not in, and refuses
    /// with `reply_parent_not_found` — which is correct, and useless, because the parent
    /// is one tap away.
    ///
    /// Tapping it is safe in the way that matters: it reveals, it does not post, follow or
    /// subscribe anything. What it changes is what the *account* can see, not what it does.
    FoldedComments,
    /// The post's sound strip — `Original sound by <creator>`.
    ///
    /// Not something to tap. It is read for its *value*, because it is the one description
    /// on the rail that differs between two ordinary posts: comments and shares both read
    /// `0` on a low-engagement feed, so a fingerprint built from them alone cannot tell two
    /// cards apart and every swipe looks unproven. Measured 18/08/2026 — see
    /// `nurture::hierarchy::PostFingerprint`.
    SoundLink,
    /// The bottom bar's Home tab — the way *back* to the feed from anywhere else.
    ///
    /// Distinct from [`Self::FeedTab`], which is a tab *inside* the feed and is therefore
    /// only visible once you are already there. A phone parked on Profile, Shop or Inbox
    /// shows this one and not that one, and telling them apart is the difference between
    /// a session that recovers and one that waits thirty seconds and gives up.
    HomeTab,
    /// The badge that marks a card as a **photo post** rather than a video.
    ///
    /// The gate for paging sideways, and it has to be this rather than the page
    /// counter. Measured on an SM-N950F, 12/08/2026: the `1 / 7` indicator in the
    /// top-right corner is a **transient overlay** on the feed — present on 3 of 14
    /// cards in one sweep and on 0 of 14 in another, on a feed with the same photo
    /// posts in it — while this badge sits beside the caption and stays. A gate that
    /// reads the counter fires only if it happens to look while the overlay is up,
    /// which in a real session is never: the loop gets there after the watch dwell.
    ///
    /// Being wrong here is expensive in one direction. A sideways swipe on a *video*
    /// card is TikTok's open-the-author's-profile gesture, so a false positive walks
    /// the session off the feed — which is why this is a catalogued, fail-closed
    /// label and not a heuristic.
    PhotoBadge,
    /// The like control while the post is **not** liked.
    Like,
    /// The like control once the post **is** liked. This is the state evidence
    /// that replaces pixel matching, so it has to be measured, not assumed.
    Liked,
    /// Opens the comment drawer. The label embeds the count, so it is a substring
    /// match.
    Comments,
    Share,
    Bookmark,
    /// Follow the author. Embeds the author name.
    Follow,
    /// A LIVE post has no action rail; a feed loop must recognise and swipe past.
    LiveRoom,
    /// The Send button inside the comment drawer.
    ///
    /// The one control whose label is an unresolved **resource reference** rather than
    /// text, so it survives a language change and breaks on an app update — the
    /// opposite fragility from every other control here. It therefore lives in
    /// [`TIKTOK_RESOURCE_SETS`], keyed by app version, and is measured as different on
    /// the two phones in this fleet. See the module docs.
    CommentSend,
    /// The per-comment Reply button inside the drawer.
    ///
    /// Lives in `text`, not `content-desc` — see [`LabelAttribute`]. One per
    /// top-level comment row, so a locator for it returns **many** elements and the
    /// right one is a geometric question, not a matching one.
    CommentReply,
    /// The bottom-bar control that opens TikTok's own composer.
    ///
    /// Named for what the app calls it, which is **not** what it does here: on the
    /// measured build its `content-desc` is `Quay` ("record"), even though the publish
    /// path uses it to reach the gallery picker rather than the camera. Do not
    /// "correct" it to something that reads better — it is a measured string.
    ComposerOpen,
    /// The composer's own shutter button — the big round one that takes a photo.
    ///
    /// **Never tapped by the publish path**, and that is the point of it: it is the *anchor*
    /// the unlabelled gallery entry is measured against. The entry beside it carries no
    /// `content-desc` and no `text`, so it can only be reached by arithmetic, and arithmetic
    /// from a located element survives a screen size the numbers were not taken on — while
    /// arithmetic from a remembered coordinate does not.
    ///
    /// Requiring it is therefore a *provenance* rule rather than a navigation one: a build
    /// that has not had this measured has no anchor, so its gallery entry would be reached
    /// by numbers copied off another phone, and the two controls next to it open the effects
    /// panel.
    ComposerShutter,
    /// The album dropdown at the top of the gallery picker, showing the current album.
    ///
    /// Opening it lists albums **by directory name**, which is how the publish path
    /// selects a campaign's own images: the import directory is named with the
    /// `importId`, so the album is matched against a string this project wrote itself.
    PickerAlbumMenu,
    /// The picker's "everything" tab. Selected by default on the measured build.
    PickerTabAll,
    /// The picker's photos-only tab.
    PickerTabPhotos,
    /// Turns on multi-selection, which a carousel needs.
    PickerMultiSelect,
    /// Confirms the selection and moves on to the edit step.
    ///
    /// Measured with `clickable=false` while nothing is selected and clickable once
    /// something is — the same armed-signal shape as the comment drawer's Send button,
    /// so "enough images are selected" is *checkable* rather than timed.
    PickerNext,
    /// The bottom-bar tab that opens **our own** profile.
    ///
    /// `Exact`, and that is not a style choice. `Hồ sơ <tên>` appears on the action rail
    /// as the author link, so `Contains` would match the *author's* profile and walk the
    /// session onto a stranger's page — the same trap that once made `Contains("Follow")`
    /// match the `Đã follow` tab. Description matching is case-insensitive, which makes a
    /// loose match likelier still.
    ///
    /// **Measured** on a Redmi Note 12 (`trill` 46.3.3, vi) 13/08/2026 — see AGENTS.md 9.36.
    /// `None` on `musically`/en, which has not been looked at.
    ProfileTab,
    /// The **author's** profile link on the action rail. Embeds the author name, and unlike
    /// [`Self::Follow`] it is there whether or not this account already follows them.
    ///
    /// That difference is the whole reason it exists. `read_author_label` used to read the
    /// author only out of the Follow button, so on a post by a creator the account already
    /// follows — or on the account's own post — there was no button, no author, and the
    /// arrival check refused a perfectly good open as `target_open_no_baseline`. The gate is
    /// right to refuse when it cannot tell one post from another; it just had only one place
    /// to look.
    ///
    /// **Measured 24/08/2026** on `.../@.lt.gi.mang.v/photo/7668947001618320660`, the same
    /// post before and after tapping Follow on `ce051715ac247a3f01`
    /// (`trill` en, SM-G950F): with Follow present the rail carried both
    /// `content-desc="Follow Đà Lạt Gói Mang Về"` and
    /// `content-desc="Đà Lạt Gói Mang Về profile"`; after following, the first was gone and
    /// the second unchanged. `musically`/en (`ce11171beb408a1501`) reads identically.
    ///
    /// **The two builds put the name on opposite sides**, which is why
    /// `bare_author_label` strips by the catalogued needle rather than by a fixed prefix:
    /// English is `<tên> profile`, Vietnamese is `Hồ sơ <tên>`.
    ///
    /// The space in the needle is load-bearing in both, for the same reason it is in
    /// `Follow `: the bottom bar carries our *own* profile tab as a bare `Profile` / `Hồ sơ`,
    /// description matching is case-insensitive, and a match on that would read the wrong
    /// account's name as the author of the post.
    AuthorProfileLink,
    /// The like button **that carries the count**, which is not the one [`Self::Like`] finds.
    ///
    /// Measured 24/08/2026 on `ce051715ac247a3f01` (`trill` en 38.3.2): the action rail holds
    /// three like-ish nodes — `content-desc="Like"` on an `ImageView` (what `Like` matches,
    /// and it has no number), `content-desc="Like video. 22 likes"` on the `Button`, and
    /// `content-desc="Like or undo like"` once per comment row. Only the middle one states a
    /// total, and `Contains("likes")` matches it alone: `liked` and `like` are different
    /// words, so neither of the others contains it.
    ///
    /// `None` on the Vietnamese set: the string is a translation and translations are
    /// measured here, never derived. Until somebody dumps a vi rail, a like *threshold*
    /// refuses on that build rather than reading a number it guessed.
    LikeCount,
    /// Advances out of the picker's edit step toward the post screen.
    ///
    /// **Not measured.** Everything after `Tiếp` is unmeasured — see AGENTS.md 9.10, the
    /// picker labels stop there.
    ComposerNext,
    /// The caption input on the post screen.
    ///
    /// **Not measured on any build**, and the publish path refuses without it — a carousel
    /// that goes out with an empty caption is wrong in a way nothing here can correct
    /// afterwards, because there is no delete on Android.
    ///
    /// # What this label must be, and what it must not
    ///
    /// It has to be a **class or a resource id, never the placeholder string**. The publish
    /// path types the caption and then reads it back through the same locator to prove the
    /// text took; a placeholder like `Add a caption…` stops matching the instant a character
    /// is typed, so a placeholder-based entry would type correctly and then report that it had
    /// not. The comment drawer hit the same wall and is located by class for the same reason.
    ComposerCaption,
    /// The button that actually publishes the post.
    ///
    /// **Not measured**, and the most consequential of the three: without it the composer
    /// refuses *before* opening, which is the only point where refusing is free.
    PostButton,
    /// Opens the sheet on our own post that contains the delete row.
    ///
    /// **Looked for and not found**, which is stronger than "not measured". Measured on a
    /// Redmi Note 12, `com.ss.android.ugc.trill` 46.3.3, Vietnamese UI, 13/08/2026: on our
    /// own post page the three-dots control visible in a screenshot carries **no
    /// `content-desc` and no `text`** — the full attribute inventory of that screen is in
    /// AGENTS.md 9.37. So it cannot be located by this catalogue's mechanism at all;
    /// locating it would take geometry, and geometry for an irreversible tap on a screen
    /// nobody calibrated is exactly what this project refuses to invent.
    ///
    /// One caveat, kept because it is honest: that dump was taken with TikTok's share sheet
    /// auto-opened over the top of the page, and only on that one build. A second look on a
    /// clean page could still find a label.
    ///
    /// Refuses at *driver selection* rather than mid-run: a post that went out and cannot be
    /// taken down is a promise the session cannot keep, so the refusal has to happen before
    /// anything is published.
    PostDeleteMenu,
    /// The delete row inside that sheet.
    ///
    /// **Not measured**, and the decoy is now confirmed by measurement rather than
    /// predicted: on the own-post page the *only* string containing `xóa` anywhere, in
    /// either attribute, is `Thêm hoặc xóa video này khỏi mục Yêu thích.` — the favourites
    /// toggle. A `Contains("xóa")` locator would tap that. So this needs `locate_all` and
    /// must refuse on more than one match, and it needs an exact string rather than a
    /// substring.
    PostDelete,
    /// The confirm button in the delete dialog.
    ///
    /// **Not measured.** Absent means refuse before the sheet is even opened — reaching a
    /// confirmation dialog with no idea which button confirms is the worst place to stop.
    PostDeleteConfirm,
}

impl TikTokControl {
    /// Every control, for tests that must cover all of them.
    ///
    /// **What actually holds, stated narrowly.** Two exhaustive matches — [`Self::ordinal`] and
    /// [`TikTokLabels::translated`] — make the compiler refuse a new variant until it is
    /// handled in both, and the fixed length below forces a bump once it *is* added here. So
    /// adding a control without noticing this file is not possible.
    ///
    /// But nothing mechanically forces a new variant *into* `ALL`: a variant given an ordinal
    /// and left out of this array would still pass `every_control_appears_in_all`, because
    /// that test sizes itself from `ALL` and iterates `ALL`. Closing that would take a macro
    /// generating enum and array together. Until then, treat `ALL` as hard to drift rather
    /// than impossible — which is still a long way from the hand-written list it replaced,
    /// where a new control was simply never checked.
    ///
    /// **That drift had already happened, and this is the repair.** Adding
    /// [`Self::JourneySkip`] found four variants — `HomeTab`, `SoundLink`, `DialogDismiss`
    /// and `FoldedComments` — that carried ordinals 23–26 and were absent from this array,
    /// so `no_entry_carries_an_empty_label` had never checked a single one of them. They
    /// are here now, the ordinals run 0–28 with no holes, and `every_control_appears_in_all`
    /// is finally strong enough to notice: with `seen` sized from this array, a variant
    /// whose ordinal exceeds it now panics on the index instead of passing quietly.
    pub const ALL: [Self; 33] = [
        Self::FeedTab,
        Self::PhotoBadge,
        Self::Like,
        Self::Liked,
        Self::Comments,
        Self::Share,
        Self::Bookmark,
        Self::Follow,
        Self::LiveRoom,
        Self::CommentSend,
        Self::CommentReply,
        Self::ComposerOpen,
        Self::ComposerShutter,
        Self::PickerAlbumMenu,
        Self::PickerTabAll,
        Self::PickerTabPhotos,
        Self::PickerMultiSelect,
        Self::PickerNext,
        Self::ProfileTab,
        Self::AuthorProfileLink,
        Self::LikeCount,
        Self::ComposerNext,
        Self::ComposerCaption,
        Self::PostButton,
        Self::PostDeleteMenu,
        Self::PostDelete,
        Self::PostDeleteConfirm,
        Self::HomeTab,
        Self::SoundLink,
        Self::DialogDismiss,
        Self::FoldedComments,
        Self::JourneySkip,
        Self::JourneyDone,
    ];

    /// A stable position per variant, matched exhaustively on purpose.
    ///
    /// `#[cfg(test)]` rather than `#[allow(dead_code)]`: it exists to make the completeness
    /// test un-driftable and has no production caller, and saying that with the attribute
    /// keeps clippy honest about the rest of the file.
    #[cfg(test)]
    fn ordinal(self) -> usize {
        match self {
            Self::FeedTab => 0,
            Self::PhotoBadge => 1,
            Self::Like => 2,
            Self::Liked => 3,
            Self::Comments => 4,
            Self::Share => 5,
            Self::Bookmark => 6,
            Self::Follow => 7,
            Self::LiveRoom => 8,
            Self::CommentSend => 9,
            Self::CommentReply => 10,
            Self::ComposerOpen => 11,
            Self::PickerAlbumMenu => 12,
            Self::PickerTabAll => 13,
            Self::PickerTabPhotos => 14,
            Self::PickerMultiSelect => 15,
            Self::PickerNext => 16,
            Self::ProfileTab => 17,
            Self::ComposerNext => 18,
            Self::PostButton => 19,
            Self::PostDeleteMenu => 20,
            Self::PostDelete => 21,
            Self::PostDeleteConfirm => 22,
            Self::HomeTab => 23,
            Self::SoundLink => 24,
            Self::DialogDismiss => 25,
            Self::FoldedComments => 26,
            Self::JourneySkip => 27,
            Self::JourneyDone => 28,
            Self::AuthorProfileLink => 29,
            Self::LikeCount => 30,
            Self::ComposerShutter => 31,
            Self::ComposerCaption => 32,
        }
    }
}

/// Which attribute a label lives in.
///
/// Both are needed, measured. TikTok's action rail is described
/// (`content-desc="Thích"`) while things *inside* the comment drawer are not: the
/// reply button on `com.ss.android.ugc.trill` has an **empty** `content-desc` and
/// carries `Trả lời` in `text`. A locator that only ever reads `content-desc`
/// cannot find it at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelAttribute {
    /// `content-desc` — Appium's "accessibility id".
    Description,
    /// The rendered `text` of the node.
    Text,
    /// The suffix of the fully-qualified `resource-id`.
    ///
    /// The third attribute, and the one that only earns its place when the other two cannot
    /// work. It is **language-proof and build-specific** — the exact opposite trade to a
    /// string — so a label using it must be keyed by app version, which is what
    /// [`TikTokResourceLabels`] is for.
    ResourceId,
    /// The widget class. Language-proof, build-proof, and **not unique**.
    ClassName,
}

/// How a label is matched: which attribute, and exactly or as a substring.
///
/// Substring is not a convenience: `Đọc hoặc viết bình luận. 21,1K bình luận`
/// carries a comment count that changes per post, so an exact match can never hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelMatch {
    Exact(&'static str),
    Contains(&'static str),
    /// Exact match on `text` rather than `content-desc`.
    Text(&'static str),
    /// Substring match on `text`.
    TextContains(&'static str),
    /// The node's widget class, e.g. `android.widget.EditText`.
    ///
    /// The third thing a control can be identified by, and the only one that works for an
    /// **input field**: its `text` is the placeholder until somebody types, at which point the
    /// locator that found it stops matching. The comment drawer already located its field this
    /// way for exactly that reason; [`TikTokControl::ComposerCaption`] needs the same and,
    /// until now, the catalogue had no way to say it — the documentation demanded a strategy
    /// that could not be written down.
    ///
    /// Weaker than an id and stronger than a string: a class is language-proof and
    /// build-proof, and it is not unique. A caller that needs one node must check.
    ClassName(&'static str),
    /// The suffix of the node's `resource-id`, e.g. `:id/snr` for `com.…:id/snr`.
    ///
    /// # Only for controls whose strings genuinely cannot work
    ///
    /// Measured 29/08/2026, and the album pill in TikTok's image picker is the case that
    /// forced it. Its `text` is **the album currently showing** — `All` before a choice, and
    /// the campaign's own import id after one — so a text locator names a value that changes
    /// the moment it is used. And `All` **also** belongs to the media-type tab one row below,
    /// so the string is ambiguous on screen before anything is chosen at all.
    ///
    /// Its `resource-id` has neither problem: `…:id/snr` is the same node whatever it is
    /// displaying.
    ///
    /// # And the price, which is why this is not the default
    ///
    /// Resource ids are **reassigned on every app rebuild**. This project already measured
    /// that on its own fleet: the comment drawer's Send button is `@2131823284` on TikTok
    /// 46.3.3 and `@2131823293` on 46.4.3. So a label of this kind is only ever valid for the
    /// exact `versionName` it was read from, and belongs in [`TIKTOK_RESOURCE_SETS`] rather
    /// than in a language set — a build this table does not name gets `None` and refuses.
    ResourceId(&'static str),
}

impl LabelMatch {
    pub fn value(&self) -> &'static str {
        match self {
            Self::Exact(value)
            | Self::Contains(value)
            | Self::Text(value)
            | Self::TextContains(value)
            | Self::ClassName(value)
            | Self::ResourceId(value) => value,
        }
    }

    pub fn is_exact(&self) -> bool {
        // A resource-id suffix identifies one control, so it belongs with the exact forms
        // rather than with the substring ones. A class deliberately does **not**: several
        // nodes share one, which is the whole reason a caller has to check.
        matches!(self, Self::Exact(_) | Self::Text(_) | Self::ResourceId(_))
    }

    pub fn attribute(&self) -> LabelAttribute {
        match self {
            Self::Exact(_) | Self::Contains(_) => LabelAttribute::Description,
            Self::Text(_) | Self::TextContains(_) => LabelAttribute::Text,
            Self::ResourceId(_) => LabelAttribute::ResourceId,
            Self::ClassName(_) => LabelAttribute::ClassName,
        }
    }

    /// The driver query this label becomes.
    ///
    /// One place rather than three. Every caller — the nurture loop, the Interaction
    /// path, the probe — needs exactly this translation, and a copy that forgets a
    /// variant fails by *finding nothing*, which is indistinguishable from the
    /// control being absent.
    pub fn to_query(&self) -> crate::driver::ElementQuery<'static> {
        match *self {
            Self::Exact(value) => crate::driver::ElementQuery::Description { value, exact: true },
            Self::Contains(value) => crate::driver::ElementQuery::Description {
                value,
                exact: false,
            },
            Self::Text(value) => crate::driver::ElementQuery::Text { value, exact: true },
            Self::TextContains(value) => crate::driver::ElementQuery::Text {
                value,
                exact: false,
            },
            Self::ResourceId(value) => crate::driver::ElementQuery::ResourceIdSuffix(value),
            Self::ClassName(value) => crate::driver::ElementQuery::ClassName(value),
        }
    }
}

/// One measured set: an app build, a UI language, and the labels seen on it.
#[derive(Debug, Clone, Copy)]
pub struct TikTokLabels {
    pub package: &'static str,
    /// ISO 639-1 language of the *UI*, from `persist.sys.locale` — never from
    /// `ro.product.locale`, which is the factory default and read `en-GB` on a
    /// phone whose UI was Vietnamese.
    pub language: &'static str,
    /// Where these strings came from, so a future reader can re-check them.
    pub measured_on: &'static str,
    /// The app `versionName` these translations were read on, or empty when
    /// unrecorded.
    ///
    /// Informational only for this table — translations have measured as
    /// version-stable, so a mismatch here is a note for the next reader, not a
    /// refusal. Version-*dependent* labels live in [`TIKTOK_RESOURCE_SETS`], which is
    /// keyed by this value rather than merely recording it.
    pub measured_app_version: &'static str,
    feed_tab: Option<LabelMatch>,
    /// The bottom bar's Home tab. `None` means a session that finds TikTok on another tab
    /// cannot get back to the feed and will refuse rather than swipe somewhere unknown.
    home_tab: Option<LabelMatch>,
    /// The sound strip, matched by its stable prefix and read for its whole value.
    sound_link: Option<LabelMatch>,
    /// The decline button on a modal. `None` means modals are not cleared on this build.
    dialog_dismiss: Option<LabelMatch>,
    /// The new-user journey's `Skip`. `None` means a phone that lands in the onboarding
    /// journey on this build stays there — it is not a screen any other label can clear.
    journey_skip: Option<LabelMatch>,
    /// The journey's final `Done`. `None` for the same reason, and with the same cost:
    /// the suggested-follow page has no decline and ignores Back, so without this label
    /// the last step of the journey is a dead end.
    journey_done: Option<LabelMatch>,
    /// The `View folded comments` control, in `text`. `None` means a folded parent stays
    /// unreachable on this build, which is a refusal rather than a wrong reply.
    folded_comments: Option<LabelMatch>,
    /// Matched on the node's rendered `text`, not its `content-desc`: measured as a
    /// plain `TextView` reading `Ảnh` beside the caption, with no description at all.
    photo_badge: Option<LabelMatch>,
    like: Option<LabelMatch>,
    liked: Option<LabelMatch>,
    comments: Option<LabelMatch>,
    share: Option<LabelMatch>,
    bookmark: Option<LabelMatch>,
    /// Follow the author. Embeds the author name, and **the trailing space is
    /// load-bearing**.
    ///
    /// Measured the hard way on an SM-N950F, 11/08/2026: the feed's own tab row carries
    /// `content-desc="Đã follow"` ("Following"), and uiautomator's `descriptionContains`
    /// is **case-insensitive**, so a plain `Contains("Follow")` matched the *tab*. Two
    /// consequences, both real: reading the author name off the rail returned `Đã follow`,
    /// and the nurture loop's follow action would have tapped the tab and switched feeds
    /// instead of following anybody.
    ///
    /// Every measured value is `Follow <name>`, so requiring the space separates the two
    /// without needing a second catalogue entry.
    follow: Option<LabelMatch>,
    live_room: Option<LabelMatch>,
    /// The per-comment Reply button.
    ///
    /// Unlike [`Self::comment_send`] this **is** a translation, so it is
    /// language-dependent and update-stable — the opposite trade. It is also not
    /// unique: every comment row has one, and picking the wrong one posts the reply
    /// under a stranger's comment. Match it, then choose by geometry
    /// (`crate::interaction_hierarchy`).
    /// The drawer's Send button **when this build renders it as a string**.
    ///
    /// Normally version-keyed and absent here — see the module docs. But `keyed by
    /// version` describes the *reason*, not a law: 46.3.3 and 46.4.3 leave the control as
    /// an unresolved `@2131…` reference, which changes on every rebuild, while 38.3.2
    /// renders `Post comment`. A string is language-keyed by nature, so writing it into
    /// the version table would have told a Vietnamese 38.3.2 phone to look for English.
    ///
    /// [`TikTokControls::label`] takes the resource id when there is one and this
    /// otherwise, so a build that has both keeps the id — which is the one that cannot be
    /// wrong about the language.
    comment_send: Option<LabelMatch>,
    comment_reply: Option<LabelMatch>,
    /// The composer opener in the bottom bar.
    composer_open: Option<LabelMatch>,
    composer_shutter: Option<LabelMatch>,
    composer_caption: Option<LabelMatch>,
    /// The gallery picker's own controls.
    ///
    /// Measured 11/08/2026 on both fleet phones. What is **not** here, and cannot be:
    /// the composer's *gallery entry* is an unlabelled `FrameLayout` with neither
    /// `content-desc` nor `text`, so it has to be found by geometry — and the image grid
    /// cells are unlabelled too. Everything a label *can* identify is here; the rest is
    /// documented in AGENTS.md §9.10 rather than guessed at.
    picker_album_menu: Option<LabelMatch>,
    picker_tab_all: Option<LabelMatch>,
    picker_tab_photos: Option<LabelMatch>,
    picker_multi_select: Option<LabelMatch>,
    picker_next: Option<LabelMatch>,
    /// The author's profile link on the rail — see [`TikTokControl::AuthorProfileLink`].
    /// `None` means this build has no second source for the author name and a followed post
    /// still cannot be told apart from any other.
    author_profile_link: Option<LabelMatch>,
    /// The like control that states a total — see [`TikTokControl::LikeCount`].
    like_count: Option<LabelMatch>,
    /// Our own profile tab. `Exact` when measured — see [`TikTokControl::ProfileTab`].
    profile_tab: Option<LabelMatch>,
    composer_next: Option<LabelMatch>,
    post_button: Option<LabelMatch>,
    post_delete_menu: Option<LabelMatch>,
    post_delete: Option<LabelMatch>,
    post_delete_confirm: Option<LabelMatch>,
}

impl TikTokLabels {
    /// The translated label for a control, or `None` when unmeasured.
    ///
    /// Deliberately **not public**: [`TikTokControl::CommentSend`] is not in this
    /// table, and a caller reaching in here would get `None` for it and read that as
    /// "this build has no Send button". [`controls_for`] is the door.
    /// This build's reply-control string, for callers that must tell an author label
    /// apart from a reply button without resolving a whole control set.
    ///
    /// Needed because the author candidates arrive as a widget-class sweep, and the
    /// measured reply control is the same widget class as an author name.
    pub fn reply_label(&self) -> Option<&'static str> {
        self.comment_reply.map(|label| label.value())
    }

    fn translated(&self, control: TikTokControl) -> Option<LabelMatch> {
        match control {
            TikTokControl::FeedTab => self.feed_tab,
            TikTokControl::HomeTab => self.home_tab,
            TikTokControl::SoundLink => self.sound_link,
            TikTokControl::DialogDismiss => self.dialog_dismiss,
            TikTokControl::JourneySkip => self.journey_skip,
            TikTokControl::JourneyDone => self.journey_done,
            TikTokControl::FoldedComments => self.folded_comments,
            TikTokControl::PhotoBadge => self.photo_badge,
            TikTokControl::Like => self.like,
            TikTokControl::Liked => self.liked,
            TikTokControl::Comments => self.comments,
            TikTokControl::Share => self.share,
            TikTokControl::Bookmark => self.bookmark,
            TikTokControl::Follow => self.follow,
            TikTokControl::LiveRoom => self.live_room,
            TikTokControl::CommentReply => self.comment_reply,
            TikTokControl::ComposerOpen => self.composer_open,
            TikTokControl::ComposerShutter => self.composer_shutter,
            TikTokControl::PickerAlbumMenu => self.picker_album_menu,
            TikTokControl::PickerTabAll => self.picker_tab_all,
            TikTokControl::PickerTabPhotos => self.picker_tab_photos,
            TikTokControl::PickerMultiSelect => self.picker_multi_select,
            TikTokControl::PickerNext => self.picker_next,
            TikTokControl::ProfileTab => self.profile_tab,
            TikTokControl::AuthorProfileLink => self.author_profile_link,
            TikTokControl::LikeCount => self.like_count,
            TikTokControl::ComposerCaption => self.composer_caption,
            TikTokControl::ComposerNext => self.composer_next,
            TikTokControl::PostButton => self.post_button,
            TikTokControl::PostDeleteMenu => self.post_delete_menu,
            TikTokControl::PostDelete => self.post_delete,
            TikTokControl::PostDeleteConfirm => self.post_delete_confirm,
            // Only for a build that renders it as text; the `@2131…` builds carry it in
            // the version table and are resolved before this is ever reached.
            TikTokControl::CommentSend => self.comment_send,
        }
    }
}

/// Labels that are unresolved Android resource references, keyed by app version.
///
/// One entry means somebody opened that control on a build reporting that `versionName` and
/// copied the id out. A version with no entry makes those controls [`None`] — the flow refuses
/// rather than tapping an id that may since have been reassigned to another button.
///
/// **`versionName` is a weaker key than it looks, and this is deliberate.** `versionName` plus
/// `versionCode` is a much better lookup key than the name alone — [`parse_version_code`]
/// exists, and a [`crate::DeviceCapabilitySnapshot`] records both, because a hot fix can ship
/// under an unchanged `versionName`. It is still **not a build identity**: both numbers are
/// declared by the app, and a rebuilt or sideloaded APK can reuse either. Only a hash over the
/// package (and its splits) identifies a build. This table keys on the name alone, so a phone
/// running a different `38.3.2` matches a row measured on some other `38.3.2` and is handed ids
/// that may have moved.
///
/// The alternative was a `version_code` field on every row set to `None`, because the fleet's
/// dumps were taken before the code was being recorded and nobody can now say which build each
/// row came from. Such a field is a real check **only if it fails closed** — a row with no code
/// treated as unmeasured — and that would make `comment_send` refuse on all twenty phones until
/// somebody re-measures them, which stops the nurture path this week to reduce a risk that has
/// not yet been observed once. A field that instead falls *through* when it is `None` is not a
/// check at all; it is the appearance of one.
///
/// So the key stays as measured, and the exposure is written down here instead:
///
/// * `picker_album_menu` is protected downstream: the publish path taps the pill and then
///   **reads the album name back** ([`crate::tiktok_composer`]), so a reassigned id refuses
///   before any image is chosen.
/// * `comment_send` is **not** protected that way, and an earlier version of this note claiming
///   every resource-keyed control sits before something irreversible was simply wrong. That
///   control *is* the Send button: [`crate::tiktok_drawer`] locates it and taps it, and what
///   follows is a comment on somebody else's video from a real account. A reassigned id here is
///   a wrong tap on a live screen, not a refusal.
/// * Re-measuring is the fix, and it is cheap: [`parse_version_code`] already runs on the
///   fleet, so a row measured from today on can record the code it was read at and this note
///   can shrink to the rows that still cannot.
#[derive(Debug, Clone, Copy)]
pub struct TikTokResourceLabels {
    pub package: &'static str,
    /// Exact `versionName`, as `dumpsys package <pkg> | grep versionName` prints it.
    ///
    /// Not a build identity on its own — see the note on this struct.
    pub app_version: &'static str,
    pub measured_on: &'static str,
    comment_send: Option<LabelMatch>,
    /// The picker's album pill — see [`LabelMatch::ResourceId`] for why this control is here
    /// and not in a language set.
    picker_album_menu: Option<LabelMatch>,
}

impl TikTokResourceLabels {
    fn resource(&self, control: TikTokControl) -> Option<LabelMatch> {
        match control {
            TikTokControl::CommentSend => self.comment_send,
            TikTokControl::PickerAlbumMenu => self.picker_album_menu,
            _ => None,
        }
    }
}

/// Every resource-id set that has been read off a device, by app version.
pub const TIKTOK_RESOURCE_SETS: &[TikTokResourceLabels] = &[
    // The four `com.zhiliaoapp.musically` phones on this farm. Both versions here leave the
    // Send button as an unresolved reference, the way 46.3.3 and 46.4.3 do — unlike
    // `trill` 38.3.2, which renders it and is therefore described by language instead.
    //
    // Identified by the contract rather than by position: of the four `@2131…` controls in
    // this drawer, `@2131823247` (`id/cx0`, [911,1305][1048,1389]) is the only one whose
    // `enabled` is **false with the field empty and true once it holds text**. The other
    // three stay enabled throughout, and the probe's own guess — the first thing to appear
    // alongside the text — picked an emoji tile.
    // **`trill` 38.3.2 — the build sixteen of the twenty phones run.** Its comment Send
    // button resolves to a rendered string, so `comment_send` stays `None` here and
    // `label()` falls through to the language set. The album pill is the opposite case: it
    // has no usable string at all, so the id is the only way to reach it.
    TikTokResourceLabels {
        package: "com.ss.android.ugc.trill",
        app_version: "38.3.2",
        measured_on: "SM-G950F 98895a3355424e484f, Android 9, 29/08/2026 (example label_scout)",
        // Rendered on this build, so the language set describes it. See `label()`.
        comment_send: None,
        // `TextView`, `resource-id=com.ss.android.ugc.trill:id/snr`, `[483,115][543,172]`,
        // reading `All` at the moment it was measured — which is exactly why the id and not
        // the text: that string is the album currently showing, and it becomes the campaign's
        // own import id one tap later.
        //
        // Distinct from the media-type tab row directly below it (`…:id/slq`), whose leftmost
        // entry also reads `All`. Two nodes, one string, different jobs — so `Text("All")`
        // could never have picked between them either.
        // **`:id/snr`, not `snr`.** The driver turns this into the Java regex `.*<suffix>`,
        // so a bare three-character suffix also matches `…:id/asnr` — and an obfuscated build
        // names controls with exactly those. The `:id/` keeps the match on a resource-id
        // boundary, which is the difference between naming a control and naming a substring.
        picker_album_menu: Some(LabelMatch::ResourceId(":id/snr")),
    },
    TikTokResourceLabels {
        package: "com.zhiliaoapp.musically",
        app_version: "46.2.1",
        measured_on: "SM-G950F ce0517152c898c6f0d, Android 9, 18/08/2026 (probe --measure-comment)",
        comment_send: Some(LabelMatch::Exact("@2131823247")),
        picker_album_menu: None,
    },
    TikTokResourceLabels {
        package: "com.zhiliaoapp.musically",
        app_version: "46.2.42",
        measured_on: "SM-G950F ce0517155ab38c390d, Android 9, 18/08/2026 (probe --measure-comment)",
        // The same id as 46.2.1, measured separately rather than assumed — and worth an
        // entry of its own even so. The id moving between 46.3.3 and 46.4.3 is what this
        // table exists for; the id *not* moving between two other versions is not evidence
        // that it never does, and a lookup keyed by version cannot guess.
        comment_send: Some(LabelMatch::Exact("@2131823247")),
        picker_album_menu: None,
    },
    TikTokResourceLabels {
        package: "com.ss.android.ugc.trill",
        app_version: "46.3.3",
        measured_on: "Redmi Note 12, Android 15, 10/08/2026 (probe --measure-comment)",
        // `android.widget.Button` at [904,1379][1047,1467] whose `enabled` went
        // false -> true the moment the field held text.
        comment_send: Some(LabelMatch::Exact("@2131823284")),
        picker_album_menu: None,
    },
    TikTokResourceLabels {
        package: "com.ss.android.ugc.trill",
        app_version: "46.4.3",
        measured_on: "SM-N950F, Android 8.0, 11/08/2026 (probe --measure-comment)",
        // Same contract, different id — this pair is the whole reason for the split.
        // `android.widget.Button` at [911,1175][1048,1259]: `enabled=false` with the
        // field empty *and* while focused, `enabled=true` after typing. The 46.3.3 id
        // `@2131823284` does not appear anywhere in this build's tree, so a
        // language-keyed lookup would have refused a working phone.
        comment_send: Some(LabelMatch::Exact("@2131823293")),
        picker_album_menu: None,
    },
];

/// The labels for one device: translations by language, resource ids by app version.
///
/// The only way to read a label. It is `Copy` and cheap — two references.
#[derive(Debug, Clone, Copy)]
pub struct TikTokControls {
    translated: &'static TikTokLabels,
    resources: Option<&'static TikTokResourceLabels>,
}

/// A set with nothing measured, for the tests that check what a refusal does.
///
/// Exists because the refusal paths used to be exercised by pointing at a real catalogue
/// entry that happened to be missing the control — and then the control got measured, and
/// the test stopped testing anything it was named for. Measuring a label should never
/// silently delete a refusal's only coverage.
///
/// `#[cfg(test)]`: nothing in the product may reach for a set that refuses everything.
#[cfg(test)]
pub(crate) fn nothing_measured() -> TikTokControls {
    static NOTHING: TikTokLabels = TikTokLabels {
        package: "com.example.unmeasured",
        language: "zz",
        measured_on: "nothing — this set exists to be refused",
        measured_app_version: "",
        feed_tab: None,
        home_tab: None,
        dialog_dismiss: None,
        journey_skip: None,
        journey_done: None,
        // Never seen on this build. Absent means a folded parent is refused rather than
        // replied to blind — measure it with `label_scout <serial> --no-launch` while the
        // control is on screen; it is in `text`.
        folded_comments: None,
        sound_link: None,
        photo_badge: None,
        like: None,
        liked: None,
        comments: None,
        share: None,
        bookmark: None,
        follow: None,
        live_room: None,
        comment_send: None,
        comment_reply: None,
        composer_open: None,
        composer_shutter: None,
        picker_album_menu: None,
        picker_tab_all: None,
        picker_tab_photos: None,
        picker_multi_select: None,
        picker_next: None,
        author_profile_link: None,
        like_count: None,
        profile_tab: None,
        composer_caption: None,
        composer_next: None,
        post_button: None,
        post_delete_menu: None,
        post_delete: None,
        post_delete_confirm: None,
    };
    TikTokControls {
        translated: &NOTHING,
        resources: None,
    }
}

/// The label set behind [`every_publish_control_measured`].
///
/// At module scope so a second constructor can pair it with **no** resource table — which is
/// the only way to express "a language set that does carry an album-pill string", the case a
/// fallback would resolve and every real set cannot.
#[cfg(test)]
static ALL: TikTokLabels = TikTokLabels {
    package: "com.example.fully-measured",
    language: "zz",
    measured_on: "nothing — this set exists so the success path has a fixture",
    measured_app_version: "",
    feed_tab: Some(LabelMatch::Exact("fixture-feed-tab")),
    home_tab: Some(LabelMatch::Exact("fixture-home-tab")),
    dialog_dismiss: None,
    journey_skip: None,
    journey_done: None,
    folded_comments: None,
    sound_link: None,
    photo_badge: None,
    like: None,
    liked: None,
    comments: None,
    share: Some(LabelMatch::Exact("fixture-share")),
    bookmark: None,
    follow: None,
    live_room: None,
    comment_send: None,
    comment_reply: None,
    composer_open: Some(LabelMatch::Exact("fixture-composer-open")),
    composer_shutter: Some(LabelMatch::Exact("fixture-shutter")),
    // **A string, deliberately, and the resolver must never use it.**
    picker_album_menu: Some(LabelMatch::Text("fixture-album-menu")),
    picker_tab_all: Some(LabelMatch::Text("fixture-tab-all")),
    picker_tab_photos: Some(LabelMatch::Text("fixture-tab-photos")),
    picker_multi_select: Some(LabelMatch::Text("fixture-multi-select")),
    picker_next: Some(LabelMatch::Text("fixture-picker-next")),
    author_profile_link: None,
    like_count: None,
    profile_tab: None,
    composer_caption: Some(LabelMatch::ClassName("fixture-caption")),
    composer_next: Some(LabelMatch::Text("fixture-edit-next")),
    post_button: Some(LabelMatch::Text("fixture-post")),
    post_delete_menu: None,
    post_delete: None,
    post_delete_confirm: None,
};

/// A set where every control the publish path needs **is** measured.
///
/// The mirror of [`nothing_measured`], and it exists for the mirror reason. That one
/// keeps a refusal's coverage alive after a label gets measured; this one keeps the
/// *success* path testable while the real catalogue still refuses. Today no shipped set
/// resolves a composer plan — `composer_next` and `post_button` have never been read off
/// any phone — so without this the only reachable branch of
/// `tiktok_composer::ComposerPlan::resolve` would be the error, and every step past it
/// would be untested code that first runs on a live account.
///
/// The strings are **not** measurements and must never be copied into a real set: they
/// are named so that anything matching them on a real screen would be a coincidence.
///
/// `#[cfg(test)]`: nothing in the product may reach for a set nobody measured.
#[cfg(test)]
pub(crate) fn every_publish_control_measured() -> TikTokControls {
    // The album pill resolves through `TIKTOK_RESOURCE_SETS` only — see
    // `LabelMatch::ResourceId` — so a fixture claiming "everything measured" needs a resource
    // set as well as a language set, or the one control that cannot be a string stays absent.
    static ALL_RESOURCES: TikTokResourceLabels = TikTokResourceLabels {
        package: "com.example.fully-measured",
        app_version: "0",
        measured_on: "nothing — this set exists so the success path has a fixture",
        comment_send: None,
        picker_album_menu: Some(LabelMatch::ResourceId("fixture-album-menu")),
    };
    TikTokControls {
        translated: &ALL,
        resources: Some(&ALL_RESOURCES),
    }
}

/// The same language set with **no resource table at all**.
///
/// Its language row carries a string for the album pill, which the resolver must ignore. That
/// is the only shape that can catch a fallback being put back: every *real* set has `None`
/// there, so a sweep over the catalogue stays green whether the fallback exists or not.
#[cfg(test)]
pub(crate) fn an_album_pill_string_the_resolver_must_ignore() -> TikTokControls {
    TikTokControls {
        translated: &ALL,
        resources: None,
    }
}

/// The same fixture with the Post button unmeasured, which is the state every real
/// build is in today.
///
/// Kept as its own helper rather than a mutated copy because `TikTokControls` holds a
/// `&'static`, and because the *distinction* is the thing under test: a build can be
/// driven as far as the edit step and still refuse to publish.
#[cfg(test)]
pub(crate) fn every_publish_control_but_post_measured() -> TikTokControls {
    // Written out rather than derived from the fixture above: a `static` needs a const
    // initialiser, and functional-update syntax is not one.
    static NO_POST: TikTokLabels = TikTokLabels {
        package: "com.example.no-post-button",
        language: "zz",
        measured_on: "nothing — this set exists so the pre-publish steps have a fixture",
        measured_app_version: "",
        feed_tab: Some(LabelMatch::Exact("fixture-feed-tab")),
        home_tab: Some(LabelMatch::Exact("fixture-home-tab")),
        dialog_dismiss: None,
        journey_skip: None,
        journey_done: None,
        folded_comments: None,
        sound_link: None,
        photo_badge: None,
        like: None,
        liked: None,
        comments: None,
        share: Some(LabelMatch::Exact("fixture-share")),
        bookmark: None,
        follow: None,
        live_room: None,
        comment_send: None,
        comment_reply: None,
        composer_open: Some(LabelMatch::Exact("fixture-composer-open")),
        composer_shutter: Some(LabelMatch::Exact("fixture-shutter")),
        picker_album_menu: Some(LabelMatch::Text("fixture-album-menu")),
        picker_tab_all: Some(LabelMatch::Text("fixture-tab-all")),
        picker_tab_photos: Some(LabelMatch::Text("fixture-tab-photos")),
        picker_multi_select: Some(LabelMatch::Text("fixture-multi-select")),
        picker_next: Some(LabelMatch::Text("fixture-picker-next")),
        author_profile_link: None,
        like_count: None,
        profile_tab: None,
        // A class, because that is what this control's contract allows — a `Text` fixture
        // trained every composer test on the one strategy the documentation forbids.
        composer_caption: Some(LabelMatch::ClassName("fixture-caption")),
        composer_next: Some(LabelMatch::Text("fixture-edit-next")),
        post_button: None,
        post_delete_menu: None,
        post_delete: None,
        post_delete_confirm: None,
    };
    static NO_POST_RESOURCES: TikTokResourceLabels = TikTokResourceLabels {
        package: "com.example.no-post-button",
        app_version: "0",
        measured_on: "nothing — this set exists so the pre-publish steps have a fixture",
        comment_send: None,
        picker_album_menu: Some(LabelMatch::ResourceId("fixture-album-menu")),
    };
    TikTokControls {
        translated: &NO_POST,
        resources: Some(&NO_POST_RESOURCES),
    }
}

/// The publish tail measured **except the caption field**.
///
/// A third fixture rather than a reuse, because the test that claimed to cover this state used
/// the no-Post one — its assertion checked `PostButton`, and no test exercised a missing
/// caption at all. A build with a measured edit step and Post button but no caption field is
/// exactly the state that could publish a bare carousel.
///
/// `#[cfg(test)]`: nothing in the product may reach for a set nobody measured.
#[cfg(test)]
pub(crate) fn every_publish_control_but_caption_measured() -> TikTokControls {
    static NO_CAPTION: TikTokLabels = TikTokLabels {
        package: "com.example.no-caption-field",
        language: "zz",
        measured_on: "nothing — this set exists so the missing-caption state has a fixture",
        measured_app_version: "",
        feed_tab: Some(LabelMatch::Exact("fixture-feed-tab")),
        home_tab: Some(LabelMatch::Exact("fixture-home-tab")),
        dialog_dismiss: None,
        journey_skip: None,
        journey_done: None,
        folded_comments: None,
        sound_link: None,
        photo_badge: None,
        like: None,
        liked: None,
        comments: None,
        share: Some(LabelMatch::Exact("fixture-share")),
        bookmark: None,
        follow: None,
        live_room: None,
        comment_send: None,
        comment_reply: None,
        composer_open: Some(LabelMatch::Exact("fixture-composer-open")),
        composer_shutter: Some(LabelMatch::Exact("fixture-shutter")),
        picker_album_menu: None,
        picker_tab_all: Some(LabelMatch::Text("fixture-tab-all")),
        picker_tab_photos: Some(LabelMatch::Text("fixture-tab-photos")),
        picker_multi_select: Some(LabelMatch::Text("fixture-multi-select")),
        picker_next: Some(LabelMatch::Text("fixture-picker-next")),
        author_profile_link: None,
        like_count: None,
        profile_tab: None,
        composer_caption: None,
        composer_next: Some(LabelMatch::Text("fixture-edit-next")),
        post_button: Some(LabelMatch::Text("fixture-post")),
        post_delete_menu: None,
        post_delete: None,
        post_delete_confirm: None,
    };
    static NO_CAPTION_RESOURCES: TikTokResourceLabels = TikTokResourceLabels {
        package: "com.example.no-caption-field",
        app_version: "0",
        measured_on: "nothing — this set exists so the missing-caption state has a fixture",
        comment_send: None,
        picker_album_menu: Some(LabelMatch::ResourceId("fixture-album-menu")),
    };
    TikTokControls {
        translated: &NO_CAPTION,
        resources: Some(&NO_CAPTION_RESOURCES),
    }
}

impl TikTokControls {
    /// The label for a control, or `None` when it was never measured for this
    /// device. `None` means refuse — do not substitute another language or version.
    pub fn label(&self, control: TikTokControl) -> Option<LabelMatch> {
        match control {
            // The resource id wins when this build has one: an id is language-proof, and a
            // string is not. Falling through to the translation is what lets a build that
            // *resolved* the reference — 38.3.2 renders `Post comment` — be described at all,
            // rather than refusing every phone in the fleet because no `@2131…` was found.
            // Two controls read the resource table first, for opposite reasons.
            //
            // `CommentSend` falls **through** to the translation: a build that resolved the
            // reference — 38.3.2 renders `Post comment` — must still be describable, rather
            // than refusing every phone in the fleet because no `@2131…` was found.
            //
            // `PickerAlbumMenu` deliberately does **not** have a translation to fall through
            // to, on any set, and it must not grow one: the pill's text is the album it is
            // currently showing, so a string there names a value that changes the moment it is
            // used — and `All` is ambiguous with the media-type tab besides. The id or
            // nothing.
            TikTokControl::CommentSend => self
                .resources
                .and_then(|set| set.resource(control))
                .or_else(|| self.translated.translated(control)),
            TikTokControl::PickerAlbumMenu => self.resources.and_then(|set| set.resource(control)),
            other => self.translated.translated(other),
        }
    }

    pub fn package(&self) -> &'static str {
        self.translated.package
    }

    pub fn language(&self) -> &'static str {
        self.translated.language
    }

    /// Where the translations came from.
    pub fn measured_on(&self) -> &'static str {
        self.translated.measured_on
    }

    /// The app version whose resource ids are in use, or `None` when this device's
    /// version has never been measured.
    pub fn resource_version(&self) -> Option<&'static str> {
        self.resources.map(|set| set.app_version)
    }

    /// One line of provenance for a session log, naming what actually resolved.
    ///
    /// **This used to warn on exactly the phones that had nothing wrong.** The old version
    /// branched on whether a *resource set matched*, not on whether anything was missing —
    /// so it printed "CHƯA đo resource id cho phiên bản app này" whenever
    /// [`TIKTOK_RESOURCE_SETS`] had no entry for the device's version, regardless of whether
    /// that build needs a resource id at all.
    ///
    /// Measured on this farm, 23/08/2026, and the reading was inverted: the eleven
    /// `trill` 38.3.2 phones got the alarming line, because that table has no 38.3.2 entry —
    /// while `trill/en` carries `comment_send: Some("Post comment")` and therefore needs no
    /// id, and [`Self::label`] resolves it from the translation. The three `musically` phones
    /// got the reassuring version line, and they are the ones whose Send button is an
    /// unresolved `@2131…` that *only* a matching resource set can name. The loudest warning
    /// in the log was pointed at the healthy majority.
    ///
    /// So it now reports the Send button by **what resolved it**, which is the only
    /// version-keyed control there is ([`TikTokResourceLabels::resource`] matches nothing
    /// else). The genuine failure — no id and no translation — keeps a warning, and it is now
    /// the only case that gets one.
    ///
    /// This is also the one reader of [`TikTokLabels::measured_app_version`]. Until now that
    /// field was written for three label sets and read by no production code, while its own
    /// doc called itself "a note for the next reader" — a note nobody was shown. It belongs
    /// here: translations are applied to *any* version of a (package, language) pair, so the
    /// version they were actually read on is a real caveat about every phone using them.
    pub fn provenance(&self) -> String {
        let mut line = format!(
            "{} / {} ({})",
            self.package(),
            self.language(),
            self.measured_on()
        );
        let measured_app_version = self.translated.measured_app_version;
        if !measured_app_version.is_empty() {
            line.push_str(&format!(", nhãn đo trên app {measured_app_version}"));
        }
        // **Which Send button this build uses, decided by where the label actually came
        // from** — not by whether a resource set exists at all.
        //
        // The earlier version keyed on `self.resources.is_some()`, and that was true for as
        // long as the only reason to have a resource set *was* the Send button. It stopped
        // being true the moment a second control needed one: `trill` 38.3.2 now has a set for
        // the picker's album pill while its Send button is a rendered string, and the line
        // told the operator the opposite — pointing a debugging session at a resource id that
        // is not involved.
        let send_from_resource = self
            .resources
            .and_then(|resources| resources.resource(TikTokControl::CommentSend));
        match (
            send_from_resource,
            self.resources,
            self.translated.translated(TikTokControl::CommentSend),
        ) {
            (Some(_), Some(resources), _) => line.push_str(&format!(
                "; nút Gửi theo resource id, đo trên app {} ({})",
                resources.app_version, resources.measured_on
            )),
            // **Two different situations, and the old sentence claimed the safe one for
            // both.** A resource row that explicitly records `comment_send: None` is a
            // measurement: somebody looked at that build and found a rendered string. No
            // resource row at all is the absence of one — the string comes from the language
            // set, which was measured on some *other* version of this app, and whether this
            // one renders the button is unknown.
            (_, Some(_), Some(_)) => {
                line.push_str("; nút Gửi đọc theo chữ — đã đo trên đúng bản build này")
            }
            (_, None, Some(_)) => line.push_str(&format!(
                "; nút Gửi đọc theo chữ, nhưng CHƯA đo trên bản app này — chuỗi lấy từ bộ \
                 ngôn ngữ đo trên app {}",
                if measured_app_version.is_empty() {
                    "(không ghi)"
                } else {
                    measured_app_version
                }
            )),
            (_, _, None) => line.push_str(
                "; CHƯA đo được nút Gửi cho bản app này — phiên sẽ bỏ bình luận cả phiên",
            ),
        }
        line
    }
}

/// Resolve the labels for one device.
///
/// `None` only when the (package, language) pair itself is unmeasured — that is the
/// fail-closed base, because without translations nothing on the rail can be found. An
/// unmeasured `app_version` is *not* fatal: it leaves the resource-id controls absent,
/// so a build this project has never opened the drawer on can still like and read while
/// refusing to press a Send button it cannot identify.
///
/// Pass the `versionName` exactly as the device reports it; an empty string means "not
/// read", which is treated the same as unmeasured.
pub fn controls_for(package: &str, language: &str, app_version: &str) -> Option<TikTokControls> {
    let translated = labels_for(package, language)?;
    let app_version = app_version.trim();
    let resources = (!app_version.is_empty())
        .then(|| {
            TIKTOK_RESOURCE_SETS
                .iter()
                .find(|set| set.package == package && set.app_version == app_version)
        })
        .flatten();
    Some(TikTokControls {
        translated,
        resources,
    })
}

/// `versionName=46.4.3` out of `dumpsys package <pkg>`.
///
/// Takes the **first** match: `dumpsys` prints one per installed package record, and a
/// device with a system copy and an update prints both — the first is the active one.
pub fn parse_version_name(dumpsys: &str) -> Option<&str> {
    dumpsys.lines().find_map(|line| {
        let value = line.trim().strip_prefix("versionName=")?;
        let value = value.trim();
        (!value.is_empty()).then_some(value)
    })
}

/// The build number out of `dumpsys package <pkg>`.
///
/// The Android counterpart of an iOS `CFBundleVersion`, and read for the same reason: a
/// `DeviceCapabilitySnapshot` records a target app as name *and* build, because a
/// hot-fixed build can ship under an unchanged `versionName`.
///
/// Unlike `versionName` this does not sit alone on its line — every phone on this fleet
/// prints `versionCode=380302 minSdk=21 targetSdk=34` — so the value is taken up to the
/// first space rather than to the end of the line.
pub fn parse_version_code(dumpsys: &str) -> Option<&str> {
    dumpsys.lines().find_map(|line| {
        let at = line.find("versionCode=")? + "versionCode=".len();
        let value = line[at..].split_whitespace().next()?;
        (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())).then_some(value)
    })
}

/// Every label set that has actually been read off a device.
///
/// Keep this list honest: an entry means somebody dumped the accessibility tree on
/// that build in that language and copied the strings out. Two entries today.
pub const TIKTOK_LABEL_SETS: &[TikTokLabels] = &[
    // Global build, English UI. From the Galaxy S8+ fleet measurements recorded in
    // AGENTS.md §9 and `docs/ANDROID_PROBE_REPORT_2026-08-09.md`.
    TikTokLabels {
        package: "com.zhiliaoapp.musically",
        language: "en",
        measured_on: "Galaxy S8+ fleet, 09/08/2026 (docs/ANDROID_PROBE_REPORT_2026-08-09.md)",
        // Not recorded at the time; that is the gap `measured_app_version` closes.
        measured_app_version: "",
        feed_tab: Some(LabelMatch::Exact("For You")),
        // Read off the bottom bar on an SM-G950F on 18/08/2026, on a phone parked on its
        // Profile tab — the same `Home` the SEA build shows, which is why it is written
        // down rather than assumed from it.
        home_tab: Some(LabelMatch::Exact("Home")),
        // `Sound: Drops of Light by everyoneyusuke`, read off ce0517155ab38c390d's feed on
        // 18/08/2026. Matched on the prefix because the rest is the track and its author,
        // which is exactly the part that has to differ between two cards for a swipe to be
        // proved. Not the same string as the SEA build's `Original sound by`, which is why
        // each set is measured rather than shared.
        sound_link: Some(LabelMatch::Contains("Sound:")),
        // `Not now`, read as `text` with no `content-desc`, off ce0717171c2a64d50d held
        // behind "Turn on precise location" on 19/08/2026 — the same string and the same
        // attribute the SEA build carries, now measured here rather than assumed from there.
        //
        // The earlier note said this build's dialog had "no decline worth tapping", and that
        // was true of the dialog it happened to produce that day: "Get updates sent to your
        // email?" labels only its *accept*. It was a statement about one dialog, and it read
        // as a statement about the build. This one has a labelled decline, so the measured
        // decline is available again and Back stays as the fallback for the dialogs that
        // have none — which is exactly the split `await_feed` documents.
        //
        // Safe by construction: `Not now` declines. It cannot grant anything, which is the
        // property that made the email dialog's labelled button unusable.
        dialog_dismiss: Some(LabelMatch::Text("Not now")),
        // Not measured on `musically`. The three phones on this build were on the feed and
        // on splash when the journey was dumped on 23/08/2026, so there was nothing to
        // read — and the four phones that met an unmeasured dialog on 19/08/2026 are the
        // standing reminder that a string measured on one build is not a fact about
        // another. Copy the `trill/en` values here only after seeing them on a
        // `musically` phone's own journey page.
        journey_skip: None,
        journey_done: None,
        // Never seen on this build. Absent means a folded parent is refused rather than
        // replied to blind — measure it with `label_scout <serial> --no-launch` while the
        // control is on screen; it is in `text`.
        folded_comments: None,
        // **`Photo`, in `text`, `resource-id` `…:id/tv_label`, `clickable=false`.**
        //
        // Measured 23/08/2026 across **two** phones on 46.2.1 — ce0717171c2a64d50d (4 of 14
        // cards) and ce11171beb408a1501 (1 of 10) — by swiping the feed and dumping every
        // card. Five photo posts, all with the same node immediately to the right of
        // `…:id/title`, rendered as a `⧉ Photo` pill beside the author name and confirmed
        // by eye on three of them. **Nineteen non-photo cards carried no `Photo` node
        // anywhere**, including a LIVE shopping card and a video ad, so there were no false
        // positives to weigh against it.
        //
        // Deliberately more evidence than the `trill/en` badge got: that one was enabled off
        // a single screen on 18/08/2026 and had to be switched off the same day. Two phones
        // and a set of negative controls is the bar this file should have been held to then.
        //
        // **The y position moves, so this must be located and never tapped at a fraction.**
        // The five sightings sat at y 1332, 1566, 1698, 1704 and 1887 — the caption row
        // shifts with how much caption is expanded, and cards 13 and 14 of one sweep were
        // the *same post* at two different heights.
        //
        // **Measured on 46.2.1, and now on 46.2.42 as well.**
        //
        // 28/08/2026 on ce0517155ab38c390d, the farm's only 46.2.42 phone (surveyed 23/08/2026:
        // eleven `trill` 38.3.2, two `musically` 46.2.1, this one), locale `en`, via
        // `.claude/skills/run-riviu-managers-phone/hunt_badge_4642.ps1 -ForceStop`. The badge is
        // there and it is the same shape: **one** `android.widget.TextView`, `text="Photo"`,
        // `resource-id=com.zhiliaoapp.musically:id/zxh`, `bounds=[277,1544][370,1597]` — inside
        // the y range the 46.2.1 sightings spanned (1332/1566/1698/1704/1887). Nothing else in
        // the tree carries the word: `text` containing `Photo` matched exactly once. The kept
        // frame shows a real seven-slide carousel with the badge beside the author name, so the
        // claim is checkable by eye and not only by grep.
        //
        // The five other rail labels a feed card can carry were read off the same dump and all
        // match this set: `Like`, `Read or add comments. 1 comments`, `Share video. 3 shares`,
        // `Follow Bích Vân`, `Bích Vân profile`, `Like video. 20 likes`. The four that did not
        // appear are absent for state, not for build — the post was not liked, it was not a LIVE
        // card, and the comment drawer was closed.
        //
        // That closes the question an independent review raised: the concern was not that the
        // badge might be *absent* on this build (harmless — the phone just keeps not paging
        // carousels) but that it might be present meaning something else. It is not.
        //
        // Two things the run measured that are worth keeping. The card rendered **page dots, not
        // digits**, so it is one of the majority that no counter can page — consistent with 6 of
        // 10 photo posts on an earlier fleet sweep. And `uiautomator dump` failed with
        // `could not get idle state` on 21 of 22 rounds: a playing video never goes idle, which
        // is the same property that once nearly got this badge written down as absent.
        //
        // The network blocker is gone: that phone is on `VNPT Riviu Dalat_5G` now, 0% loss to
        // 1.1.1.1 at 54 ms and `www.tiktok.com` resolving. The old note said `Riviu 3 Ruijie 5G`
        // had no upstream, and that stays true of the SSID rather than of the phone.
        //
        // Read on a video montage first and nearly written down as "this build has no badge
        // at all": the card looped a ~10 s `SeekBar` and changed picture constantly, which
        // looks like a carousel and is not one. A `SeekBar` is **not** a photo signal —
        // measured on card 10 of the same sweep, a plain video with `Effect · 97` had one.
        photo_badge: Some(LabelMatch::Text("Photo")),
        like: Some(LabelMatch::Exact("Like")),
        liked: Some(LabelMatch::Exact("Video liked")),
        comments: Some(LabelMatch::Contains("comments")),
        // `Share video.  shares` on the same 18/08/2026 feed dump — the count sits between
        // the two words on a card with no shares yet, so only the prefix can be matched.
        share: Some(LabelMatch::Contains("Share video")),
        bookmark: None,
        follow: Some(LabelMatch::Contains("Follow ")),
        live_room: Some(LabelMatch::Contains("Tap to watch LIVE")),
        // Never measured on this build: the S8+ fleet work stopped before the
        // comment drawer was dumped.
        comment_send: None,
        // `Reply`, in `text`, one Button per comment row — measured on
        // ce0717171c2a64d50d, 18/08/2026, three rows at [286,927], [349,1140] and
        // [286,1763]. Same string as the SEA build in English, measured separately
        // rather than assumed from it: the two builds disagree about the sound strip and
        // about the Send button, so agreement here is a fact rather than a rule.
        comment_reply: Some(LabelMatch::Text("Reply")),
        composer_open: None,
        composer_shutter: None,
        // The S8+ fleet work never opened the composer on this build.
        picker_album_menu: None,
        picker_tab_all: None,
        picker_tab_photos: None,
        picker_multi_select: None,
        picker_next: None,
        // Measured 24/08/2026 on `ce11171beb408a1501` (musically, en) standing on
        // `.../@.lt.gi.mang.v/photo/7668947001618320660`: the rail reads
        // `Đà Lạt Gói Mang Về profile`, identical in shape to the `trill`/en build.
        author_profile_link: Some(LabelMatch::Contains(" profile")),
        // Measured 24/08/2026: `Like video. 22 likes`. `Contains("likes")` is unique on the
        // rail — `Like` and `Video liked` do not contain it.
        like_count: Some(LabelMatch::Contains("likes")),
        // Publish tail and the whole delete path: declared, not measured. `None` here is
        // the refusal — the composer stops before opening and the delete driver is not
        // offered at all, which is the only safe order for an action with no undo.
        profile_tab: None,
        composer_caption: None,
        composer_next: None,
        post_button: None,
        post_delete_menu: None,
        post_delete: None,
        post_delete_confirm: None,
    },
    // SEA build, Vietnamese UI. Read off a Redmi Note 12 (Android 15) on
    // 10/08/2026; see `docs/re/genfarmer/README.md` and AGENTS.md §9.
    TikTokLabels {
        package: "com.ss.android.ugc.trill",
        language: "vi",
        measured_on: "Redmi Note 12, Android 15, 10/08/2026",
        measured_app_version: "46.3.3",
        feed_tab: Some(LabelMatch::Exact("Đề xuất")),
        // Not a new measurement: the `composer_open` note below already records this
        // build's five bottom tabs as read off a real dump on 11/08/2026 —
        // `Trang chủ`, `Cửa hàng`, `Quay`, `Hộp thư`, `Hồ sơ`. This is the first of them.
        home_tab: Some(LabelMatch::Exact("Trang chủ")),
        // Unmeasured on the vi build: the 10/08 dump was not kept for this strip.
        sound_link: None,
        dialog_dismiss: None,
        // Unmeasured, and the language is the reason to be careful rather than to guess:
        // the journey page renders in the UI language, so a Vietnamese build shows
        // `Bỏ qua` / `Xong` rather than `Skip` / `Done`. Writing those two words down
        // from the dictionary is exactly what this table forbids — the Redmi that
        // produced this set is not part of the farm and has never shown the journey.
        journey_skip: None,
        journey_done: None,
        // Never seen on this build. Absent means a folded parent is refused rather than
        // replied to blind — measure it with `label_scout <serial> --no-launch` while the
        // control is on screen; it is in `text`.
        folded_comments: None,
        // Read off an SM-N950F on 12/08/2026, on photo cards in the For-You feed and
        // on a post page opened from a link — the same string on both.
        photo_badge: Some(LabelMatch::Text("Ảnh")),
        like: Some(LabelMatch::Exact("Thích")),
        // Read off a post that was liked and then unliked again by
        // `probe --measure-liked`, which is the only way to see this string. Worth
        // noting how far it is from a guess: not `Đã thích`, and the word order is
        // reversed relative to the English `Video liked`.
        liked: Some(LabelMatch::Exact("Đã thích video")),
        comments: Some(LabelMatch::Contains("bình luận")),
        share: Some(LabelMatch::Contains("Chia sẻ video")),
        bookmark: Some(LabelMatch::Contains("Yêu thích")),
        // This build keeps the English verb even in a Vietnamese UI — the reason
        // translations cannot be derived, only measured.
        follow: Some(LabelMatch::Contains("Follow ")),
        live_room: None,
        // Read off the saved drawer dump on 11/08/2026: three
        // `android.widget.Button` nodes with `text="Trả lời"`, `clickable=true` and an
        // **empty** `content-desc`, at x 315..419 — one per comment row, each sitting
        // below its own comment body and to the right of it.
        comment_send: None,
        comment_reply: Some(LabelMatch::Text("Trả lời")),
        // Read off the bottom bar on 11/08/2026: an `android.widget.Button`,
        // clickable, at x 432..648 y 2135 on a 1080x2400 screen — the middle of five
        // tabs (`Trang chủ`, `Cửa hàng`, this, `Hộp thư`, `Hồ sơ`).
        composer_open: Some(LabelMatch::Exact("Quay")),
        composer_shutter: None,
        // Read off the picker on 11/08/2026, on both fleet phones (Redmi Note 12 / app
        // 46.3.3 and SM-N950F / app 46.4.3 — the strings agree). The two tabs carry a
        // real `content-desc`; the album name and the two buttons carry only `text`,
        // which is why both attributes exist in `LabelMatch`.
        // **Was `Text("Gần đây")`, and that was wrong for the same reason the English set's
        // `All` was**: the pill shows the album *currently selected*, so `Gần đây` names the
        // default album and stops being true the moment the publish path chooses the
        // campaign's own. A locator that is correct only until it is used is not a locator.
        //
        // Left `None` rather than deleted: this control now resolves through
        // `TIKTOK_RESOURCE_SETS` only, and no resource id has been read off this build.
        picker_album_menu: None,
        picker_tab_all: Some(LabelMatch::Exact("Tất cả")),
        picker_tab_photos: Some(LabelMatch::Exact("Ảnh")),
        picker_multi_select: Some(LabelMatch::Text("Chọn nhiều")),
        // `clickable=false` until something is selected: an armed signal, not just a
        // button.
        picker_next: Some(LabelMatch::Text("Tiếp")),
        // Measured on a Redmi Note 12, 13/08/2026, from the bottom tab bar at y=2135:
        // `Trang chủ`, `Cửa hàng`, `Quay`, `Hộp thư`, `Hồ sơ`.
        //
        // `Exact`, and the hazard is no longer a prediction: the **same dump** carried
        // `content-desc="Hồ sơ Ánh đây"` — the author's profile link on the action rail —
        // beside `content-desc="Hồ sơ"`. `Contains("Hồ sơ")` matches both, and picking the
        // wrong one walks the delete path onto a stranger's profile.
        // The author's link on the action rail, the twin of the trap above: the same dump
        // carried `Hồ sơ Ánh đây` beside the bare `Hồ sơ`. The trailing space keeps this off
        // our own tab, which is what makes reading a name out of it safe.
        author_profile_link: Some(LabelMatch::Contains("Hồ sơ ")),
        // Not measured on a Vietnamese rail. `None` means a like threshold refuses on this
        // build instead of parsing a string nobody has read.
        like_count: None,
        profile_tab: Some(LabelMatch::Exact("Hồ sơ")),
        // The rest of the publish tail and the whole delete path: declared, not measured.
        // `None` is the refusal — the composer stops before opening and the delete driver
        // is not offered at all, which is the only safe order for an action with no undo.
        composer_caption: None,
        composer_next: None,
        post_button: None,
        post_delete_menu: None,
        post_delete: None,
        post_delete_confirm: None,
    },
    // SEA build, **English** UI. Sixteen of the eighteen phones on this fleet, and a pair
    // that had never been read — so every one of them refused to nurture with
    // "chưa đo nhãn TikTok cho com.ss.android.ugc.trill + ngôn ngữ en".
    //
    // Read off an SM-G955F (Android 9, app 38.3.2) on 18/08/2026 with
    // `cargo run -p riviu-android-driver --example label_scout`: once on the Profile tab
    // the phone happened to be parked on, and once on the For-You feed after tapping Home.
    // Everything below appeared in one of those two dumps; everything that did not is
    // `None`, which is this table's whole contract.
    TikTokLabels {
        package: "com.ss.android.ugc.trill",
        language: "en",
        measured_on: "SM-G955F, Android 9, 18/08/2026 (example label_scout)",
        measured_app_version: "38.3.2",
        feed_tab: Some(LabelMatch::Exact("For You")),
        // The bottom bar, read on the Profile screen: `Home`, `Shop`, `Create`, `Inbox`,
        // `Profile`. This is the one that gets a parked session back to the feed.
        home_tab: Some(LabelMatch::Exact("Home")),
        // `Original sound by Jacketkat` and `Original sound by BapMidnight`, read off two
        // different phones on 18/08/2026. Matched on the prefix and read for the whole
        // value — the creator's name is what makes two cards distinguishable.

        // **`Sound: <track> by <author>`** — 9 of 9 cards sampled on ce051715cb22c30403,
        // 18/08/2026, including a photo post. Matched on the prefix because the rest is
        // the track and its author, which is exactly the part that has to differ between
        // two cards for a swipe to be proved.
        //
        // This replaces `Original sound by`, which was written down this morning off a
        // single card and turned out to be the rare form: zero of the nine. It cost the
        // sound component of the fingerprint on almost every card, which is survivable —
        // comments and shares still differ — but it is exactly the case the sound was
        // added for, a low-engagement feed where both of those read the same.
        sound_link: Some(LabelMatch::Contains("Sound:")),
        // `Not now`, read as `text` with no `content-desc`, off an SM-G955U1 held behind
        // "Save login for next time?" on 18/08/2026.
        dialog_dismiss: Some(LabelMatch::Text("Not now")),
        // `Skip`, `android.widget.Button`, in `text` with no `content-desc`, read on
        // **five phones at once** on 23/08/2026 — 9889db374744474635, 98895a3355424e484f,
        // ce03171392f9390c01, ce031713dd735a1103 and ce051715ac247a3f01 — all sitting on
        // `NewUserJourneyActivity` behind "TikTok is better with friends!". Identical on
        // all five down to the resource id (`id/cxx`); only the bounds differed, which is
        // why this is located rather than tapped at a fraction.
        //
        // Exact, never `TextContains`: the very next screen's *title* is "Skip finding
        // Facebook friends?" and a substring match would return that `TextView` instead of
        // the button beside `Find friends`. See [`TikTokControl::JourneySkip`] for the
        // affirmative siblings this must never reach.
        journey_skip: Some(LabelMatch::Text("Skip")),
        // `Done`, `android.widget.Button` (`id/b9r`), in `text`, on the journey's
        // suggested-follow page. Measured on 9889db374744474635 the same day, including
        // the part that mattered: after tapping it the account still read **Following 1**
        // against five suggestions offered, so it dismisses rather than accepting. Back
        // does nothing on that page — the dump was byte-identical across one.
        journey_done: Some(LabelMatch::Text("Done")),
        // `View folded comments`, read as `text` with no `content-desc`, off
        // ce0417145199e0490c on 19/08/2026 — the phone was holding the post where three
        // farm accounts had just commented, and TikTok had folded the newest of them away.
        //
        // The fold is **progressive**, which is the part worth writing down: on the same
        // post, earlier in the same hour, two replies found their parent in the open list
        // without any of this. It is not a property of the post or of the build, it is what
        // happens once these accounts have commented on a post a few times.
        folded_comments: Some(LabelMatch::Text("View folded comments")),
        // `Photo` appears in `text` on this card, and the card is a photo post — but the
        // vi build's badge was confirmed on *two* devices and on a post page before being
        // written down, and one screen is not that. Left unmeasured: the cost is that a
        // carousel is never paged sideways, and the cost of being wrong is a sideways
        // swipe on a video, which opens the author's profile and walks the session off
        // the feed.
        // **Measured, switched off for a day, and back on now that the traversal works.**
        // The label was never in doubt: `Photo` in `text`, present on all three reads of a
        // photo card and absent from ten video cards read twice each, on ce051715cb22c30403,
        // 18/08/2026.
        //
        // What it switches on was. Enabling it the first time turned the carousel traversal
        // on for this build, and across nine phones **both** sessions that met a photo post
        // ended at zero videos while every session that met none watched normally. Same
        // trail each time: `gặp bài ảnh — vuốt ngang`, `đã xem 2/10 ảnh`, an unproven
        // vertical swipe, then a card with no action rail.
        //
        // The cause was the gesture, not this label and not the counter. TikTok's image
        // pager acts on a thrown finger and ignores a dragged one, and `plan_swipe` sends a
        // drag: bowed into the vertical axis, decelerating to a crawl, then held still
        // before the lift. The page did not turn, the counter read the same number twice,
        // and the loop concluded the post had ended. `swipe_slide` now sends
        // `TouchPointPlanner::plan_flick`, measured at 19 turns out of 19 against the old
        // gesture's 13 out of 40 — the numbers and the one-component-at-a-time method are
        // written down there.
        photo_badge: Some(LabelMatch::Text("Photo")),
        like: Some(LabelMatch::Exact("Like")),
        // Not seen: reading it needs a post to be liked and then unliked
        // (`probe --measure-liked`). The engine confirms a like without it — the
        // not-liked label is an exact match, so its disappearance is the proof.
        // Measured on ce051715081fe20f03, 18/08/2026, by liking one card and reading it
        // back: `Like video. 22 likes` became `Video liked` and the count went to 23.
        //
        // Absent, this build could not confirm a like *or* notice one. `Like` is on the
        // rail in **both** states — it is a separate node from the one that toggles — so
        // the fallback check, "the not-liked label went away", could never fire here and
        // every attempt reported `NotConfirmed`. Worse than the miscount: the
        // `AlreadyLiked` guard runs *before* the tap, so with nothing to recognise, the
        // loop tapped Like on a post it had already liked — which removes the like.
        liked: Some(LabelMatch::Exact("Video liked")),
        comments: Some(LabelMatch::Contains("comments")),
        share: Some(LabelMatch::Contains("Share video")),
        bookmark: Some(LabelMatch::Contains("Favorites")),
        // Absent from the measured screen, whose author was already followed. The vi
        // build keeps the English `Follow ` and this build very likely does too — which
        // is exactly the kind of "very likely" this table exists to refuse.
        // `Follow <author>`, read off six consecutive cards on ce051715cb22c30403,
        // 18/08/2026 — `Follow Cindy…`, `Follow University of Melbourne`, `Follow TM Su`.
        // Only on cards whose author is not followed yet, which is why it took scrolling
        // rather than one dump and why sixteen phones went without it: absent from the
        // first card anyone looked at is not absent from the build.
        //
        // The trailing space is load-bearing — it is what keeps this off the `Following`
        // tab, which is a different control on the same screen.
        follow: Some(LabelMatch::Contains("Follow ")),
        live_room: None,
        // `android.widget.Button` at [927,1310][1048,1384], `clickable=true`, which
        // appeared once the field held text — measured on ce051715cb22c30403,
        // 18/08/2026, with `probe --measure-comment`.
        //
        // Every phone on this farm runs 38.3.2, and the only measured versions were
        // 46.3.3 and 46.4.3 from two other handsets. So commenting could not work on any
        // of the twenty — whatever the operator set `comment_prob` to, and whatever AI
        // key was configured. The session said so each time, and the reason was this
        // one absent label rather than anything about the key.
        comment_send: Some(LabelMatch::Exact("Post comment")),
        // `Reply`, in `text` — the same attribute the Vietnamese build puts `Trả lời` in,
        // and one Button per comment row. Read off ce051715cb22c30403 on 18/08/2026 with
        // `probe --measure-comment`: author at [155,819][480,861], body at
        // [155,866][1048,933], and this at [242,944][334,986] — above/below exactly as
        // `locate_parent_in_elements` requires, well inside `AUTHOR_REACH` and
        // `REPLY_REACH`.
        //
        // Sixteen of the twenty phones on this farm run this build, and without this
        // every reply refused with `reply_control_unmeasured`. Threading was unreachable
        // on the whole fleet.
        comment_reply: Some(LabelMatch::Text("Reply")),
        // **Measured 29/08/2026 on SM-G950F `98895a3355424e484f`, app 38.3.2, locale `en-US`**
        // (`label_scout`, read-only). `android.widget.Button`, `content-desc="Create"`, empty
        // `text`, `clickable=true`, `bounds=[432,1929][648,2076]`,
        // `resource-id=com.ss.android.ugc.trill:id/jmh`. The bar is five 216x147 tabs on a
        // 1080x2076 usable area: `Home`, `Shop`, `Create`, `Inbox`, `Profile`.
        //
        // `Exact`, and the string occurs **exactly once in the whole tree** — 184 elements
        // checked, no other node carries `Create` in either `content-desc` or `text`. That
        // check is the point of writing it down: `Contains` on a five-tab bar is how a locator
        // starts matching a caption.
        //
        // Same shape as the Vietnamese build's `Quay`, and the same warning applies — this is a
        // measured string, not a translation to correct. It opens the camera; the gallery lives
        // inside, behind an unlabelled control.
        //
        // **Measuring this does not enable posting**, and that is deliberate: the five picker
        // controls below and the whole tail are still `None`, so `Composer::readiness` refuses
        // before anything opens. What it unblocks is the *measurement*: `probe
        // --measure-composer` and `--measure-gallery-entry` both begin by locating
        // `ComposerOpen` and bail without it, so the instrument for the next step was itself
        // locked behind this label.
        composer_open: Some(LabelMatch::Exact("Create")),
        // **Measured 29/08/2026, same session and same phone as `composer_open` above.**
        // `android.widget.Button`, `content-desc="Record video"`, `bounds=[375,1545][705,1875]`
        // — 330x330, horizontally centred on a 1080-wide screen.
        //
        // Catalogued because it anchors the thing that cannot be catalogued. The gallery
        // entry is an unlabelled `FrameLayout` at `[765,1590][1005,1830]`
        // (`resource-id=…:id/bos`), and the measurement that matters is the *relationship*:
        // it shares the shutter's vertical centre exactly (both at y=1710) and sits one
        // margin in from the right edge. Two other unlabelled circles sit to the shutter's
        // left, `resource-id=…:id/egr`, and they open the effects panel — which is what an
        // earlier note's "bottom-left" guess would have tapped.
        composer_shutter: Some(LabelMatch::Exact("Record video")),
        // **The picker, measured 29/08/2026 on SM-G950F `98895a3355424e484f`, app 38.3.2,
        // locale `en-US`, 1080x2220.** Reached by tapping `Create`, then the gallery entry
        // described below. Every string here is `text`; not one of these controls carries a
        // `content-desc`, which is the opposite split from the action rail on the same build.
        //
        // **The album menu is measured and still `None`, and that is not an oversight.** It is
        // the pill at the top of the picker, `TextView [483,115][543,172]`,
        // `resource-id=…:id/snr` — and its text is *the album currently showing*, `All` right
        // now and the campaign's `riviu-…` id once one is chosen. So a text locator for it is
        // wrong twice over: it names a value that changes the moment it is used, and `All`
        // **also** belongs to the media-type tab at `[151,255][209,312]` (`…:id/slq`), so the
        // string is ambiguous on screen before anything is chosen. This catalogue matches on
        // `text`/`content-desc` only, so the menu needs a locator this file cannot express;
        // until the driver has one, the publish path refuses here.
        picker_album_menu: None,
        // Ambiguous for the same reason, and left refusing for it: two nodes read `All`.
        picker_tab_all: None,
        // Unique on screen. The tab order differs from the Vietnamese build (All, Videos,
        // Photos here), which is why the tabs are measured per build rather than translated.
        picker_tab_photos: Some(LabelMatch::Text("Photos")),
        // `TextView [126,1937][422,1991]`, `…:id/k6p`, and `clickable=false` on the text node
        // itself — the clickable wrapper is its parent, the same shape the Vietnamese build had.
        picker_multi_select: Some(LabelMatch::Text("Select multiple")),
        // `TextView [552,1896][1044,2028]`, `…:id/q4g`.
        //
        // **The armed signal is `clickable`, and `enabled` is useless for it here.** Measured
        // both states on this phone:
        //
        // ```text
        //   nothing selected   clickable=false  enabled=true
        //   one image selected clickable=true   enabled=true
        // ```
        //
        // That matters beyond this label. `ElementBox` (`driver.rs`) carries `enabled` and not
        // `clickable`, and `AndroidUiSession::locate` reads exactly two attributes — so the
        // primitive this project has **cannot see this flag**, and an `await_armed` written
        // against `enabled` would read "armed" before a single image was chosen. The composer
        // driver needs `ElementBox` to grow `clickable` before it can prove a selection took.
        //
        // Also measured, and it closes off a tempting alternative: with one image selected the
        // grid renders **no per-cell numeral** — no single-digit text appears anywhere on
        // screen. So slide *order* cannot be read back from the picker on this build, and grid
        // scrolling cannot be made safe by re-identifying cells after a flick.
        picker_next: Some(LabelMatch::Text("Next")),
        // Measured 24/08/2026 — `Đà Lạt Gói Mang Về profile`, and it survives following the
        // creator while `Follow ` does not. English puts the name first, so the needle is a
        // suffix; the leading space keeps it off the bare `Profile` tab.
        author_profile_link: Some(LabelMatch::Contains(" profile")),
        // Measured 24/08/2026: `Like video. 22 likes`. `Contains("likes")` is unique on the
        // rail — `Like` and `Video liked` do not contain it.
        like_count: Some(LabelMatch::Contains("likes")),
        profile_tab: Some(LabelMatch::Exact("Profile")),
        composer_caption: None,
        composer_next: None,
        post_button: None,
        post_delete_menu: None,
        post_delete: None,
        post_delete_confirm: None,
    },
];

/// The label set for an app build and UI language, or `None` to refuse.
pub fn labels_for(package: &str, language: &str) -> Option<&'static TikTokLabels> {
    let language = normalise_language(language);
    TIKTOK_LABEL_SETS
        .iter()
        .find(|set| set.package == package && set.language == language)
}

/// Reduce a locale tag to the language subtag: `vi-VN` and `vi_VN` both mean `vi`.
///
/// Region is dropped on purpose. TikTok translates by language; keying on region
/// would refuse `vi-VN` on a set measured as plain `vi` for no reason.
pub fn normalise_language(locale: &str) -> String {
    locale
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Redmi Note 12's build, language and app version.
    fn redmi() -> TikTokControls {
        controls_for("com.ss.android.ugc.trill", "vi-VN", "46.3.3").expect("measured set")
    }

    #[test]
    fn an_unmeasured_build_or_language_refuses() {
        // The whole point: no silent fallback to another language.
        assert!(controls_for("com.ss.android.ugc.trill", "th", "46.3.3").is_none());
        assert!(controls_for("com.example.unknown", "vi", "46.3.3").is_none());
        assert!(controls_for("", "", "").is_none());
    }

    #[test]
    fn the_photo_badge_is_measured_on_the_build_that_pages_carousels() {
        // Matched on `text`, because that is where the badge lives — it has no
        // `content-desc` at all. Reading it as a description finds nothing.
        let badge = redmi().label(TikTokControl::PhotoBadge).expect("badge");
        assert_eq!(badge, LabelMatch::Text("Ảnh"));
        assert!(
            badge.is_exact(),
            "the badge is the whole string, not a substring"
        );
    }

    #[test]
    fn a_build_with_no_measured_badge_pages_no_carousels() {
        // The rule this test is really about — an unmeasured badge must not be swiped
        // sideways, because a sideways swipe on a *video* card is TikTok's
        // open-the-author's-profile gesture. It is stated against the set that refuses
        // everything rather than against `musically/en`, which was measured on 23/08/2026
        // and no longer serves as an example of an unmeasured build.
        assert_eq!(nothing_measured().label(TikTokControl::PhotoBadge), None);
        // And the shape that makes it a catalogued label rather than a heuristic: exact, in
        // `text`, so it cannot half-match a caption.
        let badge = controls_for("com.zhiliaoapp.musically", "en", "")
            .expect("the English set exists")
            .label(TikTokControl::PhotoBadge)
            .expect("measured 23/08/2026 on two 46.2.1 phones");
        assert_eq!(badge, LabelMatch::Text("Photo"));
        assert!(
            badge.is_exact(),
            "the badge is the whole string, not a substring"
        );
    }

    #[test]
    fn the_vietnamese_sea_build_returns_what_the_device_showed() {
        let set = redmi();
        assert_eq!(
            set.label(TikTokControl::FeedTab),
            Some(LabelMatch::Exact("Đề xuất"))
        );
        assert_eq!(
            set.label(TikTokControl::Like),
            Some(LabelMatch::Exact("Thích"))
        );
        // The count changes per post, so this must not be an exact match.
        let comments = set.label(TikTokControl::Comments).expect("comments");
        assert!(!comments.is_exact());
        assert_eq!(comments.value(), "bình luận");
    }

    #[test]
    fn an_unmeasured_control_is_absent_rather_than_guessed() {
        // Encoding a guess would produce a locator that matches nothing and reads
        // as "no LIVE card here" — a wrong answer, not a missing one. `LiveRoom`
        // needs a LIVE post in the feed to measure, and none has appeared.
        assert_eq!(redmi().label(TikTokControl::LiveRoom), None);
        // The English set did measure it, so the field is not simply unused.
        let english = controls_for("com.zhiliaoapp.musically", "en", "").expect("measured set");
        assert_eq!(
            english.label(TikTokControl::LiveRoom),
            Some(LabelMatch::Contains("Tap to watch LIVE"))
        );
    }

    #[test]
    fn both_like_states_are_measured_and_distinct() {
        // The loop tells liked from not-liked by these two strings, so they must
        // differ and neither may be a prefix of the other — `Đã thích video`
        // contains no `Thích` as an exact match, which is what makes the
        // exact-match state check sound.
        // Every set that can *tap* a like must be able to *recognise* one. This used to
        // `continue` past a set that had only half the pair, which is how the SEA build in
        // English kept a `like` and no `liked` — and with it, a loop that could neither
        // confirm a like nor tell it had already left one, on sixteen of eighteen phones.
        for set in TIKTOK_LABEL_SETS {
            let Some(like) = set.translated(TikTokControl::Like) else {
                continue;
            };
            let liked = set.translated(TikTokControl::Liked).unwrap_or_else(|| {
                panic!(
                    "{} / {} taps a like it cannot recognise: measure the liked state with                      `cargo run -p riviu-android-driver --example label_scout -- <serial>                      --no-launch --tap Like`",
                    set.package, set.language
                )
            });
            assert_ne!(like.value(), liked.value(), "{}", set.package);
            assert!(like.is_exact() && liked.is_exact(), "{}", set.package);
        }
        assert_eq!(
            redmi().label(TikTokControl::Liked),
            Some(LabelMatch::Exact("Đã thích video"))
        );
    }

    #[test]
    fn english_stays_english_where_the_build_kept_it() {
        // `Follow` survives untranslated on the Vietnamese build; that is measured,
        // not an oversight, and it is why translation cannot be derived.
        assert_eq!(
            redmi().label(TikTokControl::Follow),
            Some(LabelMatch::Contains("Follow "))
        );
    }

    #[test]
    fn the_follow_label_cannot_match_the_following_tab() {
        // Measured on an SM-N950F, 11/08/2026, and it cost a failed two-phone gate to
        // find. The feed's own tab row carries `content-desc="Đã follow"` ("Following"),
        // and uiautomator's `descriptionContains` is **case-insensitive** — so the
        // trailing space is the entire difference between "the author's Follow button" and
        // "a tab that switches feeds".
        //
        // Two things went wrong without it: reading the author name off the rail returned
        // `Đã follow`, and the nurture loop's follow action would have tapped the tab.
        for set in TIKTOK_LABEL_SETS {
            let Some(follow) = set.translated(TikTokControl::Follow) else {
                continue;
            };
            let needle = follow.value().to_lowercase();
            assert!(
                needle.ends_with(' '),
                "{} Follow {:?} must end with a space",
                set.package,
                follow.value()
            );
            for tab in ["Đã follow", "Following", "đã follow"] {
                assert!(
                    !tab.to_lowercase().contains(&needle),
                    "{} Follow {:?} matches the {tab:?} tab",
                    set.package,
                    follow.value()
                );
            }
            // And it still matches every author label measured on a device.
            for author in ["Follow Bích Vân", "Follow Mộng Quỳnh", "Follow Hương Thảo"] {
                assert!(
                    author.to_lowercase().contains(&needle),
                    "{} Follow {:?} no longer matches {author:?}",
                    set.package,
                    follow.value()
                );
            }
        }
    }

    #[test]
    fn the_carousel_is_on_where_the_traversal_was_proved_and_nowhere_else() {
        // This label decides whether a build is swiped sideways at all, and the two builds
        // are in different positions, so it is asserted in both directions rather than as
        // one rule.
        //
        // `trill/en` was measured on ce051715cb22c30403 on 18/08/2026, switched **off** the
        // same day because turning it on ended both sessions that met a photo post at zero
        // videos, and switched back on once the cause was found: the sideways gesture was a
        // drag, and TikTok's image pager only acts on a fling. See the comment beside the
        // field, and `TouchPointPlanner::plan_flick` for the measurement that separates the
        // two gestures.
        assert_eq!(
            controls_for("com.ss.android.ugc.trill", "en", "")
                .expect("set")
                .label(TikTokControl::PhotoBadge),
            Some(LabelMatch::Text("Photo")),
            "the badge lives in `text` and has no content-desc; reading it as a description \
             finds nothing"
        );
        // `musically/en` carries the same string, measured 23/08/2026 on two 46.2.1 phones
        // — five photo cards against nineteen non-photo controls with no false positive. It
        // was `None` before that, and the rule it stood for still holds for any build nobody
        // has looked at: measure it on the build before switching it on for the build.
        assert_eq!(
            controls_for("com.zhiliaoapp.musically", "en", "")
                .expect("set")
                .label(TikTokControl::PhotoBadge),
            Some(LabelMatch::Text("Photo")),
            "measured on two phones; see the provenance beside the field"
        );
    }

    #[test]
    fn the_sound_strip_matches_the_form_the_feed_actually_uses() {
        // Written down as `Original sound by` this morning off one card, and measured as
        // the rare form the same day: nine of nine sampled cards say `Sound: <track> by
        // <author>`. The cost of the wrong prefix is quiet — the fingerprint falls back
        // to comments and shares, which is exactly what fails on the low-engagement feed
        // this field was added for.
        for (package, language) in [
            ("com.ss.android.ugc.trill", "en"),
            ("com.zhiliaoapp.musically", "en"),
        ] {
            assert_eq!(
                controls_for(package, language, "")
                    .expect("set")
                    .label(TikTokControl::SoundLink),
                Some(LabelMatch::Contains("Sound:")),
                "{package} / {language}"
            );
        }
    }
    #[test]
    fn every_build_on_the_farm_can_reach_the_send_button() {
        // Read off all twenty phones on 18/08/2026 with `dumpsys package … versionName`.
        // Before this, the only measured versions were 46.3.3 and 46.4.3 — two handsets
        // that are not on this farm at all — so commenting was impossible on every one of
        // the twenty, and the refusal blamed the AI key.
        //
        // A list of literal builds rather than a rule, because that is what it is: there
        // is no deriving one version's Send control from another's, which is the whole
        // reason for the table. When the farm updates, this test fails and the answer is
        // to measure the new build, not to relax the assertion.
        for (package, language, version, phones) in [
            ("com.ss.android.ugc.trill", "en", "38.3.2", 16),
            ("com.zhiliaoapp.musically", "en", "46.2.1", 3),
            ("com.zhiliaoapp.musically", "en", "46.2.42", 1),
        ] {
            let controls = controls_for(package, language, version)
                .unwrap_or_else(|| panic!("{package} {version} has no measured label set"));
            assert!(
                controls.label(TikTokControl::CommentSend).is_some(),
                "{phones} phone(s) run {package} {version} and cannot post a comment: \
                 measure the drawer with `RIVIU_TIKTOK_PACKAGE={package} probe <serial> \
                 --measure-comment` and look for the control whose `enabled` goes false \
                 -> true when the field holds text"
            );
            // **And can answer one.** A build that can comment but not reply cannot be in a
            // thread at all — every reply refuses with `reply_control_unmeasured`, which is
            // where all twenty phones stood until this was measured. The pair lives in one
            // test because a campaign needs both, and half of it is no use.
            assert!(
                controls.label(TikTokControl::CommentReply).is_some(),
                "{phones} phone(s) run {package} {version} and cannot reply to a comment: \
                 the per-row control lives in `text`, not `content-desc` — open the drawer \
                 with `probe --measure-comment` and look for one Button per comment row, \
                 sitting just below each body"
            );
        }
    }
    #[test]
    fn a_build_that_renders_the_send_button_is_described_by_its_language() {
        // The version table holds `@2131…` references because those change on every
        // rebuild. 38.3.2 does not leave one — it renders `Post comment` — and a string
        // is language-keyed by nature, so it belongs with the translations. Writing it
        // into the version table instead would tell a Vietnamese 38.3.2 phone to look
        // for English, which is the failure this module exists to stop.
        let fleet = controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("set");
        assert_eq!(
            fleet.label(TikTokControl::CommentSend),
            Some(LabelMatch::Exact("Post comment")),
            "every phone on the farm runs this build"
        );

        // And a build that *does* leave a reference keeps it: the id wins, because an id
        // cannot be wrong about the language and a string can.
        let referenced = controls_for("com.ss.android.ugc.trill", "vi", "46.4.3").expect("set");
        assert_eq!(
            referenced.label(TikTokControl::CommentSend),
            Some(LabelMatch::Exact("@2131823293"))
        );
    }
    #[test]
    fn the_send_button_is_keyed_by_app_version_not_by_language() {
        // The measurement this whole split exists for: two phones, same package, same
        // Vietnamese UI, different app version, **different Send id**. If this ever
        // collapses to one value, re-measure before simplifying the table away.
        let older = controls_for("com.ss.android.ugc.trill", "vi", "46.3.3").expect("set");
        let newer = controls_for("com.ss.android.ugc.trill", "vi", "46.4.3").expect("set");
        assert_eq!(
            older.label(TikTokControl::CommentSend),
            Some(LabelMatch::Exact("@2131823284"))
        );
        assert_eq!(
            newer.label(TikTokControl::CommentSend),
            Some(LabelMatch::Exact("@2131823293"))
        );
        assert_ne!(
            older.label(TikTokControl::CommentSend),
            newer.label(TikTokControl::CommentSend)
        );
        // And the translations are the same set, so keying everything by version would
        // have refused both phones for no reason.
        assert_eq!(
            older.label(TikTokControl::Like),
            newer.label(TikTokControl::Like)
        );
    }

    #[test]
    fn an_unmeasured_app_version_refuses_only_the_resource_ids() {
        // A TikTok update must not take the whole backend down: liking and reading are
        // translations and keep working, while the one control whose id may have moved
        // refuses. This is the difference between a degraded backend and a dead one.
        let updated = controls_for("com.ss.android.ugc.trill", "vi", "99.0.0").expect("set");
        assert_eq!(updated.label(TikTokControl::CommentSend), None);
        assert_eq!(updated.resource_version(), None);
        assert_eq!(
            updated.label(TikTokControl::Like),
            Some(LabelMatch::Exact("Thích"))
        );
        assert_eq!(
            updated.label(TikTokControl::CommentReply),
            Some(LabelMatch::Text("Trả lời"))
        );
        // An unread version is the same as an unmeasured one — never a silent match on
        // whichever entry happens to be first.
        assert_eq!(
            controls_for("com.ss.android.ugc.trill", "vi", "")
                .expect("set")
                .label(TikTokControl::CommentSend),
            None
        );
        // `trill/vi` has no text for Send and, on an unread version, no resource set either
        // — so this is the one shape that genuinely cannot comment, and the only one the
        // provenance line still warns about. It used to warn whenever a resource set merely
        // failed to match, which pointed the alarm at the healthy majority of this farm.
        let line = updated.provenance();
        assert!(line.contains("CHƯA đo được nút Gửi"), "{line}");
        assert!(line.contains("bỏ bình luận"), "{line}");
    }

    /// **No language set may carry a string for the album pill, ever.**
    ///
    /// The invariant behind `label()` reading this control out of the resource table alone.
    /// Both sets that once had one were wrong in the same way: the pill shows the album
    /// *currently selected*, so `All` and `Gần đây` each named a default that stopped being
    /// true one tap later — and `All` was ambiguous with the media-type tab besides.
    ///
    /// Written as a sweep over the catalogue rather than as a comment, because the tempting
    /// edit is to "fix" a refusing build by putting its visible string back.
    #[test]
    fn no_language_set_describes_the_album_pill_with_a_string() {
        for set in TIKTOK_LABEL_SETS {
            assert_eq!(
                set.translated(TikTokControl::PickerAlbumMenu),
                None,
                "{} / {} gives the album pill a string locator; that string is the album it                  happens to be showing, so it stops being true the moment the publish path                  chooses the campaign's own. Measure the `resource-id` instead.",
                set.package,
                set.language
            );
        }
        // And at least one build does reach it, or the rule above is vacuous.
        assert!(
            controls_for("com.ss.android.ugc.trill", "en", "38.3.2")
                .expect("the fleet's build")
                .label(TikTokControl::PickerAlbumMenu)
                .is_some(),
            "no build can reach the album pill at all, so nothing can publish anywhere"
        );
    }

    /// **A phone whose app version was not read borrows nobody's resource ids.**
    ///
    /// Resource ids are reassigned on every app rebuild — measured on this fleet: the comment
    /// drawer's Send button is `@2131823284` on 46.3.3 and `@2131823293` on 46.4.3. So an
    /// unread `versionName` has to mean *absent*, not *any set for this package*, or a phone
    /// gets a locator pointing at whatever control that id belongs to now.
    #[test]
    fn an_unread_app_version_does_not_fall_back_to_another_builds_ids() {
        let known = controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("known");
        assert!(known.label(TikTokControl::PickerAlbumMenu).is_some());

        for version in ["", "  ", "38.3.1", "99.0.0"] {
            let set = controls_for("com.ss.android.ugc.trill", "en", version)
                .expect("the language set still resolves");
            assert_eq!(
                set.label(TikTokControl::PickerAlbumMenu),
                None,
                "app version {version:?} borrowed a resource id measured on another build"
            );
            // The language-based labels are untouched: an unknown version costs the ids and
            // nothing else, so the phone can still watch and read.
            assert!(set.label(TikTokControl::FeedTab).is_some());
        }
    }

    #[test]
    fn the_picker_controls_are_measured_and_split_across_both_attributes() {
        // Read off the picker on both fleet phones, 11/08/2026. Worth pinning because the
        // split across attributes is measured, not incidental: the two tabs carry a real
        // `content-desc` while the buttons carry only `text`, and a locator that reads the
        // wrong attribute finds nothing at all — which the publish path would read as "this
        // control is not on screen".
        let set = redmi();
        assert_eq!(
            set.label(TikTokControl::PickerTabAll),
            Some(LabelMatch::Exact("Tất cả")),
            "the tabs are described"
        );
        assert_eq!(
            set.label(TikTokControl::PickerTabAll).unwrap().attribute(),
            LabelAttribute::Description
        );
        // **The album pill is no longer in this list, and its absence is the finding.** It
        // used to be `Text("Gần đây")`, which named the album that happened to be selected
        // when somebody looked — and stopped being true one tap later. It now resolves only
        // through a version-keyed resource id, and none has been read off this build.
        assert_eq!(
            set.label(TikTokControl::PickerAlbumMenu),
            None,
            "a text locator for the album pill is a locator for a value that changes"
        );
        for control in [TikTokControl::PickerMultiSelect, TikTokControl::PickerNext] {
            assert_eq!(
                set.label(control).unwrap().attribute(),
                LabelAttribute::Text,
                "{control:?} was measured in `text`, not `content-desc`"
            );
        }
        // And they are absent on the build nobody has opened the composer on, so the
        // publish path refuses there rather than tapping an iPhone's coordinates.
        let english = controls_for("com.zhiliaoapp.musically", "en", "").expect("set");
        assert_eq!(english.label(TikTokControl::PickerNext), None);
    }

    #[test]
    fn the_version_name_comes_out_of_dumpsys_as_the_device_prints_it() {
        // Verbatim shape from `dumpsys package com.ss.android.ugc.trill` on an
        // SM-N950F, 11/08/2026 — note the leading whitespace.
        let dumpsys = "  Package [com.ss.android.ugc.trill] (a1b2):\n    \
                       versionCode=464302 targetSdk=34\n    versionName=46.4.3\n";
        assert_eq!(parse_version_name(dumpsys), Some("46.4.3"));
        // A device carrying a system copy plus an update prints two records; the first
        // is the active one.
        assert_eq!(
            parse_version_name("    versionName=46.4.3\n    versionName=40.0.0\n"),
            Some("46.4.3")
        );
        assert_eq!(parse_version_name("    versionName=\n"), None);
        assert_eq!(parse_version_name("no such field"), None);
        assert_eq!(parse_version_name(""), None);
    }

    #[test]
    fn the_version_code_stops_at_the_field_that_follows_it() {
        // Verbatim from `dumpsys package com.ss.android.ugc.trill` on SM-G955F,
        // 17/08/2026. Unlike `versionName` this shares its line, so a parser that read to
        // the end of the line would record the build as "380302 minSdk=21 targetSdk=34".
        let dumpsys = "  Package [com.ss.android.ugc.trill] (a1b2):\n    \
                       versionCode=380302 minSdk=21 targetSdk=34\n    versionName=38.3.2\n";
        assert_eq!(parse_version_code(dumpsys), Some("380302"));
        assert_eq!(
            parse_version_code("    versionCode=274 minSdk=26\n"),
            Some("274")
        );
        // Digits only: anything else means the line was not what we thought it was, and a
        // build number is hashed into a device profile id -- nearly right is wrong.
        assert_eq!(parse_version_code("    versionCode=unknown\n"), None);
        assert_eq!(parse_version_code("    versionCode=\n"), None);
        assert_eq!(parse_version_code("no such field"), None);
        assert_eq!(parse_version_code(""), None);
    }

    #[test]
    fn locale_tags_reduce_to_the_language() {
        assert_eq!(normalise_language("vi-VN"), "vi");
        assert_eq!(normalise_language("vi_VN"), "vi");
        assert_eq!(normalise_language("EN-gb"), "en");
        assert_eq!(normalise_language("  vi  "), "vi");
        assert_eq!(normalise_language(""), "");
    }

    #[test]
    fn every_control_appears_in_all() {
        // Catches a duplicate or a hole in the ordinals. It does **not** catch a variant left
        // out of `ALL` entirely — this sizes itself from `ALL` and iterates `ALL`, so a
        // variant with ordinal 23 that nobody added here is invisible to it. See `ALL`'s doc
        // for what does hold: two exhaustive matches plus the fixed array length.
        let mut seen = [false; TikTokControl::ALL.len()];
        for control in TikTokControl::ALL {
            let ordinal = control.ordinal();
            assert!(
                !seen[ordinal],
                "{control:?} appears twice in ALL (ordinal {ordinal})"
            );
            seen[ordinal] = true;
        }
        for (ordinal, covered) in seen.iter().enumerate() {
            assert!(
                *covered,
                "a control with ordinal {ordinal} exists but is missing from ALL"
            );
        }
    }

    #[test]
    fn the_profile_tab_cannot_match_the_author_profile_link() {
        // Measured hazard, not a hypothetical: one dump from a Redmi Note 12 on 13/08/2026
        // carried BOTH `Hồ sơ` (our own tab, bottom bar) and `Hồ sơ Ánh đây` (the author's
        // profile link on the action rail). A `Contains` label would match either, and the
        // delete path following the wrong one lands on a stranger's profile.
        //
        // The same shape as `the_follow_label_cannot_match_the_following_tab`, and the
        // reason both tests exist: on this build the dangerous string is a *prefix* of the
        // safe one, so only exactness separates them.
        let mut measured = 0;
        for set in TIKTOK_LABEL_SETS {
            let Some(label) = set.translated(TikTokControl::ProfileTab) else {
                continue;
            };
            measured += 1;
            assert!(
                matches!(label, LabelMatch::Exact(_)),
                "{} ProfileTab must be Exact; Contains would match the author's profile link",
                set.package
            );
            // The hazard has two measured shapes, and the tab text is a substring of both.
            // vi (Redmi, 13/08/2026): `Hồ sơ` and the author link `Hồ sơ Ánh đây` — prefix.
            // en (SM-G955F, 18/08/2026): `Profile` and the author link `Ngọc Lệ profile` —
            // suffix, and lowercase, so it only collides once case is folded, which
            // description matching does.
            //
            // This used to assert `format!("{X} …").starts_with(X)`, which is true of every
            // string and therefore proved nothing. What matters is that `Exact` separates
            // the two and `Contains` would not.
            for author_link in [
                format!("{} Ánh đây", label.value()),
                format!("Ngọc Lệ {}", label.value().to_lowercase()),
            ] {
                assert!(
                    author_link
                        .to_lowercase()
                        .contains(&label.value().to_lowercase()),
                    "{}: the author link must be the collision this Exact label avoids",
                    set.package
                );
                assert_ne!(
                    author_link.as_str(),
                    label.value(),
                    "{}: Exact is what keeps the author link out",
                    set.package
                );
            }
        }
        assert_eq!(
            measured, 2,
            "both measured sets carry ProfileTab; bump this when another is measured"
        );
    }

    #[test]
    fn the_publish_tail_and_the_delete_path_are_unmeasured_and_therefore_refuse() {
        // The point of the whole catalogue, applied to the one action that cannot be
        // undone. These six are declared so the code can name them and refuse; measuring
        // them is a device task. If a future edit fills one in without a measurement in
        // AGENTS.md, this test is what fails.
        for set in TIKTOK_LABEL_SETS {
            for control in [
                TikTokControl::ComposerNext,
                TikTokControl::ComposerCaption,
                TikTokControl::PostButton,
                TikTokControl::PostDeleteMenu,
                TikTokControl::PostDelete,
                TikTokControl::PostDeleteConfirm,
            ] {
                assert!(
                    set.translated(control).is_none(),
                    "{} {control:?} claims a measurement that AGENTS.md does not record;                      add the measurement first, then this assertion",
                    set.package
                );
            }
        }
    }

    /// A set that can *recognise* the feed must also be able to *reach* it.
    ///
    /// Not a tidiness rule — it is what the fleet measured. `feed_tab` is a tab **inside**
    /// the feed, so a phone parked on Profile, Inbox or Shop shows none of it; the way back
    /// is the Home tab on the bottom bar. On 18/08/2026 a whole-fleet run had the two
    /// `com.zhiliaoapp.musically` phones fail with "chờ 30s mà chưa thấy tab feed" for
    /// exactly this reason, while the sixteen `trill` phones — whose set had `home_tab` —
    /// recovered and watched. A phone is left wherever the last session left it, so this is
    /// the ordinary case rather than an edge one.
    #[test]
    fn every_set_that_knows_the_feed_also_knows_the_way_back_to_it() {
        for set in TIKTOK_LABEL_SETS {
            if set.translated(TikTokControl::FeedTab).is_none() {
                continue;
            }
            assert!(
                set.translated(TikTokControl::HomeTab).is_some(),
                "{} / {} can tell it is on the feed but cannot get there: measure the \
                 bottom bar's Home tab with `cargo run -p riviu-android-driver --example \
                 label_scout`",
                set.package,
                set.language
            );
        }
    }

    /// **Every resource entry is a bounded resource id, not just non-`None`.**
    ///
    /// The empty-label sweep read the language table and, of the resource table, only
    /// `comment_send`. So `picker_album_menu: Some(ResourceId(""))` was green — and an empty
    /// suffix becomes the regex `.*`, which matches the first node on screen carrying any
    /// resource id at all. `Some(Text("snr"))` was green too, silently turning an id lookup
    /// into a search of rendered text.
    #[test]
    fn every_resource_entry_is_a_bounded_resource_id() {
        let mut checked = 0;
        for set in TIKTOK_RESOURCE_SETS {
            assert!(!set.package.is_empty() && !set.app_version.is_empty());
            assert!(
                !set.measured_on.is_empty(),
                "{} needs provenance",
                set.package
            );
            for control in TikTokControl::ALL {
                let Some(label) = set.resource(control) else {
                    continue;
                };
                assert!(
                    !label.value().trim().is_empty(),
                    "{} / {} gives {control:?} an empty locator; the driver turns that into \
                     `.*`, which matches the first node carrying any resource id",
                    set.package,
                    set.app_version
                );
                // `comment_send` is measured as a `content-desc` that happens to be an
                // unresolved `@2131…` reference; everything else in this table is a real
                // `resource-id` and has to say so.
                if control != TikTokControl::CommentSend {
                    assert!(
                        matches!(label, LabelMatch::ResourceId(_)),
                        "{} / {} gives {control:?} a {:?} locator; this table is keyed by app \
                         version because resource ids move, and a string does not belong in it",
                        set.package,
                        set.app_version,
                        label.attribute()
                    );
                    assert!(
                        label.value().contains(":id/"),
                        "{} / {} stores {control:?} as {:?}; without the `:id/` boundary the \
                         regex `.*{}` also matches an id that merely ends in those characters",
                        set.package,
                        set.app_version,
                        label.value(),
                        label.value()
                    );
                }
                checked += 1;
            }
        }
        assert!(checked >= 4, "only {checked} resource entries scanned");
    }

    /// **A resource set belongs to one package as well as one version.**
    ///
    /// Dropping the package from the lookup left every case in the version test green, because
    /// they all use `trill`. A `musically` phone whose `versionName` happens to equal a `trill`
    /// row would then borrow the other app's ids.
    #[test]
    fn a_resource_set_is_not_shared_between_two_packages() {
        // `trill` 38.3.2 has an album pill; `musically` at the same version string has no row
        // at all, and must not inherit one.
        assert!(controls_for("com.ss.android.ugc.trill", "en", "38.3.2")
            .expect("catalogued")
            .label(TikTokControl::PickerAlbumMenu)
            .is_some());
        assert_eq!(
            controls_for("com.zhiliaoapp.musically", "en", "38.3.2")
                .expect("the language set exists")
                .label(TikTokControl::PickerAlbumMenu),
            None,
            "a musically phone borrowed a trill resource id"
        );
        // And the reverse, on a version only `musically` has.
        assert!(controls_for("com.zhiliaoapp.musically", "en", "46.2.1")
            .expect("catalogued")
            .label(TikTokControl::CommentSend)
            .is_some());
    }

    /// **The album pill has no fallback, proved by a set that carries a string.**
    ///
    /// The sweep over the catalogue is true and stays true whether the fallback exists or not,
    /// because every real language set has `None` there. This fixture is the only shape that
    /// can tell: a language row with a string and no resource table at all.
    #[test]
    fn a_language_string_for_the_album_pill_is_ignored_even_when_present() {
        let with_a_string = an_album_pill_string_the_resolver_must_ignore();
        assert_eq!(
            with_a_string.label(TikTokControl::PickerAlbumMenu),
            None,
            "the resolver fell back to a language string; that string names the album that              happens to be showing and stops being true one tap later"
        );
        // The rest of the same set still resolves, so this is not measuring an empty fixture.
        assert!(with_a_string.label(TikTokControl::ComposerOpen).is_some());
    }

    /// **The caption field is never located by a string.**
    ///
    /// Its own documentation says so — a placeholder stops matching the moment a character is
    /// typed, so the readback that proves the caption took would report that it had not. The
    /// rule had no test, and both success fixtures used `Text`, training every composer test
    /// on the one strategy the control forbids.
    #[test]
    fn the_caption_field_is_never_located_by_a_string() {
        let mut checked = 0;
        for set in TIKTOK_LABEL_SETS {
            if let Some(label) = set.translated(TikTokControl::ComposerCaption) {
                assert!(
                    matches!(label, LabelMatch::ClassName(_) | LabelMatch::ResourceId(_)),
                    "{} / {} locates the caption field by {:?}; a placeholder stops matching                      as soon as a character is typed",
                    set.package,
                    set.language,
                    label.attribute()
                );
                checked += 1;
            }
        }
        // And the fixtures, which are what the composer's tests actually run against.
        for controls in [
            every_publish_control_measured(),
            every_publish_control_but_post_measured(),
        ] {
            let label = controls
                .label(TikTokControl::ComposerCaption)
                .expect("these fixtures measure the caption");
            assert!(
                matches!(label, LabelMatch::ClassName(_) | LabelMatch::ResourceId(_)),
                "a fixture locates the caption by {:?}, which teaches the wrong strategy",
                label.attribute()
            );
            checked += 1;
        }
        assert!(checked >= 2, "only {checked} caption locators scanned");
    }

    /// **The album pill has no fallback, proved with a set that would use one.**
    ///
    /// The existing sweep asserts no language set carries a string — true, and it stays true
    /// if the resolver regains its `or_else`. This one gives a fixture *both* a language string
    /// and no resource entry, so a fallback is the only way it could resolve.
    #[test]
    fn the_album_pill_never_falls_back_to_a_language_string() {
        // `every_publish_control_but_caption_measured` has `picker_album_menu: None` in its
        // language set and a resource entry; strip the version and the id goes away.
        let with_id = every_publish_control_but_caption_measured();
        assert!(with_id.label(TikTokControl::PickerAlbumMenu).is_some());

        // The Vietnamese set is the real case: it once had `Text("Gần đây")`, and it has no
        // resource row on any version.
        for version in ["", "46.3.3", "99.9.9"] {
            assert_eq!(
                controls_for("com.ss.android.ugc.trill", "vi", version)
                    .expect("the language set exists")
                    .label(TikTokControl::PickerAlbumMenu),
                None,
                "the album pill resolved from somewhere other than the resource table"
            );
        }
    }

    #[test]
    fn no_entry_carries_an_empty_label() {
        for set in TIKTOK_LABEL_SETS {
            assert!(!set.package.is_empty());
            assert!(!set.language.is_empty());
            assert!(
                !set.measured_on.is_empty(),
                "{} needs provenance",
                set.package
            );
            // Every control, not a hand-written subset. The old list omitted whatever was
            // added last, so a new label simply went unchecked — silently, which is the
            // failure mode this module exists to prevent.
            for control in TikTokControl::ALL {
                if let Some(label) = set.translated(control) {
                    assert!(
                        !label.value().trim().is_empty(),
                        "{} {control:?} is an empty label",
                        set.package
                    );
                }
            }
        }
        for set in TIKTOK_RESOURCE_SETS {
            assert!(!set.package.is_empty());
            assert!(
                !set.app_version.is_empty(),
                "a resource set keyed by nothing matches nothing"
            );
            assert!(
                !set.measured_on.is_empty(),
                "{} {} needs provenance",
                set.package,
                set.app_version
            );
            // These are ids, not words, and an id that is not a `@`-reference means
            // somebody pasted a translation into the wrong table.
            if let Some(send) = set.resource(TikTokControl::CommentSend) {
                assert!(
                    send.value().starts_with('@'),
                    "{} {} CommentSend {:?} does not look like a resource reference",
                    set.package,
                    set.app_version,
                    send.value()
                );
            }
        }
    }

    #[test]
    fn no_two_resource_sets_claim_the_same_package_and_version() {
        // A duplicate would make the lookup depend on table order, which is exactly the
        // kind of silent ambiguity this module refuses everywhere else.
        for (index, set) in TIKTOK_RESOURCE_SETS.iter().enumerate() {
            for other in &TIKTOK_RESOURCE_SETS[index + 1..] {
                assert!(
                    set.package != other.package || set.app_version != other.app_version,
                    "duplicate resource set for {} {}",
                    set.package,
                    set.app_version
                );
            }
        }
    }

    #[test]
    fn every_english_build_on_the_farm_can_decline_a_dialog_it_understands() {
        // A dialog holding the phone is the single most common way a session ends at zero
        // videos, and `await_feed`'s ladder has two rungs for it: tap a *measured decline*
        // when there is one, press Back when there is not. Back is the weaker rung — it did
        // not clear "Turn on precise location" on 19/08/2026, and the session failed with
        // `chờ 30s mà chưa thấy tab feed` while a button reading `Not now` was on screen.
        //
        // So the invariant is per build, not per fleet: both English builds run here, and a
        // decline measured on one of them is not a decline on the other.
        for package in ["com.ss.android.ugc.trill", "com.zhiliaoapp.musically"] {
            assert_eq!(
                controls_for(package, "en", "")
                    .expect("set")
                    .label(TikTokControl::DialogDismiss),
                Some(LabelMatch::Text("Not now")),
                "{package}: measure the decline with `label_scout <serial> --no-launch` while \
                 the dialog is up — it is in `text`, not `content-desc`"
            );
        }
    }
}

/// The provenance line, pinned against the reading that was inverted on the live farm.
#[cfg(test)]
mod provenance_tests {
    use super::*;

    /// The eleven phones that were being warned at. `trill/en` renders the Send button as
    /// text, so a missing resource set costs it nothing — and the line must not imply
    /// otherwise.
    #[test]
    fn a_build_that_renders_the_send_button_is_not_warned_about_resource_ids() {
        let set = controls_for("com.ss.android.ugc.trill", "en", "38.3.2").expect("measured");
        let line = set.provenance();
        assert!(
            set.label(TikTokControl::CommentSend).is_some(),
            "sanity: this build resolves Send from the translation"
        );
        assert!(
            !line.contains("CHƯA"),
            "nothing is missing on this build, so nothing may shout: {line}"
        );
        // **The sentence now distinguishes two situations the old one conflated.** This build
        // has a resource row that explicitly records `comment_send: None`, which is a
        // *measurement* — somebody looked and found a rendered string. A build with no row at
        // all gets a different sentence, because there the string comes from a language set
        // measured on some other version and nobody has checked this one.
        assert!(
            line.contains("đã đo trên đúng bản build này"),
            "it should say the string was measured on this build: {line}"
        );
        assert!(
            !line.contains("CHƯA đo trên bản app này"),
            "this build has a resource row, so it is not the unknown case: {line}"
        );

        // And an English build with no resource row says so instead of claiming the
        // measurement — the sentence that sent a reader looking at the wrong thing.
        let unknown = controls_for("com.ss.android.ugc.trill", "en", "99.9.9").expect("set");
        let line = unknown.provenance();
        assert!(
            line.contains("CHƯA đo trên bản app này"),
            "an unmeasured version must not claim its Send button was checked: {line}"
        );
        // The field that had no reader until this line existed.
        assert!(
            line.contains("nhãn đo trên app 38.3.2"),
            "measured_app_version belongs here: {line}"
        );
    }

    /// The three phones that were getting the reassuring line. Their Send button is an
    /// unresolved `@2131…`, so the matching resource set is exactly what makes them work —
    /// and naming its version is real information rather than false comfort.
    #[test]
    fn a_build_that_needs_a_resource_id_names_the_version_it_came_from() {
        let set = controls_for("com.zhiliaoapp.musically", "en", "46.2.1").expect("measured");
        assert_eq!(
            set.translated.translated(TikTokControl::CommentSend),
            None,
            "sanity: this build has no text for Send"
        );
        assert!(set.label(TikTokControl::CommentSend).is_some());
        let line = set.provenance();
        assert!(line.contains("resource id"), "{line}");
        assert!(line.contains("46.2.1"), "{line}");
        assert!(!line.contains("CHƯA"), "{line}");
    }

    /// The only case that should carry a warning: no id **and** no translation, so the
    /// session genuinely cannot comment. Same phones as above, on a version nobody measured.
    #[test]
    fn the_warning_now_fires_only_when_send_is_truly_unresolvable() {
        let set = controls_for("com.zhiliaoapp.musically", "en", "99.9.9").expect("measured");
        assert_eq!(set.label(TikTokControl::CommentSend), None);
        let line = set.provenance();
        assert!(
            line.contains("CHƯA đo được nút Gửi"),
            "the real gap must say so: {line}"
        );
        assert!(
            line.contains("bỏ bình luận"),
            "and say what it costs: {line}"
        );
    }

    /// Whatever the branch, the line always names the two keys a reader needs to go and
    /// re-measure: which package and which UI language these strings came from.
    #[test]
    fn every_branch_names_the_package_and_the_language() {
        for (package, language, version) in [
            ("com.ss.android.ugc.trill", "en", "38.3.2"),
            ("com.ss.android.ugc.trill", "vi", "46.3.3"),
            ("com.zhiliaoapp.musically", "en", "46.2.1"),
            ("com.zhiliaoapp.musically", "en", ""),
        ] {
            let line = controls_for(package, language, version)
                .expect("measured set")
                .provenance();
            assert!(line.contains(package), "{line}");
            assert!(line.starts_with(package), "package comes first: {line}");
        }
    }
}
