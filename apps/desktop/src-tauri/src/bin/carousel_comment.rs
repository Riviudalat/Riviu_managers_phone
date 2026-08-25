//! What the AI writes about a photo post, from slide one alone versus from every slide.
//!
//! ```text
//! cargo run -p riviu-managers-phone --bin carousel_comment -- <dir-of-slide-pngs>
//! ```
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
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        println!("usage: carousel_comment <dir-of-slide-pngs>");
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
        EvidenceKind::CarouselSlides,
        direction.as_deref(),
        &ocr,
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
            EvidenceKind::CarouselSlides,
            direction.as_deref(),
            count,
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
