//! Android 446 accessibility adapter. Every tap is derived from a fresh hierarchy.
use super::*;
use crate::ui_automation::tree::{Node, Tree};
use crate::{ElementBox, UiSession};
use anyhow::{ensure, Context};
use sha2::{Digest, Sha256};

pub const MEASURED_VERSION: &str = "446.0.0.32.78";
pub const ADAPTER_ENABLED: bool = true;
pub const MAX_MEASURED_IMAGES: usize = 14;
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
                && node.attr("text") == "Allow"
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

pub async fn observed_profile_account(session: &dyn UiSession) -> anyhow::Result<String> {
    tap_resource(session, "barcelona_tab_profile").await?;
    let profile = wait_for(session, "own profile header", |tree| {
        profile_account(tree).is_ok()
    })
    .await?;
    profile_account(&profile).context("Threads own profile account unavailable")
}

pub fn selected_ordinal(description: &str) -> Option<usize> {
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
                && node.attr("content-desc").contains("Photo thumbnail")
                && node.visible(ANDROID_PACKAGE))
            .then(|| node.rect().map(|rect| (index, rect)))
            .flatten()
        })
        .collect()
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
            && node.attr("text") == "Save to drafts?"
    })?;
    unique(tree, |node| node.attr("text") == "Save")?;
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
        "Threads measured adapter accepts 1 to 14 images"
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
                || node.attr("content-desc") == "Remove attachment"
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
    tap_text(session, "Recents").await?;
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
            && photo_cells(tree).len() == count
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
        cells.len() == count,
        "Threads all imported photo cells must be visible"
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
                node.attr("class") == "android.widget.GridView" && node.attr("scrollable") == "true"
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
                    |(i, _)| selected_ordinal(selected.nodes[*i].attr("content-desc")).is_some()
                )
                .count()
                == ordinal + 1,
            "Threads selected photo count differs"
        );
    }
    tap_text(session, "Done").await?;
    tap_resource(session, "new_thread_screen_composer").await?;
    session.type_text(&bundle.caption).await?;
    let typed = observe(session).await?;
    if typed
        .nodes
        .iter()
        .any(|node| node.attr("content-desc") == "Dismiss typeahead")
    {
        tap_description(session, "Dismiss typeahead").await?;
    }
    let ready = wait_for(session, "caption and enabled Post", |tree| {
        validate_ready(tree, &bundle.caption, &expected).is_ok()
    })
    .await?;
    validate_ready(&ready, &bundle.caption, &expected)?;
    Ok(PreparedComposer {
        account,
        import_id: import_id.into(),
        selected_count: count,
        selected_order: source_cell_positions,
        caption_sha256: bundle.caption_sha256.clone(),
    })
}

pub fn validate_ready(tree: &Tree, caption: &str, account: &str) -> anyhow::Result<ElementBox> {
    let field = unique(tree, |node| {
        node.attr("resource-id") == "new_thread_screen_composer"
            && node.attr("class") == "android.widget.EditText"
    })?;
    ensure!(
        tree.nodes[field].attr("text") == caption,
        "Threads caption readback differs"
    );
    unique(tree, |node| {
        node.attr("content-desc") == format!("{account} ")
    })?;
    let post = unique(tree, |node| {
        node.attr("resource-id") == "new_thread_screen_post_button"
            && node.attr("enabled") == "true"
    })?;
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
        shares.len() == 1,
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
            if value == "now" {
                return Some((0, 60));
            }
            for (unit, factor) in [("s", 1), ("m", 60), ("h", 3600), ("d", 86400)] {
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
    shares[0].1.rect().context("Threads Share bounds missing")
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
    if initial
        .nodes
        .iter()
        .any(|node| node.attr("resource-id") == "PermalinkScreen")
    {
        tap_resource(session, "navigation_bar_back_button").await?;
    }
    tap_resource(session, "barcelona_tab_profile").await?;
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
    for _ in 0..12 {
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
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
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
            let share = own_post_share(&detail, caption, account, elapsed)?;
            let old_clipboard = session.get_clipboard(8192).await?;
            session
                .set_clipboard("plaintext", b"riviu-threads-awaiting-link")
                .await?;
            let result = async {
                session.tap(share.centre()).await?;
                tap_clickable_resource(session, "post_share_sheet_copy_link_button").await?;
                let mut copied = None;
                for _ in 0..15 {
                    let (_, bytes) = session.get_clipboard(8192).await?;
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
            if let Err(error) = session
                .set_clipboard(&old_clipboard.0, &old_clipboard.1)
                .await
            {
                tracing::warn!("Threads clipboard restore: {error}");
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
        let tree = fixture(include_str!("../../fixtures/threads_446/composer.xml"));
        assert!(validate_ready(&tree, "Fixture caption", "fixture.account").is_ok());
        assert!(validate_ready(&tree, "Changed caption", "fixture.account").is_err());
        assert!(validate_ready(&tree, "Fixture caption", "other.account").is_err());
    }
    #[test]
    fn existing_draft_is_preserved_only_through_measured_save_dialog() {
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
    fn selection_uses_ordinal_and_ignores_broken_denominator() {
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
