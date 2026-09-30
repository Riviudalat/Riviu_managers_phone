//! Immutable identity required before a new publication crosses its effect boundary.
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};

pub const VERIFICATION_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishVerificationBuild {
    pub udid: String,
    pub package: String,
    pub version: String,
    pub locale: String,
}

pub fn builds_from_preflight(
    report: &crate::PublishPreflightReport,
) -> Vec<PublishVerificationBuild> {
    report
        .assignments
        .iter()
        .filter_map(|row| {
            Some(PublishVerificationBuild {
                udid: row.udid.clone(),
                package: row.package_name.clone()?,
                version: row.version.clone()?,
                locale: row.locale.clone()?,
            })
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishSubmissionProof {
    pub verification_contract_version: u32,
    pub expected_account: String,
    pub submitted_at: String,
    pub package: String,
    pub version: String,
    pub locale: String,
    pub caption_sha256: String,
    pub bundle_id: String,
    pub media_kind: crate::PublishMediaKind,
}

pub fn normalize_publish_account(account: &str) -> anyhow::Result<String> {
    let normalized = account.trim().trim_start_matches('@').to_ascii_lowercase();
    ensure!(
        !normalized.is_empty()
            && normalized.len() <= 24
            && !normalized.ends_with('.')
            && normalized
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_')),
        "Tài khoản TikTok chưa có định danh hợp lệ"
    );
    Ok(normalized)
}

/// Strict canonical identity shared by publishing, recovery and the Sheet outbox.
pub fn canonical_publication_author(link: &str) -> anyhow::Result<(crate::SocialNetwork, String)> {
    let url = url::Url::parse(link)?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443),
        "Invalid publication URL"
    );
    if matches!(url.host_str(), Some("threads.com" | "www.threads.com")) {
        let author = url
            .path_segments()
            .and_then(|mut s| s.next())
            .context("Threads author missing")?;
        let author = crate::threads_publish::normalize_threads_account(
            author.strip_prefix('@').context("Threads author missing")?,
        )?;
        ensure!(
            crate::threads_publish::canonical_post_permalink(link, &author)? == link,
            "Threads URL must be canonical"
        );
        return Ok((crate::SocialNetwork::Threads, author));
    }
    let targets = crate::interaction::parse_tiktok_links(link);
    let target = targets
        .first()
        .and_then(|t| t.target.as_ref())
        .context("Canonical publication identity missing")?;
    ensure!(
        targets.len() == 1 && target.normalized_url == link,
        "Publication URL must be canonical"
    );
    Ok((
        crate::SocialNetwork::TikTok,
        target.author.to_ascii_lowercase(),
    ))
}

impl PublishSubmissionProof {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            self.verification_contract_version == VERIFICATION_CONTRACT_VERSION,
            "Cần kiểm tra lại lượt đăng theo cơ chế xác minh hiện tại"
        );
        if self.package == crate::threads_publish::ANDROID_PACKAGE {
            crate::threads_publish::normalize_threads_account(&self.expected_account)?;
        } else {
            normalize_publish_account(&self.expected_account)?;
        }
        chrono::DateTime::parse_from_rfc3339(&self.submitted_at)?;
        ensure!(
            !self.bundle_id.trim().is_empty(),
            "Thiếu định danh nội dung trước Đăng"
        );
        ensure!(
            self.caption_sha256.len() == 64
                && self.caption_sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "Thiếu dấu kiểm chứng caption trước Đăng"
        );
        if self.package == crate::threads_publish::ANDROID_PACKAGE {
            ensure!(
                crate::threads_publish::measured_mobile_build(
                    &self.package,
                    &self.version,
                    &self.locale
                ),
                "Threads build/locale chưa được đo"
            );
        } else {
            crate::tiktok_share::PublishVerificationPlan::for_build(
                &self.package,
                &self.locale,
                &self.version,
            )?;
        }
        Ok(())
    }
}
