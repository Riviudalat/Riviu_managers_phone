//! The outbox between a published carousel and the operator's partner sheet.
//!
//! # Two deliveries, and only one of them is the post
//!
//! Every published carousel owes a row to a Google Sheet: its link in column D, `bot` as
//! the poster, and the partner names out of the campaign's workbook from column K onward.
//! The obvious shape is an HTTP call at the end of the post step, and it is wrong in a way
//! that costs real money: a network error would then make a **published** post read as a
//! failed one — and a failed post is exactly what an operator retries, which on Android
//! publishes a second carousel that nothing here can take down.
//!
//! So the row lands in the database first and travels afterwards. The two failures stay
//! separate all the way through: [`crate::publish::PublishCampaignState`] says whether the
//! carousel is on the account, and `state` here says whether the sheet knows about it.
//! Retrying the second never re-runs the first.

use super::*;

/// One row owed to the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SheetOutboxRow {
    pub assignment_id: String,
    pub campaign_id: String,
    pub post_url: String,
    pub poster: String,
    /// Partner names in workbook order — the order they are written across columns K+.
    pub partners: Vec<String>,
    pub attempts: u32,
    pub last_error: Option<String>,
}

impl Database {
    /// Record that a published post owes the sheet a row.
    ///
    /// Called in the same breath as the post's own success, and **never** in a way that can
    /// fail the post: the caller treats an error here as a logged problem, not as a failed
    /// carousel.
    ///
    /// Re-queuing the same assignment replaces the pending row rather than adding one.
    /// `assignment_id` is the primary key precisely so this cannot produce two rows for one
    /// post — the operator would see the same link twice in column D with no way to tell
    /// which to remove. An already-`sent` row is left alone: it is not owed again.
    pub fn queue_publish_sheet_row(
        &self,
        assignment_id: &str,
        campaign_id: &str,
        post_url: &str,
        poster: &str,
        partners: &[String],
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO publish_sheet_outbox(
                 assignment_id,campaign_id,post_url,poster,partners_json,state,
                 attempts,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,'pending',0,?6,?6)
             ON CONFLICT(assignment_id) DO UPDATE SET
                 post_url=excluded.post_url,
                 poster=excluded.poster,
                 partners_json=excluded.partners_json,
                 state='pending',
                 last_error=NULL,
                 updated_at=excluded.updated_at
             WHERE publish_sheet_outbox.state <> 'sent'",
            params![
                assignment_id,
                campaign_id,
                post_url,
                poster,
                serde_json::to_string(partners)?,
                now
            ],
        )?;
        Ok(())
    }

    /// The rows still owed to the sheet, oldest first.
    ///
    /// Includes `failed` as well as `pending`, which is the point of keeping them apart from
    /// `sent`: a push that failed is still owed, and the only thing `failed` adds is a
    /// message an operator can read. There is no attempt ceiling here — a row that stops
    /// being retried is a link nobody ever pastes, and the sheet is the operator's record of
    /// what went out.
    pub fn pending_publish_sheet_rows(&self, limit: usize) -> anyhow::Result<Vec<SheetOutboxRow>> {
        let conn = self.conn()?;
        let mut statement = conn.prepare(
            "SELECT assignment_id,campaign_id,post_url,poster,partners_json,attempts,last_error
             FROM publish_sheet_outbox
             WHERE state <> 'sent'
             ORDER BY created_at ASC
             LIMIT ?1",
        )?;
        let rows = statement
            .query_map(params![limit as i64], |row| {
                let partners: String = row.get(4)?;
                Ok(SheetOutboxRow {
                    assignment_id: row.get(0)?,
                    campaign_id: row.get(1)?,
                    post_url: row.get(2)?,
                    poster: row.get(3)?,
                    // A row whose JSON cannot be parsed still travels, with no partner
                    // names, rather than stopping the queue behind it: the link in column D
                    // is the part the operator cannot reconstruct.
                    partners: serde_json::from_str(&partners).unwrap_or_default(),
                    attempts: row.get::<_, i64>(5)?.max(0) as u32,
                    last_error: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The sheet has the row. Terminal — nothing re-sends a `sent` row.
    pub fn mark_publish_sheet_sent(&self, assignment_id: &str) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE publish_sheet_outbox
             SET state='sent',attempts=attempts+1,last_error=NULL,updated_at=?2
             WHERE assignment_id=?1",
            params![assignment_id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// The push failed. Still owed; the message is for a person to read.
    ///
    /// Refuses to move a row that is already `sent`. Without that guard a late error from a
    /// retry of an already-delivered row would reopen it, and the next sweep would paste the
    /// same link into column D a second time.
    pub fn mark_publish_sheet_failed(
        &self,
        assignment_id: &str,
        error: &str,
    ) -> anyhow::Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE publish_sheet_outbox
             SET state='failed',attempts=attempts+1,last_error=?3,updated_at=?2
             WHERE assignment_id=?1 AND state <> 'sent'",
            params![assignment_id, Utc::now().to_rfc3339(), error],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::publish::{
        PublishBundle, PublishCampaignRequest, PublishCleanupPolicy, PublishImage,
        PublishMediaKind, PublishVisibility,
    };
    use std::path::PathBuf;

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-sheet-outbox-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn bundle(id: &str) -> PublishBundle {
        PublishBundle {
            id: id.into(),
            source_path: format!("/fixture/{id}"),
            name: id.into(),
            media_kind: PublishMediaKind::Image,
            images: vec![PublishImage {
                path: format!("/fixture/{id}/01.png"),
                file_name: "01.png".into(),
                order: 1,
                sha256: "a".repeat(64),
                byte_len: 3,
                width: 1,
                height: 1,
            }],
            caption_path: format!("/fixture/{id}/caption.txt"),
            caption: "caption".into(),
            caption_sha256: "b".repeat(64),
            total_bytes: 3,
        }
    }

    /// One campaign with one assignment, and that assignment's id.
    fn seed(db: &Database) -> (String, String) {
        let request = PublishCampaignRequest {
            request_id: Uuid::new_v4().to_string(),
            source_root: "/fixture/root".into(),
            bundle_ids: vec!["bundle-a".into()],
            udids: vec!["phone-a".into()],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        };
        let campaign = db
            .create_publish_campaign(&request, &[bundle("bundle-a")])
            .expect("create campaign");
        // `create_publish_campaign` hands back the *plan* — bundle, udid, ordinal — while the
        // row ids are assigned by the insert, so the id has to be read back.
        let assignment = db
            .get_publish_campaign(&campaign.id)
            .expect("read back")
            .expect("campaign exists")
            .assignments
            .into_iter()
            .next()
            .expect("one assignment")
            .id;
        (campaign.id, assignment)
    }

    /// A queued row comes back with the names in the order the workbook had them.
    #[test]
    fn a_queued_row_keeps_the_partner_order_the_workbook_had() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        let partners = vec!["Quán B".to_string(), "Quán A".to_string()];
        db.queue_publish_sheet_row(
            &assignment,
            &campaign,
            "https://www.tiktok.com/@a/photo/1",
            "bot",
            &partners,
        )
        .expect("queue");

        let rows = db.pending_publish_sheet_rows(10).expect("read back");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].post_url, "https://www.tiktok.com/@a/photo/1");
        assert_eq!(rows[0].poster, "bot");
        // Not sorted: the names go across columns K, L, M… in this order, so reordering
        // them here would silently rearrange the operator's sheet.
        assert_eq!(rows[0].partners, partners);
        assert_eq!(rows[0].attempts, 0);
        let _ = std::fs::remove_file(path);
    }

    /// **A row the sheet already has is never re-opened, by any route.**
    ///
    /// The one property this table exists for. Column D is pasted by hand nowhere — every
    /// link in it came from here — so a second write puts the same link on two rows and
    /// there is nothing in the sheet that says which is the duplicate. Three routes could
    /// reopen it and all three are closed: re-queuing the assignment, a late failure from a
    /// retry, and the pending sweep picking it up again.
    #[test]
    fn a_row_the_sheet_already_has_is_never_sent_a_second_time() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        db.mark_publish_sheet_sent(&assignment).expect("sent");
        assert!(
            db.pending_publish_sheet_rows(10).expect("read").is_empty(),
            "a sent row must not come back out of the queue"
        );

        // Route 1: the post path runs again and re-queues.
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/2", "bot", &[])
            .expect("re-queue");
        assert!(
            db.pending_publish_sheet_rows(10).expect("read").is_empty(),
            "re-queuing an already-sent assignment reopened it"
        );

        // Route 2: a late error arrives for a push that had in fact landed.
        db.mark_publish_sheet_failed(&assignment, "timeout")
            .expect("late failure");
        assert!(
            db.pending_publish_sheet_rows(10).expect("read").is_empty(),
            "a late failure reopened a row the sheet already has"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A failed push is still owed, and carries a message a person can read.
    #[test]
    fn a_failed_push_stays_in_the_queue_with_its_reason() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        db.mark_publish_sheet_failed(&assignment, "webhook trả 500")
            .expect("failed");

        let rows = db.pending_publish_sheet_rows(10).expect("read");
        assert_eq!(rows.len(), 1, "a failed push is still owed to the sheet");
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].last_error.as_deref(), Some("webhook trả 500"));

        // And it can still succeed afterwards.
        db.mark_publish_sheet_sent(&assignment).expect("sent");
        assert!(db.pending_publish_sheet_rows(10).expect("read").is_empty());
        let _ = std::fs::remove_file(path);
    }

    /// **One assignment can owe the sheet at most one row, whatever the caller does.**
    ///
    /// Enforced by the primary key rather than by callers remembering. A second row for the
    /// same post is the failure mode the operator cannot untangle: two links, one post, and
    /// nothing in the sheet saying which to delete.
    #[test]
    fn re_queuing_replaces_the_pending_row_rather_than_adding_one() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &["A".into()])
            .expect("queue");
        db.mark_publish_sheet_failed(&assignment, "mạng đứt")
            .expect("failed");
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/2", "bot", &["B".into()])
            .expect("re-queue");

        let rows = db.pending_publish_sheet_rows(10).expect("read");
        assert_eq!(rows.len(), 1, "a second row was created for one post");
        assert_eq!(rows[0].post_url, "https://a/2", "the newer link wins");
        assert_eq!(rows[0].partners, vec!["B".to_string()]);
        assert!(
            rows[0].last_error.is_none(),
            "re-queuing must clear the stale reason, or an operator reads an old failure"
        );
        let _ = std::fs::remove_file(path);
    }

    /// A row whose JSON is unreadable still travels, carrying the link.
    ///
    /// The link is the part nobody can reconstruct; the partner names are in a file on
    /// disk. Refusing the whole row over its names would hold the queue behind it and lose
    /// the one thing that matters.
    #[test]
    fn a_row_with_unreadable_partners_still_carries_its_link() {
        let (db, path) = fixture();
        let (campaign, assignment) = seed(&db);
        db.queue_publish_sheet_row(&assignment, &campaign, "https://a/1", "bot", &[])
            .expect("queue");
        db.conn()
            .expect("conn")
            .execute(
                "UPDATE publish_sheet_outbox SET partners_json='not json' WHERE assignment_id=?1",
                params![&assignment],
            )
            .expect("corrupt the column");

        let rows = db.pending_publish_sheet_rows(10).expect("read");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].post_url, "https://a/1");
        assert!(rows[0].partners.is_empty());
        let _ = std::fs::remove_file(path);
    }
}
