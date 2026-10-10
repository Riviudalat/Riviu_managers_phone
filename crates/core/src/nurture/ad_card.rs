//! Sponsored cards on the feed, read from one hierarchy snapshot.
//!
//! Two surfaces, both taken from the `com.zhiliaoapp.musically` 45.7.3 APK
//! (`target/tiktok-45.7.3-research`, read-only: `aapt2-resources.txt`, `layout_index.jsonl`,
//! `strings/nxstring_default.tsv` and the decompiled
//! `commercialize/feed/CommerceVideoDelegateKt`) and from the box 31–50 acceptance run of
//! 10/10/2026 (`ce031713b064841e0c`, Galaxy S8+, en; screenshot `evidence-h/b064-norail.png`).
//! **Neither has been measured on a phone's own accessibility dump yet** — the end-card was
//! only ever seen on a screenshot — so both recognisers fail closed: an unmatched tree is an
//! ordinary card, exactly as before.
//!
//! * The **playing ad** is an ordinary card with its rail, plus the CTA strip
//!   `ad_feed_card_container_v2` inside the interact area (layouts `b7l` and `d07`, a child of
//!   `y8z`, `visibility=gone` until the card is an ad), the server's `Sponsored` label under
//!   the author name, or the bottom label `Ad` ([`AD_LABEL_ID`]). [`sponsored_card`] reads
//!   those. The `Ad` label is the one **measured on a phone dump**: box 31–50, 10/10/2026,
//!   `ce0417141d4decde0c` (Galaxy S8+, 45.7.3, en), a Shopee photo ad whose tree had no
//!   `ad_feed_card_container` and no `Sponsored` at all — the session liked it.
//! * The **end-card** comes up when the ad's video ends. `CommerceVideoDelegateKt` inflates
//!   the Lynx mask through `view_stub_ad_mask` (a child of `a4v`, the card root) and sets the
//!   whole `widget_container` **INVISIBLE** — the rail, caption and author drop out of the
//!   tree while the feed tabs and the bottom bar stay. The mask draws the advertiser, one CTA
//!   button and `Close`. Its Close emits `action_ad_hide_lynx_mask`, which only hides the mask
//!   and sets `widget_container` visible again (no navigation, no landing page); a swipe up on
//!   it is the mask's own `action_ad_swipe_up_video`, which asks the vertical pager for the
//!   next item — so the feed pager never sees the raw gesture. [`end_card`] reads that.
use crate::{
    tiktok_labels::TikTokControls, types::TapPoint, ui_automation::tree::Tree, ElementBox,
};

/// The only package whose ad surfaces were researched. The resource *names* are stable
/// across builds (their numbers are not, and nothing here reads a number).
const PACKAGE: &str = "com.zhiliaoapp.musically";

/// CTA labels of the APK's default (English) locale, exact `text`. `Book now` is the one on
/// the 10/10/2026 screenshot; the rest are the other ad CTA strings in the same table. The
/// CTA is matched to be *named in the log and avoided* — it is never tapped.
const CTA_TEXTS: &[&str] = &[
    "Book now",
    "Shop now",
    "Learn more",
    "Download",
    "Install now",
    "Sign up",
    "Order now",
    "Apply now",
    "Contact us",
    "Play now",
    "Play game",
    "Get quote",
    "Get offer",
    "Watch now",
    "View now",
    "View profile",
    "Visit website",
    "Send message",
    "Interested",
    "Open now",
    "Get showtimes",
];

/// The bottom label that marks a feed card as an ad on 45.7.3: a `Button` with exact text
/// `Ad`, inside `lzt` (the root of layout `b5d`, `FeedBottomLabelLayout`) under the caption.
/// Measured at [32,1903] 60×42 on `ce0417141d4decde0c` (Galaxy S8+, 1080×2220, en),
/// 10/10/2026, `evidence-j/4dec-ad-photo.xml`. The id is an obfuscated name and is renamed
/// per build, so it is bound to [`AD_LABEL_BUILD`] through the session's resource set.
const AD_LABEL_ID: &str = ":id/hnj";
const AD_LABEL_BUILD: &str = "45.7.3";

/// The label's text per locale, exact. `en` is measured (above). `vi` is **not** measured on
/// a dump: `QC` and `Quảng cáo` are the `vi-VN` values of the 45.7.3 string ids whose default
/// value is `Ad` (`nxstring_vi-VN.tsv`), and which of them this label uses is unknown.
fn ad_label_texts(language: &str) -> &'static [&'static str] {
    match language {
        "en" => &["Ad"],
        "vi" => &["Quảng cáo", "QC"],
        _ => &[],
    }
}

