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
            // AGENTS.md §9.189: measured independently on ce04171435f104080c.
            ("com.zhiliaoapp.musically", Some("45.4.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/k3j"),
                next: ElementQuery::ResourceIdSuffix(":id/w86"),
            }),
            // AGENTS.md §9.189: measured independently on ce051715e15b2c2e02.
            ("com.zhiliaoapp.musically", Some("46.1.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kfk"),
                next: ElementQuery::ResourceIdSuffix(":id/wrj"),
            }),
            // AGENTS.md §9.189: measured independently on ce031713aadf361905.
            ("com.zhiliaoapp.musically", Some("46.4.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/knq"),
                next: ElementQuery::ResourceIdSuffix(":id/x4j"),
            }),

            // 08/09/2026, ce031713b0c610ab0c, code 2024507030; each corner
            // carries its ordinal and Next confirms exactly three selected photos.
            ("com.zhiliaoapp.musically", Some("45.7.3")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/k_x"),
                next: ElementQuery::ResourceIdSuffix(":id/wjp"),
            }),
            // 08/09/2026, ce011711c354be2005, versionCode 2024600410:
            // kek carries ordinals 1,2,3 and wpw confirms Next (3).
            ("com.zhiliaoapp.musically", Some("46.0.41")) => Some(Self {
                package: "com.zhiliaoapp.musically",
                selector: ElementQuery::ResourceIdSuffix(":id/kek"),
                next: ElementQuery::ResourceIdSuffix(":id/wpw"),
            }),
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

