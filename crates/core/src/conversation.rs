//! Reviewed, per-post conversations and deterministic interleaving.
use anyhow::{ensure, Context};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStep {
    pub id: String,
    pub topic: String,
    pub speaker_id: String,
    pub text: String,
    pub parent_step_id: Option<String>,
    #[serde(default)]
    pub mention_role_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetConversation {
    pub target_key: String,
    pub steps: Vec<ConversationStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRole {
    pub role_id: String,
    pub udid: String,
    pub username: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptedConversation {
    pub schema_version: u8,
    pub duration_minutes: u32,
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub seed: u64,
    pub target_scripts: Vec<TargetConversation>,
    pub role_bindings: Vec<ConversationRole>,
}

/// Markdown speaker labels are syntax; only a leading @recipient is interpreted as a tag.
pub fn parse_conversation(raw: &str) -> anyhow::Result<Vec<ConversationStep>> {
    ensure!(raw.len() <= 256 * 1024, "Kịch bản quá dài");
    let mut topic = "Hội thoại".to_owned();
    let mut steps: Vec<ConversationStep> = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim().trim_matches('*').trim();
        if line.is_empty() || line.chars().all(|c| matches!(c, '-' | '_' | ' ')) {
            continue;
        }
        let clean = line.trim_start_matches(['*', '-', ' ', '#']);
        if !clean.starts_with('@') {
            topic = clean.trim_matches('*').trim().to_owned();
            continue;
        }
        let (speaker, body) = clean
            .split_once(':')
            .with_context(|| format!("Dòng {} thiếu dấu : sau người nói", index + 1))?;
        let speaker =
            crate::publish_submission::normalize_publish_account(speaker.trim_matches('*'))?;
        let mut body = body.trim_start_matches(['*', ' ']);
        let mut mentions = Vec::new();
        while let Some(rest) = body.strip_prefix('@') {
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'))
                .unwrap_or(rest.len());
            ensure!(end > 0, "Tag chưa có tên ở dòng {}", index + 1);
            mentions.push(crate::publish_submission::normalize_publish_account(
                &rest[..end],
            )?);
            body = rest[end..].trim_start_matches(['*', ' ', ':', ',']);
        }
        ensure!(
            !body.trim().is_empty(),
            "Dòng {} chưa có nội dung",
            index + 1
        );
        let parent = mentions
            .first()
            .and_then(|role| {
                steps
                    .iter()
                    .rev()
                    .find(|step| step.topic == topic && step.speaker_id == *role)
            })
            .map(|step| step.id.clone());
        steps.push(ConversationStep {
            id: format!("step-{}", steps.len() + 1),
            topic: topic.clone(),
            speaker_id: speaker,
            text: body.trim().to_owned(),
            parent_step_id: parent,
            mention_role_ids: mentions,
        });
    }
    ensure!(
        !steps.is_empty() && steps.len() <= 64,
        "Mỗi bài cần từ 1 đến 64 câu"
    );
    Ok(steps)
}

impl ScriptedConversation {
    pub fn total_steps(&self) -> usize {
        self.target_scripts
            .iter()
            .map(|target| target.steps.len())
            .sum()
    }
    pub fn step(&self, target: &str, ordinal: u8) -> Option<&ConversationStep> {
        self.target_scripts
            .iter()
            .find(|script| script.target_key == target)?
            .steps
            .get(usize::from(ordinal))
    }
    pub fn role(&self, id: &str) -> Option<&ConversationRole> {
        self.role_bindings.iter().find(|role| role.role_id == id)
    }
    pub fn duration_ms(&self) -> anyhow::Result<i64> {
        match (&self.starts_at, &self.ends_at) {
            (Some(start), Some(end)) => Ok((chrono::DateTime::parse_from_rfc3339(end)?
                - chrono::DateTime::parse_from_rfc3339(start)?)
            .num_milliseconds()),
            (None, None) => Ok(i64::from(self.duration_minutes) * 60_000),
            _ => anyhow::bail!("Khung giờ cần đủ giờ bắt đầu và kết thúc"),
        }
    }
    pub fn validate(
        &self,
        targets: &[crate::ResolvedTikTokTarget],
        actors: &[String],
    ) -> anyhow::Result<()> {
        ensure!(
            self.schema_version == 1,
            "Phiên bản kịch bản chưa được hỗ trợ"
        );
        let duration = self.duration_ms()?;
        ensure!(
            (60_000..=86_400_000).contains(&duration),
            "Phiên tương tác từ 1 phút đến 24 giờ"
        );
        ensure!(
            self.total_steps() as i64 * 120_000 <= duration * 9 / 10,
            "Chưa đủ thời lượng: {} câu cần tối thiểu {} phút, gồm dự phòng",
            self.total_steps(),
            (self.total_steps() as i64 * 120_000 * 10 / 9 + 59_999) / 60_000
        );
        let expected: HashSet<_> = targets.iter().map(|t| t.target_key.as_str()).collect();
        let actual: HashSet<_> = self
            .target_scripts
            .iter()
            .map(|s| s.target_key.as_str())
            .collect();
        ensure!(
            actual == expected && actual.len() == self.target_scripts.len(),
            "Mỗi link cần đúng một kịch bản, không thiếu hoặc trùng"
        );
        let mut roles = HashSet::new();
        let mut devices = HashSet::new();
        let mut usernames = HashSet::new();
        for role in &self.role_bindings {
            ensure!(
                !role.role_id.is_empty() && roles.insert(role.role_id.as_str()),
                "Vai bị trùng hoặc trống"
            );
            ensure!(
                actors.contains(&role.udid) && devices.insert(role.udid.as_str()),
                "Mỗi vai cần một máy riêng trong phạm vi đã chọn"
            );
            let handle = crate::publish_submission::normalize_publish_account(&role.username)?;
            ensure!(usernames.insert(handle), "Tài khoản được gán cho nhiều vai");
        }
        for script in &self.target_scripts {
            ensure!(
                !script.steps.is_empty() && script.steps.len() <= 64,
                "Mỗi bài cần 1–64 câu"
            );
            let mut earlier = HashMap::new();
            for step in &script.steps {
                ensure!(
                    !step.id.is_empty() && !earlier.contains_key(&step.id),
                    "ID câu trùng hoặc trống"
                );
                ensure!(
                    roles.contains(step.speaker_id.as_str()),
                    "Vai {} chưa được gán máy",
                    step.speaker_id
                );
                ensure!(
                    !step.text.trim().is_empty() && step.text.chars().count() <= 2200,
                    "Nội dung câu trống hoặc quá 2200 ký tự"
                );
                for mention in &step.mention_role_ids {
                    ensure!(
                        roles.contains(mention.as_str()),
                        "Vai được tag {mention} chưa được gán máy"
                    );
                }
                if let Some(parent) = &step.parent_step_id {
                    let parent: &&ConversationStep = earlier
                        .get(parent)
                        .context("Câu trả lời phải trỏ câu trước đó trong cùng chủ đề")?;
                    ensure!(parent.topic == step.topic, "Parent thuộc chủ đề khác");
                }
                earlier.insert(&step.id, step);
            }
        }
        Ok(())
    }
    pub fn compile(
        &self,
        request: &crate::ThreadCampaignRequest,
    ) -> anyhow::Result<crate::ThreadPlan> {
        self.validate(&request.targets, &request.actor_udids)?;
        let mut assignments = Vec::new();
        for target in &request.targets {
            let script = self
                .target_scripts
                .iter()
                .find(|s| s.target_key == target.target_key)
                .context("Kịch bản thiếu link")?;
            for (ordinal, step) in script.steps.iter().enumerate() {
                assignments.push(crate::ThreadMessagePlan {
                    target_key: target.target_key.clone(),
                    ordinal: ordinal as u8,
                    actor_udid: self
                        .role(&step.speaker_id)
                        .context("Vai chưa gán")?
                        .udid
                        .clone(),
                    parent_ordinal: step
                        .parent_step_id
                        .as_ref()
                        .and_then(|id| script.steps.iter().position(|s| s.id == *id))
                        .map(|i| i as u8),
                    cohort: 0,
                });
            }
        }
        Ok(crate::ThreadPlan {
            request_id: request.request_id.clone(),
            assignments,
        })
    }
    pub fn mentions(&self, target: &str, ordinal: u8) -> Vec<String> {
        let Some(step) = self.step(target, ordinal) else {
            return vec![];
        };
        let mut roles = step.mention_role_ids.clone();
        if let Some(parent) = step.parent_step_id.as_ref().and_then(|id| {
            self.target_scripts
                .iter()
                .find(|s| s.target_key == target)?
                .steps
                .iter()
                .find(|s| s.id == *id)
        }) {
            if !roles.contains(&parent.speaker_id) {
                roles.push(parent.speaker_id.clone());
            }
        }
        roles
            .iter()
            .filter_map(|id| self.role(id))
            .map(|role| role.username.trim_start_matches('@').to_owned())
            .collect()
    }
    /// One turn from each target per round. Persisted offsets never depend on HashMap order.
    pub fn timeline(
        &self,
        starts: i64,
        estimate_ms: i64,
    ) -> anyhow::Result<Vec<(String, u8, i64)>> {
        let duration = self.duration_ms()?;
        let total = self.total_steps();
        ensure!(
            total > 0 && total as i64 * estimate_ms <= duration * 9 / 10,
            "Thời gian thực thi dự kiến vượt khung giờ"
        );
        let slack = duration * 9 / 10 - total as i64 * estimate_ms;
        let weights: Vec<i64> = (0..total)
            .map(|index| {
                ((self
                    .seed
                    .wrapping_add((index as u64).wrapping_mul(6364136223846793005)))
                    % 401) as i64
                    + 800
            })
            .collect();
        let weight_sum: i64 = weights.iter().sum();
        let mut index = 0usize;
        let mut elapsed = 0;
        let mut result = Vec::new();
        for ordinal in 0..64 {
            for script in &self.target_scripts {
                if ordinal >= script.steps.len() {
                    continue;
                }
                result.push((script.target_key.clone(), ordinal as u8, starts + elapsed));
                elapsed += estimate_ms + slack * weights[index] / weight_sum;
                index += 1;
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (ScriptedConversation, Vec<crate::ResolvedTikTokTarget>) {
        let targets = crate::parse_tiktok_links(
            "https://www.tiktok.com/@a/video/123\nhttps://www.tiktok.com/@b/video/456",
        )
        .into_iter()
        .map(|line| line.target.unwrap())
        .collect::<Vec<_>>();
        let steps=parse_conversation("**Nhánh: Hỏi địa chỉ**\n* **@a:** Chỗ này ở đâu?\n* **@b:** *@a* Có địa chỉ trong bài nhé\n* **@a:** *@b* Cảm ơn bạn\n* **@c:** *@d* Cuối tuần đi không?\n* **@d:** *@c* Để xem lịch nhé").unwrap();
        let script = ScriptedConversation {
            schema_version: 1,
            duration_minutes: 120,
            starts_at: None,
            ends_at: None,
            seed: 9,
            target_scripts: targets
                .iter()
                .map(|t| TargetConversation {
                    target_key: t.target_key.clone(),
                    steps: steps.clone(),
                })
                .collect(),
            role_bindings: ["a", "b", "c", "d"]
                .iter()
                .map(|r| ConversationRole {
                    role_id: (*r).into(),
                    udid: format!("phone-{r}"),
                    username: (*r).into(),
                })
                .collect(),
        };
        (script, targets)
    }
    #[test]
    fn markdown_keeps_repeated_speakers_and_independent_roots() {
        let (script, targets) = fixture();
        script
            .validate(
                &targets,
                &script
                    .role_bindings
                    .iter()
                    .map(|r| r.udid.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let steps = &script.target_scripts[0].steps;
        assert_eq!(steps[2].speaker_id, "a");
        assert_eq!(steps[2].parent_step_id.as_deref(), Some("step-2"));
        assert!(steps[3].parent_step_id.is_none());
        assert_eq!(steps[4].parent_step_id.as_deref(), Some("step-4"));
        assert_eq!(script.mentions(&targets[0].target_key, 2), vec!["b"]);
    }
    #[test]
    fn duration_plan_alternates_posts_without_exceeding_window_or_rerolling() {
        let (script, targets) = fixture();
        let timeline = script.timeline(1000, 120_000).unwrap();
        assert_eq!(timeline.len(), 10);
        assert_eq!(timeline[0].0, targets[0].target_key);
        assert_eq!(timeline[1].0, targets[1].target_key);
        assert_eq!(timeline[2].1, 1);
        assert_eq!(timeline, script.timeline(1000, 120_000).unwrap());
        assert!(timeline
            .windows(2)
            .all(|pair| pair[1].2 >= pair[0].2 + 120_000));
        assert!(
            timeline.last().unwrap().2 + 120_000 <= 1000 + script.duration_ms().unwrap() * 9 / 10
        );
    }
    #[test]
    fn invalid_graph_missing_target_duplicate_actor_and_short_window_are_rejected() {
        let (script, targets) = fixture();
        let actors = script
            .role_bindings
            .iter()
            .map(|r| r.udid.clone())
            .collect::<Vec<_>>();
        let mut bad = script.clone();
        bad.target_scripts[0].steps[0].parent_step_id = Some("step-2".into());
        assert!(bad.validate(&targets, &actors).is_err());
        let mut bad = script.clone();
        bad.target_scripts.pop();
        assert!(bad.validate(&targets, &actors).is_err());
        let mut bad = script.clone();
        bad.role_bindings[1].udid = bad.role_bindings[0].udid.clone();
        assert!(bad.validate(&targets, &actors).is_err());
        let mut bad = script;
        bad.duration_minutes = 5;
        assert!(bad.validate(&targets, &actors).is_err());
    }
}
