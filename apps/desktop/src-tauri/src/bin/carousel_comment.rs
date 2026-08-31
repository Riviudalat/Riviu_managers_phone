//! What the AI writes about a photo post, from slide one alone versus from every slide.
//!
//! ```text
//! cargo run -p riviu-managers-phone --bin carousel_comment -- <dir-of-slide-pngs>
//! cargo run -p riviu-managers-phone --bin carousel_comment -- --link <url-bai-tiktok>
//! ```
//!
//! **`--link` is the headless gate for the web evidence path.** It calls
//! `tiktok_web::fetch_post_context` and `fetch_slides` — the same two the campaign calls —
//! so the pictures, the slide count and the caption all come from where production gets
//! them, and still no phone is touched and nothing is posted.
//!
//! **It writes nothing to any phone and posts nothing.** It reads the operator's real settings
//! and API key, hands the frames to `prepare_comment_for_frames` — the same call the campaign
//! makes — and prints the drafts.
//!
//! It exists because the defect that started this is not visible in any of the gates around it.
//! `carousel_gate` proves the phone photographs both slides; the sheet tests prove both survive
//! into the picture the model sees. Neither answers the operator's actual complaint, which was
//! that *the comment talked about the wrong thing*. The only way to see that is to read what
//! comes back, and to read it beside what the old evidence would have produced — so this runs
//! both: slide one on its own as `Moments`, which is exactly what the campaign used to send,
//! and every slide as `CarouselSlides`, which is what it sends now.
//!
//! Directory in, so it runs on the pictures `carousel_gate` already saved rather than taking a
//! phone for itself. Nothing here needs a device.

use std::path::PathBuf;
use std::sync::Arc;

use riviu_core::db::{Database, SecretStore};
use riviu_core::openai_client::{
    prepare_comment_for_frames, prepare_grounded_comments_batch, EvidenceKind,
};
use riviu_signing::CredentialStore;

/// The same seam `AppState::bootstrap` uses, so the key comes out of the same place the app put
/// it. Duplicated rather than shared only because it is private to the lib and four lines long.
struct KeyringSecrets {
    credentials: CredentialStore,
}

/// Whether the final caption came from a source the harness can name as authoritative.
///
/// `--link` reads the post page and `--caption` is an explicit fixture supplied by the
/// operator. A replayed directory with neither has pixels only. The final text check matters
/// because an empty lookup or `--caption "   "` is absence, not authoritative emptiness.
fn caption_is_authoritative(
    link: Option<&str>,
    forced_caption: Option<&str>,
    caption: Option<&str>,
) -> bool {
    let has_caption = caption.is_some_and(|caption| !caption.trim().is_empty());
    let has_source =
        link.is_some() || forced_caption.is_some_and(|caption| !caption.trim().is_empty());
    has_caption && has_source
}

fn prefer_forced_caption(caption: Option<String>, forced_caption: Option<&str>) -> Option<String> {
    forced_caption
        .filter(|caption| !caption.trim().is_empty())
        .map(str::to_owned)
        .or(caption)
}