fn snapshot_has_album(
    xml: &str,
    controls: PickerControls,
    query: ElementQuery<'_>,
    album: &str,
) -> anyhow::Result<bool> {
    let mut reader = Reader::from_str(xml);
    let mut matches = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(node) | Event::Empty(node) => {
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
                if a("displayed") != "true" || a("package") != controls.package {
                    continue;
                }
                let matched = match query {
                    ElementQuery::ResourceIdSuffix(suffix) => {
                        a("resource-id").starts_with(&format!("{}:", controls.package))
                            && a("resource-id").ends_with(suffix)
                    }
                    ElementQuery::Description { value, exact } => {
                        if exact {
                            a("content-desc") == value
                        } else {
                            a("content-desc").contains(value)
                        }
                    }
                    ElementQuery::Text { value, exact } => {
                        if exact {
                            a("text") == value
                        } else {
                            a("text").contains(value)
                        }
                    }
                    ElementQuery::ClassName(class) => a("class") == class,
                };
                if matched {
                    matches.push(a("text").trim() == album);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(matches == [true])
}

/// Match the visible contiguous slice back to the isolated album's initial grid.
/// TikTok 45.7.3, 09/09/2026: selecting image 10 in an 11-photo album hides
/// ordinals 1..3 and shifts the remaining controls uniformly by -181 px. A scroll
/// may translate y, but it must not reorder, resize, skip an interior cell or
/// replace the ordinal immediately before the next blank selection.
fn visible_selection(
    rows: Vec<ElementBox>,
    next: &[ElementBox],
    screen: Screen,
    initial: &[ElementBox],
    count: usize,
) -> Option<(Vec<ElementBox>, ElementBox)> {
    let [next] = next else {
        return None;
    };
    if !on_screen(next, screen)
        || !next.enabled
        || (count > 0
            && (!next.clickable || explicit_count(std::slice::from_ref(next)) != Some(count)))
    {
        return None;
    }
    let rows = ordered_controls(rows, screen, next.y)?;
    let offset = if count == 0 {
        0
    } else {
        rows.first()?
            .description
            .as_deref()?
            .trim()
            .parse::<usize>()
            .ok()?
            .checked_sub(1)?
    };
    if offset > count
        || offset + rows.len() > initial.len()
        || (count == 0 && rows.len() != initial.len())
        || count > offset + rows.len()
    {
        return None;
    }
    let shift = rows.first()?.y - initial.get(offset)?.y;
    for (index, row) in rows.iter().enumerate() {
        let absolute = offset + index;
        let reference = initial.get(absolute)?;
        let text = row.description.as_deref().unwrap_or("").trim();
        if (absolute < count && text.parse::<usize>() != Ok(absolute + 1))
            || (absolute >= count && !text.is_empty())
            || row.x != reference.x
            || row.width != reference.width
            || row.height != reference.height
            || row.y - reference.y != shift
        {
            return None;
        }
    }
    Some((rows, next.clone()))
}

impl<P: TapPlanner> Composer<'_, P> {
    /// One selection algorithm for 1..=13 photos, including TikTok's automatic
    /// scrolling. Every successful tap yields the next validated snapshot directly.
    pub(super) async fn select_verified(
        &mut self,
        controls: PickerControls,
        screen: Screen,
        wanted: usize,
        album: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<Selection> {
        if !(1..=13).contains(&wanted) {
            return Ok(Selection::NotEnoughSelected);
        }
        if stop.load(Ordering::Relaxed) {
            return Ok(Selection::Stopped);
        }
        let session = self.session;
        let album_query = self.plan.album_menu;
        let read = || async {
            let snapshot = session.hierarchy_source_snapshot().await?;
            let parsed = picker_snapshot(&snapshot.xml, controls)?;
            Ok::<_, anyhow::Error>(
                snapshot_has_album(&snapshot.xml, controls, album_query, album)?.then_some(parsed),
            )
        };
        let Some((initial, next)) = read().await? else {
            return Ok(Selection::NotEnoughSelected);
        };
        let [next_button] = next.as_slice() else {
            return Ok(Selection::NotEnoughSelected);
        };
        let Some(initial) = ordered_controls(initial, screen, next_button.y) else {
            return Ok(Selection::NotEnoughSelected);
        };
        if initial.len() != wanted
            || !selected_prefix(&initial, 0)
            || explicit_count(&next).is_some_and(|n| n != 0)
        {
            return Ok(Selection::NotEnoughSelected);
        }
        let Some(mut current) = visible_selection(initial.clone(), &next, screen, &initial, 0)
        else {
            return Ok(Selection::NotEnoughSelected);
        };
        let mut count = 0;
        let mut scrolls = 0;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(Selection::Stopped);
            }
            let (rows, next) = current;
            if count == wanted {
                return Ok(Selection::Armed {
                    next,
                    counted: Some(wanted),
                });
            }
            if let Some(row) = rows
                .iter()
                .find(|row| row.description.as_deref().unwrap_or("").trim().is_empty())
            {
                self.tap_inside(row).await?;
                let expected = count + 1;
                let deadline = Instant::now() + ARM_WINDOW;
                current = loop {
                    if stop.load(Ordering::Relaxed) {
                        return Ok(Selection::Stopped);
                    }
                    let Some((rows, next)) = read().await? else {
                        return Ok(Selection::NotEnoughSelected);
                    };
                    if let Some(observed) =
                        visible_selection(rows, &next, screen, &initial, expected)
                    {
                        break observed;
                    }
                    if Instant::now() >= deadline {
                        return Ok(Selection::NotEnoughSelected);
                    }
                    sleep(POLL, stop).await;
                };
                count = expected;
            } else {
                // The selected tail is visible but the next cell is below the viewport.
                // Scroll only one measured row so overlapping ordinals retain identity.
                if scrolls >= 3
                    || rows
                        .last()
                        .and_then(|row| row.description.as_deref())
                        .and_then(|text| text.trim().parse::<usize>().ok())
                        != Some(count)
                {
                    return Ok(Selection::NotEnoughSelected);
                }
                let row_height = rows
                    .windows(2)
                    .map(|rows| rows[1].y - rows[0].y)
                    .find(|distance| *distance > 1.0)
                    .context("picker rows missing")?;
                let from = rows.last().unwrap().y;
                self.session
                    .swipe(crate::SwipeGesture {
                        from: crate::TapPoint {
                            x: screen.width() * 0.5,
                            y: from,
                        },
                        to: crate::TapPoint {
                            x: screen.width() * 0.5,
                            y: from - row_height,
                        },
                        duration_ms: 450,
                    })
                    .await?;
                scrolls += 1;
                sleep(Duration::from_millis(450), stop).await;
                let Some((rows, next)) = read().await? else {
                    return Ok(Selection::NotEnoughSelected);
                };
                let Some(observed) = visible_selection(rows, &next, screen, &initial, count) else {
                    return Ok(Selection::NotEnoughSelected);
                };
                current = observed;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn measured_global_videos_have_one_corner_ordinal_and_one_next_count() {
        for (version, xml) in [
            (
                "45.4.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-45.4.3-en/video-selected.xml"
                ),
            ),
            (
                "45.7.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-45.7.3-en/video-selected.xml"
                ),
            ),
            (
                "46.0.41",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.0.41-en/video-selected.xml"
                ),
            ),
            (
                "46.1.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.1.3-en/video-selected.xml"
                ),
            ),
            (
                "46.2.1",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.2.1-en/video-selected.xml"
                ),
            ),
            (
                "46.4.3",
                include_str!(
                    "../../fixtures/tiktok-publish/musically-46.4.3-en/video-selected.xml"
                ),
            ),
        ] {
            let labels =
                crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", version)
                    .unwrap();
            let controls = PickerControls::for_labels(&labels).unwrap();
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert_eq!(rows.len(), 1, "{version}");
            assert!(selected_prefix(&rows, 1));
            assert_eq!(explicit_count(&next), Some(1));
        }
    }

    use super::*;
    #[test]
    fn new_fleet_photo_ordinals_and_next_count_match_each_measured_version() {
        for (version, initial, one, two) in [
            (
                "45.4.3",
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/picker.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/one.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-45.4.3-en/two.xml"),
            ),
            (
                "46.1.3",
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/picker.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/one.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.1.3-en/two.xml"),
            ),
            (
                "46.4.3",
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/picker.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/one.xml"),
                include_str!("../../fixtures/tiktok-publish/musically-46.4.3-en/two.xml"),
            ),
        ] {
            let labels =
                crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en-US", version)
                    .unwrap();
            let controls = PickerControls::for_labels(&labels).unwrap();
            for (count, xml) in [initial, one, two].into_iter().enumerate() {
                let (rows, next) = picker_snapshot(xml, controls).unwrap();
                assert_eq!(rows.len(), 2);
                assert!(selected_prefix(&rows, count));
                assert_eq!(explicit_count(&next), (count > 0).then_some(count));
            }
            assert!(ComposerPlan::missing_for_carousel(&labels).is_empty());
        }
    }
    use crate::TapPoint;
    use parking_lot::Mutex;
    struct Picker {
        selected: Mutex<usize>,
        taps: Mutex<Vec<TapPoint>>,
        drop_at: Option<usize>,
        wrong_album: bool,
        total: usize,
        autoscroll: bool,
        reads: std::sync::atomic::AtomicUsize,
        corrupt_after_ten: Option<&'static str>,
        hidden_thirteenth: bool,
        swipes: Mutex<usize>,
    }
    impl Picker {
        fn new(drop_at: Option<usize>) -> Self {
            Self {
                selected: Mutex::new(0),
                taps: Mutex::new(Vec::new()),
                drop_at,
                wrong_album: false,
                total: 6,
                autoscroll: false,
                reads: std::sync::atomic::AtomicUsize::new(0),
                corrupt_after_ten: None,
                hidden_thirteenth: false,
                swipes: Mutex::new(0),
            }
        }
        fn rows(&self) -> Vec<ElementBox> {
            let selected = *self.selected.lock();
            if self.autoscroll {
                let labels =
                    crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3")
                        .unwrap();
                let controls = PickerControls::for_labels(&labels).unwrap();
                let xml = if selected >= 10 {
                    include_str!("../../fixtures/tiktok-publish/picker-11-autoscroll-musically-45.7.3-en.xml")
                } else {
                    include_str!(
                        "../../fixtures/tiktok-publish/picker-11-initial-musically-45.7.3-en.xml"
                    )
                };
                let mut rows = picker_snapshot(xml, controls).unwrap().0;
                let offset = if selected >= 10 { 3 } else { 0 };
                for (index, row) in rows.iter_mut().enumerate() {
                    row.description = Some(if index + offset < selected {
                        (index + offset + 1).to_string()
                    } else {
                        String::new()
                    });
                }
                if selected >= 10 {
                    match self.corrupt_after_ten {
                        Some("gap") => {
                            rows.remove(2);
                        }
                        Some("misordered") => {
                            rows[1].description = Some("8".into());
                        }
                        Some("missing_tail") => {
                            rows.remove(rows.len() - 2);
                        }
                        _ => {}
                    }
                }
                return rows;
            }
            let scrolled = self.hidden_thirteenth && *self.swipes.lock() > 0;
            let offset = if scrolled { 3 } else { 0 };
            let visible_end = if self.hidden_thirteenth && selected > 0 && !scrolled {
                12
            } else {
                self.total
            };
            (offset..visible_end)
                .map(|index| ElementBox {
                    x: 268.0 + (index % 3) as f64 * 358.0,
                    y: 375.0 + ((index - offset) / 3) as f64 * 362.0,
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
            self.reads.fetch_add(1, Ordering::Relaxed);
            let rows = self.rows();
            let count = *self.selected.lock();
            let album =
                if self.wrong_album || (self.corrupt_after_ten == Some("album") && count >= 10) {
                    "Other"
                } else {
                    "album"
                };
            let cells=rows.iter().map(|r|format!(r#"<node package="fixture" class="android.widget.Button" resource-id="fixture:id/h4b" text="{}" bounds="[{},{}][{},{}]" displayed="true" enabled="true" clickable="true"/>"#,r.description.as_deref().unwrap_or(""),r.x,r.y,r.x+r.width,r.y+r.height)).collect::<String>();
            Ok(crate::driver::HierarchySourceSnapshot {
                generation: 1,
                xml: format!(
                    r#"<hierarchy><node package="fixture" class="android.widget.TextView" resource-id="fixture:fixture-album-menu" text="{album}" displayed="true"/>{cells}<node package="fixture" class="android.widget.Button" resource-id="fixture:id/q4g" text="Next ({count})" bounds="[552,1936][1044,2028]" displayed="true" enabled="true" clickable="{}"/></hierarchy>"#,
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
            let index = if (self.autoscroll && *self.selected.lock() >= 10)
                || (self.hidden_thirteenth && *self.swipes.lock() > 0)
            {
                index + 3
            } else {
                index
            };
            self.taps.lock().push(p);
            if self.drop_at != Some(index) {
                *self.selected.lock() += 1;
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            assert!(
                self.hidden_thirteenth,
                "scroll only when the selected tail hides the next image"
            );
            *self.swipes.lock() += 1;
            Ok(())
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
                    description: Some(
                        if self.wrong_album
                            || (self.corrupt_after_ten == Some("album") && count >= 10)
                        {
                            "Other"
                        } else {
                            "album"
                        }
                        .into(),
                    ),
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
                session.total,
                "album",
                &AtomicBool::new(false),
            )
            .await
            .unwrap()
    }
    #[tokio::test(start_paused = true)]
    async fn eleven_photos_follow_measured_automatic_scroll_after_tenth_tap() {
        let mut session = Picker::new(None);
        session.total = 11;
        session.autoscroll = true;
        assert!(matches!(
            run_picker(&session).await,
            Selection::Armed {
                counted: Some(11),
                ..
            }
        ));
        assert_eq!(session.taps.lock().len(), 11);
        assert_eq!(
            session.reads.load(Ordering::Relaxed),
            12,
            "one initial XML plus one readback per tap; confirmed snapshot is reused"
        );
        let taps = session.taps.lock();
        assert_eq!((taps[9].x, taps[9].y), (311.5, 1445.5));
        assert_eq!((taps[10].x, taps[10].y), (669.5, 1264.5));
    }
    #[tokio::test(start_paused = true)]
    async fn every_supported_count_uses_one_confirmed_snapshot_per_selection() {
        for total in 1..=13 {
            let mut session = Picker::new(None);
            session.total = total;
            assert!(
                matches!(run_picker(&session).await,Selection::Armed{counted:Some(count),..} if count==total),
                "{total}"
            );
            assert_eq!(session.taps.lock().len(), total);
            assert_eq!(session.reads.load(Ordering::Relaxed), total + 1);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn thirteenth_hidden_by_selection_tray_is_reached_by_one_anchored_scroll() {
        let mut session = Picker::new(None);
        session.total = 13;
        session.hidden_thirteenth = true;
        assert!(matches!(
            run_picker(&session).await,
            Selection::Armed {
                counted: Some(13),
                ..
            }
        ));
        assert_eq!(session.taps.lock().len(), 13);
        assert_eq!(*session.swipes.lock(), 1);
        assert_eq!(session.reads.load(Ordering::Relaxed), 15);
    }
    #[tokio::test(start_paused = true)]
    async fn scrolling_never_hides_dropped_selection_bad_ordinals_or_changed_album() {
        for corruption in ["gap", "misordered", "missing_tail", "album"] {
            let mut session = Picker::new(None);
            session.total = 11;
            session.autoscroll = true;
            session.corrupt_after_ten = Some(corruption);
            assert_eq!(
                run_picker(&session).await,
                Selection::NotEnoughSelected,
                "{corruption}"
            );
            assert_eq!(
                session.taps.lock().len(),
                10,
                "{corruption}: no eleventh tap after bad proof"
            );
        }
        let mut session = Picker::new(Some(9));
        session.total = 11;
        session.autoscroll = true;
        assert_eq!(run_picker(&session).await, Selection::NotEnoughSelected);
        assert_eq!(
            session.taps.lock().len(),
            10,
            "dropped tenth selection is never tapped again"
        );
    }
    #[test]
    fn measured_eleven_scroll_keeps_contiguous_ordinals_and_exact_next() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        let (initial, _) = picker_snapshot(
            include_str!("../../fixtures/tiktok-publish/picker-11-initial-musically-45.7.3-en.xml"),
            controls,
        )
        .unwrap();
        for (xml, count) in [
            (
                include_str!(
                    "../../fixtures/tiktok-publish/picker-11-nine-musically-45.7.3-en.xml"
                ),
                9,
            ),
            (
                include_str!(
                    "../../fixtures/tiktok-publish/picker-11-autoscroll-musically-45.7.3-en.xml"
                ),
                10,
            ),
        ] {
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert!(visible_selection(
                rows.clone(),
                &next,
                Screen::new(1080.0, 2220.0).unwrap(),
                &initial,
                count
            )
            .is_some());
            let mut wrong = rows;
            wrong.last_mut().unwrap().x += 1.0;
            assert!(visible_selection(
                wrong,
                &next,
                Screen::new(1080.0, 2220.0).unwrap(),
                &initial,
                count
            )
            .is_none());
        }
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
    fn measured_45_7_3_preserves_count_and_caption_through_sound_reproof() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en", "45.7.3").unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        for (count, xml) in [
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/06-camera.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/07-one.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/08-two.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/09-three.xml"),
        ]
        .into_iter()
        .enumerate()
        {
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert!(selected_prefix(&rows, count));
            assert_eq!(explicit_count(&next), (count > 0).then_some(count));
        }
        assert!(ComposerPlan::missing_for_carousel(&labels).is_empty());
        assert!(ComposerPlan::resolve(&labels)
            .unwrap()
            .can_publish_carousel());
        assert_eq!(
            labels
                .label(TikTokControl::ComposerCaption)
                .unwrap()
                .to_query(),
            ElementQuery::ResourceIdSuffix(":id/gpr")
        );
        for xml in [
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/18-typed.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-45.7.3-en/20-caption-return.xml"),
        ] {
            assert!(xml.contains(
                "text=\"RiviuBoxCalibration\" resource-id=\"com.zhiliaoapp.musically:id/gpr\""
            ));
        }
    }

    #[test]
    fn measured_46_0_41_picker_proves_each_photo_and_caption_survives_sound_reproof() {
        let labels =
            crate::tiktok_labels::controls_for("com.zhiliaoapp.musically", "en-US", "46.0.41")
                .unwrap();
        let controls = PickerControls::for_labels(&labels).unwrap();
        let fixtures = [
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-gallery.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-one.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-two.xml"),
            include_str!("../../fixtures/tiktok-publish/musically-46.0.41-en/q460-three.xml"),
        ];
        for (count, xml) in fixtures.into_iter().enumerate() {
            let (rows, next) = picker_snapshot(xml, controls).unwrap();
            assert!(selected_prefix(&rows, count));
            assert_eq!(explicit_count(&next), (count > 0).then_some(count));
            assert_eq!(next[0].clickable, count > 0);
        }
        assert!(ComposerPlan::missing_for_carousel(&labels).is_empty());
        assert!(ComposerPlan::resolve(&labels)
            .unwrap()
            .can_publish_carousel());
        assert_eq!(
            labels
                .label(TikTokControl::ComposerCaption)
                .unwrap()
                .to_query(),
            ElementQuery::ResourceIdSuffix(":id/guf")
        );
        for xml in [
            include_str!(
                "../../fixtures/tiktok-publish/musically-46.0.41-en/q460-caption-typed.xml"
            ),
            include_str!(
                "../../fixtures/tiktok-publish/musically-46.0.41-en/q460-caption-return.xml"
            ),
        ] {
            assert!(xml.contains(
                "text=\"RiviuCalibration20260908\" resource-id=\"com.zhiliaoapp.musically:id/guf\""
            ));
            assert!(xml.contains(
                "text=\"Add a catchy title\" resource-id=\"com.zhiliaoapp.musically:id/guj\""
            ));
        }
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
