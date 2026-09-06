//! Bookmark state lives on a measured descendant, not the clickable rail button.
use super::*;
use crate::driver::ElementBox;
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::HashMap;

const PACKAGE: &str = "com.ss.android.ugc.trill";

#[derive(Default)]
struct Node {
    attrs: HashMap<String, String>,
    parent: Option<usize>,
}

impl Node {
    fn attr(&self, key: &str) -> &str {
        self.attrs.get(key).map(String::as_str).unwrap_or_default()
    }
}

fn bounds(raw: &str) -> Option<(f64, f64, f64, f64)> {
    let (start, end) = raw.strip_prefix('[')?.strip_suffix(']')?.split_once("][")?;
    let (x, y) = start.split_once(',')?;
    let (right, bottom) = end.split_once(',')?;
    let (x, y, right, bottom) = (
        x.parse::<f64>().ok()?,
        y.parse::<f64>().ok()?,
        right.parse::<f64>().ok()?,
        bottom.parse::<f64>().ok()?,
    );
    (x.is_finite()
        && y.is_finite()
        && right.is_finite()
        && bottom.is_finite()
        && x >= 0.0
        && y >= 0.0
        && right > x
        && bottom > y)
        .then_some((x, y, right - x, bottom - y))
}

/// The 38.3.2/en fixture has f1p(Button) -> f14(FrameLayout) -> f0y(ImageView).
/// Another unrelated f0y ViewGroup exists in the same snapshot; id-only lookup is ambiguous.
pub(super) fn parse_control(source: &str) -> anyhow::Result<Option<StatefulElementBox>> {
    anyhow::ensure!(
        source.len() <= 16 * 1024 * 1024,
        "bookmark hierarchy too large"
    );
    let mut reader = Reader::from_str(source);
    let mut nodes: Vec<Node> = Vec::new();
    let mut parents = Vec::new();
    loop {
        let event = reader.read_event()?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                anyhow::ensure!(
                    parents.len() < 256 && nodes.len() < 32768,
                    "bookmark hierarchy limit"
                );
                let mut node = Node {
                    parent: parents.last().copied(),
                    ..Default::default()
                };
                for attr in start.attributes() {
                    let attr = attr?;
                    node.attrs.insert(
                        std::str::from_utf8(attr.key.as_ref())?.into(),
                        attr.decoded_and_normalized_value(
                            XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )?
                        .into_owned(),
                    );
                }
                nodes.push(node);
                if matches!(event, Event::Start(_)) {
                    parents.push(nodes.len() - 1);
                }
            }
            Event::End(_) => {
                anyhow::ensure!(parents.pop().is_some(), "invalid bookmark hierarchy");
            }
            Event::Eof => {
                anyhow::ensure!(parents.is_empty(), "incomplete bookmark hierarchy");
                break;
            }
            Event::DocType(_) => anyhow::bail!("doctype in bookmark hierarchy"),
            _ => {}
        }
    }
    let buttons: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.attr("package") == PACKAGE
                && node.attr("class") == "android.widget.Button"
                && node.attr("resource-id") == "com.ss.android.ugc.trill:id/f1p"
                && node.attr("content-desc") == "Add or remove this video from Favorites."
                && node.attr("displayed") == "true"
        })
        .collect();
    let [(index, button)] = buttons.as_slice() else {
        return Ok(None);
    };
    if button.attr("enabled") != "true" || button.attr("clickable") != "true" {
        return Ok(None);
    }
    let Some((x, y, width, height)) = bounds(button.attr("bounds")) else {
        return Ok(None);
    };
    let icons: Vec<_> = nodes
        .iter()
        .filter(|node| {
            let parent = node.parent.and_then(|i| nodes.get(i));
            node.attr("package") == PACKAGE
                && node.attr("class") == "android.widget.ImageView"
                && node.attr("resource-id") == "com.ss.android.ugc.trill:id/f0y"
                && node.attr("displayed") == "true"
                && parent.is_some_and(|p| {
                    p.parent == Some(*index)
                        && p.attr("class") == "android.widget.FrameLayout"
                        && p.attr("resource-id") == "com.ss.android.ugc.trill:id/f14"
                        && p.attr("package") == PACKAGE
                })
        })
        .collect();
    let selected = match icons.as_slice() {
        [icon]
            if bounds(icon.attr("bounds")).is_some_and(|(ix, iy, iw, ih)| {
                ix >= x && iy >= y && ix + iw <= x + width && iy + ih <= y + height
            }) =>
        {
            match icon.attr("selected") {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            }
        }
        _ => None,
    };
    Ok(Some(StatefulElementBox {
        element: ElementBox {
            x,
            y,
            width,
            height,
            description: Some(button.attr("content-desc").into()),
            enabled: true,
            clickable: true,
        },
        checked: None,
        selected,
    }))
}

