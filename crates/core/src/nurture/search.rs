//! Keyword navigation uses current accessibility nodes; it never falls back to FYP.
use crate::{driver::UiSession, ui_automation::tree::Tree, ElementBox};
use anyhow::{ensure, Context};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn fold(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn nodes(
    tree: &Tree,
    package: &str,
    predicate: impl Fn(&crate::ui_automation::tree::Node) -> bool,
) -> Vec<ElementBox> {
    tree.nodes
        .iter()
        .enumerate()
        .filter(|(i, n)| n.visible(package) && tree.ancestors_visible(*i) && predicate(n))
        .filter_map(|(_, n)| n.rect())
        .filter(|r| r.enabled)
        .collect()
}
fn unique(mut values: Vec<ElementBox>) -> anyhow::Result<Option<ElementBox>> {
    values.dedup_by(|a, b| a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height);
    ensure!(
        values.len() <= 1,
        "Có nhiều nút tìm kiếm cùng khớp; chưa thao tác"
    );
    Ok(values.pop())
}
fn query_matches(tree: &Tree, package: &str, keyword: &str) -> bool {
    tree.nodes.iter().any(|n| {
        n.visible(package)
            && n.attr("class") == "android.widget.EditText"
            && fold(n.attr("text")) == fold(keyword)
    })
}

fn result_cards(tree: &Tree, package: &str, version: &str) -> Vec<ElementBox> {
    let mut results = nodes(tree, package, |n| {
        n.attr("clickable") == "true"
            && (n.attr("content-desc").starts_with("Video by ")
                || n.attr("content-desc").starts_with("Video của "))
    });
    // Selected Videos grids captured on the current Android fleet, 19/09/2026.
    // Cards have no description; both caption and author must be inside the
    // measured card/grid pair. IDs never cross build boundaries.
    let measured = match (package, version) {
        ("com.zhiliaoapp.musically", "45.4.3") => Some(("ty7", "men", "b79")),
        ("com.zhiliaoapp.musically", "45.7.3") => Some(("u_u", "mo3", "b96")),
        ("com.zhiliaoapp.musically", "46.0.41") => Some(("ufz", "msw", "bc6")),
        ("com.zhiliaoapp.musically", "46.1.3") => Some(("uhi", "mui", "bce")),
        ("com.zhiliaoapp.musically", "46.4.3") => Some(("uu9", "n3c", "bdu")),
        _ => None,
    };
    if let Some((card_id, grid_id, author_id)) = measured.filter(|_| results.is_empty()) {
        let card_id = format!("{package}:id/{card_id}");
        let grid_id = format!("{package}:id/{grid_id}");
        let author_id = format!(":id/{author_id}");
        for (index, card) in tree.nodes.iter().enumerate() {
            if !card.visible(package)
                || !tree.ancestors_visible(index)
                || card.attr("resource-id") != card_id
                || card.attr("clickable") != "true"
                || !card.parent.is_some_and(|parent| {
                    let grid = &tree.nodes[parent];
                    grid.attr("class") == "android.widget.GridView"
                        && grid.attr("resource-id") == grid_id
                })
            {
                continue;
            }
            let descendant_has = |suffix: &str| {
                tree.nodes.iter().enumerate().any(|(child, node)| {
                    if !node.visible(package)
                        || !tree.ancestors_visible(child)
                        || !node.attr("resource-id").ends_with(suffix)
                        || node.attr("text").trim().is_empty()
                    {
                        return false;
                    }
                    let mut parent = node.parent;
                    while let Some(p) = parent {
                        if p == index {
                            return true;
                        }
                        parent = tree.nodes[p].parent;
                    }
                    false
                })
            };
            if descendant_has(":id/desc") && descendant_has(&author_id) {
                if let Some(rect) = card.rect().filter(|r| r.enabled) {
                    results.push(rect);
                }
            }
        }
    }
    results.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    results
}
async fn read(session: &dyn UiSession, package: &str) -> anyhow::Result<Tree> {
    ensure!(
        session.active_app_bundle().await? == package,
        "Đã rời TikTok khi tìm kiếm"
    );
    Tree::parse(session.hierarchy_source_snapshot().await?)
}

fn location_cancel(tree: &Tree, package: &str, version: &str) -> Option<ElementBox> {
    if package != "com.zhiliaoapp.musically" {
        return None;
    }
    let panel_id = match version {
        "45.7.3" => ":id/pwe",
        "46.0.41" => ":id/q2t",
        _ => return None,
    };
    let matching = |query| tree.matching(package, query);
    let titles = matching(crate::ElementQuery::Text {
        value: "See relevant content and places nearby",
        exact: true,
    });
    let messages=matching(crate::ElementQuery::Text{value:"Open your device settings and go to Locations > While Using the App. You can turn this off at any time.",exact:true});
    let controls = matching(crate::ElementQuery::ResourceIdSuffix(":id/button3"));
    let ([title], [message], [control]) =
        (titles.as_slice(), messages.as_slice(), controls.as_slice())
    else {
        return None;
    };
    let node = &tree.nodes[*control];
    let panels = matching(crate::ElementQuery::ResourceIdSuffix(panel_id));
    let [panel] = panels.as_slice() else {
        return None;
    };
    let panel = *panel;
    if !tree.inside(*title, panel) || !tree.inside(*message, panel) || node.attr("text") != "Cancel"
    {
        return None;
    }
    node.rect().filter(|r| r.enabled && r.clickable)
}

async fn read_search(
    session: &dyn UiSession,
    package: &str,
    version: &str,
    stop: &AtomicBool,
    deadline: Instant,
) -> anyhow::Result<Tree> {
    let tree = read(session, package).await?;
    if let Some(cancel) = location_cancel(&tree, package, version) {
        check(stop, deadline)?;
        session.tap(cancel.centre()).await?;
        super::sleep_interruptible(Duration::from_millis(400), stop).await;
        return read(session, package).await;
    }
    Ok(tree)
}
fn check(stop: &AtomicBool, deadline: Instant) -> anyhow::Result<()> {
    ensure!(!stop.load(Ordering::Relaxed), "Đã dừng tìm kiếm");
    ensure!(
        Instant::now() < deadline,
        "Hết thời gian mở video theo từ khóa"
    );
    Ok(())
}

pub(super) async fn open(
    session: &dyn UiSession,
    package: &str,
    keyword: &str,
    stop: &AtomicBool,
    deadline: Instant,
) -> anyhow::Result<()> {
    ensure!(
        !keyword.trim().is_empty()
            && keyword.chars().count() <= 100
            && !keyword.chars().any(char::is_control),
        "Từ khóa tìm kiếm không hợp lệ"
    );
    let deadline = deadline.min(Instant::now() + Duration::from_secs(75));
    let version = session.app_version(package).await.unwrap_or_default();
    let mut tree = read_search(session, package, &version, stop, deadline).await?;
    // Trill 38.3.2/en, SM-G955F, 2026-09-16: LIVE and Search share :id/gka.
    // Search is to the right of the located For You tab; LIVE is to its left.
    let feed_right = nodes(&tree, package, |n| {
        ["for you", "đề xuất", "dành cho bạn"].contains(&fold(n.attr("content-desc")).as_str())
    })
    .into_iter()
    .map(|r| r.x + r.width)
    .max_by(f64::total_cmp);
    if nodes(&tree, package, |n| {
        n.attr("class") == "android.widget.EditText"
    })
    .is_empty()
    {
        let button = unique(nodes(&tree, package, |n| {
            n.attr("clickable") == "true"
                && (["search", "tìm kiếm"].contains(&fold(n.attr("content-desc")).as_str())
                    || (package == "com.ss.android.ugc.trill"
                        && version == "38.3.2"
                        && n.attr("resource-id").ends_with(":id/gka")
                        && n.rect()
                            .is_some_and(|r| feed_right.is_some_and(|right| r.x >= right))))
        }))?
        .context("Không xác định được nút Tìm kiếm TikTok")?;
        check(stop, deadline)?;
        session.tap(button.centre()).await?;
    }
    let input = loop {
        check(stop, deadline)?;
        tree = read_search(session, package, &version, stop, deadline).await?;
        if let Some(input) = unique(nodes(&tree, package, |n| {
            n.attr("class") == "android.widget.EditText"
        }))? {
            break input;
        }
        super::sleep_interruptible(Duration::from_millis(400), stop).await;
    };
    check(stop, deadline)?;
    session.tap(input.centre()).await?;
    check(stop, deadline)?;
    session.type_text(keyword.trim()).await?;
    tree = read_search(session, package, &version, stop, deadline).await?;
    ensure!(
        query_matches(&tree, package, keyword),
        "Ô tìm kiếm chưa chứa đúng từ khóa"
    );
    let search = unique(nodes(&tree, package, |n| {
        n.attr("clickable") == "true"
            && ["search", "tìm kiếm"].contains(&fold(n.attr("text")).as_str())
    }))?
    .context("Không thấy nút xác nhận Tìm kiếm")?;
    check(stop, deadline)?;
    session.tap(search.centre()).await?;
    loop {
        check(stop, deadline)?;
        tree = read_search(session, package, &version, stop, deadline).await?;
        if nodes(&tree, package, |n| {
            n.attr("class") == "android.widget.EditText"
        })
        .is_empty()
        {
            super::sleep_interruptible(Duration::from_millis(400), stop).await;
            continue;
        }
        ensure!(
            query_matches(&tree, package, keyword),
            "Từ khóa đã thay đổi khi tìm kiếm"
        );
        if nodes(&tree, package, |n| {
            ["videos", "video"].contains(&fold(n.attr("content-desc")).as_str())
        })
        .len()
            == 1
        {
            break;
        }
        super::sleep_interruptible(Duration::from_millis(400), stop).await;
    }
    let mut prior_tab: Option<ElementBox> = None;
    let mut tab_taps = 0;
    let first = loop {
        check(stop, deadline)?;
        tree = read_search(session, package, &version, stop, deadline).await?;
        if nodes(&tree, package, |n| {
            n.attr("class") == "android.widget.EditText"
        })
        .is_empty()
        {
            prior_tab = None;
            super::sleep_interruptible(Duration::from_millis(400), stop).await;
            continue;
        }
        ensure!(
            query_matches(&tree, package, keyword),
            "Kết quả không thuộc từ khóa đã nhập"
        );
        let tabs = nodes(&tree, package, |n| {
            ["videos", "video"].contains(&fold(n.attr("content-desc")).as_str())
        });
        // During insertion of LIVE, TikTok briefly exposes both old and new
        // Videos tab geometries. This is pending rendering, not permission to
        // pick one arbitrarily or a permanent search failure.
        let [tab] = tabs.as_slice() else {
            prior_tab = None;
            super::sleep_interruptible(Duration::from_millis(400), stop).await;
            continue;
        };
        let videos_selected = tree.nodes.iter().enumerate().any(|(i, n)| {
            n.visible(package)
                && tree.ancestors_visible(i)
                && n.attr("enabled") == "true"
                && ["videos", "video"].contains(&fold(n.attr("content-desc")).as_str())
                && (n.attr("selected") == "true" || n.attr("clickable") == "false")
        });
        if !videos_selected {
            {
                // TikTok inserts LIVE before Videos after loading results. Require two
                // agreeing fresh geometries and verify selection before touching a card.
                if prior_tab.as_ref().is_some_and(|prior| {
                    (prior.x - tab.x).abs() < 2.0
                        && (prior.y - tab.y).abs() < 2.0
                        && (prior.width - tab.width).abs() < 2.0
                        && (prior.height - tab.height).abs() < 2.0
                }) {
                    ensure!(tab_taps < 3, "Tab Videos không giữ được trạng thái đã chọn");
                    check(stop, deadline)?;
                    session.tap(tab.centre()).await?;
                    tab_taps += 1;
                    prior_tab = None;
                } else {
                    prior_tab = Some(tab.clone());
                }
            }
            super::sleep_interruptible(Duration::from_millis(400), stop).await;
            continue;
        }
        let results = result_cards(&tree, package, &version);
        if let Some(first) = results.into_iter().next() {
            break first;
        }
        if tree.nodes.iter().any(|n| {
            n.visible(package)
                && ["no results found", "không tìm thấy kết quả"]
                    .contains(&fold(n.attr("text")).as_str())
        }) {
            anyhow::bail!("Không có video cho từ khóa đã nhập");
        }
        super::sleep_interruptible(Duration::from_millis(400), stop).await;
    };
    check(stop, deadline)?;
    session.tap(first.centre()).await?;
    loop {
        check(stop, deadline)?;
        if on_video(session, package, keyword).await? {
            return Ok(());
        }
        super::sleep_interruptible(Duration::from_millis(400), stop).await;
    }
}

pub(super) async fn on_video(
    session: &dyn UiSession,
    package: &str,
    keyword: &str,
) -> anyhow::Result<bool> {
    let tree = read(session, package).await?;
    let version = session.app_version(package).await.unwrap_or_default();
    Ok(video_matches_build(&tree, package, keyword, &version))
}
#[cfg(test)]
fn video_matches(tree: &Tree, package: &str, _keyword: &str) -> bool {
    video_matches_build(
        tree,
        package,
        _keyword,
        if package == "com.ss.android.ugc.trill" {
            "38.3.2"
        } else {
            "45.7.3"
        },
    )
}
fn video_matches_build(tree: &Tree, package: &str, _keyword: &str, version: &str) -> bool {
    let texts: Vec<_> = tree.nodes.iter().filter(|n| n.visible(package)).collect();
    // Trill 38.3.2/en, 2026-09-16: viewer replaces the original query with a
    // related suggestion. Exact query is proven in the results grid before entry;
    // Back + Search above the action rail identify the resulting search viewer.
    let backs = nodes(tree, package, |n| {
        ["back", "quay lại"].contains(&fold(n.attr("content-desc")).as_str())
    });
    let searches = nodes(tree, package, |n| {
        n.attr("class") == "android.widget.Button"
            && ["search", "tìm kiếm"].contains(&fold(n.attr("text")).as_str())
    });
    let header = backs.len() == 1
        && searches.len() == 1
        && backs[0].y < searches[0].y + searches[0].height
        && searches[0].y < backs[0].y + backs[0].height;
    // Same measured search viewer after a vertical swipe hides its search header.
    // The video surface and its inline comment input below the action rail remain;
    // FYP has a bottom navigation bar instead. Entry provenance was checked above.
    let video_bottom = nodes(tree, package, |n| {
        n.attr("resource-id").ends_with(":id/long_press_layout")
            && n.attr("content-desc") == "Video"
    })
    .into_iter()
    .map(|r| r.y + r.height)
    .max_by(f64::total_cmp);
    let input_id = match (package, version) {
        ("com.ss.android.ugc.trill", "38.3.2") => Some("com.ss.android.ugc.trill:id/cnd"),
        ("com.zhiliaoapp.musically", "45.7.3") => Some("com.zhiliaoapp.musically:id/e7q"),
        ("com.zhiliaoapp.musically", "46.0.41") => Some("com.zhiliaoapp.musically:id/eal"),
        _ => None,
    };
    let inline_comment = nodes(tree, package, |n| {
        n.attr("class") == "android.widget.EditText" && input_id == Some(n.attr("resource-id"))
    })
    .iter()
    .any(|r| video_bottom.is_some_and(|bottom| r.y >= bottom));
    let comments = texts.iter().any(|n| {
        let d = fold(n.attr("content-desc"));
        d.starts_with("read or add comments") || d.starts_with("đọc hoặc thêm bình luận")
    });
    let feed = texts.iter().any(|n| {
        ["for you", "đề xuất", "dành cho bạn"].contains(&fold(n.attr("content-desc")).as_str())
    });
    (header || inline_comment) && comments && !feed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    const PACKAGE: &str = "com.ss.android.ugc.trill";
    #[test]
    fn location_prompt_requires_exact_explanation_and_cancel_within_same_panel() {
        let source =
            include_str!("../../fixtures/tiktok-search/global45.7.3-location-dialog.fixture");
        let pkg = "com.zhiliaoapp.musically";
        assert!(location_cancel(&xml(source), pkg, "45.7.3").is_some());
        assert!(location_cancel(
            &xml(&source.replace("Cancel", "Open settings")),
            pkg,
            "45.7.3"
        )
        .is_none());
        assert!(location_cancel(&xml(source), pkg, "unknown").is_none());
        assert!(location_cancel(
            &xml(&source.replace("places nearby", "private messages")),
            pkg,
            "45.7.3"
        )
        .is_none());
        let newer =
            include_str!("../../fixtures/tiktok-search/global46.0.41-location-dialog.fixture");
        assert!(location_cancel(&xml(newer), pkg, "46.0.41").is_some());
        assert!(location_cancel(&xml(newer), pkg, "45.7.3").is_none());
    }
    #[test]
    fn global_46_scrolled_search_keeps_measured_input_proof_after_header_collapses() {
        let source = include_str!("../../fixtures/tiktok-search/global-46.0.41-scrolled.fixture");
        assert!(video_matches_build(
            &xml(source),
            "com.zhiliaoapp.musically",
            "Đà Lạt",
            "46.0.41"
        ));
        assert!(!video_matches_build(
            &xml(source),
            "com.zhiliaoapp.musically",
            "Đà Lạt",
            "unknown"
        ));
        assert!(!video_matches_build(
            &xml(&source.replace(":id/eal", ":id/other")),
            "com.zhiliaoapp.musically",
            "Đà Lạt",
            "46.0.41"
        ));
        let feed = source.replace("content-desc=\"Search\"", "content-desc=\"For You\"");
        assert!(!video_matches_build(
            &xml(&feed),
            "com.zhiliaoapp.musically",
            "Đà Lạt",
            "46.0.41"
        ));
    }
    #[test]
    fn current_fleet_search_result_cards_are_recognized_only_on_measured_builds() {
        let samples = [
            (
                "45.4.3",
                "b79",
                include_str!("../../fixtures/tiktok-search/global-45.4.3-videos.fixture"),
            ),
            (
                "46.0.41",
                "bc6",
                include_str!("../../fixtures/tiktok-search/global-46.0.41-videos.fixture"),
            ),
            (
                "46.1.3",
                "bce",
                include_str!("../../fixtures/tiktok-search/global-46.1.3-videos.fixture"),
            ),
            (
                "46.4.3",
                "bdu",
                include_str!("../../fixtures/tiktok-search/global-46.4.3-videos.fixture"),
            ),
        ];
        for (version, author, source) in samples {
            let tree = xml(source);
            assert!(
                !result_cards(&tree, "com.zhiliaoapp.musically", version).is_empty(),
                "{version}: selected Videos grid has measured cards"
            );
            assert!(result_cards(&tree, "com.zhiliaoapp.musically", "unknown").is_empty());
            assert!(result_cards(
                &xml(&source.replace(&format!(":id/{author}"), ":id/unrelated")),
                "com.zhiliaoapp.musically",
                version
            )
            .is_empty());
            assert!(result_cards(
                &xml(&source.replace("android.widget.GridView", "android.widget.FrameLayout")),
                "com.zhiliaoapp.musically",
                version
            )
            .is_empty());
        }
    }
    #[test]
    fn global_search_cards_require_measured_build_grid_caption_and_author() {
        const PKG: &str = "com.zhiliaoapp.musically";
        let source = format!(
            r#"<hierarchy><node package="{PKG}" class="android.widget.GridView" resource-id="{PKG}:id/mo3" enabled="true" bounds="[0,304][1080,2094]">
          <node package="{PKG}" class="android.widget.FrameLayout" resource-id="{PKG}:id/u_u" clickable="true" enabled="true" bounds="[11,315][535,1373]">
            <node package="{PKG}" resource-id="{PKG}:id/desc" text="Đà Lạt" enabled="true" bounds="[32,1169][503,1263]"/>
            <node package="{PKG}" resource-id="{PKG}:id/b96" text="QR HOTEL DALAT" enabled="true" bounds="[106,1279][366,1318]"/>
          </node></node></hierarchy>"#
        );
        let tree = xml(&source);
        assert_eq!(result_cards(&tree, PKG, "45.7.3").len(), 1);
        assert!(result_cards(&tree, PKG, "46.2.1").is_empty());
        assert!(
            result_cards(&xml(&source.replace(":id/b96", ":id/other")), PKG, "45.7.3").is_empty()
        );
        assert!(result_cards(
            &xml(&source.replace("android.widget.GridView", "android.widget.ListView")),
            PKG,
            "45.7.3"
        )
        .is_empty());
    }
    fn xml(body: &str) -> Tree {
        Tree::parse(crate::HierarchySourceSnapshot {
            xml: format!("<hierarchy>{body}</hierarchy>"),
            generation: 1,
        })
        .unwrap()
    }
    fn node(text: &str, desc: &str, class: &str, bounds: &str, extra: &str) -> String {
        format!(r#"<node package="{PACKAGE}" text="{text}" content-desc="{desc}" class="{class}" bounds="{bounds}" enabled="true" clickable="true" {extra}/ >"#).replace("/ >","/>")
    }
    fn viewer() -> String {
        node(
            "",
            "Back",
            "android.widget.ImageView",
            "[0,50][100,150]",
            "",
        ) + &node(
            "Search",
            "",
            "android.widget.Button",
            "[800,50][1000,150]",
            "",
        ) + &node(
            "related suggestion",
            "",
            "android.widget.TextView",
            "[120,50][700,150]",
            "",
        ) + &node(
            "",
            "Read or add comments. 5 comments",
            "android.widget.Button",
            "[900,900][1000,1100]",
            "",
        )
    }
    #[test]
    fn viewer_requires_search_chrome_and_rejects_for_you_even_when_caption_mentions_keyword() {
        assert!(video_matches(&xml(&viewer()), PACKAGE, "đà lạt"));
        assert!(!video_matches(
            &xml(&(viewer()
                + &node(
                    "đà lạt",
                    "For You",
                    "android.widget.TextView",
                    "[100,50][200,150]",
                    ""
                ))),
            PACKAGE,
            "đà lạt"
        ));
        assert!(!video_matches(
            &xml(&viewer().replace("Search", "Other")),
            PACKAGE,
            "đà lạt"
        ));
        assert!(!query_matches(
            &xml(&node(
                "đà lạt",
                "",
                "android.widget.TextView",
                "[0,0][200,100]",
                ""
            )),
            PACKAGE,
            "đà lạt"
        ));
        let scrolled = node(
            "",
            "Video",
            "android.view.View",
            "[0,0][1080,1965]",
            r#"resource-id="com.ss.android.ugc.trill:id/long_press_layout""#,
        ) + &node(
            "Add comment...",
            "",
            "android.widget.EditText",
            "[64,1977][700,2055]",
            r#"resource-id="com.ss.android.ugc.trill:id/cnd""#,
        ) + &node(
            "",
            "Read or add comments. 5 comments",
            "android.widget.Button",
            "[900,900][1000,1100]",
            "",
        );
        assert!(video_matches(&xml(&scrolled), PACKAGE, "đà lạt"));
    }
    struct Phone {
        stage: Mutex<usize>,
        typed: Mutex<String>,
        wrong_query: bool,
        duplicate_tabs: Mutex<usize>,
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn tap(&self, p: crate::TapPoint) -> anyhow::Result<()> {
            let mut stage = self.stage.lock().unwrap();
            match *stage {
                0 => *stage = 1,
                1 if p.x > 500.0 => *stage = 2,
                2 => {
                    assert_eq!(
                        *self.duplicate_tabs.lock().unwrap(),
                        0,
                        "no tap while tab is ambiguous"
                    );
                    *stage = 3;
                }
                3 => *stage = 4,
                _ => {}
            }
            Ok(())
        }
        async fn swipe(&self, _: crate::SwipeGesture) -> anyhow::Result<()> {
            panic!("navigation must not swipe")
        }
        async fn type_text(&self, text: &str) -> anyhow::Result<()> {
            *self.typed.lock().unwrap() = text.into();
            Ok(())
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("no FYP fallback")
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
        async fn active_app_bundle(&self) -> anyhow::Result<String> {
            Ok(PACKAGE.into())
        }
        async fn app_version(&self, _: &str) -> Option<String> {
            Some("38.3.2".into())
        }
        async fn hierarchy_source_snapshot(
            &self,
        ) -> anyhow::Result<crate::HierarchySourceSnapshot> {
            let stage = *self.stage.lock().unwrap();
            let typed = self.typed.lock().unwrap();
            let query = if self.wrong_query {
                "wrong"
            } else {
                typed.as_str()
            };
            let input = node(
                query,
                "",
                "android.widget.EditText",
                "[100,50][700,150]",
                "",
            );
            let body = match stage {
                0 => {
                    node(
                        "",
                        "For You",
                        "android.widget.TextView",
                        "[600,50][800,200]",
                        "",
                    ) + &node(
                        "",
                        "",
                        "android.widget.ImageView",
                        "[0,50][150,200]",
                        r#"resource-id="com.ss.android.ugc.trill:id/gka""#,
                    ) + &node(
                        "",
                        "",
                        "android.widget.ImageView",
                        "[900,50][1050,200]",
                        r#"resource-id="com.ss.android.ugc.trill:id/gka""#,
                    )
                }
                1 => {
                    input
                        + &node(
                            "Search",
                            "",
                            "android.widget.Button",
                            "[800,50][1050,150]",
                            "",
                        )
                }
                2 => {
                    let mut remaining = self.duplicate_tabs.lock().unwrap();
                    let duplicate = if *remaining > 0 {
                        *remaining -= 1;
                        node(
                            "",
                            "Videos",
                            "android.widget.FrameLayout",
                            "[178,200][358,300]",
                            "",
                        )
                    } else {
                        String::new()
                    };
                    input
                        + &duplicate
                        + &node(
                            "",
                            "Videos",
                            "android.widget.FrameLayout",
                            "[300,200][500,300]",
                            "",
                        )
                }
                3 => {
                    input
                        + &node(
                            "",
                            "Videos",
                            "android.widget.FrameLayout",
                            "[300,200][500,300]",
                            r#"selected="true""#,
                        )
                        + &node(
                            "",
                            "Video by fixture",
                            "android.widget.Button",
                            "[0,350][500,900]",
                            "",
                        )
                }
                _ => viewer(),
            };
            Ok(crate::HierarchySourceSnapshot {
                xml: format!("<hierarchy>{body}</hierarchy>"),
                generation: 1,
            })
        }
    }
    #[tokio::test]
    async fn types_unicode_and_opens_result_only_after_exact_query_readback() {
        let phone = Phone {
            stage: Mutex::new(0),
            typed: Mutex::new(String::new()),
            wrong_query: false,
            duplicate_tabs: Mutex::new(2),
        };
        open(
            &phone,
            PACKAGE,
            "đà lạt",
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(*phone.stage.lock().unwrap(), 4);
        assert_eq!(*phone.typed.lock().unwrap(), "đà lạt");
    }
    #[tokio::test]
    async fn stale_query_and_cancellation_refuse_before_result_selection() {
        let phone = Phone {
            stage: Mutex::new(0),
            typed: Mutex::new(String::new()),
            wrong_query: true,
            duplicate_tabs: Mutex::new(0),
        };
        assert!(open(
            &phone,
            PACKAGE,
            "đà lạt",
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5)
        )
        .await
        .is_err());
        assert_eq!(*phone.stage.lock().unwrap(), 1);
        let phone = Phone {
            stage: Mutex::new(0),
            typed: Mutex::new(String::new()),
            wrong_query: false,
            duplicate_tabs: Mutex::new(0),
        };
        assert!(open(
            &phone,
            PACKAGE,
            "đà lạt",
            &AtomicBool::new(true),
            Instant::now() + Duration::from_secs(5)
        )
        .await
        .is_err());
        assert_eq!(*phone.stage.lock().unwrap(), 0);
    }
}
