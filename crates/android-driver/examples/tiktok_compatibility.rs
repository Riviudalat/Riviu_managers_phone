//! Read installed TikTok build support without opening a session or changing a phone.
use riviu_android_driver::AndroidDriver;

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let serials: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(!serials.is_empty(), "pass explicit device serials");
    let driver = AndroidDriver::new(&common::repo_config())?;
    let mut rows = Vec::new();
    for serial in serials {
        let row = match driver.tiktok_build(&serial).await {
            Ok((package, version, locale)) => {
                let labels = riviu_core::tiktok_labels::controls_for(&package, &locale, &version);
                serde_json::json!({
                    "serial": serial, "package":package, "version":version, "locale":locale,
                    "carousel":labels.as_ref().is_some_and(|labels|riviu_core::tiktok_composer::ComposerPlan::missing_for_carousel(labels).is_empty()),
                    "sound":riviu_core::tiktok_sound::SoundPickerPlan::resolve(&package,&locale,&version).is_some(),
                    "video":riviu_core::tiktok_composer::VideoPickerPlan::resolve(&package,&locale,&version).is_some(),
                    "commentSend":labels.as_ref().is_some_and(|labels|labels.label(riviu_core::tiktok_labels::TikTokControl::CommentSend).is_some()),
                    "accountRead":labels.is_some_and(riviu_core::tiktok_account::account_read_supported),
                    "postLink":labels.is_some_and(|labels|labels.label(riviu_core::tiktok_labels::TikTokControl::ProfileTab).is_some()&&labels.post_tile_id().is_some()),
                })
            }
            Err(error) => serde_json::json!({"serial":serial,"error":format!("{error:#}")}),
        };
        rows.push(row);
    }
    println!("{}", serde_json::to_string_pretty(&rows)?);
    Ok(())
}
