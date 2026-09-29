//! Fail-closed contract for publishing through the Threads mobile app.

use image::{imageops::FilterType, DynamicImage, GenericImageView, RgbImage};
use serde::{Deserialize, Serialize};
use url::Url;

pub const ANDROID_PACKAGE: &str = "com.instagram.barcelona";
pub const MAX_CAPTION_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadsPublishStep {
    ResolveApp,
    OpenComposer,
    SelectMedia,
    EnterCaption,
    ArmPublish,
    RecordIntent,
    DispatchOnce,
    VerifyOwnPost,
    CapturePermalink,
}

pub const THREADS_PUBLISH_FLOW: &[ThreadsPublishStep] = &[
    ThreadsPublishStep::ResolveApp,
    ThreadsPublishStep::OpenComposer,
    ThreadsPublishStep::SelectMedia,
    ThreadsPublishStep::EnterCaption,
    ThreadsPublishStep::ArmPublish,
    ThreadsPublishStep::RecordIntent,
    ThreadsPublishStep::DispatchOnce,
    ThreadsPublishStep::VerifyOwnPost,
    ThreadsPublishStep::CapturePermalink,
];

pub fn validate_caption(caption: &str) -> anyhow::Result<()> {
    let length = caption.chars().count();
    anyhow::ensure!(!caption.trim().is_empty(), "nội dung Threads đang trống");
    anyhow::ensure!(
        length <= MAX_CAPTION_CHARS,
        "nội dung Threads có {length} ký tự; tối đa {MAX_CAPTION_CHARS}"
    );
    Ok(())
}

/// No tuple is admitted until its accessibility hierarchy and post-link verifier
/// have been captured as fixtures. Package detection alone is not automation support.
pub fn measured_mobile_build(_package: &str, _version: &str, _locale: &str) -> bool {
    false
}

pub fn normalize_threads_account(account: &str) -> anyhow::Result<String> {
    let normalized = account.trim().trim_start_matches('@').to_ascii_lowercase();
    anyhow::ensure!(
        !normalized.is_empty()
            && normalized.len() <= 30
            && !normalized.ends_with('.')
            && normalized
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_')),
        "invalid Threads account"
    );
    Ok(normalized)
}

/// Accept only the canonical post URL reached after Threads redirects its share URL.
/// A clipboard `threads.com/share/...` link alone does not identify the author.
pub fn canonical_post_permalink(
    redirected_url: &str,
    expected_account: &str,
) -> anyhow::Result<String> {
    let expected = normalize_threads_account(expected_account)?;
    let parsed = Url::parse(redirected_url)?;
    anyhow::ensure!(
        parsed.scheme() == "https"
            && matches!(parsed.host_str(), Some("threads.com" | "www.threads.com")),
        "Threads permalink must use HTTPS on threads.com"
    );
    anyhow::ensure!(
        parsed.username().is_empty() && parsed.password().is_none(),
        "Threads permalink has credentials"
    );
    let segments = parsed
        .path_segments()
        .map(Iterator::collect::<Vec<_>>)
        .unwrap_or_default();
    anyhow::ensure!(
        segments.len() == 3 && segments[1] == "post",
        "Threads link is not a canonical post"
    );
    let author = segments[0].strip_prefix('@').unwrap_or_default();
    anyhow::ensure!(
        normalize_threads_account(author)? == expected,
        "Threads permalink belongs to another account"
    );
    anyhow::ensure!(
        (8..=32).contains(&segments[2].len())
            && segments[2]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
        "Threads permalink has an invalid post id"
    );
    Ok(format!(
        "https://www.threads.com/@{expected}/post/{}",
        segments[2]
    ))
}

