//! Android 446 accessibility adapter. Every tap is derived from a fresh hierarchy.
use super::*;
use crate::ui_automation::tree::{Node, Tree};
use crate::{ElementBox, UiSession};
use anyhow::{ensure, Context};
use sha2::{Digest, Sha256};

pub const MEASURED_VERSION: &str = "446.0.0.32.78";
pub const PROFILE_PROBE_VERSION: &str = "449.0.0.54.82";
pub const ADAPTER_ENABLED: bool = true;
pub const MAX_MEASURED_IMAGES: usize = 15;
const CELL: &str = "com.instagram.barcelona:id/gallery_picker_grid_item_container";

pub async fn observe(session: &dyn UiSession) -> anyhow::Result<Tree> {
    ensure_foreground(session).await?;
    ensure!(
        session.active_app_bundle().await? == ANDROID_PACKAGE,
        "Threads is not foreground"
    );
    Tree::parse(session.hierarchy_source_snapshot().await?)
}

fn threads_media_permission_button(tree: &Tree) -> anyhow::Result<ElementBox> {
    const INSTALLER: &str = "com.google.android.packageinstaller";
    let has_message = tree.nodes.iter().any(|node| {
        node.visible(INSTALLER)
            && node.attr("resource-id") == "com.android.packageinstaller:id/permission_message"
            && node.attr("text")
                == "Allow Threads to access photos, media, and files on your device?"
    });
    ensure!(has_message, "Threads media permission prompt differs");
    let buttons = tree
        .nodes
        .iter()
        .filter(|node| {
            node.visible(INSTALLER)
                && node.attr("resource-id")
                    == "com.android.packageinstaller:id/permission_allow_button"
                && node
                    .attr("text")
                    .chars()
                    .filter(|character| character.is_alphanumeric())
                    .collect::<String>()
                    == "Allow"
                && node.attr("enabled") == "true"
        })
        .filter_map(Node::rect)
        .collect::<Vec<_>>();
    ensure!(
        buttons.len() == 1,
        "Threads media permission Allow button ambiguous"
    );
    Ok(buttons[0].clone())
}

pub async fn ensure_foreground(session: &dyn UiSession) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut granted = false;
    loop {
        let package = session.active_app_bundle().await?;
        if package == ANDROID_PACKAGE {
            return Ok(());
        }
        if package == "com.google.android.packageinstaller" && !granted {
            let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
            session
                .tap(threads_media_permission_button(&tree)?.centre())
                .await?;
            granted = true;
        } else {
            ensure!(
                package == "com.google.android.packageinstaller" && granted,
                "Threads is not foreground: {package}"
            );
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "Threads did not open after media permission"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

async fn wait_for(
    session: &dyn UiSession,
    state: &str,
    ready: impl Fn(&Tree) -> bool,
) -> anyhow::Result<Tree> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let tree = observe(session).await?;
        if ready(&tree) {
            return Ok(tree);
        }
        ensure!(
            std::time::Instant::now() < deadline,
            "Threads timed out waiting for {state}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }
}

fn unique(tree: &Tree, predicate: impl Fn(&Node) -> bool) -> anyhow::Result<usize> {
    let matches = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| {
            node.visible(ANDROID_PACKAGE) && node.rect().is_some() && predicate(node)
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    ensure!(
        matches.len() == 1,
        "Threads target must be unique ({} matches)",
        matches.len()
    );
    Ok(matches[0])
}

pub async fn tap_resource(session: &dyn UiSession, resource: &str) -> anyhow::Result<()> {
    let predicate =
        |node: &Node| node.attr("resource-id") == resource && node.attr("enabled") == "true";
    let tree = wait_for(session, resource, |tree| unique(tree, predicate).is_ok()).await?;
    let index = unique(&tree, predicate)?;
    session
        .tap(
            tree.nodes[index]
                .rect()
                .context("Threads target has no bounds")?
                .centre(),
        )
        .await
}

pub async fn tap_clickable_resource(session: &dyn UiSession, resource: &str) -> anyhow::Result<()> {
    let predicate = |node: &Node| {
        node.attr("resource-id") == resource
            && node.attr("enabled") == "true"
            && node.attr("clickable") == "true"
    };
    let tree = wait_for(session, resource, |tree| unique(tree, predicate).is_ok()).await?;
    let index = unique(&tree, predicate)?;
    session
        .tap(
            tree.nodes[index]
                .rect()
                .context("Threads action bounds missing")?
                .centre(),
        )
        .await
}

pub async fn tap_description(session: &dyn UiSession, description: &str) -> anyhow::Result<()> {
    let predicate =
        |node: &Node| node.attr("content-desc") == description && node.attr("enabled") == "true";
    let tree = wait_for(session, description, |tree| unique(tree, predicate).is_ok()).await?;
    let index = unique(&tree, predicate)?;
    session
        .tap(
            tree.nodes[index]
                .rect()
                .context("Threads target has no bounds")?
                .centre(),
        )
        .await
}

pub async fn tap_text(session: &dyn UiSession, text: &str) -> anyhow::Result<()> {
    let predicate = |node: &Node| node.attr("text") == text && node.attr("enabled") == "true";
    let tree = wait_for(session, text, |tree| unique(tree, predicate).is_ok()).await?;
    let index = unique(&tree, predicate)?;
    session
        .tap(
            tree.nodes[index]
                .rect()
                .context("Threads target has no bounds")?
                .centre(),
        )
        .await
}

pub fn profile_account(tree: &Tree) -> anyhow::Result<String> {
    ensure!(
        tree.nodes
            .iter()
            .any(|node| node.attr("resource-id") == "barcelona_tab_profile"
                && node.attr("selected") == "true"),
        "Threads own profile not selected"
    );
    profile_header_account(tree)
}

fn profile_header_account(tree: &Tree) -> anyhow::Result<String> {
    let edit = unique(tree, |node| {
        node.attr("resource-id") == "profile_screen_edit_profile_button"
    })?;
    let header = tree.nodes[edit]
        .parent
        .context("Threads profile header missing")?;
    let name = unique(tree, |node| {
        node.attr("resource-id") == "profile_screen_full_name"
    })?;
    let candidates = tree
        .nodes
        .iter()
        .filter(|node| {
            node.parent
                .is_some_and(|parent| tree.nodes[parent].parent == Some(header) && parent != name)
                && node.attr("resource-id") == "ig_text"
                && normalize_threads_account(node.attr("text")).is_ok()
        })
        .collect::<Vec<_>>();
    ensure!(candidates.len() == 1, "Threads profile username ambiguous");
    normalize_threads_account(candidates[0].attr("text"))
}

fn profile_editor_account(tree: &Tree) -> anyhow::Result<String> {
    unique(tree, |node| {
        node.attr("resource-id") == "profile_editor_card_done_button"
    })?;
    let label = unique(tree, |node| {
        node.attr("resource-id") == "ig_text"
            && matches!(node.attr("text"), "Tên người dùng" | "Username")
    })?;
    let row = tree.nodes[label]
        .parent
        .context("Threads username editor row missing")?;
    let candidates = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            *index != label
                && node.parent == Some(row)
                && node.visible(ANDROID_PACKAGE)
                && normalize_threads_account(node.attr("text")).is_ok()
        })
        .collect::<Vec<_>>();
    ensure!(candidates.len() == 1, "Threads editor username ambiguous");
    normalize_threads_account(candidates[0].1.attr("text"))
}

fn profile_edit_label(tree: &Tree) -> Option<&'static str> {
    ["Edit profile", "Chỉnh sửa trang cá nhân"]
        .into_iter()
        .find(|label| {
            !tree
                .nodes
                .iter()
                .any(|node| node.attr("resource-id") == "MainFeedScreen")
                && tree
                    .nodes
                    .iter()
                    .any(|node| node.attr("resource-id") == "tabs_bottom_bar")
                && unique(tree, |node| node.attr("text") == *label).is_ok()
        })
}

fn profile_edit_available(tree: &Tree) -> bool {
    tree.nodes.iter().any(|node| {
        node.attr("resource-id") == "profile_screen_edit_profile_button"
            && node.visible(ANDROID_PACKAGE)
    }) || profile_edit_label(tree).is_some()
}

/// Probe account on the observed 449 feed/editor screens; never use this as publish support.
pub async fn observed_449_profile_account(session: &dyn UiSession) -> anyhow::Result<String> {
    let tree = observe(session).await?;
    if tree
        .nodes
        .iter()
        .any(|node| node.attr("resource-id") == "profile_editor_card_done_button")
    {
        anyhow::bail!("Threads profile editor is already open; close it before checking")
    }
    ensure!(
        !login_screen_open(&tree),
        "Threads account is not signed in"
    );
    if let Some(reason) = account_gate(&tree) {
        anyhow::bail!("{reason}");
    }
    if let Ok(account) = profile_header_account(&tree) {
        return Ok(account);
    }
    if !profile_edit_available(&tree) {
        ensure!(
            tree.nodes
                .iter()
                .any(|node| node.attr("resource-id") == "tabs_bottom_bar")
                && tree.nodes.iter().any(|node| {
                    node.attr("resource-id") == "barcelona_tab_profile"
                        && node.visible(ANDROID_PACKAGE)
                }),
            "Threads 449 profile tab is not available on this screen"
        );
        tap_resource(session, "barcelona_tab_profile").await?;
        let tree = wait_for(session, "Threads 449 profile", profile_edit_available).await?;
        if let Ok(account) = profile_header_account(&tree) {
            return Ok(account);
        }
        return open_449_profile_editor(session, &tree).await;
    }
    open_449_profile_editor(session, &tree).await
}

