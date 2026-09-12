//! A paged projection of durable Publish facts for the optional internal Sheet.
//! Reading a report never claims Post, changes its state, or creates an outbox row.
use super::*;
use crate::publish_sheet::{InternalReportMetadata, InternalSheetReportRow, SheetDeliverySettings};

fn object(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or(serde_json::Value::Null)
}

fn account(value: Option<&str>) -> Option<String> {
    let handle = value?.trim().trim_start_matches('@');
    (!handle.is_empty()
        && handle.len() <= 24
        && !handle.ends_with('.')
        && handle
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.')))
    .then(|| format!("@{handle}"))
}

struct Facts {
    assignment_id: String,
    udid: String,
    state: String,
    error: Option<String>,
    intent: Option<String>,
    evidence: Option<String>,
    revision: i64,
    campaign_revision: i64,
    campaign_state: String,
    request: String,
    manifest: String,
}

fn project(f: Facts) -> anyhow::Result<InternalSheetReportRow> {
    let intent = f.intent.as_deref().map(object).unwrap_or_default();
    let evidence = f.evidence.as_deref().map(object).unwrap_or_default();
    let post = evidence.get("post").unwrap_or(&evidence);
    let request: crate::PublishCampaignRequest = serde_json::from_str(&f.request)?;
    let manifest: crate::PublishBundle = serde_json::from_str(&f.manifest)?;
    let verified_url = post["postUrl"].as_str().filter(|url| {
        post["publicationVerified"].as_bool() == Some(true)
            && url::Url::parse(url).is_ok_and(|parsed| {
                parsed.scheme() == "https"
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && parsed.port_or_known_default() == Some(443)
            })
            && crate::interaction::parse_tiktok_links(url)
                .first()
                .and_then(|target| target.target.as_ref())
                .is_some_and(|target| target.normalized_url == *url)
    });
    let verified = f.state == "succeeded" && verified_url.is_some();
    let sent = matches!(
        intent["effectIntent"].as_str(),
        Some("post" | "post_carousel")
    );
    let status = if verified {
        "Đã xác minh"
    } else if f.state == "uncertain" || f.state == "succeeded" {
        "Cần kiểm tra"
    } else if sent && matches!(f.state.as_str(), "posting" | "verifying") {
        "Đã gửi"
    } else if sent {
        "Cần kiểm tra"
    } else {
        "Chưa đăng"
    };
    let device = request.target_snapshot.as_ref().and_then(|snapshot| {
        snapshot
            .included
            .iter()
            .find(|device| device.udid == f.udid)
    });
    let machine = match device {
        Some(device) => match (device.number, device.alias.trim()) {
            (Some(number), "") => format!("Máy {number}"),
            (Some(number), alias) => format!("Máy {number} — {alias}"),
            (None, "") => f.udid.clone(),
            (None, alias) => alias.into(),
        },
        None => f.udid.clone(),
    };
    let tiktok_account = account(intent["expectedAccount"].as_str())
        .or_else(|| {
            verified_url
                .and_then(|url| {
                    crate::interaction::parse_tiktok_links(url)
                        .into_iter()
                        .next()
                })
                .and_then(|link| link.target)
                .and_then(|target| account(Some(&target.author)))
        })
        .unwrap_or_default();
    let posted_at = if sent {
        intent["submittedAt"]
            .as_str()
            .filter(|at| chrono::DateTime::parse_from_rfc3339(at).is_ok())
            .map(str::to_owned)
    } else {
        None
    };
    let note = if f.state == "cancelled" || (f.campaign_state == "cancelled" && !sent && !verified)
    {
        "Đã hủy".to_owned()
    } else if verified {
        String::new()
    } else {
        let raw = evidence["verificationStatus"]["reason"]
            .as_str()
            .or_else(|| post["linkCaptureReason"].as_str())
            .or_else(|| evidence["reason"].as_str())
            .or_else(|| evidence["message"].as_str())
            .or(f.error.as_deref())
            .unwrap_or("");
        if evidence["verificationStatus"]["state"] == "needsReview"
            && !raw.to_ascii_lowercase().contains("tự kiểm tra đã dừng")
            && !raw.is_empty()
        {
            format!("{raw} · Tự kiểm tra đã dừng — chọn Kiểm tra liên kết")
        } else if evidence["verificationStatus"]["state"] == "needsReview" && raw.is_empty() {
            "Tự kiểm tra đã dừng — chọn Kiểm tra liên kết".into()
        } else {
            raw.into()
        }
    };
    let state_notes = note
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(512)
        .collect();
    let row_revision = f
        .revision
        .checked_add(f.campaign_revision)
        .context("internal report revision overflow")?;
    anyhow::ensure!(
        (0..=9_007_199_254_740_991).contains(&row_revision),
        "internal report revision range"
    );
    Ok(InternalSheetReportRow {
        row_kind: "internalReport".into(),
        metadata: InternalReportMetadata {
            report_version: 1,
            row_revision,
            machine,
            tiktok_account,
            status: status.into(),
            state_notes,
        },
        assignment_id: f.assignment_id,
        post_url: if verified {
            verified_url.unwrap_or("").into()
        } else {
            String::new()
        },
        poster: "bot".into(),
        partners: manifest.partners,
        posted_at,
    })
}

impl Database {
    /// Read credentials and the opt-in bit in one SQL statement/snapshot.
    pub fn publish_sheet_delivery_settings(&self) -> anyhow::Result<SheetDeliverySettings> {
        if let Some(secrets) = &self.secrets {
            if let Some(raw) = secrets.get_secret("publish-sheet-connection")? {
                let value: serde_json::Value = serde_json::from_str(&raw)?;
                return Ok(SheetDeliverySettings {
                    webhook_url: value["webhookUrl"].as_str().unwrap_or_default().into(),
                    token: value["token"].as_str().unwrap_or_default().into(),
                    internal_reporting: value["internalReporting"].as_bool().unwrap_or(false),
                });
            }
        }
        self.conn()?.query_row(
            "SELECT COALESCE((SELECT value FROM settings WHERE key=?1),''),COALESCE((SELECT value FROM settings WHERE key=?2),''),COALESCE((SELECT value FROM settings WHERE key=?3),'false')",
            params![crate::publish_sheet::WEBHOOK_URL_SETTING,crate::publish_sheet::WEBHOOK_TOKEN_SETTING,crate::publish_sheet::INTERNAL_REPORTING_SETTING],
            |row|Ok(SheetDeliverySettings {webhook_url:row.get(0)?,token:row.get(1)?,internal_reporting:row.get::<_,String>(2)?=="true"})
        ).map_err(Into::into)
    }

    /// Keyset pagination visits assignments beyond the first1000 without offset starvation.
    pub fn internal_publish_report_page(
        &self,
        after_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<InternalSheetReportRow>> {
        Ok(self.internal_publish_report_batch(after_id, limit)?.rows)
    }

    pub fn internal_publish_report_batch(
        &self,
        after_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<InternalPublishReportPage> {
        self.internal_publish_reports(after_id, None, limit)
    }

    pub fn internal_publish_report(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<InternalSheetReportRow>> {
        let page = self.internal_publish_reports(None, Some(assignment_id), 1)?;
        if let Some((_, reason)) = page.errors.first() {
            anyhow::bail!("{reason}");
        }
        Ok(page.rows.into_iter().next())
    }

    /// New delivery workers only enumerate campaigns which explicitly opted into v2.
    /// Historical report reads remain available to the UI, without being replayed.
    pub fn bound_internal_publish_report(
        &self,
        assignment_id: &str,
    ) -> anyhow::Result<Option<InternalSheetReportRow>> {
        let conn = self.conn()?;
        internal_report_on(&conn, assignment_id)
    }

    fn internal_publish_reports(
        &self,
        after_id: Option<&str>,
        assignment_id: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<InternalPublishReportPage> {
        let conn = self.conn()?;
        let mut statement=conn.prepare(
            "SELECT a.id,a.udid,a.state,a.error_code,a.effect_intent,a.evidence_json,a.revision,c.revision,c.state,c.request_json,b.manifest_json
             FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id JOIN publish_bundles b ON b.id=a.bundle_id
             WHERE (?1 IS NULL OR a.id>?1) AND (?2 IS NULL OR a.id=?2) AND json_valid(c.request_json) AND COALESCE(json_extract(c.request_json,'$.sheetEnabled'),1)=1
             ORDER BY a.id LIMIT ?3"
        )?;
        let facts = statement
            .query_map(
                params![
                    after_id,
                    assignment_id,
                    crate::publish_sheet::sweep_limit(limit)
                ],
                |row| {
                    Ok(Facts {
                        assignment_id: row.get(0)?,
                        udid: row.get(1)?,
                        state: row.get(2)?,
                        error: row.get(3)?,
                        intent: row.get(4)?,
                        evidence: row.get(5)?,
                        revision: row.get(6)?,
                        campaign_revision: row.get(7)?,
                        campaign_state: row.get(8)?,
                        request: row.get(9)?,
                        manifest: row.get(10)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let mut page = InternalPublishReportPage {
            next_cursor: facts.last().map(|row| row.assignment_id.clone()),
            has_more: facts.len() == crate::publish_sheet::sweep_limit(limit) as usize,
            rows: vec![],
            errors: vec![],
        };
        for fact in facts {
            let id = fact.assignment_id.clone();
            match project(fact) {
                Ok(row) => page.rows.push(row),
                Err(error) => page
                    .errors
                    .push((id, error.to_string().chars().take(200).collect())),
            }
        }
        Ok(page)
    }
}

pub(super) fn internal_report_on(
    conn: &Connection,
    assignment_id: &str,
) -> anyhow::Result<Option<InternalSheetReportRow>> {
    let facts = conn.query_row(
        "SELECT a.id,a.udid,a.state,a.error_code,a.effect_intent,a.evidence_json,a.revision,c.revision,c.state,c.request_json,b.manifest_json
         FROM publish_assignments a JOIN publish_campaigns c ON c.id=a.campaign_id JOIN publish_bundles b ON b.id=a.bundle_id
         WHERE a.id=?1 AND json_valid(c.request_json)
           AND json_extract(c.request_json,'$.sheetDelivery.version')=2
           AND json_extract(c.request_json,'$.sheetEnabled')=1",
        [assignment_id],
        |row| Ok(Facts {
            assignment_id:row.get(0)?,udid:row.get(1)?,state:row.get(2)?,error:row.get(3)?,
            intent:row.get(4)?,evidence:row.get(5)?,revision:row.get(6)?,campaign_revision:row.get(7)?,
            campaign_state:row.get(8)?,request:row.get(9)?,manifest:row.get(10)?,
        }),
    ).optional()?;
    facts.map(project).transpose()
}

pub struct InternalPublishReportPage {
    pub rows: Vec<InternalSheetReportRow>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub errors: Vec<(String, String)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(sheet: bool) -> (Database, PathBuf, String, String) {
        let path = std::env::temp_dir().join(format!("internal-sheet-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let bundle = crate::PublishBundle {
            id: "report-bundle".into(),
            source_path: "/fixture".into(),
            name: "fixture".into(),
            media_kind: crate::PublishMediaKind::Image,
            images: vec![],
            video: None,
            caption_path: "/fixture/caption.txt".into(),
            caption: "fixture".into(),
            caption_sha256: "a".repeat(64),
            total_bytes: 0,
            partners: vec!["Partner B".into(), "Partner A".into()],
        };
        let request = crate::PublishCampaignRequest {
            sheet_delivery: None,
            verification_contract_version: None,
            verification_builds: vec![],
            sheet_enabled: sheet,
            request_id: Uuid::new_v4().to_string(),
            source_root: "/fixture".into(),
            bundle_ids: vec![bundle.id.clone()],
            udids: vec!["serial-fixed".into()],
            run_at: None,
            visibility: crate::PublishVisibility::Public,
            cleanup_policy: crate::PublishCleanupPolicy::KeepImportedAssets,
            network: crate::SocialNetwork::TikTok,
            sound_policy: crate::PublishSoundPolicy::Default,
            execution_confirmed: false,
            target_snapshot: Some(crate::ResolvedTargetSnapshot {
                target_ref: crate::TargetRef::Explicit {
                    udids: vec!["serial-fixed".into()],
                },
                included: vec![crate::ResolvedTargetDevice {
                    udid: "serial-fixed".into(),
                    alias: "Phone at creation".into(),
                    number: Some(17),
                }],
                excluded: vec![],
                roster_sha256: "a".repeat(64),
            }),
        };
        let campaign = db.create_publish_campaign(&request, &[bundle]).unwrap().id;
        let assignment = db
            .get_publish_campaign(&campaign)
            .unwrap()
            .unwrap()
            .assignments[0]
            .id
            .clone();
        (db, path, campaign, assignment)
    }

    #[test]
    fn internal_report_tracks_truth_without_changing_post_or_outbox() {
        let (db, path, campaign, id) = fixture(true);
        let initial = db.internal_publish_report(&id).unwrap().unwrap();
        assert_eq!(initial.metadata.status, "Chưa đăng");
        assert_eq!(initial.post_url, "");
        assert_eq!(initial.posted_at, None);
        assert_eq!(initial.metadata.tiktok_account, "");
        assert_eq!(initial.metadata.machine, "Máy 17 — Phone at creation");
        assert_eq!(initial.partners, vec!["Partner B", "Partner A"]);
        db.update_publish_campaign_state(&campaign, crate::PublishCampaignState::Posting, None)
            .unwrap();
        db.update_publish_assignment_state(&id, crate::PublishCampaignState::Imported, None, None)
            .unwrap();
        let intent=serde_json::json!({"effectIntent":"post","submittedAt":"2026-09-09T10:00:00Z","expectedAccount":"fixture.account"}).to_string();
        assert!(db
            .claim_publish_assignment_for_posting(&id, &intent)
            .unwrap());
        let sent = db.internal_publish_report(&id).unwrap().unwrap();
        assert_eq!(sent.metadata.status, "Đã gửi");
        assert_eq!(sent.metadata.tiktok_account, "@fixture.account");
        assert_eq!(sent.posted_at.as_deref(), Some("2026-09-09T10:00:00Z"));
        assert!(sent.metadata.row_revision > initial.metadata.row_revision);
        db.update_publish_assignment_state(
            &id,
            crate::PublishCampaignState::Uncertain,
            Some("post_verification_needs_review"),
            Some(r#"{"verificationStatus":{"reason":"Observed draft may belong to another run"}}"#),
        )
        .unwrap();
        let review = db.internal_publish_report(&id).unwrap().unwrap();
        assert_eq!(review.metadata.status, "Cần kiểm tra");
        assert!(review.metadata.state_notes.contains("may belong"));
        assert_eq!(review.post_url, "");
        assert!(db.pending_publish_sheet_rows(10).unwrap().is_empty());
        assert!(!db
            .claim_publish_assignment_for_posting(&id, &intent)
            .unwrap());
        let url = "https://www.tiktok.com/@fixture.account/photo/123";
        db.update_publish_assignment_state(
            &id,
            crate::PublishCampaignState::Succeeded,
            None,
            Some(&serde_json::json!({"postUrl":url,"publicationVerified":true}).to_string()),
        )
        .unwrap();
        let proved = db.internal_publish_report(&id).unwrap().unwrap();
        assert_eq!(proved.metadata.status, "Đã xác minh");
        assert_eq!(proved.post_url, url);
        assert!(db.pending_publish_sheet_rows(10).unwrap().is_empty());
        drop(db);
        let db = Database::open(&path).unwrap();
        assert_eq!(db.internal_publish_report(&id).unwrap().unwrap(), proved);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn internal_report_never_calls_historical_success_verified_and_keeps_snapshot_fallback() {
        let (db, path, campaign, id) = fixture(true);
        let mut request = db.publish_campaign_request(&campaign).unwrap().unwrap();
        request.target_snapshot = None;
        db.conn()
            .unwrap()
            .execute(
                "UPDATE publish_campaigns SET request_json=?1 WHERE id=?2",
                params![serde_json::to_string(&request).unwrap(), campaign],
            )
            .unwrap();
        db.update_publish_assignment_state(
            &id,
            crate::PublishCampaignState::Succeeded,
            None,
            Some(r#"{"postUrl":"https://www.tiktok.com/@unproved/photo/123"}"#),
        )
        .unwrap();
        let report = db.internal_publish_report(&id).unwrap().unwrap();
        assert_eq!(report.metadata.status, "Cần kiểm tra");
        assert_eq!(report.metadata.machine, "serial-fixed");
        assert_eq!(report.metadata.tiktok_account, "");
        assert_eq!(report.post_url, "");
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn internal_report_campaign_cancel_advances_revision_and_disabled_campaigns_stay_out() {
        let (db, path, campaign, id) = fixture(true);
        let before = db.internal_publish_report(&id).unwrap().unwrap();
        db.update_publish_campaign_state(&campaign, crate::PublishCampaignState::Cancelled, None)
            .unwrap();
        let after = db.internal_publish_report(&id).unwrap().unwrap();
        assert!(after.metadata.row_revision > before.metadata.row_revision);
        assert_eq!(after.metadata.status, "Chưa đăng");
        assert_eq!(after.metadata.state_notes, "Đã hủy");
        let conn = db.conn().unwrap();
        let mut req = db.publish_campaign_request(&campaign).unwrap().unwrap();
        req.sheet_enabled = false;
        conn.execute(
            "UPDATE publish_campaigns SET request_json=?1 WHERE id=?2",
            params![serde_json::to_string(&req).unwrap(), campaign],
        )
        .unwrap();
        assert!(db
            .internal_publish_report_page(None, 50)
            .unwrap()
            .is_empty());
        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn internal_report_keyset_pagination_reaches_beyond_one_thousand() {
        let (db, path, campaign, id) = fixture(true);
        let conn = db.conn().unwrap();
        conn.execute("DELETE FROM publish_assignments WHERE id=?1", [id])
            .unwrap();
        for n in 0..1003 {
            let bundle = format!("bundle-{n:04}");
            conn.execute("INSERT INTO publish_bundles(id,campaign_id,ordinal,name,source_path,caption,caption_sha256,manifest_json,created_at) SELECT ?1,campaign_id,?2,name,source_path,caption,caption_sha256,manifest_json,created_at FROM publish_bundles WHERE id='report-bundle'",params![bundle,n+1]).unwrap();
            conn.execute("INSERT INTO publish_assignments(id,campaign_id,bundle_id,ordinal,udid,state,revision,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,'queued',0,'now','now')",params![format!("a-{n:04}"),campaign,bundle,n,format!("s-{n}")]).unwrap();
        }
        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let page = db
                .internal_publish_report_page(cursor.as_deref(), 137)
                .unwrap();
            if page.is_empty() {
                break;
            }
            cursor = page.last().map(|row| row.assignment_id.clone());
            ids.extend(page.into_iter().map(|row| row.assignment_id));
        }
        assert_eq!(ids.len(), 1003);
        assert_eq!(ids.last().unwrap(), "a-1002");
        conn.execute(
            "UPDATE publish_bundles SET manifest_json='broken' WHERE id='bundle-0000'",
            [],
        )
        .unwrap();
        let bad = db.internal_publish_report_batch(None, 1).unwrap();
        assert!(bad.rows.is_empty());
        assert_eq!(bad.errors.len(), 1);
        assert_eq!(bad.next_cursor.as_deref(), Some("a-0000"));
        let next = db
            .internal_publish_report_batch(bad.next_cursor.as_deref(), 1)
            .unwrap();
        assert_eq!(next.rows[0].assignment_id, "a-0001");
        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn internal_report_config_opt_in_is_atomic_and_omitted_value_is_preserved() {
        let (db, path, _, _) = fixture(true);
        assert!(
            !db.publish_sheet_delivery_settings()
                .unwrap()
                .internal_reporting
        );
        db.set_publish_sheet_config_with_reporting(
            "https://example.com/a",
            Some("first"),
            Some(true),
        )
        .unwrap();
        db.set_publish_sheet_config("https://example.com/a", None)
            .unwrap();
        assert!(
            db.publish_sheet_delivery_settings()
                .unwrap()
                .internal_reporting
        );
        let conn = db.conn().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_internal_setting BEFORE UPDATE ON settings WHEN NEW.key='publish_sheet_internal_reporting' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(db
            .set_publish_sheet_config_with_reporting(
                "https://example.com/b",
                Some("second"),
                Some(false)
            )
            .is_err());
        let config = db.publish_sheet_delivery_settings().unwrap();
        assert_eq!(config.webhook_url, "https://example.com/a");
        assert_eq!(config.token, "first");
        assert!(config.internal_reporting);
        drop(conn);
        drop(db);
        std::fs::remove_file(path).unwrap();
    }
}
