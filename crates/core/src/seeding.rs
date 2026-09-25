//! Frozen allocation for mixed comments and bounded desired-state actions.
use crate::interaction::{ThreadCampaignRequest, ThreadMessagePlan, ThreadPlan};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecondsRange {
    pub min: u16,
    pub max: u16,
}
impl SecondsRange {
    pub fn sample(&self, seed: u64, key: &str) -> u64 {
        u64::from(self.min) + stable(seed, key) % (u64::from(self.max - self.min) + 1)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedingConfig {
    pub standalone_count: u8,
    pub like_count: u8,
    pub save_count: u8,
    pub share_count: u8,
    pub seed: u64,
    /// Preferred actors are tried first; every candidate remains inside actor_udids.
    #[serde(default)]
    pub preferred_actors: Vec<String>,
    #[serde(default)]
    pub expected_accounts: BTreeMap<String, String>,
    pub watch_seconds: SecondsRange,
    pub comment_gap_seconds: SecondsRange,
    /// Reviewed text, keyed by target, in comment ordinal order (excluding action-only rows).
    #[serde(default)]
    pub comments: BTreeMap<String, Vec<String>>,
}

pub fn stable(seed: u64, key: &str) -> u64 {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(seed.to_le_bytes());
    h.update(key.as_bytes());
    u64::from_le_bytes(h.finalize()[..8].try_into().expect("eight digest bytes"))
}

impl SeedingConfig {
    pub fn actions_for(
        &self,
        request: &ThreadCampaignRequest,
        ordinal: u8,
    ) -> crate::interaction::InteractionActionSet {
        let comment = self.is_comment(request, ordinal);
        crate::interaction::InteractionActionSet {
            like: !comment && self.like_count > 0,
            save: !comment && self.save_count > 0,
            share: !comment && self.share_count > 0,
            follow: false,
            comment,
        }
    }
    pub fn validate(&self, request: &ThreadCampaignRequest) -> anyhow::Result<()> {
        anyhow::ensure!(
            request.scripted_conversation.is_none(),
            "Chọn kế hoạch seeding hoặc kịch bản nhập, không dùng cả hai"
        );
        let n = request.actor_udids.len();
        let mut accounts = std::collections::BTreeSet::new();
        for actor in &request.actor_udids {
            if let Some(handle) = self.expected_accounts.get(actor) {
                anyhow::ensure!(
                    accounts.insert(handle.trim().trim_start_matches('@').to_lowercase()),
                    "Hai máy đang dùng cùng tài khoản; cần tài khoản khác nhau"
                );
            }
        }
        anyhow::ensure!(
            request.actions.like == (self.like_count > 0)
                && request.actions.save == (self.save_count > 0)
                && request.actions.share == (self.share_count > 0),
            "Hành động không khớp ngân sách seeding"
        );
        anyhow::ensure!(
            !request.actions.follow,
            "Theo dõi chưa thuộc kế hoạch seeding này"
        );
        anyhow::ensure!(
            [self.like_count, self.save_count, self.share_count]
                .iter()
                .all(|v| usize::from(*v) <= n),
            "Số hành động không được vượt số máy đã chọn"
        );
        anyhow::ensure!(
            self.preferred_actors
                .iter()
                .all(|a| request.actor_udids.contains(a)),
            "Máy chỉ định ngoài phạm vi"
        );
        for range in [&self.watch_seconds, &self.comment_gap_seconds] {
            anyhow::ensure!(
                range.min <= range.max && range.max <= 600,
                "Khoảng chờ phải từ 0–600 giây và min không vượt max"
            );
        }
        if request.actions.comment {
            anyhow::ensure!(
                (1..=64).contains(&request.message_count)
                    && self.standalone_count <= request.message_count,
                "Tổng bình luận từ 1–64, bình luận đơn không vượt tổng"
            );
            let threaded = request.message_count - self.standalone_count;
            anyhow::ensure!(
                threaded != 1 && (threaded == 0 || n >= 2),
                "Hội thoại cần ít nhất hai tài khoản và hai câu"
            );
        }
        for target in &request.targets {
            if let Some(texts) = self.comments.get(&target.target_key) {
                anyhow::ensure!(
                    texts.len() == usize::from(request.message_count)
                        && texts
                            .iter()
                            .all(|s| !s.trim().is_empty() && s.len() <= 4000),
                    "Nội dung đã duyệt không khớp tổng bình luận"
                );
            }
        }
        Ok(())
    }

    pub fn action_rows(&self, request: &ThreadCampaignRequest) -> usize {
        if self.like_count > 0 || self.save_count > 0 || self.share_count > 0 {
            request.actor_udids.len()
        } else {
            0
        }
    }

    pub fn is_comment(&self, request: &ThreadCampaignRequest, ordinal: u8) -> bool {
        request.actions.comment && usize::from(ordinal) >= self.action_rows(request)
    }

    pub fn plan(&self, request: &ThreadCampaignRequest) -> ThreadPlan {
        let mut assignments = Vec::new();
        for target in &request.targets {
            let mut actors = request.actor_udids.clone();
            actors.sort_by_key(|a| {
                (
                    self.preferred_actors
                        .iter()
                        .position(|v| v == a)
                        .unwrap_or(usize::MAX),
                    stable(self.seed, &format!("{}:{a}", target.target_key)),
                )
            });
            let offset = self.action_rows(request);
            for (i, actor) in actors.iter().take(offset).enumerate() {
                assignments.push(ThreadMessagePlan {
                    target_key: target.target_key.clone(),
                    ordinal: i as u8,
                    actor_udid: actor.clone(),
                    parent_ordinal: None,
                    cohort: 0,
                });
            }
            if !request.actions.comment {
                continue;
            }
            for i in 0..usize::from(self.standalone_count) {
                assignments.push(ThreadMessagePlan {
                    target_key: target.target_key.clone(),
                    ordinal: (offset + i) as u8,
                    actor_udid: actors[i % actors.len()].clone(),
                    parent_ordinal: None,
                    cohort: 0,
                });
            }
            let remaining = usize::from(request.message_count - self.standalone_count);
            if remaining == 0 {
                continue;
            }
            // At most two actors per available pair of sentences; every active group
            // gets a root and a reply and never exceeds four accounts.
            let active = actors.len().min(remaining);
            let sizes = cluster_sizes(active);
            let mut actor_at = 0;
            let mut ordinal = offset + usize::from(self.standalone_count);
            let extra = remaining - active;
            for (group, size) in sizes.iter().enumerate() {
                let count = size + extra / sizes.len() + usize::from(group < extra % sizes.len());
                for turn in 0..count {
                    assignments.push(ThreadMessagePlan {
                        target_key: target.target_key.clone(),
                        ordinal: ordinal as u8,
                        actor_udid: actors[actor_at + turn % size].clone(),
                        parent_ordinal: (turn > 0).then(|| (ordinal - 1) as u8),
                        cohort: (group + 1) as u16,
                    });
                    ordinal += 1;
                }
                actor_at += size;
            }
        }
        ThreadPlan {
            request_id: request.request_id.clone(),
            assignments,
        }
    }
}

pub fn cluster_sizes(count: usize) -> Vec<usize> {
    if count < 2 {
        return Vec::new();
    }
    let mut left = count;
    let mut sizes = Vec::new();
    while left > 4 {
        let size = if left == 5 { 3 } else { 4 };
        sizes.push(size);
        left -= size;
    }
    sizes.push(left);
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clusters_never_leave_one_account_or_exceed_four() {
        assert_eq!(cluster_sizes(5), vec![3, 2]);
        assert_eq!(cluster_sizes(9), vec![4, 3, 2]);
        for n in 2..=64 {
            let s = cluster_sizes(n);
            assert_eq!(s.iter().sum::<usize>(), n);
            assert!(s.iter().all(|v| (2..=4).contains(v)));
        }
    }
}
