//! A thumbnail opens preview; the measured corner button selects the photo.
use super::*;
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PickerControls {
    pub package: &'static str,
    pub selector: ElementQuery<'static>,
    pub next: ElementQuery<'static>,
}

impl PickerControls {
    pub fn for_labels(labels: &TikTokControls) -> Option<Self> {
        // 07/09/2026, 98895a3355424e484f: h4b text changes empty -> 1..6;
        // Next becomes Next (6). A thumbnail-centre tap opens preview instead.
        match (labels.package(), labels.resource_version()) {
            ("com.ss.android.ugc.trill", Some("38.3.2")) => Some(Self {
                package: "com.ss.android.ugc.trill",
                selector: ElementQuery::ResourceIdSuffix(":id/h4b"),
                next: ElementQuery::ResourceIdSuffix(":id/q4g"),
            }),
            // 07/09/2026, ce0517155ab38c390d: six kh7 corner controls in the
            // isolated album, measured separately from the 38.3.2 picker.
            ("com.zhiliaoapp.musically", Some("46.2.42")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kh7"),
                next: ElementQuery::ResourceIdSuffix(":id/wud"),
            }),
            // 07/09/2026, ce0717171c2a64d50d: 46.2.1 uses kir at the same
            // measured corner positions, not the 46.2.42 resource ID.
            ("com.zhiliaoapp.musically", Some("46.2.1")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kir"),
                next: ElementQuery::ResourceIdSuffix(":id/wwo"),
            }),
            _ => None,
        }
    }
}

fn label_count(text: &str) -> Option<usize> {
    let text = text.trim();
    let number = text
        .strip_prefix("Next")
        .or_else(|| text.strip_prefix("Tiếp"))?
        .trim();
    let number = number
        .strip_prefix('(')
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(number);
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

pub(super) fn explicit_count(rows: &[ElementBox]) -> Option<usize> {
    let [row] = rows else {
        return None;
    };
    label_count(row.description.as_deref()?)
}

fn ordered_controls(
    mut rows: Vec<ElementBox>,
    screen: Screen,
    bottom: f64,
) -> Option<Vec<ElementBox>> {
    if rows.is_empty() {
        return None;
    }
    if rows.iter().any(|row| {
        !on_screen(row, screen) || row.y + row.height > bottom || !row.enabled || !row.clickable
    }) {
        return None;
    }
    rows.sort_by(|a, b| a.y.total_cmp(&b.y).then_with(|| a.x.total_cmp(&b.x)));
    if rows
        .windows(2)
        .any(|p| p[0].x == p[1].x && p[0].y == p[1].y)
    {
        return None;
    }
    Some(rows)
}

fn selected_prefix(rows: &[ElementBox], count: usize) -> bool {
    rows.iter().enumerate().all(|(index, row)| {
        let text = row.description.as_deref().unwrap_or("").trim();
        if index < count {
            text.parse::<usize>() == Ok(index + 1)
        } else {
            text.is_empty()
        }
    })
}

fn picker_snapshot(
    xml: &str,
    controls: PickerControls,
) -> anyhow::Result<(Vec<ElementBox>, Vec<ElementBox>)> {
    anyhow::ensure!(xml.len() <= 16 * 1024 * 1024, "picker snapshot too large");
    let suffix = |query| match query {
        ElementQuery::ResourceIdSuffix(value) => value,
        _ => "unmeasured",
    };
    let mut reader = Reader::from_str(xml);
    let mut selectors = Vec::new();
    let mut next = Vec::new();
    let mut nodes = 0;
    let mut depth = 0usize;
    loop {
        let event = reader.read_event()?;
        if matches!(event, Event::Start(_)) {
            depth += 1;
            anyhow::ensure!(depth <= 256, "picker depth limit");
        }
        match event {
            Event::Start(node) | Event::Empty(node) => {
                nodes += 1;
                anyhow::ensure!(nodes <= 32768, "picker node limit");
                let mut attrs = HashMap::new();
                for attr in node.attributes() {
                    let attr = attr?;
                    attrs.insert(
                        std::str::from_utf8(attr.key.as_ref())?.to_string(),
                        attr.decoded_and_normalized_value(
                            XmlVersion::Implicit1_0,
                            reader.decoder(),
                        )?
                        .into_owned(),
                    );
                }
                let a = |key: &str| attrs.get(key).map(String::as_str).unwrap_or("");
                let id = a("resource-id");
                let is_selector = id.ends_with(suffix(controls.selector));
                let is_next = id.ends_with(suffix(controls.next));
                if (!is_selector && !is_next)
                    || a("displayed") != "true"
                    || a("package") != controls.package
                    || !id.starts_with(&format!("{}:", a("package")))
                {
                    continue;
                }
                let Some((start, end)) = a("bounds")
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.split_once("]["))
                else {
                    anyhow::bail!("picker bounds missing");
                };
                let (x, y) = start.split_once(',').context("picker bounds")?;
                let (r, b) = end.split_once(',').context("picker bounds")?;
                let (x, y, r, b) = (
                    x.parse::<f64>()?,
                    y.parse::<f64>()?,
                    r.parse::<f64>()?,
                    b.parse::<f64>()?,
                );
                let row = ElementBox {
                    x,
                    y,
                    width: r - x,
                    height: b - y,
                    description: Some(a("text").into()),
                    enabled: a("enabled") == "true",
                    clickable: a("clickable") == "true",
                };
                if is_selector {
                    anyhow::ensure!(
                        a("class") == "android.widget.Button",
                        "picker selector class"
                    );
                    selectors.push(row);
                } else {
                    next.push(row);
                }
            }
            Event::DocType(_) => anyhow::bail!("doctype in picker"),
            Event::End(_) => {
                depth = depth.checked_sub(1).context("invalid picker nesting")?;
            }
            Event::Eof => {
                anyhow::ensure!(depth == 0, "incomplete picker snapshot");
                break;
            }
            _ => {}
        }
    }
    Ok((selectors, next))
}