async fn open_449_profile_editor(session: &dyn UiSession, tree: &Tree) -> anyhow::Result<String> {
    if tree.nodes.iter().any(|node| {
        node.attr("resource-id") == "profile_screen_edit_profile_button"
            && node.visible(ANDROID_PACKAGE)
    }) {
        tap_resource(session, "profile_screen_edit_profile_button").await?;
    } else {
        tap_text(
            session,
            profile_edit_label(&tree).context("Threads 449 profile edit control missing")?,
        )
        .await?;
    }
    let editor = wait_for(session, "Threads 449 profile editor", |tree| {
        tree.nodes
            .iter()
            .any(|node| node.attr("resource-id") == "profile_editor_card_done_button")
    })
    .await?;
    let account = profile_editor_account(&editor);
    let restore = tap_resource(session, "navigation_bar_back_button").await;
    restore?;
    account
}

/// Read the own profile already on screen without navigating an unmeasured build.
pub async fn visible_profile_account(session: &dyn UiSession) -> anyhow::Result<String> {
    ensure!(
        session.active_app_bundle().await? == ANDROID_PACKAGE,
        "Threads is not foreground; open the own profile before checking this build"
    );
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    // On newer builds the profile tab may omit selected=true. The own-profile
    // edit control and a unique username in its header still provide read-only proof.
    profile_header_account(&tree).context(
        "Threads own profile is not readable on this build; open the own profile and retry",
    )
}

pub async fn observed_profile_account(session: &dyn UiSession) -> anyhow::Result<String> {
    return_to_profile(session).await?;
    for _ in 0..12 {
        let profile = wait_for(session, "own profile", |tree| {
            tree.nodes.iter().any(|node| {
                node.attr("resource-id") == "barcelona_tab_profile"
                    && node.attr("selected") == "true"
            })
        })
        .await?;
        if let Ok(account) = profile_account(&profile) {
            return Ok(account);
        }
        if let Some(scroll) = profile
            .nodes
            .iter()
            .find(|node| {
                node.attr("resource-id") == "IgLazyColumn" && node.attr("scrollable") == "true"
            })
            .and_then(Node::rect)
        {
            let x = scroll.x + scroll.width / 2.0;
            session
                .swipe(crate::SwipeGesture {
                    from: crate::TapPoint {
                        x,
                        y: scroll.y + scroll.height * 0.3,
                    },
                    to: crate::TapPoint {
                        x,
                        y: scroll.y + scroll.height * 0.8,
                    },
                    duration_ms: 500,
                })
                .await?;
        } else {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
    anyhow::bail!("Threads own profile account unavailable")
}

fn share_sheet_open(tree: &Tree) -> bool {
    tree.nodes.iter().any(|node| {
        node.attr("resource-id") == "post_share_sheet_copy_link_button"
            && node.attr("clickable") == "true"
    }) && tree.nodes.iter().any(|node| {
        matches!(node.attr("content-desc"), "Close sheet" | "Đóng trang")
            && node.attr("clickable") == "true"
    })
}

fn login_screen_open(tree: &Tree) -> bool {
    let instagram_login = tree
        .nodes
        .iter()
        .any(|node| node.attr("resource-id") == "login_screen_login_with_ig_button")
        && tree
            .nodes
            .iter()
            .any(|node| node.attr("resource-id") == "login_screen_create_ig_account_button");
    let credential_login = ["Username, email or mobile number", "Password"]
        .iter()
        .all(|label| {
            tree.nodes
                .iter()
                .any(|node| node.attr("text") == *label || node.attr("content-desc") == *label)
        });
    instagram_login || credential_login
}

fn account_gate(tree: &Tree) -> Option<&'static str> {
    let has_text = |text: &str| {
        tree.nodes
            .iter()
            .any(|node| node.attr("text").contains(text))
    };
    if has_text("Confirm you're human to use your account") {
        Some("Threads account requires human verification")
    } else if has_text("Enter your mobile number") {
        Some("Threads account requires phone verification")
    } else if has_text("Share this and future threads to your Instagram stories") {
        Some("Threads account has an onboarding dialog")
    } else {
        None
    }
}

fn measured_gallery_labels(tree: &Tree) -> anyhow::Result<(&'static str, &'static str)> {
    let mut matches = [("Recents", "Done"), ("Mới đây", "Xong")]
        .into_iter()
        .filter(|(title, done)| {
            unique(tree, |node| node.attr("text") == *title).is_ok()
                && unique(tree, |node| node.attr("text") == *done).is_ok()
        });
    let labels = matches
        .next()
        .context("Không nhận diện được ngôn ngữ thư viện Threads")?;
    ensure!(
        matches.next().is_none(),
        "Ngôn ngữ thư viện Threads không rõ ràng"
    );
    Ok(labels)
}

async fn close_share_sheet(session: &dyn UiSession) -> anyhow::Result<bool> {
    if !share_sheet_open(&observe(session).await?) {
        return Ok(false);
    }
    let tree = observe(session).await?;
    let description = if tree
        .nodes
        .iter()
        .any(|node| node.attr("content-desc") == "Đóng trang")
    {
        "Đóng trang"
    } else {
        "Close sheet"
    };
    tap_description(session, description).await?;
    Ok(true)
}

async fn return_to_profile(session: &dyn UiSession) -> anyhow::Result<()> {
    for _ in 0..4 {
        let tree = observe(session).await?;
        ensure!(
            !login_screen_open(&tree),
            "Threads account is not signed in"
        );
        if let Some(button) = instagram_onboarding_done(&tree)? {
            session.tap(button.centre()).await?;
            continue;
        }
        if let Some(reason) = account_gate(&tree) {
            anyhow::bail!("{reason}");
        }
        if share_sheet_open(&tree) {
            close_share_sheet(session).await?;
        } else if tree
            .nodes
            .iter()
            .any(|node| node.attr("resource-id") == "PermalinkScreen")
        {
            tap_resource(session, "navigation_bar_back_button").await?;
        } else if let Some(composer) = tree
            .nodes
            .iter()
            .find(|node| node.attr("resource-id") == "new_thread_screen_composer")
        {
            ensure!(
                composer.attr("text").is_empty()
                    && !tree.nodes.iter().any(|node| {
                        matches!(
                            node.attr("content-desc"),
                            "Photo preview" | "Remove attachment" | "Gỡ file đính kèm"
                        )
                    }),
                "Threads composer has a draft; keep it for operator review"
            );
            tap_resource(session, "navigation_bar_back_button").await?;
        } else {
            tap_resource(session, "barcelona_tab_profile").await?;
            return Ok(());
        }
    }
    anyhow::bail!("Threads could not return from share sheet or post detail to profile")
}

fn instagram_onboarding_done(tree: &Tree) -> anyhow::Result<Option<ElementBox>> {
    let title = tree.nodes.iter().any(|node| {
        matches!(node.attr("text"),
        "Share this and future threads to your Instagram stories" |
        "Chia sẻ cuộc trò chuyện này và các cuộc trò chuyện trong tương lai lên tin trên Instagram")
    });
    if !title {
        return Ok(None);
    }
    let switches = tree
        .nodes
        .iter()
        .filter(|node| {
            node.visible(ANDROID_PACKAGE)
                && (node.attr("class") == "android.widget.Switch"
                    || node.attr("checkable") == "true")
        })
        .collect::<Vec<_>>();
    ensure!(
        !switches.is_empty() && switches.iter().all(|node| node.attr("checked") == "false"),
        "Threads Instagram sharing setting is enabled or unreadable; operator review required"
    );
    let index = unique(tree, |node| {
        matches!(node.attr("text"), "Done" | "Xong") && node.attr("enabled") == "true"
    })?;
    let mut current = Some(index);
    while let Some(index) = current {
        let node = &tree.nodes[index];
        if node.attr("clickable") == "true" && node.attr("enabled") == "true" {
            return Ok(Some(
                node.rect()
                    .context("Threads Instagram Done bounds missing")?,
            ));
        }
        current = node.parent;
    }
    anyhow::bail!("Threads Instagram Done action unavailable")
}

pub fn selected_ordinal(description: &str) -> Option<usize> {
    if let Some(selection) = description.strip_prefix("Đã chọn ") {
        let (ordinal, remainder) = selection.split_once('/')?;
        let (total, photo) = remainder.split_once(". ")?;
        total.parse::<usize>().ok()?;
        if !photo.starts_with("Hình nhỏ ảnh") {
            return None;
        }
        return ordinal.parse().ok().filter(|ordinal| *ordinal > 0);
    }
    let (ordinal, remainder) = description.split_once(" of ")?;
    let (_, photo) = remainder.split_once(" selected. ")?;
    if !photo.starts_with("Photo thumbnail") {
        return None;
    }
    ordinal.parse().ok().filter(|ordinal| *ordinal > 0)
}

fn photo_cells(tree: &Tree) -> Vec<(usize, ElementBox)> {
    tree.nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| {
            (node.attr("resource-id") == CELL
                && (node.attr("content-desc").contains("Photo thumbnail")
                    || node.attr("content-desc").contains("Hình nhỏ ảnh"))
                && node.visible(ANDROID_PACKAGE))
            .then(|| node.rect().map(|rect| (index, rect)))
            .flatten()
        })
        .collect()
}

