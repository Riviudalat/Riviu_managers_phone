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
use riviu_core::openai_client::{prepare_comment_for_frames, EvidenceKind};
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

    // First, the old evidence, reproduced exactly: slide one on its own, described as moments.
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
         evidence_support={} relevance={}",
            result.text, result.caption, result.evidence_support, result.relevance
        ),
        Err(error) => println!("lỗi: {error:#}"),
    }

    println!("\n--- BẰNG CHỨNG MỚI: {} ảnh ---", frames.len());
    match prepare_comment_for_frames(&settings, &frames, EvidenceKind::CarouselSlides, None, &ocr)
        .await
    {
        Ok((result, mode)) => println!(
            "[{mode}] {}
         caption thấy: {:?}
         evidence_support={} relevance={}",
            result.text, result.caption, result.evidence_support, result.relevance
        ),
        Err(error) => println!("lỗi: {error:#}"),
    }
    Ok(())
}
