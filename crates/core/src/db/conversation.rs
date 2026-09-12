use super::*;

#[derive(Debug, thiserror::Error)]
#[error("Phiên hội thoại đã có worker đang chạy")]
pub struct ConversationAlreadyRunning;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSession {
    pub started_at_ms: i64,
    pub ends_at_ms: i64,
    pub next_at_ms: i64,
    pub cursor: usize,
}

impl Database {
    pub fn claim_conversation_session(
        &self,
        campaign: &str,
        script: &crate::conversation::ScriptedConversation,
        token: &str,
        now: i64,
    ) -> anyhow::Result<ConversationSession> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let start = script
            .starts_at
            .as_deref()
            .map(chrono::DateTime::parse_from_rfc3339)
            .transpose()?
            .map_or(now, |at| at.timestamp_millis());
        let end = start + script.duration_ms()?;
        tx.execute("INSERT OR IGNORE INTO interaction_conversation_sessions(campaign_id,started_at_ms,ends_at_ms,next_at_ms,cursor) VALUES(?1,?2,?3,?2,0)",params![campaign,start,end])?;
        let changed=tx.execute("UPDATE interaction_conversation_sessions SET owner=?2 WHERE campaign_id=?1 AND owner IS NULL",params![campaign,token])?;
        if changed != 1 {
            return Err(ConversationAlreadyRunning.into());
        }
        let session=tx.query_row("SELECT started_at_ms,ends_at_ms,next_at_ms,cursor FROM interaction_conversation_sessions WHERE campaign_id=?1",[campaign],|r|Ok(ConversationSession { started_at_ms:r.get(0)?,ends_at_ms:r.get(1)?,next_at_ms:r.get(2)?,cursor:r.get::<_,i64>(3)? as usize }))?;
        tx.commit()?;
        Ok(session)
    }
    pub fn conversation_session(
        &self,
        campaign: &str,
    ) -> anyhow::Result<Option<ConversationSession>> {
        Ok(self.conn()?.query_row("SELECT started_at_ms,ends_at_ms,next_at_ms,cursor FROM interaction_conversation_sessions WHERE campaign_id=?1",[campaign],|r|Ok(ConversationSession {started_at_ms:r.get(0)?,ends_at_ms:r.get(1)?,next_at_ms:r.get(2)?,cursor:r.get::<_,i64>(3)? as usize})).optional()?)
    }
    pub fn release_conversation_session(&self, campaign: &str, token: &str) -> anyhow::Result<()> {
        self.conn()?.execute("UPDATE interaction_conversation_sessions SET owner=NULL WHERE campaign_id=?1 AND owner=?2",params![campaign,token])?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn finish_conversation_turn(
        &self,
        campaign: &str,
        token: &str,
        assignment: &str,
        cursor: usize,
        next: i64,
        started: i64,
        finished: i64,
        tuple: &str,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        anyhow::ensure!(tx.execute("UPDATE interaction_conversation_sessions SET cursor=?3,next_at_ms=?4 WHERE campaign_id=?1 AND owner=?2",params![campaign,token,cursor as i64,next])?==1,"Worker hội thoại đã đổi");
        tx.execute("INSERT INTO interaction_conversation_turns(assignment_id,started_at_ms,finished_at_ms,tuple) VALUES(?1,?2,?3,?4) ON CONFLICT(assignment_id) DO UPDATE SET started_at_ms=?2,finished_at_ms=?3,tuple=?4",params![assignment,started,finished,tuple])?;
        tx.commit()?;
        Ok(())
    }
    pub fn conversation_turn_estimate(&self, tuple: &str) -> anyhow::Result<i64> {
        let conn = self.conn()?;
        let mut stmt=conn.prepare("SELECT MAX(0,t.finished_at_ms-t.started_at_ms) FROM interaction_conversation_turns t JOIN interaction_assignments a ON a.id=t.assignment_id WHERE t.tuple=?1 AND a.state='succeeded' ORDER BY t.finished_at_ms DESC LIMIT 20")?;
        let mut durations = stmt
            .query_map([tuple], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if durations.len() < 5 {
            return Ok(120_000);
        }
        durations.sort_unstable();
        Ok(durations[(durations.len() * 9).div_ceil(10).saturating_sub(1)].max(120_000))
    }
}

pub(super) fn check_conversation_send_deadline(
    conn: &Connection,
    assignment: &str,
) -> anyhow::Result<()> {
    let session:Option<(i64,Option<String>)>=conn.query_row("SELECT s.ends_at_ms,s.owner FROM interaction_conversation_sessions s JOIN interaction_assignments a ON a.campaign_id=s.campaign_id WHERE a.id=?1",[assignment],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((end, owner)) = session {
        anyhow::ensure!(
            Utc::now().timestamp_millis() < end && owner.is_some(),
            "Phiên đã hết giờ hoặc dừng; không gửi câu mới"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        Database,
        PathBuf,
        String,
        crate::conversation::ScriptedConversation,
    ) {
        let path = std::env::temp_dir().join(format!("conversation-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let target = crate::parse_tiktok_links("https://www.tiktok.com/@a/video/123")
            .remove(0)
            .target
            .unwrap();
        let script = crate::conversation::ScriptedConversation {
            schema_version: 1,
            duration_minutes: 120,
            starts_at: None,
            ends_at: None,
            seed: 1,
            target_scripts: vec![crate::conversation::TargetConversation {
                target_key: target.target_key.clone(),
                steps: crate::conversation::parse_conversation("@a: Ở đâu?\n@b: @a Có trong bài")
                    .unwrap(),
            }],
            role_bindings: vec![
                crate::conversation::ConversationRole {
                    role_id: "a".into(),
                    udid: "phone-a".into(),
                    username: "a".into(),
                },
                crate::conversation::ConversationRole {
                    role_id: "b".into(),
                    udid: "phone-b".into(),
                    username: "b".into(),
                },
            ],
        };
        let request = crate::ThreadCampaignRequest {
            scripted_conversation: Some(script.clone()),
            request_id: Uuid::new_v4().to_string(),
            targets: vec![target],
            actor_udids: vec!["phone-a".into(), "phone-b".into()],
            message_count: 2,
            instruction: String::new(),
            max_words: 12,
            mode: crate::ThreadMode::Threaded,
            shape: crate::ThreadShape::Chain,
            cohort_size: None,
            manual_comments: vec![],
            actions: crate::InteractionActionSet {
                like: false,
                save: false,
                comment: true,
            },
            mentions: vec![],
            mention_parent: true,
        };
        let plan = crate::plan_threads(&request).unwrap();
        let id = db.create_interaction_campaign(&request, &plan).unwrap();
        (db, path, id, script)
    }
    #[test]
    fn session_claim_rejects_second_coordinator_and_restores_fixed_window() {
        let (db, path, id, script) = fixture();
        let start = Utc::now().timestamp_millis();
        let one = db
            .claim_conversation_session(&id, &script, "owner", start)
            .unwrap();
        assert_eq!(one.ends_at_ms, start + 7_200_000);
        assert!(db
            .claim_conversation_session(&id, &script, "other", start)
            .is_err());
        db.release_conversation_session(&id, "wrong").unwrap();
        assert!(db
            .claim_conversation_session(&id, &script, "other", start)
            .is_err());
        db.release_conversation_session(&id, "owner").unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let resumed = db
            .claim_conversation_session(&id, &script, "second", start + 60_000)
            .unwrap();
        assert_eq!(resumed.started_at_ms, start);
        assert_eq!(resumed.ends_at_ms, one.ends_at_ms);
        let assignment = db
            .get_interaction_campaign(&id)
            .unwrap()
            .unwrap()
            .assignments[0]
            .id
            .clone();
        assert!(check_conversation_send_deadline(&db.conn().unwrap(), &assignment).is_ok());
        db.conn()
            .unwrap()
            .execute(
                "UPDATE interaction_conversation_sessions SET ends_at_ms=0 WHERE campaign_id=?1",
                [&id],
            )
            .unwrap();
        assert!(check_conversation_send_deadline(&db.conn().unwrap(), &assignment).is_err());
    }
    #[test]
    fn stale_turn_cannot_move_cursor_and_current_turn_survives_restart() {
        let (db, path, id, script) = fixture();
        let at = Utc::now().timestamp_millis();
        db.claim_conversation_session(&id, &script, "first", at)
            .unwrap();
        let assignment = db
            .get_interaction_campaign(&id)
            .unwrap()
            .unwrap()
            .assignments[0]
            .id
            .clone();
        assert!(db
            .finish_conversation_turn(
                &id,
                "other",
                &assignment,
                1,
                at + 90000,
                at,
                at + 30000,
                "tuple/root"
            )
            .is_err());
        assert_eq!(db.conversation_session(&id).unwrap().unwrap().cursor, 0);
        db.finish_conversation_turn(
            &id,
            "first",
            &assignment,
            1,
            at + 90000,
            at,
            at + 30000,
            "tuple/root",
        )
        .unwrap();
        db.release_conversation_session(&id, "first").unwrap();
        drop(db);
        let db = Database::open(&path).unwrap();
        let resumed = db
            .claim_conversation_session(&id, &script, "next", at + 60000)
            .unwrap();
        assert_eq!(resumed.cursor, 1);
        assert_eq!(resumed.next_at_ms, at + 90000);
    }
}