impl SecretStore for KeyringSecrets {
    fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
        self.credentials.app_secret(name)
    }
    fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
        self.credentials.set_app_secret(name, value)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // `--link` fetches the post the way the campaign does; a bare directory is the older mode
    // that replays pictures a phone already took.
    let link = std::env::args()
        .position(|arg| arg == "--link")
        .and_then(|at| std::env::args().nth(at + 1));
    // `--caption "..."` forces one, for measuring the caption's effect against pictures that
    // are already on disk.
    let forced_caption = std::env::args()
        .position(|arg| arg == "--caption")
        .and_then(|at| std::env::args().nth(at + 1));

    let (frames, mut caption, transcript, coverage, kind) = if let Some(link) = &link {
        let context = riviu_core::tiktok_web::fetch_post_context(link)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        println!(
            "link     {link}\ncaption  {} ký tự\nthời lượng {:?}s\n\
             số ảnh   {}\nphụ đề   {:?} (có tiếng gốc: {:?})\n",
            context
                .caption
                .as_deref()
                .map(|c| c.chars().count())
                .unwrap_or(0),
            context.duration_secs,
            context.slide_urls.len(),
            context.subtitle_langs(),
            context.has_original_audio,
        );

        // Fetched here rather than beside the pictures, because a video has no pictures on the
        // web and this is the whole reason `--link` works on one at all.
        let transcript = match context.could_have_transcript() {
            true => {
                let track = context.transcript_track().expect("checked");
                let text = riviu_core::tiktok_web::fetch_transcript(track).await;
                match &text {
                    Some(text) => println!(
                        "lời thoại {} ({}) — {} từ:\n  {text}\n",
                        track.lang,
                        track.source,
                        text.split_whitespace().count()
                    ),
                    None => println!("lời thoại {} có nhưng tải về không được\n", track.lang),
                }
                text
            }
            false => {
                println!("lời thoại: không có (bài ảnh, hoặc video dùng nhạc nền)\n");
                None
            }
        };

        if context.slide_urls.is_empty() {
            // **A video, and the cover is the only picture the web will give.** Production
            // does not use it — a campaign's video frames still come off a phone's stream —
            // so this is the harness standing in for one, and it is labelled as `Moments`
            // because that is what the campaign calls a video's frames.
            let Some(cover) = context.cover_url.clone() else {
                anyhow::bail!("bài này không có ảnh nào và cũng không có ảnh bìa");
            };
            println!("ảnh      1 ảnh bìa (video: production lấy khung từ máy, không từ web)");
            let frames = riviu_core::tiktok_web::fetch_slides(&[cover]).await;
            if frames.is_empty() {
                anyhow::bail!("không tải được ảnh bìa");
            }
            // The cover is one frame of nothing in particular, so the span is zero and the
            // note will say the video went unwatched — which is exactly what happened.
            (
                frames,
                context.caption,
                transcript,
                Some(riviu_core::openai_client::PostCoverage::Video {
                    seen_secs: 0,
                    total_secs: context.duration_secs,
                }),
                EvidenceKind::Moments,
            )
        } else {
            let picks = riviu_core::tiktok_web::pick_slide_indices(
                context.slide_urls.len(),
                riviu_core::openai_client::SHEET_MAX_FRAMES,
            );
            println!(
                "lấy ảnh  {:?} trong {} ảnh",
                picks.iter().map(|index| index + 1).collect::<Vec<_>>(),
                context.slide_urls.len()
            );
            let chosen: Vec<String> = picks
                .iter()
                .filter_map(|index| context.slide_urls.get(*index))
                .cloned()
                .collect();
            let frames = riviu_core::tiktok_web::fetch_slides(&chosen).await;
            if frames.is_empty() {
                anyhow::bail!("tra được ảnh nhưng CDN không trả về tấm nào");
            }
            let total = context.slide_urls.len();
            (
                frames,
                context.caption,
                transcript,
                Some(riviu_core::openai_client::PostCoverage::Slides { total }),
                EvidenceKind::CarouselSlides,
            )
        }
    } else {
        let Some(dir) = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .filter(|arg| !arg.to_string_lossy().starts_with("--"))
        else {
            println!("usage: carousel_comment <dir-of-slide-pngs> | --link <url>");
            return Ok(());
        };
        let mut slides: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
            .collect();
        slides.sort();
        if slides.is_empty() {
            anyhow::bail!("không có ảnh .png nào trong {}", dir.display());
        }
        let frames: Vec<Vec<u8>> = slides.iter().map(std::fs::read).collect::<Result<_, _>>()?;
        for (index, path) in slides.iter().enumerate() {
            println!("slide {}: {}", index + 1, path.display());
        }
        (frames, None, None, None, EvidenceKind::CarouselSlides)
    };
    caption = prefer_forced_caption(caption, forced_caption.as_deref());
    let caption_is_authoritative = caption_is_authoritative(
        link.as_deref(),
        forced_caption.as_deref(),
        caption.as_deref(),
    );
    let brief = riviu_core::openai_client::PostBrief {
        caption: caption.as_deref(),
        caption_is_authoritative,
        transcript: transcript.as_deref(),
        coverage,
    };

    let data = dirs::data_dir()
        .map(|base| base.join("riviu-managers-phone"))
        .ok_or_else(|| anyhow::anyhow!("no data dir"))?;
    let db = Database::open(data.join("riviu.db"))?.with_secrets(Arc::new(KeyringSecrets {
        credentials: CredentialStore::system()?,
    }));
    let settings = db.get_nurture_settings()?;
    if settings.api_key.trim().is_empty() {
        anyhow::bail!("chưa có API key trong hồ sơ — bin này không tự nhập được");
    }
    println!(
        "model    {} @ {}\nngôn ngữ {}\n",
        settings.model, settings.base_url, settings.comment_lang
    );

    let ocr = app_lib::interaction_ocr::DesktopFrameTextSource;

    // `--new-only` skips it, for the runs that are measuring cost rather than comparing
    // evidence: the old path is a second API call whose numbers would land in the same
    // aggregate and quietly halve whatever average is being computed.
    let new_only = std::env::args().any(|arg| arg == "--new-only");
    // `--direction` exists because leaving it out flatters the numbers. The campaign builds a
    // direction per phone that quotes the comment before it, so in a real twenty-phone run every
    // draft prompt is different and none of them hit the prompt cache. Measuring with no
    // direction at all made twenty identical prompts and a cache hit on nineteen of them.
    let direction = std::env::args()
        .position(|arg| arg == "--direction")
        .and_then(|at| std::env::args().nth(at + 1));
    // `--batch N` asks for the whole fleet's comments the way a twenty-phone link would, in two
    // API calls rather than 2N. What it prints is what has to be judged: whether N comments come
    // back distinct, whether the gate still refuses what it used to refuse, and what the two
    // calls actually cost against the per-comment path printed above.
    let batch: Option<usize> = std::env::args()
        .position(|arg| arg == "--batch")
        .and_then(|at| std::env::args().nth(at + 1))
        .and_then(|value| value.parse().ok());

    // First, the old evidence, reproduced exactly: slide one on its own, described as moments.
    if !new_only {
        println!("--- BẰNG CHỨNG CŨ: chỉ ảnh 1 ---");
        match prepare_comment_for_frames(
        &settings,
        std::slice::from_ref(&frames[0]),
        EvidenceKind::Moments,
        None,
        &ocr,
        Default::default(),
    )
    .await
    {
        Ok((result, mode)) => println!(
            "[{mode}] {}
         caption thấy: {:?}
         evidence_support={} relevance={} prompt_tokens={} completion_tokens={} cost={} (gom ca hai request va ca retry)",
            result.text, result.caption, result.evidence_support, result.relevance, result.prompt_tokens,
            result.completion_tokens,
            result
                .cost_usd
                .map(|usd| format!("${usd:.6}"))
                .unwrap_or_else(|| "khong bao".into())
        ),
        Err(error) => println!("lỗi: {error:#}"),
    }
    }

    println!("\n--- BẰNG CHỨNG MỚI: {} ảnh ---", frames.len());
    match prepare_comment_for_frames(
        &settings,
        &frames,
        kind,
        direction.as_deref(),
        &ocr,
        brief,
    )
    .await
    {
        Ok((result, mode)) => println!(
            "[{mode}] {}
         caption thấy: {:?}
         evidence_support={} relevance={} prompt_tokens={} completion_tokens={} cost={} (gom ca hai request va ca retry)",
            result.text, result.caption, result.evidence_support, result.relevance, result.prompt_tokens,
            result.completion_tokens,
            result
                .cost_usd
                .map(|usd| format!("${usd:.6}"))
                .unwrap_or_else(|| "khong bao".into())
        ),
        Err(error) => println!("lỗi: {error:#}"),
    }
    if let Some(count) = batch {
        println!("\n--- GỘP: {count} câu trong hai lượt gọi ---");
        let started = std::time::Instant::now();
        let batched = prepare_grounded_comments_batch(
            &settings,
            &frames,
            kind,
            direction.as_deref(),
            count,
            brief,
        )
        .await;
        let mut total = 0.0f64;
        let mut reported = false;
        let mut texts = Vec::new();
        for (index, result) in batched.results.iter().enumerate() {
            match result {
                Ok(comment) => {
                    if let Some(usd) = comment.cost_usd {
                        total += usd;
                        reported = true;
                    }
                    println!(
                        "  {:>2}. {} {}  [ev={} rel={}]",
                        index + 1,
                        if batched.from_batch.get(index) == Some(&true) {
                            "[gop]"
                        } else {
                            "[LUI VE TUNG CAU]"
                        },
                        comment.text,
                        comment.evidence_support,
                        comment.relevance
                    );
                    texts.push(comment.text.clone());
                }
                Err(error) => println!("  {:>2}. LỖI: {error:#}", index + 1),
            }
        }
        let unique: std::collections::BTreeSet<&String> = texts.iter().collect();
        for refusal in &batched.refusals {
            println!("     cổng từ chối: {refusal}");
        }
        println!(
            "  => {} câu, {} khác nhau, {} từ gộp, {:.1}s, tổng {}",
            texts.len(),
            unique.len(),
            batched.accepted_from_batch(),
            started.elapsed().as_secs_f64(),
            if reported {
                format!(
                    "${total:.6} (${:.6}/câu)",
                    total / texts.len().max(1) as f64
                )
            } else {
                "khong bao gia".to_string()
            }
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{caption_is_authoritative, prefer_forced_caption};

    /// A fetched or explicitly supplied caption is fixture/source data. A bare directory has
    /// only pixels, and a link whose lookup found no caption must not manufacture authority.
    #[test]
    fn caption_authority_follows_the_source_and_requires_real_text() {
        assert!(caption_is_authoritative(
            Some("https://www.tiktok.com/@riviu/video/1"),
            None,
            Some("caption fetched from the post")
        ));
        assert!(caption_is_authoritative(
            None,
            Some("fixture caption"),
            Some("fixture caption")
        ));
        assert!(caption_is_authoritative(
            Some("https://www.tiktok.com/@riviu/video/1"),
            Some("fixture overrides fetched text"),
            Some("fixture overrides fetched text")
        ));
        assert!(!caption_is_authoritative(None, None, None));
        assert!(!caption_is_authoritative(
            None,
            None,
            Some("caption OCR cục bộ")
        ));
        assert!(!caption_is_authoritative(
            Some("https://www.tiktok.com/@riviu/video/1"),
            None,
            None
        ));
        assert!(!caption_is_authoritative(None, Some("   "), Some("   ")));
    }

    #[test]
    fn an_empty_fixture_does_not_erase_an_authoritative_web_caption() {
        assert_eq!(
            prefer_forced_caption(Some("caption from web".into()), Some("   ")),
            Some("caption from web".into())
        );
        assert_eq!(
            prefer_forced_caption(Some("caption from web".into()), Some("fixture caption")),
            Some("fixture caption".into())
        );
    }
}
