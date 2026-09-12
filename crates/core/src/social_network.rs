//! Which social app a campaign targets.
//!
//! Distinct from [`crate::DevicePlatform`] (iOS vs Android). Today only TikTok is
//! implemented; Instagram and Threads are reserved so configs and call sites can
//! name a network without renaming every `tiktok_*` module.

use serde::{Deserialize, Serialize};

use crate::interaction::{parse_tiktok_links, TikTokLinkLine};
use crate::DeviceDriver;

/// Social app a nurture / interaction / publish campaign is aimed at.
///
/// Serde uses `snake_case` (`tiktok`, `instagram`, `threads`). Missing values
/// default to [`SocialNetwork::TikTok`] so older JSON keeps working.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SocialNetwork {
    /// Renamed explicitly: serde's snake_case would emit `tik_tok`.
    #[default]
    #[serde(rename = "tiktok")]
    TikTok,
    Instagram,
    Threads,
}

impl SocialNetwork {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TikTok => "tiktok",
            Self::Instagram => "instagram",
            Self::Threads => "threads",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::TikTok => "TikTok",
            Self::Instagram => "Instagram",
            Self::Threads => "Threads",
        }
    }

    pub fn is_implemented(self) -> bool {
        matches!(self, Self::TikTok)
    }

    fn refuse_unimplemented(self) -> anyhow::Error {
        anyhow::anyhow!("{} chưa được hỗ trợ; hiện chỉ TikTok", self.display_name())
    }
}

/// Resolve the foreground/package id for `network` on `udid`.
///
/// TikTok keeps the existing [`DeviceDriver::resolve_tiktok_package`] path.
/// Instagram and Threads refuse until real locators exist.
pub async fn resolve_app_package(
    network: SocialNetwork,
    driver: &dyn DeviceDriver,
    udid: &str,
) -> anyhow::Result<String> {
    match network {
        SocialNetwork::TikTok => driver.resolve_tiktok_package(udid).await,
        SocialNetwork::Instagram | SocialNetwork::Threads => Err(network.refuse_unimplemented()),
    }
}

/// Parse target links for `network`.
///
/// TikTok reuses [`parse_tiktok_links`]. Other networks refuse clearly.
pub fn parse_network_links(
    network: SocialNetwork,
    raw: &str,
) -> anyhow::Result<Vec<TikTokLinkLine>> {
    match network {
        SocialNetwork::TikTok => Ok(parse_tiktok_links(raw)),
        SocialNetwork::Instagram | SocialNetwork::Threads => Err(network.refuse_unimplemented()),
    }
}

/// Application-neutral boundary; the old TikTok-specific wire shape remains readable.
pub fn parse_app_targets(
    network: SocialNetwork,
    raw: &str,
) -> anyhow::Result<Vec<crate::app_automation::AppTarget>> {
    let lines = parse_network_links(network, raw)?;
    let mut targets = Vec::new();
    for line in lines {
        let target = line
            .target
            .ok_or_else(|| anyhow::anyhow!("Liên kết dòng {} chưa hợp lệ", line.line_no))?;
        targets.push(crate::app_automation::AppTarget::from(&target));
    }
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize, Serialize)]
    struct NetworkHolder {
        #[serde(default)]
        network: SocialNetwork,
    }

    #[test]
    fn serde_defaults_to_tiktok_and_uses_snake_case() {
        assert_eq!(
            serde_json::from_str::<SocialNetwork>("\"tiktok\"").unwrap(),
            SocialNetwork::TikTok
        );
        assert_eq!(
            serde_json::from_str::<NetworkHolder>("{}").unwrap().network,
            SocialNetwork::TikTok
        );
        assert_eq!(
            serde_json::to_string(&SocialNetwork::Instagram).unwrap(),
            "\"instagram\""
        );
        assert_eq!(SocialNetwork::default(), SocialNetwork::TikTok);
        assert!(!SocialNetwork::Instagram.is_implemented());
        assert!(!SocialNetwork::Threads.is_implemented());
        assert!(SocialNetwork::TikTok.is_implemented());
    }

    #[test]
    fn parse_network_links_keeps_tiktok_and_refuses_others() {
        let lines = parse_network_links(
            SocialNetwork::TikTok,
            "https://www.tiktok.com/@a/video/1234567890123456789",
        )
        .expect("tiktok ok");
        assert_eq!(lines.len(), 1);
        assert!(lines[0].target.is_some());

        let err = parse_network_links(SocialNetwork::Instagram, "https://instagram.com/p/x")
            .expect_err("instagram refused");
        assert!(err.to_string().contains("Instagram"));
        let err = parse_network_links(SocialNetwork::Threads, "https://threads.net/@a")
            .expect_err("threads refused");
        assert!(err.to_string().contains("Threads"));
    }

    #[test]
    fn refuse_message_is_stable_for_package_seam() {
        let message = SocialNetwork::Instagram.refuse_unimplemented().to_string();
        assert!(message.contains("Instagram"));
        assert!(message.contains("TikTok"));
        let message = SocialNetwork::Threads.refuse_unimplemented().to_string();
        assert!(message.contains("Threads"));
    }
}
