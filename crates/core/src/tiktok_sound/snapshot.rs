//! Carousel sound sheet measured on musically/en/46.2.42, 2026-09-07 (machine13).
//! Read one hierarchy generation for rows, selected tab and inline equalizer.
use super::*;
use quick_xml::{events::Event, Reader, XmlVersion};
use std::collections::HashMap;

const PACKAGE: &str = "com.zhiliaoapp.musically";

#[derive(Debug)]
struct Node {
    id: String,
    selected: bool,
    rect: ElementBox,
}

fn parse(xml: &str) -> anyhow::Result<Vec<Node>> {
    anyhow::ensure!(xml.len() <= 16 * 1024 * 1024, "sound snapshot size limit");
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let (mut count, mut depth) = (0, 0usize);
    loop {
        let event = reader.read_event()?;
        if matches!(event, Event::Start(_)) {
            depth += 1;
            anyhow::ensure!(depth <= 256, "sound snapshot depth limit");
        }
        match event {
            Event::Start(node) | Event::Empty(node) => {
                count += 1;
                anyhow::ensure!(count <= 32768, "sound snapshot node limit");
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
                if a("package") != PACKAGE
                    || a("displayed") != "true"
                    || !a("resource-id").starts_with(&format!("{PACKAGE}:id/"))
                {
                    continue;
                }
                let id = a("resource-id").trim_start_matches(PACKAGE);
                if !matches!(
                    id,
                    ":id/x2k"
                        | ":id/viewpager_container"
                        | ":id/vertical_item_music_new_rl"
                        | ":id/title"
                        | ":id/zdw"
                        | ":id/nve"
                ) {
                    continue;
                }
                let (start, end) = a("bounds")
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .and_then(|s| s.split_once("]["))
                    .context("sound bounds")?;
                let (x, y) = start.split_once(',').context("sound bounds")?;
                let (right, bottom) = end.split_once(',').context("sound bounds")?;
                let (x, y, right, bottom) = (
                    x.parse::<f64>()?,
                    y.parse::<f64>()?,
                    right.parse::<f64>()?,
                    bottom.parse::<f64>()?,
                );
                anyhow::ensure!(
                    [x, y, right, bottom].iter().all(|v| v.is_finite())
                        && x >= 0.0
                        && y >= 0.0
                        && right > x
                        && bottom > y,
                    "invalid sound bounds"
                );
                out.push(Node {
                    id: id.into(),
                    selected: a("selected") == "true",
                    rect: ElementBox {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                        description: Some(a("text").into()),
                        enabled: a("enabled") == "true",
                        clickable: a("clickable") == "true",
                    },
                });
            }
            Event::End(_) => depth = depth.checked_sub(1).context("sound nesting")?,
            Event::DocType(_) => anyhow::bail!("doctype in sound snapshot"),
            Event::Eof => {
                anyhow::ensure!(depth == 0, "incomplete sound snapshot");
                break;
            }
            _ => {}
        }
    }
    Ok(out)
}

fn hot_tab(nodes: &[Node]) -> anyhow::Result<(&ElementBox, bool)> {
    let tabs: Vec<_> = nodes.iter().filter(|n| n.id == ":id/x2k").collect();
    let hot: Vec<_> = tabs
        .iter()
        .filter(|n| n.rect.description.as_deref() == Some("Hot"))
        .collect();
    let [hot] = hot.as_slice() else {
        anyhow::bail!("Hot tab missing or ambiguous");
    };
    let selected: Vec<_> = tabs.iter().filter(|n| n.selected).collect();
    anyhow::ensure!(selected.len() == 1, "sound tab selection unreadable");
    anyhow::ensure!(hot.rect.enabled, "Hot tab disabled");
    Ok((&hot.rect, hot.selected))
}

pub(super) async fn select_hot_tab(session: &dyn UiSession) -> anyhow::Result<()> {
    let deadline = Instant::now() + PICKER_WINDOW;
    let mut tapped = false;
    loop {
        let parsed = parse(&session.hierarchy_source_snapshot().await?.xml)?;
        if let Ok((tab, selected)) = hot_tab(&parsed) {
            if selected {
                return Ok(());
            }
            if !tapped {
                session
                    .tap(tab.centre())
                    .await
                    .context("select Hot sound section")?;
                tapped = true;
            }
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Hot sound section did not become selected"
        );
        tokio::time::sleep(POLL).await;
    }
}

fn contains(outer: &ElementBox, inner: &ElementBox) -> bool {
    inner.x >= outer.x
        && inner.y >= outer.y
        && inner.x + inner.width <= outer.x + outer.width
        && inner.y + inner.height <= outer.y + outer.height
}