fn match_scrolled_gallery(
    source: &[Vec<u8>],
    top_png: &[u8],
    top_cells: &[(usize, ElementBox)],
    bottom_png: &[u8],
    bottom_cells: &[(usize, ElementBox)],
) -> anyhow::Result<Vec<usize>> {
    let count = source.len();
    ensure!(
        count == 15 && top_cells.len() < count && bottom_cells.len() < count,
        "Threads scrolled album layout differs"
    );
    let bottom_start = count - bottom_cells.len();
    ensure!(
        bottom_start > 0 && bottom_start < top_cells.len(),
        "Threads album observations do not overlap"
    );
    let full_height = top_cells
        .iter()
        .chain(bottom_cells)
        .map(|(_, rect)| rect.height)
        .fold(0.0_f64, f64::max);
    let mut samples = vec![Vec::new(); count];
    for (png, cells, start) in [
        (top_png, top_cells, 0),
        (bottom_png, bottom_cells, bottom_start),
    ] {
        let screen = image::load_from_memory(png)?;
        for (local, (_, rect)) in cells.iter().enumerate() {
            if rect.height < full_height * 0.95 {
                continue;
            }
            ensure!(
                rect.x >= 0.0
                    && rect.y >= 0.0
                    && rect.width >= 48.0
                    && rect.height >= 48.0
                    && rect.x + rect.width <= screen.width() as f64
                    && rect.y + rect.height <= screen.height() as f64,
                "Threads thumbnail is outside screenshot"
            );
            samples[start + local].push(center_sample(&screen.crop_imm(
                rect.x as u32,
                rect.y as u32,
                rect.width as u32,
                rect.height as u32,
            )));
        }
    }
    ensure!(
        samples.iter().all(|set| !set.is_empty()),
        "Threads album has a photo without a full observation"
    );
    let overlap = samples.iter().filter(|set| set.len() == 2).count();
    ensure!(overlap >= 2, "Threads album overlap cannot be verified");
    for set in &samples {
        if set.len() == 2 {
            ensure!(
                thumbnail_error(&set[0], &set[1]) <= 8.0,
                "Threads gallery order changed between observations"
            );
        }
    }
    let mut order = Vec::with_capacity(count);
    for bytes in source {
        let image = center_sample(&image::load_from_memory(bytes)?);
        let mut scores = samples
            .iter()
            .enumerate()
            .map(|(position, set)| {
                (
                    position,
                    set.iter()
                        .map(|sample| thumbnail_error(&image, sample))
                        .fold(f64::INFINITY, f64::min),
                )
            })
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| a.1.total_cmp(&b.1));
        ensure!(
            scores[0].1 <= 15.0 && scores[1].1 - scores[0].1 >= 8.0,
            "Threads thumbnail identity is ambiguous"
        );
        order.push(scores[0].0);
    }
    ensure!(
        order.iter().collect::<std::collections::HashSet<_>>().len() == count,
        "Threads thumbnail maps to more than one source image"
    );
    Ok(order)
}

fn scrolled_gallery_bottom_ready(
    tree: &Tree,
    import_id: &str,
    count: usize,
    top_count: usize,
) -> bool {
    let cells = photo_cells(tree);
    tree.nodes.iter().any(|node| node.attr("text") == import_id)
        && cells.len() < top_count
        && cells.len() > count - top_count
        && cells
            .last()
            .is_some_and(|(_, rect)| rect.height >= rect.width * 0.95)
}

async fn select_scrolled_gallery(
    session: &dyn UiSession,
    gallery: &Tree,
    source: &[Vec<u8>],
    import_id: &str,
) -> anyhow::Result<Vec<usize>> {
    let top_cells = photo_cells(gallery);
    let top_png = session.screenshot_png().await?;
    let grid = gallery
        .nodes
        .iter()
        .find(|node| {
            node.attr("class") == "android.widget.GridView" && node.attr("scrollable") == "true"
        })
        .and_then(Node::rect)
        .context("Threads gallery scroll viewport missing")?;
    // Measured on Machine 8, Threads 446/vi, 1080x2220: viewport
    // [0,358][1080,2094], rows 356px apart. The previous 16% swipe
    // moved only 278px, leaving the last row clipped. 60% reaches the
    // album boundary in both directions, including the camera cell.
    let scroll = |down: bool| crate::SwipeGesture {
        from: crate::TapPoint {
            x: grid.x + grid.width / 2.0,
            y: grid.y + grid.height * if down { 0.80 } else { 0.20 },
        },
        to: crate::TapPoint {
            x: grid.x + grid.width / 2.0,
            y: grid.y + grid.height * if down { 0.20 } else { 0.80 },
        },
        duration_ms: 450,
    };
    session.swipe(scroll(true)).await?;
    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
    let bottom = wait_for(session, "last imported photo", |tree| {
        scrolled_gallery_bottom_ready(tree, import_id, source.len(), top_cells.len())
    })
    .await?;
    let bottom_cells = photo_cells(&bottom);
    let bottom_png = session.screenshot_png().await?;
    let order = match_scrolled_gallery(source, &top_png, &top_cells, &bottom_png, &bottom_cells)?;
    let bottom_start = source.len() - bottom_cells.len();
    let mut at_bottom = true;
    for (ordinal, &position) in order.iter().enumerate() {
        let top_usable =
            position < top_cells.len() && top_cells[position].1.height >= 0.95 * grid.width / 3.0;
        let bottom_usable = position >= bottom_start
            && bottom_cells[position - bottom_start].1.height >= 0.95 * grid.width / 3.0;
        ensure!(
            top_usable || bottom_usable,
            "Threads photo is not fully visible"
        );
        if (at_bottom && !bottom_usable) || (!at_bottom && !top_usable) {
            at_bottom = !at_bottom;
            session.swipe(scroll(at_bottom)).await?;
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
        }
        let reference_cells = if at_bottom { &bottom_cells } else { &top_cells };
        let reference_png = if at_bottom { &bottom_png } else { &top_png };
        let offset = if at_bottom { bottom_start } else { 0 };
        let local = position - offset;
        let fresh = observe(session).await?;
        let cells = photo_cells(&fresh);
        ensure!(
            fresh
                .nodes
                .iter()
                .any(|node| node.attr("text") == import_id)
                && cells.len() == reference_cells.len()
                && cells.iter().zip(reference_cells).all(|((_, a), (_, b))| {
                    a.x == b.x && a.y == b.y && a.width == b.width && a.height == b.height
                }),
            "Threads gallery moved before selection"
        );
        let (index, rect) = &cells[local];
        ensure!(
            selected_ordinal(fresh.nodes[*index].attr("content-desc")).is_none(),
            "Threads photo already selected"
        );
        let current = image::load_from_memory(&session.screenshot_png().await?)?;
        let reference = image::load_from_memory(reference_png)?;
        let sample = |image: &image::DynamicImage| {
            center_sample(&image.crop_imm(
                rect.x as u32,
                rect.y as u32,
                rect.width as u32,
                rect.height as u32,
            ))
        };
        ensure!(
            thumbnail_error(&sample(&current), &sample(&reference)) <= 8.0,
            "Threads thumbnail changed before selection"
        );
        session.tap(rect.centre()).await?;
        let selected = wait_for(session, "photo selection readback", |tree| {
            let visible = photo_cells(tree);
            visible.len() == reference_cells.len()
                && visible.get(local).is_some_and(|(i, _)| {
                    selected_ordinal(tree.nodes[*i].attr("content-desc")) == Some(ordinal + 1)
                })
        })
        .await
        .with_context(|| format!("Threads photo {} selection readback", ordinal + 1))?;
        let visible = photo_cells(&selected);
        for (earlier_ordinal, &earlier_position) in order.iter().take(ordinal + 1).enumerate() {
            if (offset..offset + visible.len()).contains(&earlier_position) {
                let cell = visible[earlier_position - offset].0;
                ensure!(
                    selected_ordinal(selected.nodes[cell].attr("content-desc"))
                        == Some(earlier_ordinal + 1),
                    "Threads photo order readback differs"
                );
            }
        }
        ensure!(
            visible
                .iter()
                .filter(|(i, _)| {
                    selected_ordinal(selected.nodes[*i].attr("content-desc")).is_some()
                })
                .count()
                == order
                    .iter()
                    .take(ordinal + 1)
                    .filter(|&&p| (offset..offset + visible.len()).contains(&p))
                    .count(),
            "Threads selected photo count differs"
        );
    }
    Ok(order)
}

