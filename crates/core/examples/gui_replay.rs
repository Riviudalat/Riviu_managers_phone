//! Run compatibility fixtures without connecting any phone or invoking an AI provider.
use std::io::Read;
fn main() -> anyhow::Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    let pack = riviu_core::ui_automation::profile::CompatibilityPack::parse(&bytes)?;
    let count = pack.verify_fixtures()?;
    println!(
        "{}",
        serde_json::json!({"ok":true,"profile":pack.id,"revision":pack.revision,"fixtures":count,"deviceActions":0})
    );
    Ok(())
}