pub(crate) async fn read_bookmark_control(
    session: &dyn UiSession,
    labels: TikTokControls,
) -> anyhow::Result<Option<StatefulElementBox>> {
    if (
        labels.package(),
        labels.resource_version(),
        labels.language(),
    ) == (PACKAGE, Some("38.3.2"), "en")
    {
        return parse_control(&session.hierarchy_source_snapshot().await?.xml);
    }
    // False on an unmeasured container is not evidence of Unsaved.
    let Some(label) = labels.label(TikTokControl::Bookmark) else {
        return Ok(None);
    };
    Ok(session
        .locate(label.to_query())
        .await?
        .map(|element| StatefulElementBox {
            element,
            checked: None,
            selected: None,
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    const UNSAVED: &str =
        include_str!("../../../../fixtures/tiktok/bookmark-trill-38.3.2-en-unsaved.xml");
    const SAVED: &str =
        include_str!("../../../../fixtures/tiktok/bookmark-trill-38.3.2-en-saved.xml");

    struct CardPhone {
        authors: Mutex<VecDeque<&'static str>>,
        taps: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for CardPhone {
        async fn tap(&self, _: TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("unexpected swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected typing")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("unexpected home")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected tap")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn locate(
            &self,
            query: crate::ElementQuery<'_>,
        ) -> anyhow::Result<Option<ElementBox>> {
            let crate::ElementQuery::Description { value, .. } = query else {
                return Ok(None);
            };
            let description = match value {
                " profile" => format!(
                    "{} profile",
                    self.authors.lock().unwrap().pop_front().unwrap_or("A")
                ),
                "Sound:" => "Sound: fixture".into(),
                _ => return Ok(None),
            };
            Ok(Some(ElementBox {
                x: 10.0,
                y: 10.0,
                width: 10.0,
                height: 10.0,
                enabled: true,
                clickable: true,
                description: Some(description),
            }))
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            Ok(crate::HierarchySourceSnapshot {
                generation: 1,
                xml: if self.taps.load(Ordering::Relaxed) > 0 {
                    SAVED
                } else {
                    UNSAVED
                }
                .into(),
            })
        }
    }

    #[tokio::test]
    async fn changing_card_during_bookmark_read_never_arms_or_taps() {
        let labels = crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap();
        for authors in [vec!["A", "B"], vec!["A", "A", "A", "B"]] {
            let phone = CardPhone {
                authors: Mutex::new(authors.into()),
                taps: AtomicUsize::new(0),
            };
            let result = tiktok_save(&mut HierarchySaveAdapter::new(&phone, labels), |_| {
                panic!("changed card must not arm")
            })
            .await;
            assert!(!result.effect_boundary_crossed);
            assert_eq!(phone.taps.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn stable_card_uses_one_tap_and_the_measured_saved_icon() {
        let phone = CardPhone {
            authors: Mutex::new(VecDeque::new()),
            taps: AtomicUsize::new(0),
        };
        let labels = crate::tiktok_labels::controls_for(PACKAGE, "en", "38.3.2").unwrap();
        let armed = AtomicUsize::new(0);
        let result = tiktok_save(&mut HierarchySaveAdapter::new(&phone, labels), |_| {
            armed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .await;
        assert_eq!(result.verdict, SaveVerdict::Saved);
        assert_eq!(armed.load(Ordering::Relaxed), 1);
        assert_eq!(phone.taps.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn reads_measured_icon_state_not_the_false_container() {
        assert_eq!(
            parse_control(UNSAVED).unwrap().unwrap().selected,
            Some(false)
        );
        let saved = parse_control(SAVED).unwrap().unwrap();
        assert_eq!(saved.selected, Some(true));
        assert_eq!(saved.checked, None);
    }

    #[test]
    fn missing_or_ambiguous_icon_never_means_unsaved() {
        for source in [
            SAVED.replace("id/f0y", "id/unknown"),
            SAVED.replace("selected=\"true\"", "selected=\"unknown\""),
        ] {
            assert_eq!(parse_control(&source).unwrap().unwrap().selected, None);
        }
        let source = SAVED.replace("<hierarchy>", &format!("<hierarchy>{SAVED}"));
        assert!(parse_control(&source).unwrap().is_none());
        assert!(parse_control("<hierarchy>").is_err());
    }

    #[test]
    fn ignores_same_resource_outside_measured_button_ancestry() {
        let outside = r#"<node class="android.widget.ImageView" package="com.ss.android.ugc.trill" resource-id="com.ss.android.ugc.trill:id/f0y" selected="false" displayed="true" bounds="[0,0][20,20]"/>"#;
        assert_eq!(
            parse_control(&SAVED.replace("</hierarchy>", &format!("{outside}</hierarchy>")))
                .unwrap()
                .unwrap()
                .selected,
            Some(true)
        );
        assert_eq!(
            parse_control(&SAVED.replace("id/f14", "id/other"))
                .unwrap()
                .unwrap()
                .selected,
            None
        );
        assert_eq!(
            parse_control(&SAVED.replace("[942,1373][1060,1491]", "[0,0][100,100]"))
                .unwrap()
                .unwrap()
                .selected,
            None
        );
    }
}