impl<P: TapPlanner> Composer<'_, P> {
    pub(super) async fn select_verified(
        &mut self,
        controls: PickerControls,
        screen: Screen,
        count: usize,
        album: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<Selection> {
        let read = || async {
            picker_snapshot(
                &self.session.hierarchy_source_snapshot().await?.xml,
                controls,
            )
        };
        let (initial, initial_next) = read().await?;
        let [initial_next] = initial_next.as_slice() else {
            return Ok(Selection::NotEnoughSelected);
        };
        let Some(initial) = ordered_controls(initial, screen, initial_next.y) else {
            return Ok(Selection::NotEnoughSelected);
        };
        if count == 0 || initial.len() != count || !selected_prefix(&initial, 0) {
            return Ok(Selection::NotEnoughSelected);
        }
        for index in 0..count {
            if stop.load(Ordering::Relaxed) {
                return Ok(Selection::Stopped);
            }
            if !self.pill_reads(album, stop).await? {
                return Ok(Selection::NotEnoughSelected);
            }
            let Some(rows) = ordered_controls(read().await?.0, screen, initial_next.y) else {
                return Ok(Selection::NotEnoughSelected);
            };
            if rows.len() != count
                || !selected_prefix(&rows, index)
                || !rows.iter().zip(&initial).all(|(a, b)| {
                    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
                })
            {
                return Ok(Selection::NotEnoughSelected);
            }
            self.tap_inside(&rows[index]).await?;
            let deadline = Instant::now() + ARM_WINDOW;
            loop {
                if stop.load(Ordering::Relaxed) {
                    return Ok(Selection::Stopped);
                }
                let (observed, next) = read().await?;
                if explicit_count(&next) == Some(index + 1)
                    && ordered_controls(observed, screen, initial_next.y)
                        .is_some_and(|r| r.len() == count && selected_prefix(&r, index + 1))
                {
                    break;
                }
                if Instant::now() >= deadline {
                    return Ok(Selection::NotEnoughSelected);
                }
                sleep(POLL, stop).await;
            }
        }
        if !self.pill_reads(album, stop).await? {
            return Ok(Selection::NotEnoughSelected);
        }
        let (_, next) = read().await?;
        match next.as_slice() {
            [next]
                if next.clickable
                    && next.enabled
                    && explicit_count(std::slice::from_ref(next)) == Some(count) =>
            {
                Ok(Selection::Armed {
                    next: next.clone(),
                    counted: Some(count),
                })
            }
            _ => Ok(Selection::NotEnoughSelected),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TapPoint;
    use parking_lot::Mutex;
    struct Picker {
        selected: Mutex<usize>,
        taps: Mutex<Vec<TapPoint>>,
        drop_at: Option<usize>,
        wrong_album: bool,
    }
    impl Picker {
        fn new(drop_at: Option<usize>) -> Self {
            Self {
                selected: Mutex::new(0),
                taps: Mutex::new(Vec::new()),
                drop_at,
                wrong_album: false,
            }
        }
        fn rows(&self) -> Vec<ElementBox> {
            let selected = *self.selected.lock();
            (0..6)
                .map(|index| ElementBox {
                    x: 268.0 + (index % 3) as f64 * 358.0,
                    y: 375.0 + (index / 3) as f64 * 362.0,
                    width: 72.0,
                    height: 72.0,
                    description: Some(if index < selected {
                        (index + 1).to_string()
                    } else {
                        String::new()
                    }),
                    clickable: true,
                    enabled: true,
                })
                .collect()
        }
    }
    #[async_trait::async_trait]
    impl UiSession for Picker {
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::driver::HierarchySourceSnapshot> {
            let rows = self.rows();
            let count = *self.selected.lock();
            let cells=rows.iter().map(|r|format!(r#"<node package="fixture" class="android.widget.Button" resource-id="fixture:id/h4b" text="{}" bounds="[{},{}][{},{}]" displayed="true" enabled="true" clickable="true"/>"#,r.description.as_deref().unwrap_or(""),r.x,r.y,r.x+r.width,r.y+r.height)).collect::<String>();
            Ok(crate::driver::HierarchySourceSnapshot {
                generation: 1,
                xml: format!(
                    r#"<hierarchy>{cells}<node package="fixture" class="android.widget.Button" resource-id="fixture:id/q4g" text="Next ({count})" bounds="[552,1936][1044,2028]" displayed="true" enabled="true" clickable="{}"/></hierarchy>"#,
                    count > 0
                ),
            })
        }
        async fn tap(&self, p: TapPoint) -> anyhow::Result<()> {
            let index = self
                .rows()
                .iter()
                .position(|r| {
                    p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height
                })
                .expect("only corner buttons may be tapped");
            self.taps.lock().push(p);
            if self.drop_at != Some(index) {
                *self.selected.lock() += 1;
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("no scrolling fallback")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("selection must not type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn back(&self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("no arbitrary controls")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn locate(&self, q: ElementQuery<'_>) -> anyhow::Result<Option<ElementBox>> {
            Ok(self.locate_all_described(q).await?.into_iter().next())
        }
        async fn locate_all_described(
            &self,
            q: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            let key = match q {
                ElementQuery::Description { value, .. }
                | ElementQuery::Text { value, .. }
                | ElementQuery::ResourceIdSuffix(value)
                | ElementQuery::ClassName(value) => value,
            };
            if key == ":id/h4b" {
                return Ok(self.rows());
            }
            let count = *self.selected.lock();
            Ok(match key {
                "fixture-picker-next" => vec![ElementBox {
                    x: 552.0,
                    y: 1936.0,
                    width: 492.0,
                    height: 92.0,
                    description: Some(if count == 0 {
                        "Next".into()
                    } else {
                        format!("Next ({count})")
                    }),
                    clickable: count > 0,
                    enabled: true,
                }],
                "fixture-album-menu" => vec![ElementBox {
                    x: 150.0,
                    y: 90.0,
                    width: 400.0,
                    height: 80.0,
                    description: Some(if self.wrong_album { "Other" } else { "album" }.into()),
                    enabled: true,
                    clickable: true,
                }],
                _ => vec![],
            })
        }
    }
    async fn run_picker(session: &Picker) -> Selection {
        let screen = Screen::new(1080.0, 2220.0).unwrap();
        let plan =
            ComposerPlan::resolve(&crate::tiktok_labels::every_publish_control_measured()).unwrap();
        let mut composer = Composer::new(session, plan, |r: &ElementBox| r.centre());
        composer
            .select_verified(
                PickerControls {
                    package: "fixture",
                    selector: ElementQuery::ResourceIdSuffix(":id/h4b"),
                    next: ElementQuery::ResourceIdSuffix(":id/q4g"),
                },
                screen,
                6,
                "album",
                &AtomicBool::new(false),
            )
            .await
            .unwrap()
    }
    #[tokio::test(start_paused = true)]
    async fn six_photos_require_six_corner_taps_and_six_ordinals() {
        let session = Picker::new(None);
        assert!(matches!(
            run_picker(&session).await,
            Selection::Armed {
                counted: Some(6),
                ..
            }
        ));
        assert_eq!(session.taps.lock().len(), 6);
    }
    #[tokio::test(start_paused = true)]
    async fn dropped_selection_stops_without_retry_or_remaining_taps() {
        let session = Picker::new(Some(2));
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert_eq!(session.taps.lock().len(), 3);
        assert_eq!(*session.selected.lock(), 2);
    }
    #[tokio::test(start_paused = true)]
    async fn changed_album_never_taps_a_thumbnail() {
        let mut session = Picker::new(None);
        session.wrong_album = true;
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert!(session.taps.lock().is_empty());
    }
    #[test]
    fn count_requires_one_exact_picker_label() {
        for (text, wanted) in [
            ("Next (6)", Some(6)),
            ("Tiếp (13)", Some(13)),
            ("Next 2", Some(2)),
            ("Next", None),
            ("Next (1/6)", None),
            ("Preview Next 6", None),
        ] {
            assert_eq!(label_count(text), wanted);
        }
    }
    #[test]
    fn ordinal_readback_requires_every_photo_in_order() {
        let rows: Vec<_> = (1..=6)
            .map(|n| ElementBox {
                description: Some(n.to_string()),
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                enabled: true,
                clickable: true,
            })
            .collect();
        assert!(selected_prefix(&rows, 6));
        let mut missed = rows.clone();
        missed[4].description = Some(String::new());
        assert!(!selected_prefix(&missed, 6));
        let mut duplicate = rows;
        duplicate[5].description = Some("1".into());
        assert!(!selected_prefix(&duplicate, 6));
    }
    #[test]
    fn snapshot_refuses_malformed_and_ignores_foreign_controls() {
        let controls = PickerControls {
            package: "fixture",
            selector: ElementQuery::ResourceIdSuffix(":id/h4b"),
            next: ElementQuery::ResourceIdSuffix(":id/q4g"),
        };
        assert!(picker_snapshot("<hierarchy><node>", controls).is_err());
        assert!(picker_snapshot("<!DOCTYPE hierarchy><hierarchy/>", controls).is_err());
        let xml = r#"<hierarchy><node package="other" resource-id="other:id/h4b" text="1" bounds="[1,1][2,2]" class="android.widget.Button" displayed="true" enabled="true" clickable="true"/></hierarchy>"#;
        assert!(picker_snapshot(xml, controls).unwrap().0.is_empty());
    }
}