/// The sound control's description prefix on a sponsored card (45.7.3 string `Promoted
/// music`, rendered `Sound: Promoted Music by …` on the same 10/10/2026 card). Supporting
/// evidence only: it names the sound library, not the card, so it never marks a card alone.
const PROMOTED_SOUND_PREFIX: &str = "Sound: Promoted Music";

/// A sponsored card with its rail, as one snapshot shows it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct SponsoredCard {
    /// What marked it, for the operator line.
    pub(super) evidence: &'static str,
    /// Bounds on that snapshot that take a touch for the ad itself — the CTA strip, the
    /// Lynx ad body's `click_container` (the product anchor is drawn inside it and is not in
    /// the tree on its own), small clickable controls of the interact area. A swipe that
    /// leaves the card must not start on one; see [`swipe_start`].
    pub(super) touch_owners: Vec<ElementBox>,
}

/// The ad end-card as one snapshot shows it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AdEndCard {
    /// The CTA's text, for the operator line. Never a tap target.
    pub(super) cta: String,
    /// The `Close` control when it may be tapped: the only `Close` on screen, in the centre
    /// column directly under the CTA, enabled, and clickable itself or through an ancestor
    /// that contains it. `None` leaves the caller with the swipe only.
    pub(super) close: Option<ElementBox>,
}

/// Visible nodes of the package, with geometry, whose ancestors are visible too.
fn visible_nodes<'t>(tree: &'t Tree) -> impl Iterator<Item = (usize, ElementBox)> + 't {
    tree.nodes.iter().enumerate().filter_map(|(index, node)| {
        if !node.visible(PACKAGE) || !tree.ancestors_visible(index) {
            return None;
        }
        node.rect().map(|rect| (index, rect))
    })
}

/// The surface that marks a card with a rail as an ad: the CTA strip's container, the
/// `Sponsored` label or the `Ad` bottom label. Returns what was seen and where the ad takes
/// touches, from the same snapshot.
pub(super) fn sponsored_card(tree: &Tree, labels: TikTokControls) -> Option<SponsoredCard> {
    if labels.package() != PACKAGE {
        return None;
    }
    let ad_label_measured = labels.resource_version() == Some(AD_LABEL_BUILD);
    let ad_texts = ad_label_texts(labels.language());
    let mut container = false;
    let mut ad_label = false;
    let mut sponsored_label = false;
    let mut promoted_sound = false;
    for (index, _) in visible_nodes(tree) {
        let node = &tree.nodes[index];
        let id = node.attr("resource-id");
        let text = node.attr("text");
        container |= id.ends_with(":id/ad_feed_card_container_v2")
            || id.ends_with(":id/ad_feed_card_container");
        ad_label |= ad_label_measured && id.ends_with(AD_LABEL_ID) && ad_texts.contains(&text);
        sponsored_label |= labels.language() == "en"
            && node.attr("class") == "android.widget.TextView"
            && text == "Sponsored";
        promoted_sound |= node.attr("content-desc").starts_with(PROMOTED_SOUND_PREFIX);
    }
    let evidence = if container {
        "khung nút quảng cáo (ad_feed_card_container)"
    } else if ad_label && promoted_sound {
        "nhãn Ad dưới chú thích, nhạc Promoted Music"
    } else if ad_label {
        "nhãn Ad dưới chú thích"
    } else if sponsored_label {
        "nhãn Sponsored"
    } else {
        return None;
    };
    Some(SponsoredCard {
        evidence,
        touch_owners: touch_owners(tree),
    })
}

/// Where the ad takes a touch on this snapshot (see [`SponsoredCard::touch_owners`]).
///
/// Clickable containers that span much of the card (the card root, the long-press layer,
/// the whole interact column `kqi`, 993 px of 2220 on the 10/10/2026 dump) are left out: they
/// are under every point and their drag goes to the feed pager like any card's.
fn touch_owners(tree: &Tree) -> Vec<ElementBox> {
    let extent = visible_nodes(tree)
        .map(|(_, rect)| rect.y + rect.height)
        .fold(0.0_f64, f64::max);
    visible_nodes(tree)
        .filter(|(index, rect)| {
            let node = &tree.nodes[*index];
            let id = node.attr("resource-id");
            node.attr("content-desc") == "click_container"
                || id.ends_with(":id/ad_feed_card_container_v2")
                || id.ends_with(":id/ad_feed_card_container")
                || (rect.clickable && rect.enabled && rect.height < extent * 0.25)
        })
        .map(|(_, rect)| rect)
        .collect()
}