fn pool(xml: &str, plan: SoundPickerPlan, maximum: usize) -> anyhow::Result<ObservedSoundPool> {
    let nodes = parse(xml)?;
    anyhow::ensure!(hot_tab(&nodes)?.1, "Hot tab visible but not selected");
    let find = |id| {
        nodes
            .iter()
            .filter(|n| n.id == id)
            .map(|n| n.rect.clone())
            .collect::<Vec<_>>()
    };
    let viewport = exactly_one(find(":id/viewpager_container"), "sound list viewport")?;
    let titles = find(":id/title");
    let artists = find(":id/zdw");
    let mut rows = Vec::new();
    for row in find(":id/vertical_item_music_new_rl") {
        anyhow::ensure!(contains(&viewport, &row), "sound row outside list viewport");
        let row_titles = inside(&row, &titles);
        let row_artists = inside(&row, &artists);
        if row.y + row.height == viewport.y + viewport.height
            && (row_titles.is_empty() || row_artists.is_empty())
        {
            continue;
        }
        anyhow::ensure!(
            row_titles.len() == 1 && row_artists.len() == 1,
            "incomplete sound row"
        );
        anyhow::ensure!(
            contains(&row, &row_titles[0]) && contains(&row, &row_artists[0]),
            "clipped sound text"
        );
        rows.push(row);
    }
    assemble_pool(plan, rows, titles, artists, find(":id/nve"), maximum)
}

