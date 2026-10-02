//! Offline source fingerprint minting only; no driver, network or credential access.
use anyhow::{ensure, Context};
use std::{path::PathBuf, io::Write};
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(args.len() == 7, "no_public_scope SOURCE SERIAL PACKAGE ACCOUNT OUTPUT SEED");
    let source = PathBuf::from(&args[1]).canonicalize()?;
    let manifest = riviu_core::scan_publish_folder(&source, Default::default())?;
    ensure!(manifest.bundles.len() == 1, "exactly one approved fixture bundle required");
    let bundle = manifest.bundles.into_iter().next().context("fixture bundle missing")?;
    ensure!(bundle.video.is_none() && !bundle.caption.trim().is_empty(), "image draft required");
    let digest = riviu_core::frame_sha256(&serde_json::to_vec(&bundle)?);
    let value = serde_json::json!({"activationId":uuid::Uuid::new_v4().to_string(),"deviceScopes":[{"udid":args[2],"package":args[3],"expectedAccount":args[4],"targetUrl":null,"draftText":null,"sourceRoot":source,"bundleId":bundle.id,"publishFingerprint":digest,"soundPolicy":{"kind":"trendingAny","poolSize":1,"seed":args[6].parse::<u64>()?},"helperCanary":false}]});
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&args[5])?;
    file.write_all(&serde_json::to_vec_pretty(&value)?)?; file.sync_all()?;
    println!("offline scope minted with source fingerprint; no device effects");
    Ok(())
}