/// The start of the vertical swipe that leaves a sponsored card: the caller's target when it
/// is clear of every [`SponsoredCard::touch_owners`] box, otherwise the clear point nearest
/// to it inside `zone` (y bounds), in the same column. `None` when the column has no clear
/// point in the zone; the caller then keeps its target.
///
/// The clearance is the planner's own scatter: endpoint jitter σ 9 px plus a hand bias of up
/// to 7 px (`super::touch`), so 36 px keeps a 3σ draw off the box. On the 10/10/2026 dump
/// (`ce0417141d4decde0c`, 1080×2220) the column x=540 is owned by `click_container`
/// [312,1391] (the product anchor is drawn at y≈1162–1367 inside it), by the CTA strip with
/// `Get` [1564,1701] and by the caption rows below it; the clear band is the page-dot strip
/// between them, y≈1428–1527.
pub(super) fn swipe_start(
    owners: &[ElementBox],
    target: &TapPoint,
    zone: (f64, f64),
) -> Option<TapPoint> {
    const CLEARANCE: f64 = 36.0;
    let blocked: Vec<(f64, f64)> = owners
        .iter()
        .filter(|rect| {
            target.x >= rect.x - CLEARANCE && target.x <= rect.x + rect.width + CLEARANCE
        })
        .map(|rect| (rect.y - CLEARANCE, rect.y + rect.height + CLEARANCE))
        .collect();
    let clear = |y: f64| {
        y >= zone.0 && y <= zone.1 && blocked.iter().all(|(top, bottom)| y < *top || y > *bottom)
    };
    std::iter::once(target.y)
        .chain(
            blocked
                .iter()
                .flat_map(|(top, bottom)| [top - 1.0, bottom + 1.0]),
        )
        .filter(|y| clear(*y))
        .min_by(|a, b| (a - target.y).abs().total_cmp(&(b - target.y).abs()))
        .map(|y| TapPoint { x: target.x, y })
}

/// The ad end-card: one CTA button and one `Close` under it, both in the centre column of
/// the card. The caller has already established that the rail is gone and the feed chrome is
/// not — this reads the overlay itself.
pub(super) fn end_card(
    tree: &Tree,
    labels: TikTokControls,
    screen: (f64, f64),
) -> Option<AdEndCard> {
    if labels.package() != PACKAGE || labels.language() != "en" {
        return None;
    }
    let (width, height) = screen;
    if !(width > 0.0 && height > 0.0) {
        return None;
    }
    // The middle of the screen, clear of the rail on the right and the tab strip and bottom
    // bar: on the 10/10/2026 screenshot (1080×2220) the CTA sat at y≈1033 and Close at
    // y≈1181, both spanning x≈85–975.
    let centred = |rect: &ElementBox| {
        let cx = rect.x + rect.width / 2.0;
        let cy = rect.y + rect.height / 2.0;
        cx >= width * 0.25 && cx <= width * 0.75 && cy >= height * 0.15 && cy <= height * 0.9
    };
    let mut ctas: Vec<(ElementBox, String)> = Vec::new();
    let mut closes: Vec<(usize, ElementBox)> = Vec::new();
    for (index, rect) in visible_nodes(tree) {
        if !centred(&rect) {
            continue;
        }
        let node = &tree.nodes[index];
        let text = node.attr("text");
        if CTA_TEXTS.contains(&text) {
            ctas.push((rect, text.to_owned()));
        } else if text == "Close" || node.attr("content-desc") == "Close" {
            closes.push((index, rect));
        }
    }
    let ([(cta, cta_text)], [(close_index, close)]) = (ctas.as_slice(), closes.as_slice()) else {
        return None;
    };
    // Close sits under the CTA, adjacent and never overlapping: a `Close` anywhere else is
    // some other control and the pair is not the end-card.
    let below = close.y >= cta.y + cta.height && close.y - (cta.y + cta.height) <= height * 0.15;
    if !below {
        return None;
    }
    let close = tappable(tree, *close_index, close);
    Some(AdEndCard {
        cta: cta_text.clone(),
        close,
    })
}

/// The node's own box when a tap on its centre is accepted by it or by an ancestor that
/// contains it. Lynx exposes its text and its touch target as separate nodes more often than
/// not, which is why the ancestor walk exists; it stops at the card root rather than walking
/// up to the feed pager, which is clickable and contains everything.
fn tappable(tree: &Tree, index: usize, rect: &ElementBox) -> Option<ElementBox> {
    if !rect.enabled {
        return None;
    }
    if rect.clickable {
        return Some(rect.clone());
    }
    let (cx, cy) = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let mut parent = tree.nodes[index].parent;
    for _ in 0..4 {
        let Some(current) = parent else {
            break;
        };
        let node = &tree.nodes[current];
        if node.attr("resource-id").ends_with(":id/a4v") {
            break;
        }
        if let Some(ancestor) = node.rect() {
            let contains = cx >= ancestor.x
                && cx <= ancestor.x + ancestor.width
                && cy >= ancestor.y
                && cy <= ancestor.y + ancestor.height;
            if ancestor.clickable && ancestor.enabled && contains {
                return Some(rect.clone());
            }
        }
        parent = node.parent;
    }
    None
}
