use super::*;
use crate::ui_automation::{GuiRequest, GuiResponse};

impl Database {
    pub fn gui_protected_images(&self) -> anyhow::Result<std::collections::HashSet<String>> {
        let conn = self.conn()?;
        let mut statement=conn.prepare("SELECT DISTINCT screenshot_sha256 FROM gui_perception_attempts WHERE state IN ('started','unresolved','ambiguous')")?;
        let rows = statement
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    }
    pub fn recover_gui_requests(&self) -> anyhow::Result<usize> {
        Ok(self.conn()?.execute(r#"UPDATE gui_perception_attempts SET state='failed',result_json='{"reason":"gui_process_interrupted"}' WHERE state='started'"#,[])?)
    }
    pub fn claim_gui_request(&self, request: &GuiRequest, max: u32) -> anyhow::Result<()> {
        let scope = request.scope.as_ref();
        let run = scope.map_or(request.session_epoch.as_str(), |s| s.run_id.as_str());
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now = Utc::now().timestamp_millis();
        anyhow::ensure!(
            scope.and_then(|s| s.deadline_ms).is_none_or(|d| now < d),
            "gui_campaign_deadline"
        );
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM gui_perception_attempts WHERE run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        anyhow::ensure!(count < i64::from(max), "gui_request_budget_exhausted");
        tx.execute("INSERT INTO gui_perception_attempts(request_id,run_id,assignment_id,device_id,observation_id,session_epoch,started_at_ms,screenshot_sha256,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'started')",
            params![request.request_id,run,scope.and_then(|s|s.assignment_id.as_deref()),scope.map_or("",|s|s.device_id.as_str()),request.observation_id,request.session_epoch,now,request.screenshot_sha256])?;
        tx.commit()?;
        Ok(())
    }
    pub fn settle_gui_request(
        &self,
        request: &GuiRequest,
        response: Option<&GuiResponse>,
        reason: &str,
    ) -> anyhow::Result<()> {
        let state = match response.map(|r| &r.status) {
            Some(crate::ui_automation::ResolutionStatus::Resolved) => "resolved",
            Some(crate::ui_automation::ResolutionStatus::Ambiguous) => "ambiguous",
            Some(_) => "unresolved",
            None => "failed",
        };
        self.conn()?.execute("UPDATE gui_perception_attempts SET state=?2,result_json=?3 WHERE request_id=?1 AND state='started'",params![request.request_id,state,serde_json::json!({"response":response,"reason":reason}).to_string()])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_automation::{AppContext, GuiScope};
    #[test]
    fn request_budget_is_shared_across_devices_and_survives_reopen() {
        let path = std::env::temp_dir().join(format!("gui-budget-{}.db", Uuid::new_v4()));
        let request = |id: &str, device: &str| GuiRequest {
            scope: Some(GuiScope {
                run_id: "campaign".into(),
                assignment_id: Some(id.into()),
                device_id: device.into(),
                deadline_ms: None,
            }),
            protocol_version: 1,
            request_id: id.into(),
            observation_id: id.into(),
            session_epoch: device.into(),
            generation: 1,
            app: AppContext {
                package: "com.example".into(),
                version: "1".into(),
                system_locale: "en".into(),
                observed_language: None,
                width: 100,
                height: 100,
            },
            target: "profile".into(),
            expected_screen: "feed".into(),
            remaining_ms: 1000,
            nodes: Vec::new(),
            screenshot: String::new(),
            screenshot_sha256: String::new(),
        };
        {
            let db = Database::open(&path).unwrap();
            db.claim_gui_request(&request("1", "A"), 2).unwrap();
            assert!(db.claim_gui_request(&request("1", "A"), 2).is_err());
        }
        let db = Database::open(&path).unwrap();
        db.claim_gui_request(&request("2", "B"), 2).unwrap();
        assert!(db.claim_gui_request(&request("3", "C"), 2).is_err());
        let mut expired = request("expired", "D");
        expired.scope.as_mut().unwrap().run_id = "new-run".into();
        expired.scope.as_mut().unwrap().deadline_ms = Some(0);
        assert!(db.claim_gui_request(&expired, 20).is_err());
    }
}