pub(super) async fn observe(
    session: &dyn UiSession,
    plan: SoundPickerPlan,
    maximum: usize,
) -> anyhow::Result<ObservedSoundPool> {
    let deadline = Instant::now() + PICKER_WINDOW;
    let mut previous: Option<ObservedSoundPool> = None;
    loop {
        let observed = pool(
            &session.hierarchy_source_snapshot().await?.xml,
            plan,
            maximum,
        );
        match observed {
            Ok(current) => {
                if previous.as_ref().is_some_and(|p| p == &current) {
                    return Ok(current);
                }
                previous = Some(current);
            }
            Err(error) => {
                previous = None;
                if Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        anyhow::ensure!(Instant::now() < deadline, "sound pool did not stabilize");
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct CarouselSession {
        hot: AtomicBool,
        selected: AtomicBool,
        closed: AtomicBool,
        taps: AtomicUsize,
        title: &'static str,
        selection_takes: bool,
    }
    #[async_trait::async_trait]
    impl UiSession for CarouselSession {
        async fn tap(&self, point: crate::TapPoint) -> anyhow::Result<()> {
            self.taps.fetch_add(1, Ordering::Relaxed);
            if point.y < 50.0 {
                self.hot.store(true, Ordering::Relaxed);
            } else if self.selection_takes {
                self.selected.fetch_xor(true, Ordering::Relaxed);
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn home(&self) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn back(&self) -> anyhow::Result<()> {
            self.closed.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            unreachable!()
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::driver::HierarchySourceSnapshot> {
            Ok(crate::driver::HierarchySourceSnapshot {
                generation: 1,
                xml: fixture(
                    self.hot.load(Ordering::Relaxed),
                    self.selected.load(Ordering::Relaxed),
                ),
            })
        }
        async fn locate_all_described(
            &self,
            query: ElementQuery<'_>,
        ) -> anyhow::Result<Vec<ElementBox>> {
            Ok(
                if matches!(query, ElementQuery::ResourceIdSuffix(":id/tv_top_text"))
                    && self.closed.load(Ordering::Relaxed)
                {
                    vec![ElementBox {
                        x: 0.0,
                        y: 0.0,
                        width: 100.0,
                        height: 30.0,
                        description: Some(self.title.into()),
                        enabled: true,
                        clickable: false,
                    }]
                } else {
                    vec![]
                },
            )
        }
    }
    fn session(selected: bool) -> CarouselSession {
        CarouselSession {
            hot: AtomicBool::new(false),
            selected: AtomicBool::new(selected),
            closed: AtomicBool::new(false),
            taps: AtomicUsize::new(0),
            title: "Two",
            selection_takes: true,
        }
    }
    #[tokio::test(start_paused = true)]
    async fn hot_tab_and_desired_sound_are_selected_once_then_sheet_closes() {
        for selected in [false, true] {
            let s = session(selected);
            select_hot_tab(&s).await.unwrap();
            let p = observe(&s, plan(), 5).await.unwrap();
            choose_and_confirm_sound(&s, plan(), &p, 1).await.unwrap();
            assert_eq!(s.taps.load(Ordering::Relaxed), 1 + usize::from(!selected));
            assert!(s.closed.load(Ordering::Relaxed));
        }
    }
    #[tokio::test(start_paused = true)]
    async fn missing_selection_marker_never_closes_or_retries() {
        let mut s = session(false);
        s.selection_takes = false;
        select_hot_tab(&s).await.unwrap();
        let p = observe(&s, plan(), 5).await.unwrap();
        assert!(choose_and_confirm_sound(&s, plan(), &p, 1).await.is_err());
        assert_eq!(s.taps.load(Ordering::Relaxed), 2);
        assert!(!s.closed.load(Ordering::Relaxed));
    }
    #[tokio::test(start_paused = true)]
    async fn selected_marker_does_not_replace_exact_editor_readback() {
        let mut s = session(true);
        s.title = "Wrong";
        select_hot_tab(&s).await.unwrap();
        let p = observe(&s, plan(), 5).await.unwrap();
        let e = choose_and_confirm_sound(&s, plan(), &p, 1)
            .await
            .unwrap_err();
        assert!(format!("{e:#}").contains("selected sound was not confirmed"));
        assert_eq!(s.taps.load(Ordering::Relaxed), 1);
    }
    fn node(id: &str, text: &str, bounds: &str, selected: bool) -> String {
        format!(
            r#"<android.widget.TextView package="{PACKAGE}" displayed="true" enabled="true" resource-id="{PACKAGE}{id}" text="{text}" selected="{selected}" bounds="{bounds}"/>"#
        )
    }
    fn fixture(hot: bool, marker: bool) -> String {
        let mut xml = String::from("<hierarchy>");
        xml += &node(":id/x2k", "Hot", "[0,0][80,40]", hot);
        xml += &node(":id/x2k", "For You", "[100,0][200,40]", !hot);
        xml += &node(":id/viewpager_container", "", "[0,50][500,310]", false);
        for (index, title) in ["One", "Two"].iter().enumerate() {
            let y = 60 + index * 100;
            xml += &node(
                ":id/vertical_item_music_new_rl",
                "",
                &format!("[0,{y}][500,{}]", y + 90),
                false,
            );
            xml += &node(
                ":id/title",
                title,
                &format!("[60,{}][400,{}]", y + 5, y + 30),
                false,
            );
            xml += &node(
                ":id/zdw",
                "Artist",
                &format!("[60,{}][400,{}]", y + 40, y + 60),
                false,
            );
        }
        if marker {
            xml += &node(":id/nve", "", "[10,165][30,185]", false);
        }
        xml + "</hierarchy>"
    }
    fn plan() -> SoundPickerPlan {
        SoundPickerPlan::resolve(PACKAGE, "en", "46.2.42").unwrap()
    }
    #[test]
    fn visible_hot_is_not_selected_hot() {
        assert!(pool(&fixture(false, true), plan(), 5)
            .unwrap_err()
            .to_string()
            .contains("not selected"));
    }
    #[test]
    fn snapshot_binds_all_rows_and_equalizer() {
        let p = pool(&fixture(true, true), plan(), 5).unwrap();
        assert_eq!(p.candidates.len(), 2);
        assert_eq!(p.selected_index, Some(1));
        assert!(plan().close_with_back);
        assert!(matches!(
            plan().post_back_query(),
            Some(ElementQuery::ResourceIdSuffix(":id/bot"))
        ));
    }
    #[test]
    fn wrong_package_cannot_prove_tab() {
        assert!(pool(
            &fixture(true, true).replace(PACKAGE, "other.app"),
            plan(),
            5
        )
        .is_err());
    }
    #[test]
    fn truncated_xml_and_duplicate_tab_are_rejected() {
        let xml = fixture(true, true);
        assert!(pool(xml.trim_end_matches("</hierarchy>"), plan(), 5).is_err());
        let duplicate = xml.replace(
            "</hierarchy>",
            &(node(":id/x2k", "Hot", "[0,0][80,40]", true) + "</hierarchy>"),
        );
        assert!(pool(&duplicate, plan(), 5).is_err());
    }
    #[test]
    fn bottom_partial_row_is_not_a_candidate() {
        let xml = fixture(true, true).replace(
            "</hierarchy>",
            &(node(
                ":id/vertical_item_music_new_rl",
                "",
                "[0,260][500,310]",
                false,
            ) + &node(":id/title", "Partial", "[60,265][400,295]", false)
                + "</hierarchy>"),
        );
        assert_eq!(pool(&xml, plan(), 5).unwrap().candidates.len(), 2);
    }
}