#[derive(Debug, Clone, Copy)]
pub struct GalleryPhotoCell {
    pub element_index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

fn center_sample(image: &DynamicImage) -> RgbImage {
    let (width, height) = image.dimensions();
    let side = width.min(height);
    let square = image.crop_imm((width - side) / 2, (height - side) / 2, side, side);
    let inset = (side / 5).min(side.saturating_sub(1) / 2);
    square
        .crop_imm(inset, inset, side - inset * 2, side - inset * 2)
        .resize_exact(48, 48, FilterType::Triangle)
        .to_rgb8()
}

fn thumbnail_error(left: &RgbImage, right: &RgbImage) -> f64 {
    left.pixels()
        .zip(right.pixels())
        .flat_map(|(a, b)| a.0.into_iter().zip(b.0).map(|(a, b)| a.abs_diff(b) as u64))
        .sum::<u64>() as f64
        / (48 * 48 * 3) as f64
}

/// Return photo element indices in source order. `cells` must already exclude the camera tile.
/// The private import album must contain exactly the imported files before calling this.
pub fn match_imported_photos(
    source_images: &[&[u8]],
    screenshot_png: &[u8],
    cells: &[GalleryPhotoCell],
    album_title: &str,
    expected_album: &str,
    album_photo_count: usize,
) -> anyhow::Result<Vec<usize>> {
    anyhow::ensure!(
        !source_images.is_empty(),
        "Threads album has no source images"
    );
    anyhow::ensure!(
        album_title == expected_album && expected_album.starts_with("riviu-"),
        "Threads import album differs"
    );
    anyhow::ensure!(
        album_photo_count == source_images.len() && cells.len() == source_images.len(),
        "Threads album photo count differs"
    );
    let screen = image::load_from_memory(screenshot_png)?;
    let screen_bounds = screen.dimensions();
    let samples = cells
        .iter()
        .map(|cell| {
            anyhow::ensure!(
                cell.width >= 48 && cell.height >= 48,
                "Threads thumbnail is too small"
            );
            anyhow::ensure!(
                cell.x
                    .checked_add(cell.width)
                    .is_some_and(|right| right <= screen_bounds.0)
                    && cell
                        .y
                        .checked_add(cell.height)
                        .is_some_and(|bottom| bottom <= screen_bounds.1),
                "Threads thumbnail is outside screenshot"
            );
            Ok(center_sample(&screen.crop_imm(
                cell.x,
                cell.y,
                cell.width,
                cell.height,
            )))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut ordered = Vec::with_capacity(source_images.len());
    for bytes in source_images {
        let source = center_sample(&image::load_from_memory(bytes)?);
        let mut scores = samples
            .iter()
            .enumerate()
            .map(|(index, sample)| (index, thumbnail_error(&source, sample)))
            .collect::<Vec<_>>();
        scores.sort_by(|a, b| a.1.total_cmp(&b.1));
        anyhow::ensure!(
            scores[0].1 <= 15.0
                && scores
                    .get(1)
                    .is_none_or(|other| other.1 - scores[0].1 >= 12.0),
            "Threads thumbnail identity is ambiguous"
        );
        ordered.push(cells[scores[0].0].element_index);
    }
    let unique = ordered.iter().collect::<std::collections::HashSet<_>>();
    anyhow::ensure!(
        unique.len() == ordered.len(),
        "Threads thumbnail maps to more than one source image"
    );
    Ok(ordered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_records_intent_before_the_only_public_effect() {
        let intent = THREADS_PUBLISH_FLOW
            .iter()
            .position(|step| *step == ThreadsPublishStep::RecordIntent)
            .unwrap();
        let dispatch = THREADS_PUBLISH_FLOW
            .iter()
            .position(|step| *step == ThreadsPublishStep::DispatchOnce)
            .unwrap();
        assert!(intent < dispatch);
        assert_eq!(
            THREADS_PUBLISH_FLOW
                .iter()
                .filter(|step| **step == ThreadsPublishStep::DispatchOnce)
                .count(),
            1
        );
    }

    #[test]
    fn caption_uses_the_threads_character_limit() {
        assert!(validate_caption(&"x".repeat(500)).is_ok());
        assert!(validate_caption(&"x".repeat(501)).is_err());
        assert!(validate_caption("  ").is_err());
    }

    #[test]
    fn canonicalizes_redirected_share_link_for_the_expected_author() {
        assert_eq!(
            canonical_post_permalink(
                "https://www.threads.com/@sample.account/post/DdsuyOQFM19?xmt=one&slof=1",
                "@Sample.Account"
            )
            .unwrap(),
            "https://www.threads.com/@sample.account/post/DdsuyOQFM19"
        );
    }

    #[test]
    fn rejects_share_link_wrong_author_and_unsafe_urls() {
        for url in [
            "https://www.threads.com/share/BAanFW-7p0/",
            "https://www.threads.com/@other/post/DdsuyOQFM19",
            "http://www.threads.com/@sample/post/DdsuyOQFM19",
            "https://threads.com.evil.test/@sample/post/DdsuyOQFM19",
            "https://www.threads.com/@sample/post/../../other",
        ] {
            assert!(canonical_post_permalink(url, "sample").is_err(), "{url}");
        }
    }

    #[test]
    fn threads_account_accepts_thirty_characters() {
        let account = "a".repeat(30);
        assert_eq!(normalize_threads_account(&account).unwrap(), account);
        assert!(normalize_threads_account(&"a".repeat(31)).is_err());
    }

    fn png(image: &RgbImage) -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(image.clone())
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn gallery_matches_source_order_independent_of_display_order() {
        let first = RgbImage::from_fn(100, 100, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 2) as u8, 30])
        });
        let second = RgbImage::from_fn(100, 100, |x, y| {
            image::Rgb([20, (x * 2) as u8, (y * 2) as u8])
        });
        let mut screen = RgbImage::new(200, 100);
        image::imageops::replace(&mut screen, &second, 0, 0);
        image::imageops::replace(&mut screen, &first, 100, 0);
        let cells = [
            GalleryPhotoCell {
                element_index: 8,
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            GalleryPhotoCell {
                element_index: 12,
                x: 100,
                y: 0,
                width: 100,
                height: 100,
            },
        ];
        assert_eq!(
            match_imported_photos(
                &[&png(&first), &png(&second)],
                &png(&screen),
                &cells,
                "riviu-import-1",
                "riviu-import-1",
                2
            )
            .unwrap(),
            vec![12, 8]
        );
        assert!(match_imported_photos(
            &[&png(&first)],
            &png(&screen),
            &cells,
            "riviu-import-1",
            "riviu-import-1",
            2
        )
        .is_err());
        assert!(match_imported_photos(
            &[&png(&first), &png(&second)],
            &png(&screen),
            &cells,
            "Recents",
            "riviu-import-1",
            2
        )
        .is_err());
    }

    #[test]
    fn gallery_refuses_duplicate_or_unmatched_thumbnails() {
        let source = RgbImage::from_fn(100, 100, |x, y| {
            image::Rgb([(x * 2) as u8, (y * 2) as u8, 30])
        });
        let other = RgbImage::from_pixel(100, 100, image::Rgb([20, 80, 180]));
        let mut duplicate = RgbImage::new(200, 100);
        image::imageops::replace(&mut duplicate, &source, 0, 0);
        image::imageops::replace(&mut duplicate, &source, 100, 0);
        let cells = [
            GalleryPhotoCell {
                element_index: 8,
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            GalleryPhotoCell {
                element_index: 12,
                x: 100,
                y: 0,
                width: 100,
                height: 100,
            },
        ];
        assert!(match_imported_photos(
            &[&png(&source), &png(&other)],
            &png(&duplicate),
            &cells,
            "riviu-import-1",
            "riviu-import-1",
            2
        )
        .is_err());
    }

    #[test]
    fn measured_fixture_keeps_send_closed_without_after_send_verifier() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/threads_446_en_us.json")).unwrap();
        assert_eq!(fixture["observed"]["afterNewSendMeasured"], false);
        assert_eq!(fixture["observed"]["publicPostDispatched"], false);
        assert!(!measured_mobile_build(
            fixture["package"].as_str().unwrap(),
            fixture["version"].as_str().unwrap(),
            fixture["locale"].as_str().unwrap(),
        ));
    }
}
