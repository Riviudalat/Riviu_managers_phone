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
//!   `y8z`, `visibility=gone` until the card is an ad) and the server's `Sponsored` label under
//!   the author name. [`sponsored_card`] reads those.
//! * The **end-card** comes up when the ad's video ends. `CommerceVideoDelegateKt` inflates
//!   the Lynx mask through `view_stub_ad_mask` (a child of `a4v`, the card root) and sets the
//!   whole `widget_container` **INVISIBLE** — the rail, caption and author drop out of the
//!   tree while the feed tabs and the bottom bar stay. The mask draws the advertiser, one CTA
//!   button and `Close`. Its Close emits `action_ad_hide_lynx_mask`, which only hides the mask
//!   and sets `widget_container` visible again (no navigation, no landing page); a swipe up on
//!   it is the mask's own `action_ad_swipe_up_video`, which asks the vertical pager for the
//!   next item — so the feed pager never sees the raw gesture. [`end_card`] reads that.
use crate::{tiktok_labels::TikTokControls, ui_automation::tree::Tree, ElementBox};

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

/// The surface that marks a card with a rail as an ad: the CTA strip's container or the
/// `Sponsored` label. Returns what was seen, for the log.
pub(super) fn sponsored_card(tree: &Tree, labels: TikTokControls) -> Option<&'static str> {
    if labels.package() != PACKAGE {
        return None;
    }
    let mut sponsored_label = false;
    for (index, _) in visible_nodes(tree) {
        let node = &tree.nodes[index];
        let id = node.attr("resource-id");
        if id.ends_with(":id/ad_feed_card_container_v2")
            || id.ends_with(":id/ad_feed_card_container")
        {
            return Some("khung nút quảng cáo (ad_feed_card_container)");
        }
        if labels.language() == "en"
            && node.attr("class") == "android.widget.TextView"
            && node.attr("text") == "Sponsored"
        {
            sponsored_label = true;
        }
    }
    sponsored_label.then_some("nhãn Sponsored")
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