fn album_count(tree: &Tree, album: &str) -> anyhow::Result<usize> {
    let title = unique(tree, |node| node.attr("text") == album)?;
    let parent = tree.nodes[title]
        .parent
        .context("Threads album row missing")?;
    let counts = tree
        .nodes
        .iter()
        .filter(|node| node.parent == Some(parent) && node.attr("resource-id") == "ig_text")
        .filter_map(|node| node.attr("text").parse::<usize>().ok())
        .collect::<Vec<_>>();
    ensure!(counts.len() == 1, "Threads album count ambiguous");
    Ok(counts[0])
}

fn save_draft_dialog(tree: &Tree) -> anyhow::Result<()> {
    unique(tree, |node| {
        node.attr("resource-id") == "com.instagram.barcelona:id/igds_alert_dialog_headline"
            && matches!(node.attr("text"), "Save to drafts?" | "Lưu làm bản nháp?")
    })?;
    unique(tree, |node| {
        node.attr("resource-id") == "com.instagram.barcelona:id/igds_alert_dialog_primary_button"
            && matches!(node.attr("text"), "Save" | "Lưu")
            && node.attr("enabled") == "true"
    })?;
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedComposer {
    pub account: String,
    pub import_id: String,
    pub selected_count: usize,
    pub selected_order: Vec<usize>,
    pub caption_sha256: String,
}

pub async fn prepare(
    session: &dyn UiSession,
    bundle: &crate::PublishBundle,
    import_id: &str,
    expected_account: &str,
) -> anyhow::Result<PreparedComposer> {
    validate_caption(&bundle.caption)?;
    ensure!(
        bundle.video.is_none() && (1..=MAX_MEASURED_IMAGES).contains(&bundle.images.len()),
        "Threads measured adapter accepts 1 to 15 images"
    );
    ensure!(
        import_id.starts_with("riviu-"),
        "Threads native import ID missing"
    );
    let expected = normalize_threads_account(expected_account)?;
    let account = observed_profile_account(session).await?;
    ensure!(
        account == expected,
        "Threads account differs from assignment"
    );
    tap_resource(session, "barcelona_tab_create").await?;
    let mut composer = wait_for(session, "composer", |tree| {
        tree.nodes
            .iter()
            .any(|node| node.attr("resource-id") == "new_thread_screen_composer")
    })
    .await?;
    let mut field = unique(&composer, |node| {
        node.attr("resource-id") == "new_thread_screen_composer"
            && node.attr("class") == "android.widget.EditText"
    })?;
    let has_attachments = |tree: &Tree| {
        tree.nodes.iter().any(|node| {
            node.attr("content-desc") == "Photo preview"
                || matches!(
                    node.attr("content-desc"),
                    "Remove attachment" | "Gỡ file đính kèm"
                )
        })
    };
    if !composer.nodes[field].attr("text").is_empty() || has_attachments(&composer) {
        tap_resource(session, "navigation_bar_back_button").await?;
        let dialog = wait_for(session, "save existing draft dialog", |tree| {
            save_draft_dialog(tree).is_ok()
        })
        .await?;
        save_draft_dialog(&dialog)?;
        tap_resource(
            session,
            "com.instagram.barcelona:id/igds_alert_dialog_primary_button",
        )
        .await?;
        tap_resource(session, "barcelona_tab_create").await?;
        composer = wait_for(session, "new empty composer", |tree| {
            tree.nodes
                .iter()
                .any(|node| node.attr("resource-id") == "new_thread_screen_composer")
        })
        .await?;
        field = unique(&composer, |node| {
            node.attr("resource-id") == "new_thread_screen_composer"
                && node.attr("class") == "android.widget.EditText"
        })?;
    }
    ensure!(
        composer.nodes[field].attr("text").is_empty(),
        "Threads existing draft must be reviewed before publishing"
    );
    ensure!(
        !has_attachments(&composer),
        "Threads composer contains existing attachments"
    );
    tap_resource(session, "new_thread_screen_gallery_button").await?;
    let picker = wait_for(session, "gallery title", |tree| {
        tree.nodes
            .iter()
            .any(|node| matches!(node.attr("text"), "Recents" | "Mới đây"))
    })
    .await?;
    let (recents, done) = measured_gallery_labels(&picker)?;
    tap_text(session, recents).await?;
    let albums = wait_for(session, "exact import album", |tree| {
        album_count(tree, import_id).is_ok()
    })
    .await?;
    let count = album_count(&albums, import_id)?;
    ensure!(
        count == bundle.images.len(),
        "Threads album count differs from source"
    );
    tap_description(session, import_id).await?;
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let mut gallery = wait_for(session, "imported photo grid", |tree| {
        tree.nodes.iter().any(|node| node.attr("text") == import_id)
            && photo_cells(tree).len() == count.min(14)
    })
    .await?;
    ensure!(
        gallery
            .nodes
            .iter()
            .any(|node| node.attr("text") == import_id),
        "Threads gallery opened another album"
    );
    let mut cells = photo_cells(&gallery);
    ensure!(
        cells.len() == count.min(14),
        "Threads imported photo grid differs"
    );
    ensure!(
        cells.iter().all(
            |(index, _)| selected_ordinal(gallery.nodes[*index].attr("content-desc")).is_none()
        ),
        "Threads gallery contains a previous selection"
    );
    let source = bundle
        .images
        .iter()
        .map(|image| {
            let bytes = std::fs::read(&image.path)?;
            ensure!(
                format!("{:x}", Sha256::digest(&bytes)) == image.sha256,
                "Threads source image changed"
            );
            Ok(bytes)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let source_cell_positions = if count == 15 {
        select_scrolled_gallery(session, &gallery, &source, import_id).await?
    } else {
        let first_png = session.screenshot_png().await?;
        let first_match_cells = cells
            .iter()
            .map(|(index, rect)| GalleryPhotoCell {
                element_index: *index,
                x: rect.x as u32,
                y: rect.y as u32,
                width: rect.width as u32,
                height: rect.height as u32,
            })
            .collect::<Vec<_>>();
        let mut second_png = None;
        let mut second_match_cells = None;
        let full_height = cells
            .iter()
            .map(|(_, rect)| rect.height)
            .fold(0.0_f64, f64::max);
        if cells
            .iter()
            .any(|(_, rect)| rect.height < full_height * 0.95)
        {
            let grid = gallery
                .nodes
                .iter()
                .find(|node| {
                    node.attr("class") == "android.widget.GridView"
                        && node.attr("scrollable") == "true"
                })
                .and_then(Node::rect)
                .context("Threads gallery scroll viewport missing")?;
            let x = grid.x + grid.width / 2.0;
            session
                .swipe(crate::SwipeGesture {
                    from: crate::TapPoint {
                        x,
                        y: grid.y + grid.height * 0.72,
                    },
                    to: crate::TapPoint {
                        x,
                        y: grid.y + grid.height * 0.56,
                    },
                    duration_ms: 450,
                })
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(900)).await;
            gallery = wait_for(session, "same imported album after scroll", |tree| {
                tree.nodes.iter().any(|node| node.attr("text") == import_id)
                    && photo_cells(tree).len() == count
            })
            .await?;
            cells = photo_cells(&gallery);
            ensure!(
                cells
                    .iter()
                    .zip(&first_match_cells)
                    .all(|((_, rect), first)| {
                        rect.x as u32 == first.x && rect.width as u32 == first.width
                    }),
                "Threads album grid changed during scroll"
            );
            second_png = Some(session.screenshot_png().await?);
            second_match_cells = Some(
                cells
                    .iter()
                    .map(|(index, rect)| GalleryPhotoCell {
                        element_index: *index,
                        x: rect.x as u32,
                        y: rect.y as u32,
                        width: rect.width as u32,
                        height: rect.height as u32,
                    })
                    .collect::<Vec<_>>(),
            );
        }
        let mut observations = vec![(first_png.as_slice(), first_match_cells.as_slice())];
        if let (Some(png), Some(cells)) = (&second_png, &second_match_cells) {
            observations.push((png.as_slice(), cells.as_slice()));
        }
        let order = match_imported_photos_observations(
            &source.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            &observations,
            import_id,
            import_id,
            count,
        )?;
        let source_cell_positions = order
            .iter()
            .map(|index| {
                cells
                    .iter()
                    .position(|(cell, _)| cell == index)
                    .context("Threads matched cell disappeared")
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let reference = image::load_from_memory(second_png.as_ref().unwrap_or(&first_png))?;
        let reference_samples = cells
            .iter()
            .map(|(_, rect)| {
                center_sample(&reference.crop_imm(
                    rect.x as u32,
                    rect.y as u32,
                    rect.width as u32,
                    rect.height as u32,
                ))
            })
            .collect::<Vec<_>>();
        for (ordinal, &position) in source_cell_positions.iter().enumerate() {
            let fresh = observe(session).await?;
            let fresh_cells = photo_cells(&fresh);
            ensure!(
                fresh_cells.len() == cells.len()
                    && fresh_cells
                        .iter()
                        .zip(&cells)
                        .all(|((_, x), (_, y))| x.x == y.x
                            && x.y == y.y
                            && x.width == y.width
                            && x.height == y.height),
                "Threads gallery moved before selection"
            );
            let index = fresh_cells[position].0;
            ensure!(
                selected_ordinal(fresh.nodes[index].attr("content-desc")).is_none(),
                "Threads photo already selected"
            );
            let screenshot = image::load_from_memory(&session.screenshot_png().await?)?;
            let rect = &fresh_cells[position].1;
            let current = center_sample(&screenshot.crop_imm(
                rect.x as u32,
                rect.y as u32,
                rect.width as u32,
                rect.height as u32,
            ));
            ensure!(
                thumbnail_error(&reference_samples[position], &current) <= 8.0,
                "Threads thumbnail changed before selection"
            );
            session
                .tap(
                    fresh.nodes[index]
                        .rect()
                        .context("Threads photo bounds missing")?
                        .centre(),
                )
                .await?;
            let selected = wait_for(session, "photo selection readback", |tree| {
                let latest_cells = photo_cells(tree);
                source_cell_positions
                    .iter()
                    .take(ordinal + 1)
                    .enumerate()
                    .all(|(selected_position, &cell_position)| {
                        latest_cells.get(cell_position).is_some_and(|(i, _)| {
                            selected_ordinal(tree.nodes[*i].attr("content-desc"))
                                == Some(selected_position + 1)
                        })
                    })
            })
            .await
            .with_context(|| format!("Threads photo {} selection readback", ordinal + 1))?;
            let selected_cells = photo_cells(&selected);
            for (selected_position, &cell_position) in
                source_cell_positions.iter().take(ordinal + 1).enumerate()
            {
                ensure!(
                    selected_ordinal(
                        selected.nodes[selected_cells[cell_position].0].attr("content-desc")
                    ) == Some(selected_position + 1),
                    "Threads photo order readback differs"
                );
            }
            ensure!(
                photo_cells(&selected)
                    .iter()
                    .filter(
                        |(i, _)| selected_ordinal(selected.nodes[*i].attr("content-desc"))
                            .is_some()
                    )
                    .count()
                    == ordinal + 1,
                "Threads selected photo count differs"
            );
        }
        source_cell_positions
    };
    tap_text(session, done).await?;
    tap_resource(session, "new_thread_screen_composer").await?;
    enter_caption_and_wait_ready(
        session,
        &bundle.caption,
        &expected,
        std::time::Duration::from_secs(45),
    )
    .await?;
    Ok(PreparedComposer {
        account,
        import_id: import_id.into(),
        selected_count: count,
        selected_order: source_cell_positions,
        caption_sha256: bundle.caption_sha256.clone(),
    })
}

pub(super) async fn enter_caption_and_wait_ready(
    session: &dyn UiSession,
    caption: &str,
    account: &str,
    budget: std::time::Duration,
) -> anyhow::Result<()> {
    let input_error = match session.type_text(caption).await {
        Ok(()) => None,
        Err(error) => {
            let message = format!("{error:#}");
            let lower = message.to_lowercase();
            if !lower.contains("timeout") && !lower.contains("timed out") {
                return Err(error);
            }
            Some(message)
        }
    };
    // A timed-out set_text may have applied. Only observe; never type or Post again.
    let deadline = std::time::Instant::now() + budget;
    let mut consecutive = 0;
    let mut dismissed = false;
    let mut restored_header = false;
    loop {
        let diagnostic = match observe(session).await {
            Ok(tree) => {
                if let Some(description) = (!dismissed)
                    .then(|| typeahead_dismiss_description(&tree))
                    .flatten()
                {
                    tap_description(session, description).await?;
                    dismissed = true;
                    consecutive = 0;
                    "Threads typeahead dismissed".to_owned()
                } else {
                    // Machine 6, 446/en: entering a long caption scrolls the
                    // composer header out of the accessibility viewport. Restore
                    // it within the measured ScrollView, then prove the account
                    // again. Never infer identity from the profile photo.
                    if !restored_header
                        && tree
                            .nodes
                            .iter()
                            .any(|node| node.attr("resource-id") == "new_thread_screen_composer")
                        && unique(&tree, |node| {
                            node.attr("content-desc") == format!("{account} ")
                        })
                        .is_err()
                    {
                        let views = tree
                            .nodes
                            .iter()
                            .filter(|node| {
                                node.attr("class") == "android.widget.ScrollView"
                                    && node.attr("scrollable") == "true"
                            })
                            .filter_map(Node::rect)
                            .collect::<Vec<_>>();
                        if views.len() == 1 {
                            let rect = &views[0];
                            session
                                .swipe(crate::SwipeGesture {
                                    from: crate::TapPoint {
                                        x: rect.x + rect.width / 2.0,
                                        y: rect.y + rect.height * 0.3,
                                    },
                                    to: crate::TapPoint {
                                        x: rect.x + rect.width / 2.0,
                                        y: rect.y + rect.height * 0.8,
                                    },
                                    duration_ms: 500,
                                })
                                .await?;
                            restored_header = true;
                            consecutive = 0;
                            continue;
                        }
                    }
                    match validate_ready(&tree, caption, account) {
                        Ok(_) => {
                            consecutive += 1;
                            if consecutive >= 2 {
                                return Ok(());
                            }
                            "Threads caption awaiting second readback".to_owned()
                        }
                        Err(error) => {
                            consecutive = 0;
                            error.to_string()
                        }
                    }
                }
            }
            Err(error) => {
                consecutive = 0;
                error.to_string()
            }
        };
        ensure!(
            std::time::Instant::now() < deadline,
            "Threads caption/Post readiness timed out: {}; input: {}",
            diagnostic,
            input_error.as_deref().unwrap_or("completed")
        );
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

fn typeahead_dismiss_description(tree: &Tree) -> Option<&'static str> {
    ["Dismiss typeahead", "Bỏ qua menu đề xuất"]
        .into_iter()
        .find(|description| {
            unique(tree, |node| {
                node.attr("content-desc") == *description && node.attr("enabled") == "true"
            })
            .is_ok()
        })
}

pub fn validate_ready(tree: &Tree, caption: &str, account: &str) -> anyhow::Result<ElementBox> {
    let field = unique(tree, |node| {
        node.attr("resource-id") == "new_thread_screen_composer"
            && node.attr("class") == "android.widget.EditText"
    })
    .context("Threads caption field missing or ambiguous")?;
    ensure!(
        tree.nodes[field].attr("text") == caption,
        "Threads caption readback differs"
    );
    unique(tree, |node| {
        node.attr("content-desc") == format!("{account} ")
    })
    .context("Threads composer account missing or ambiguous")?;
    let post = unique(tree, |node| {
        node.attr("resource-id") == "new_thread_screen_post_button"
            && node.attr("enabled") == "true"
    })
    .context("Threads enabled Post button missing or ambiguous")?;
    tree.nodes[post]
        .rect()
        .context("Threads Post bounds missing")
}

fn descendant(tree: &Tree, child: usize, ancestor: usize) -> bool {
    let mut cursor = Some(child);
    while let Some(index) = cursor {
        if index == ancestor {
            return true;
        }
        cursor = tree.nodes[index].parent;
    }
    false
}

fn profile_caption_row(tree: &Tree, caption: &str) -> anyhow::Result<ElementBox> {
    ensure!(
        tree.nodes
            .iter()
            .any(|node| node.attr("resource-id") == "barcelona_tab_profile"
                && node.attr("selected") == "true"),
        "Threads own profile is not selected"
    );
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let caption = unique(tree, |node| {
        normalize(node.attr("text")) == normalize(caption)
    })?;
    let mut current = Some(caption);
    while let Some(index) = current {
        if tree.nodes[index].attr("resource-id") == "FeedPostRow"
            && tree.nodes[index].attr("clickable") == "true"
        {
            return tree.nodes[index]
                .rect()
                .context("Threads post row bounds missing");
        }
        current = tree.nodes[index].parent;
    }
    anyhow::bail!("Threads caption has no clickable post row")
}

/// The permalink screen exposes one original post with its caption, header and Share.
pub fn own_post_share(
    tree: &Tree,
    caption: &str,
    account: &str,
    elapsed_secs: i64,
) -> anyhow::Result<ElementBox> {
    verify_own_post(tree, caption, account, elapsed_secs)?;
    let index = unique(tree, |node| node.attr("resource-id") == "feed_post_ufi_share_button" && node.attr("enabled") == "true")?;
    tree.nodes[index].rect().context("Threads Share bounds missing")
}

fn verify_own_post(tree: &Tree, caption: &str, account: &str, elapsed_secs: i64) -> anyhow::Result<()> {
    ensure!(
        tree.nodes
            .iter()
            .any(|node| node.attr("resource-id") == "PermalinkScreen"),
        "Threads post detail not open"
    );
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = unique(tree, |node| {
        normalize(node.attr("text")) == normalize(caption)
    })?;
    let mut root = tree.nodes[text].parent;
    while let Some(index) = root {
        if tree.nodes[index].attr("resource-id") == "IgLazyColumn" {
            break;
        }
        root = tree.nodes[index].parent;
    }
    let root = root.context("Threads post detail list missing")?;
    let author = unique(tree, |node| {
        node.attr("content-desc") == format!("{account} posted")
            || node.attr("content-desc") == format!("{account} đã đăng")
    })?;
    ensure!(
        descendant(tree, author, root) && author < text,
        "Threads author differs from post detail"
    );
    let shares = tree
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            descendant(tree, *index, root)
                && *index > text
                && node.attr("resource-id") == "feed_post_ufi_share_button"
                && node.attr("enabled") == "true"
        })
        .collect::<Vec<_>>();
    ensure!(
        shares.len() <= 1,
        "Threads Share belongs to another post or is ambiguous"
    );
    let header = tree.nodes[author]
        .parent
        .context("Threads post header missing")?;
    let ages = tree
        .nodes
        .iter()
        .filter(|node| node.parent == Some(header))
        .filter_map(|node| {
            let value = node.attr("content-desc");
            if matches!(value, "now" | "vừa xong") {
                return Some((0, 60));
            }
            for (unit, factor) in [
                ("s", 1),
                ("m", 60),
                ("h", 3600),
                ("d", 86400),
                (" giây", 1),
                (" phút", 60),
                (" giờ", 3600),
                (" ngày", 86400),
            ] {
                if let Some(number) = value.strip_suffix(unit) {
                    return Some((number.parse::<i64>().ok()?.checked_mul(factor)?, factor));
                }
            }
            None
        })
        .collect::<Vec<_>>();
    ensure!(
        ages.len() == 1
            && ages[0].0 - ages[0].1 <= elapsed_secs + 60
            && ages[0].0 + ages[0].1 >= (elapsed_secs - 120).max(0),
        "Threads post timestamp differs from submission"
    );
    Ok(())
}

async fn find_own_post_share(session: &dyn UiSession, mut tree: Tree, caption: &str, account: &str, elapsed: i64) -> anyhow::Result<ElementBox> {
    verify_own_post(&tree, caption, account, elapsed)?;
    for attempt in 0..8 {
        ensure!(tree.nodes.iter().any(|n| n.attr("resource-id") == "PermalinkScreen"), "Threads post detail changed during Share search");
        // Stop before a reply's action row can be mistaken for the verified post.
        ensure!(!tree.nodes.iter().any(|n| {
            let desc = n.attr("content-desc");
            (desc.ends_with(" posted") || desc.ends_with(" đã đăng"))
                && desc != format!("{account} posted") && desc != format!("{account} đã đăng")
        }), "Threads another author appeared before Share; refusing ambiguous link");
        let shares = tree.nodes.iter().filter(|n| n.attr("resource-id") == "feed_post_ufi_share_button" && n.attr("enabled") == "true").collect::<Vec<_>>();
        ensure!(shares.len() <= 1, "Threads multiple Share targets in detail");
        let scroll = tree.nodes.iter().find(|n| n.attr("resource-id") == "IgLazyColumn" && n.attr("scrollable") == "true").and_then(Node::rect);
        if let Some(rect) = shares.first().and_then(|n| n.rect()) {
            if scroll.as_ref().is_none_or(|viewport| rect.y >= viewport.y && rect.y + rect.height <= viewport.y + viewport.height) {
                return Ok(rect);
            }
        }
        ensure!(attempt < 7, "Threads Share not visible after bounded detail scroll");
        let viewport = scroll.context("Threads detail scroll viewport missing")?;
        // Same measured IgLazyColumn gesture as the profile search below.
        let x = viewport.x + viewport.width / 2.0;
        session.swipe(crate::SwipeGesture { from: crate::TapPoint { x, y: viewport.y + viewport.height * 0.8 }, to: crate::TapPoint { x, y: viewport.y + viewport.height * 0.3 }, duration_ms: 500 }).await?;
        tree = observe(session).await?;
    }
    anyhow::bail!("Threads Share not found")
}

pub async fn resolve_share_url(link: &str, expected: &str) -> anyhow::Result<String> {
    let parsed = url::Url::parse(link)?;
    ensure!(
        parsed.scheme() == "https"
            && parsed.host_str() == Some("www.threads.com")
            && parsed.port_or_known_default() == Some(443)
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "Threads share URL invalid"
    );
    if !parsed.path().starts_with("/share/") {
        return canonical_post_permalink(link, expected);
    }
    // Threads can redirect browsers onward to login. The first redirect identifies the post.
    let response = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .head(parsed)
        .send()
        .await?;
    ensure!(
        response.status().is_redirection(),
        "Threads share did not redirect"
    );
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .context("Threads redirect location missing")?
        .to_str()?;
    canonical_post_permalink(response.url().join(location)?.as_str(), expected)
}

/// Verify only. This function cannot open a composer or tap Post, including after restart.
pub async fn verify(
    session: &dyn UiSession,
    caption: &str,
    account: &str,
    submitted_at: &str,
) -> anyhow::Result<String> {
    verify_profile(session, caption, account, submitted_at, false).await
}

/// Reconciliation refreshes the profile before searching the submitted caption.
/// The scheduler owns the two-minute delay; this function never submits a post.
pub async fn verify_after_refresh(
    session: &dyn UiSession,
    caption: &str,
    account: &str,
    submitted_at: &str,
) -> anyhow::Result<String> {
    verify_profile(session, caption, account, submitted_at, true).await
}

async fn verify_profile(
    session: &dyn UiSession,
    caption: &str,
    account: &str,
    submitted_at: &str,
    refresh: bool,
) -> anyhow::Result<String> {
    let submitted = chrono::DateTime::parse_from_rfc3339(submitted_at)?;
    ensure!(
        chrono::Utc::now()
            .signed_duration_since(submitted)
            .num_seconds()
            >= 0,
        "Threads submission timestamp is in the future"
    );
    let initial = observe(session).await?;
    ensure!(
        !initial
            .nodes
            .iter()
            .any(|node| node.attr("resource-id") == "new_thread_screen_post_button"),
        "Threads composer still open; reconcile without another Post"
    );
    if initial
        .nodes
        .iter()
        .any(|node| node.attr("text") == "Not now")
    {
        tap_text(session, "Not now").await?;
    }
    return_to_profile(session).await?;
    // Refresh the observed profile viewport before searching the newly uploaded post.
    let mut profile = wait_for(session, "own profile", |tree| {
        tree.nodes.iter().any(|node| {
            node.attr("resource-id") == "barcelona_tab_profile" && node.attr("selected") == "true"
        }) && tree
            .nodes
            .iter()
            .any(|node| node.attr("resource-id") == "IgLazyColumn")
    })
    .await?;
    for _ in 0..24 {
        if profile_account(&profile).is_ok() {
            break;
        }
        let Some(scroll) = profile
            .nodes
            .iter()
            .find(|node| {
                node.attr("resource-id") == "IgLazyColumn" && node.attr("scrollable") == "true"
            })
            .and_then(Node::rect)
        else {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            profile = observe(session).await?;
            continue;
        };
        let x = scroll.x + scroll.width / 2.0;
        session
            .swipe(crate::SwipeGesture {
                from: crate::TapPoint {
                    x,
                    y: scroll.y + scroll.height * 0.3,
                },
                to: crate::TapPoint {
                    x,
                    y: scroll.y + scroll.height * 0.8,
                },
                duration_ms: 500,
            })
            .await?;
        profile = observe(session).await?;
    }
    ensure!(
        profile_account(&profile)? == normalize_threads_account(account)?,
        "Threads own profile account changed since submission"
    );
    if refresh {
        // Reuse the existing profile scroll gesture within IgLazyColumn's observed
        // bounds (threads_446/profile.xml, 1080x2220), away from the bottom tabs.
        // With the own-profile header visible, a downward pull requests refresh.
        let fresh = observe(session).await?;
        ensure!(
            profile_account(&fresh)? == normalize_threads_account(account)?,
            "Threads own profile account changed before refresh"
        );
        let list = unique(&fresh, |node| {
            node.attr("resource-id") == "IgLazyColumn" && node.attr("scrollable") == "true"
        })?;
        let scroll = fresh.nodes[list]
            .rect()
            .context("Threads profile refresh bounds missing")?;
        let x = scroll.x + scroll.width / 2.0;
        session
            .swipe(crate::SwipeGesture {
                from: crate::TapPoint {
                    x,
                    y: scroll.y + scroll.height * 0.3,
                },
                to: crate::TapPoint {
                    x,
                    y: scroll.y + scroll.height * 0.8,
                },
                duration_ms: 500,
            })
            .await?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        wait_for(session, "own profile after refresh", |tree| {
            tree.nodes
                .iter()
                .any(|node| node.attr("resource-id") == "IgLazyColumn")
                && tree.nodes.iter().any(|node| {
                    node.attr("resource-id") == "barcelona_tab_profile"
                        && node.attr("selected") == "true"
                })
        })
        .await?;
        tap_resource(session, "barcelona_tab_profile").await?;
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        // Resolve the second tap from a new snapshot after the wait.
        tap_resource(session, "barcelona_tab_profile").await?;
        wait_for(session, "own profile after second profile tap", |tree| {
            tree.nodes.iter().any(|node| {
                node.attr("resource-id") == "barcelona_tab_profile"
                    && node.attr("selected") == "true"
            }) && tree
                .nodes
                .iter()
                .any(|node| node.attr("resource-id") == "IgLazyColumn")
        })
        .await?;
    }
    // The profile header can load before its newly submitted feed row. Keep the
    // viewport at the top while the first rows arrive, then search older rows.
    for _ in 0..8 {
        let tree = observe(session).await?;
        if profile_caption_row(&tree, caption).is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    for attempt in 0..6 {
        let tree = observe(session).await?;
        if let Ok(row) = profile_caption_row(&tree, caption) {
            session.tap(row.centre()).await?;
            let detail = wait_for(session, "post detail", |tree| {
                tree.nodes
                    .iter()
                    .any(|node| node.attr("resource-id") == "PermalinkScreen")
            })
            .await?;
            let elapsed = chrono::Utc::now()
                .signed_duration_since(submitted)
                .num_seconds();
            let share = find_own_post_share(session, detail, caption, account, elapsed).await?;
            // An empty/unreadable previous clipboard must not block Copy Link.
            // Only this explicit helper error is optional; auth/transport errors propagate.
            let old_clipboard = match session.get_clipboard(8192).await {
                Ok(value) => Some(value),
                Err(error) if format!("{error:#}").contains("clipboard_unavailable") => None,
                Err(error) => return Err(error),
            };
            session
                .set_clipboard("plaintext", b"riviu-threads-awaiting-link")
                .await?;
            let result = async {
                session.tap(share.centre()).await?;
                tap_clickable_resource(session, "post_share_sheet_copy_link_button").await?;
                let mut copied = None;
                for _ in 0..15 {
                    let bytes = match session.get_clipboard(8192).await {
                        Ok((_, bytes)) => bytes,
                        Err(error) if format!("{error:#}").contains("clipboard_unavailable") => {
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                            continue;
                        }
                        Err(error) => return Err(error),
                    };
                    if bytes != b"riviu-threads-awaiting-link" {
                        let value = String::from_utf8(bytes)?;
                        if value.trim().starts_with("https://") {
                            copied = Some(value);
                            break;
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                resolve_share_url(
                    copied
                        .context("Threads Copy Link did not update clipboard")?
                        .trim(),
                    account,
                )
                .await
            }
            .await;
            if let Some(old_clipboard) = old_clipboard {
                if let Err(error) = session.set_clipboard(&old_clipboard.0, &old_clipboard.1).await {
                    tracing::warn!("Threads clipboard restore: {error}");
                }
            }
            if let Err(error) = close_share_sheet(session).await {
                tracing::warn!("Threads share sheet cleanup: {error}");
            }
            return result;
        }
        if attempt < 5 {
            let Some(scroll) = tree
                .nodes
                .iter()
                .find(|node| {
                    node.attr("resource-id") == "IgLazyColumn" && node.attr("scrollable") == "true"
                })
                .and_then(Node::rect)
            else {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                continue;
            };
            let x = scroll.x + scroll.width / 2.0;
            session
                .swipe(crate::SwipeGesture {
                    from: crate::TapPoint {
                        x,
                        y: scroll.y + scroll.height * 0.8,
                    },
                    to: crate::TapPoint {
                        x,
                        y: scroll.y + scroll.height * 0.3,
                    },
                    duration_ms: 500,
                })
                .await?;
        }
    }
    anyhow::bail!("Threads own caption not found on profile for reconciliation")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instagram_onboarding_only_closes_with_sharing_proven_off() {
        let xml = r#"<hierarchy><node package="com.instagram.barcelona" text="Share this and future threads to your Instagram stories" bounds="[0,0][100,100]"/><node package="com.instagram.barcelona" class="android.widget.Switch" checked="false" bounds="[0,100][100,200]"/><node package="com.instagram.barcelona" text="Done" enabled="true" clickable="true" bounds="[0,200][100,300]"/></hierarchy>"#;
        assert!(instagram_onboarding_done(&fixture(xml)).unwrap().is_some());
        assert!(instagram_onboarding_done(&fixture(
            &xml.replace("checked=\"false\"", "checked=\"true\"")
        ))
        .is_err());
        assert!(
            instagram_onboarding_done(&fixture(&xml.replace("checked=\"false\"", ""))).is_err()
        );
        assert!(instagram_onboarding_done(&fixture(&xml.replace(
            "Share this and future threads to your Instagram stories",
            "Other dialog"
        )))
        .unwrap()
        .is_none());
    }
    #[test]
    fn measured_vietnamese_share_overlay_is_recognized() {
        let xml = include_str!("../../fixtures/threads_446/share_vi.xml");
        assert!(share_sheet_open(&fixture(xml)));
        assert!(!share_sheet_open(&fixture(
            &xml.replace("Đóng trang", "Other action")
        )));
    }

    #[test]
    fn machine_five_vietnamese_typeahead_hides_post_until_dismissed() {
        let open = fixture(include_str!(
            "../../fixtures/threads_446/typeahead_vi_open.xml"
        ));
        let closed = fixture(include_str!(
            "../../fixtures/threads_446/typeahead_vi_closed.xml"
        ));
        let caption = closed
            .nodes
            .iter()
            .find(|node| node.attr("resource-id") == "new_thread_screen_composer")
            .unwrap()
            .attr("text");
        assert_eq!(
            typeahead_dismiss_description(&open),
            Some("Bỏ qua menu đề xuất")
        );
        assert!(validate_ready(&open, caption, "fixture.account").is_err());
        assert_eq!(typeahead_dismiss_description(&closed), None);
        assert!(validate_ready(&closed, caption, "fixture.account").is_ok());
        assert!(validate_ready(&closed, caption, "other.account").is_err());
    }

    #[test]
    fn machine_eight_gallery_requires_scroll_to_full_last_photo() {
        let top = fixture(include_str!(
            "../../fixtures/threads_446/gallery_15_vi_top.xml"
        ));
        let partial = fixture(include_str!(
            "../../fixtures/threads_446/gallery_15_vi_partial.xml"
        ));
        let bottom = fixture(include_str!(
            "../../fixtures/threads_446/gallery_15_vi_bottom.xml"
        ));
        assert_eq!(photo_cells(&top).len(), 14);
        assert_eq!(photo_cells(&partial).len(), 15);
        assert_eq!(photo_cells(&bottom).len(), 13);
        assert!(!scrolled_gallery_bottom_ready(
            &partial,
            "riviu-fixture",
            15,
            14
        ));
        assert!(scrolled_gallery_bottom_ready(
            &bottom,
            "riviu-fixture",
            15,
            14
        ));
        assert!(!scrolled_gallery_bottom_ready(
            &bottom,
            "riviu-other",
            15,
            14
        ));
    }

    #[test]
    fn fifteen_photo_album_matches_overlapping_viewports_in_source_order() {
        fn png(image: &image::RgbImage) -> Vec<u8> {
            let mut output = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(image.clone())
                .write_to(&mut output, image::ImageFormat::Png)
                .unwrap();
            output.into_inner()
        }
        fn cells(count: usize) -> Vec<(usize, ElementBox)> {
            (0..count)
                .map(|index| {
                    (
                        index,
                        ElementBox {
                            x: (index * 100) as f64,
                            y: 0.0,
                            width: 100.0,
                            height: 100.0,
                            description: None,
                            enabled: true,
                            clickable: true,
                        },
                    )
                })
                .collect()
        }
        let images = (0..15)
            .map(|index| image::RgbImage::from_pixel(100, 100, image::Rgb([index * 16; 3])))
            .collect::<Vec<_>>();
        let source = images.iter().rev().map(png).collect::<Vec<_>>();
        let mut top = image::RgbImage::new(1400, 100);
        let mut bottom = image::RgbImage::new(1300, 100);
        for (index, image) in images.iter().enumerate() {
            if index < 14 {
                image::imageops::replace(&mut top, image, (index * 100) as i64, 0);
            }
            if index >= 2 {
                image::imageops::replace(&mut bottom, image, ((index - 2) * 100) as i64, 0);
            }
        }
        let top_cells = cells(14);
        let bottom_cells = cells(13);
        assert_eq!(
            match_scrolled_gallery(
                &source,
                &png(&top),
                &top_cells,
                &png(&bottom),
                &bottom_cells,
            )
            .unwrap(),
            (0..15).rev().collect::<Vec<_>>()
        );
        image::imageops::replace(&mut bottom, &images[4], 0, 0);
        assert!(match_scrolled_gallery(
            &source,
            &png(&top),
            &top_cells,
            &png(&bottom),
            &bottom_cells,
        )
        .is_err());
    }
    fn fixture(xml: &str) -> Tree {
        Tree::parse(crate::HierarchySourceSnapshot {
            generation: 1,
            xml: xml.into(),
        })
        .unwrap()
    }
    #[test]
    fn measured_profile_does_not_confuse_display_name_with_username() {
        let tree = fixture(include_str!("../../fixtures/threads_446/profile.xml"));
        assert_eq!(profile_account(&tree).unwrap(), "fixture.account");
    }
    #[test]
    fn read_only_profile_accepts_missing_selected_flag_with_own_edit_control() {
        let original = include_str!("../../fixtures/threads_446/profile.xml");
        let selected = "resource-id=\"barcelona_tab_profile\" text=\"\" content-desc=\"\" enabled=\"true\" selected=\"true\"";
        assert!(original.contains(selected));
        let unselected = original.replacen(selected, "resource-id=\"barcelona_tab_profile\" text=\"\" content-desc=\"\" enabled=\"true\" selected=\"false\"", 1);
        let tree = fixture(&unselected);
        assert!(profile_account(&tree).is_err());
        assert_eq!(profile_header_account(&tree).unwrap(), "fixture.account");
        let without_edit =
            unselected.replace("profile_screen_edit_profile_button", "other_control");
        assert!(profile_header_account(&fixture(&without_edit)).is_err());
    }
    #[test]
    fn account_check_recognizes_credential_login_and_verification_gates() {
        let login = fixture(
            r#"<hierarchy><node text="Username, email or mobile number" bounds="[0,0][100,100]"/><node text="Password" bounds="[0,0][100,100]"/></hierarchy>"#,
        );
        assert!(login_screen_open(&login));
        let challenge = fixture(
            r#"<hierarchy><node text="Confirm you're human to use your account" bounds="[0,0][100,100]"/></hierarchy>"#,
        );
        assert_eq!(
            account_gate(&challenge),
            Some("Threads account requires human verification")
        );
        let phone = fixture(
            r#"<hierarchy><node text="Enter your mobile number" bounds="[0,0][100,100]"/></hierarchy>"#,
        );
        assert_eq!(
            account_gate(&phone),
            Some("Threads account requires phone verification")
        );
    }
    #[test]
    fn exact_import_album_has_eleven_photos_and_order_readback() {
        let album = fixture(include_str!("../../fixtures/threads_446/albums.xml"));
        assert_eq!(album_count(&album, "riviu-fixture-import").unwrap(), 11);
        assert!(album_count(&album, "riviu-wrong-import").is_err());
        let gallery = fixture(include_str!("../../fixtures/threads_446/selected.xml"));
        let cells = photo_cells(&gallery);
        assert_eq!(cells.len(), 11);
        let mut ordinals = cells
            .iter()
            .map(|(index, _)| selected_ordinal(gallery.nodes[*index].attr("content-desc")).unwrap())
            .collect::<Vec<_>>();
        ordinals.sort();
        assert_eq!(ordinals, (1..=11).collect::<Vec<_>>());
    }
    #[test]
    fn composer_requires_exact_caption_and_account() {
        let vi_xml = include_str!("../../fixtures/threads_446_vi/composer.xml");
        let vi = fixture(vi_xml);
        assert!(validate_ready(&vi, "", "fixture.account").is_ok());
        assert!(validate_ready(&vi, "", "wrong.account").is_err());
        let tree = fixture(include_str!("../../fixtures/threads_446/composer.xml"));
        assert!(validate_ready(&tree, "Fixture caption", "fixture.account").is_ok());
        assert!(validate_ready(&tree, "Changed caption", "fixture.account").is_err());
        assert!(validate_ready(&tree, "Fixture caption", "other.account").is_err());
    }
    #[test]
    fn existing_draft_is_preserved_only_through_measured_save_dialog() {
        let vi = fixture(include_str!("../../fixtures/threads_446_vi/save_draft.xml"));
        assert!(save_draft_dialog(&vi).is_ok());
        let dialog = fixture(include_str!("../../fixtures/threads_446/save_draft.xml"));
        assert!(save_draft_dialog(&dialog).is_ok());
        let changed = fixture(
            &include_str!("../../fixtures/threads_446/save_draft.xml")
                .replace("Save to drafts?", "Discard draft?"),
        );
        assert!(save_draft_dialog(&changed).is_err());
    }
    #[test]
    fn media_permission_requires_exact_threads_prompt_and_allow_button() {
        let xml = include_str!("../../fixtures/threads_446/media_permission.xml");
        assert!(threads_media_permission_button(&fixture(xml)).is_ok());
        let padded = xml.replace("text=\"Allow\"", "text=\"&#x200E;&#x200F;Allow&#x200E;\"");
        assert!(threads_media_permission_button(&fixture(&padded)).is_ok());
        assert!(
            threads_media_permission_button(&fixture(&xml.replace("Threads", "Other app")))
                .is_err()
        );
        assert!(threads_media_permission_button(&fixture(
            &xml.replace("permission_allow_button", "permission_deny_button")
        ))
        .is_err());
    }
    #[test]
    fn vietnamese_gallery_uses_observed_labels_even_when_device_locale_is_english() {
        let gallery = fixture(include_str!("../../fixtures/threads_446_vi/gallery.xml"));
        assert_eq!(
            measured_gallery_labels(&gallery).unwrap(),
            ("Mới đây", "Xong")
        );
        assert_eq!(photo_cells(&gallery).len(), 17);
        let albums = fixture(include_str!("../../fixtures/threads_446_vi/albums.xml"));
        assert_eq!(album_count(&albums, "riviu-fixture-import").unwrap(), 13);
        let incomplete = fixture(include_str!("../../fixtures/threads_446/gallery_vi.xml"));
        assert!(measured_gallery_labels(&incomplete).is_err());
        let mixed = fixture(
            &include_str!("../../fixtures/threads_446_vi/gallery.xml")
                .replace("text=\"Xong\"", "text=\"Done\""),
        );
        assert!(measured_gallery_labels(&mixed).is_err());
    }
    #[test]
    fn own_post_verification_binds_share_to_author_caption_and_submission_time() {
        let xml = include_str!("../../fixtures/threads_446/detail.xml");
        let tree = fixture(xml);
        assert!(own_post_share(&tree, "Fixture caption", "fixture.account", 180).is_ok());
        assert!(own_post_share(&tree, "Fixture caption", "fixture.account", 130).is_ok());
        assert!(own_post_share(&tree, "Wrong caption", "fixture.account", 180).is_err());
        assert!(own_post_share(&tree, "Fixture caption", "other.account", 180).is_err());
        assert!(own_post_share(&tree, "Fixture caption", "fixture.account", 0).is_err());
        assert!(own_post_share(&tree, "Fixture caption", "fixture.account", 1000).is_err());
    }
    #[test]
    fn measured_vietnamese_post_verifies_author_and_age() {
        let tree = fixture(include_str!("../../fixtures/threads_446/detail_vi.xml"));
        let caption = tree
            .nodes
            .iter()
            .find(|node| node.attr("text").contains("#72hdalat"))
            .unwrap()
            .attr("text");
        assert!(own_post_share(&tree, caption, "fixture.account", 60).is_ok());
        assert!(own_post_share(&tree, caption, "other.account", 60).is_err());
        assert!(own_post_share(&tree, caption, "fixture.account", 3600).is_err());
    }
    #[test]
    fn selection_uses_ordinal_and_ignores_broken_denominator() {
        let selected = fixture(include_str!("../../fixtures/threads_446_vi/selected.xml"));
        let cells = photo_cells(&selected);
        assert_eq!(cells.len(), 13);
        assert_eq!(
            selected_ordinal(selected.nodes[cells[0].0].attr("content-desc")),
            Some(1)
        );
        assert_eq!(
            selected_ordinal("Đã chọn 13/0. Hình nhỏ ảnh, Đã thêm vào 1 phút trước"),
            Some(13)
        );
        assert_eq!(selected_ordinal("Đã chọn 0/0. Hình nhỏ ảnh"), None);
        assert_eq!(selected_ordinal("Đã chọn 1/0. Hình nhỏ video"), None);
        assert_eq!(selected_ordinal("Đã chọn 1/x. Hình nhỏ ảnh"), None);
        assert_eq!(
            selected_ordinal("11 of 0 selected. Photo thumbnail, Added on 8 minutes ago"),
            Some(11)
        );
        assert_eq!(
            selected_ordinal("Photo thumbnail, Added on 1 minute ago"),
            None
        );
        assert_eq!(selected_ordinal("0 of 0 selected. Photo thumbnail"), None);
        assert_eq!(selected_ordinal("1 of 0 selected. Video thumbnail"), None);
    }
}
