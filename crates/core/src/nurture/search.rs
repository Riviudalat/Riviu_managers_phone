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
async fn read(session: &dyn UiSession, package: &str) -> anyhow::Result<Tree> {
    ensure!(
        session.active_app_bundle().await? == package,
        "Đã rời TikTok khi tìm kiếm"
    );
    Tree::parse(session.hierarchy_source_snapshot().await?)
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
    let mut tree = read(session, package).await?;
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
        tree = read(session, package).await?;
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
    tree = read(session, package).await?;
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
    let videos = loop {
        check(stop, deadline)?;
        tree = read(session, package).await?;
        ensure!(
            query_matches(&tree, package, keyword),
            "Từ khóa đã thay đổi khi tìm kiếm"
        );
        if let Some(tab) = unique(nodes(&tree, package, |n| {
            ["videos", "video"].contains(&fold(n.attr("content-desc")).as_str())
        }))? {
            break tab;
        }
        super::sleep_interruptible(Duration::from_millis(400), stop).await;
    };
    check(stop, deadline)?;
    session.tap(videos.centre()).await?;
    let first = loop {
        check(stop, deadline)?;
        tree = read(session, package).await?;
        ensure!(
            query_matches(&tree, package, keyword),
            "Kết quả không thuộc từ khóa đã nhập"
        );
        let videos_selected = tree.nodes.iter().any(|n| {
            n.visible(package)
                && ["videos", "video"].contains(&fold(n.attr("content-desc")).as_str())
                && (n.attr("selected") == "true" || n.attr("clickable") == "false")
        });
        if !videos_selected {
            super::sleep_interruptible(Duration::from_millis(400), stop).await;
            continue;
        }
        let mut results = nodes(&tree, package, |n| {
            n.attr("clickable") == "true"
                && (n.attr("content-desc").starts_with("Video by ")
                    || n.attr("content-desc").starts_with("Video của "))
        });
        results.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
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
    Ok(video_matches(&tree, package, keyword))
}
fn video_matches(tree: &Tree, package: &str, _keyword: &str) -> bool {
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
    let inline_comment = nodes(tree, package, |n| {
        n.attr("class") == "android.widget.EditText" && n.attr("resource-id").ends_with(":id/cnd")
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
    }
    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn tap(&self, p: crate::TapPoint) -> anyhow::Result<()> {
            let mut stage = self.stage.lock().unwrap();
            match *stage {
                0 => *stage = 1,
                1 if p.x > 500.0 => *stage = 2,
                2 => *stage = 3,
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
                    input
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
