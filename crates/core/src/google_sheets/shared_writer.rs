//! Remote persistent mutex for cooperating schema-2 clients, not a CAS against
//! human edits. Google guarantees caller metadataId uniqueness (duplicate create
//! fails) and all-or-nothing batchUpdate. No lease/TTL/steal or POST replay.
//! https://developers.google.com/workspace/sheets/api/guides/metadata
//! https://developers.google.com/workspace/sheets/api/reference/rest/v4/spreadsheets/batchUpdate
use super::*;
use crate::db::Database;

pub(super) const LOCK_KEY: &str = "riviu.direct.shared-lock.v2";
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Phase {
    Prepared,
    AcquirePending,
    Acquired,
    Abandoning,
    MutationPending,
    Settled,
    Released,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Operation {
    token: String,
    writer: String,
    target: SheetDeliveryTarget,
    payload: Option<Value>,
    payload_hash: String,
    upgrade_confirmed: bool,
    phase: Phase,
    epoch: Option<String>,
    lock_epoch: Option<String>,
    row: Option<u32>,
    width: Option<u32>,
    duplicate: bool,
    receipt: Option<DeliveryReceipt>,
}
fn busy(message: &str) -> DirectSheetsError {
    DirectSheetsError {
        kind: DirectSheetsErrorKind::Busy,
        status: None,
        message: message.into(),
    }
}
fn uncertain() -> DirectSheetsError {
    busy("Shared Sheet operation is uncertain; retry only to read its receipt. Do not remove the remote lock until all writers have stopped and recovery is verified")
}
fn journal_error() -> DirectSheetsError {
    busy("Shared Sheet durable journal is unavailable; no new mutation can be dispatched")
}
fn protect_pending_error<T>(
    result: Result<T>,
    target: &SheetDeliveryTarget,
    db: &Database,
) -> Result<T> {
    if let Err(error) = result {
        let active = db
            .google_shared_journal_read(&journal_key(target))
            .map_err(|_| journal_error())?
            .map(|raw| serde_json::from_str::<Operation>(&raw))
            .transpose()
            .map_err(|_| journal_error())?;
        if active.is_some_and(|op| {
            matches!(
                op.phase,
                Phase::AcquirePending
                    | Phase::Acquired
                    | Phase::Abandoning
                    | Phase::MutationPending
                    | Phase::Settled
            )
        }) {
            return Err(uncertain());
        }
        Err(error)
    } else {
        result
    }
}
fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
fn lock_id(gid: u64) -> i32 {
    let hash = Sha256::digest(format!("{LOCK_KEY}:{gid}").as_bytes());
    let id = (i32::from_be_bytes([hash[0], hash[1], hash[2], hash[3]]) & 0x7fff_ffff).max(1);
    if id == writer_metadata_id(gid) {
        if id == i32::MAX {
            1
        } else {
            id + 1
        }
    } else {
        id
    }
}
fn journal_key(target: &SheetDeliveryTarget) -> String {
    format!(
        "google.sheets.shared-journal.v2.{}",
        digest(format!("{}:{}", target.spreadsheet_id, target.sheet_gid).as_bytes())
    )
}
impl Operation {
    fn serialized(&self) -> String {
        serde_json::to_string(self).expect("operation JSON")
    }
    fn lock_value(&self, committed: bool) -> String {
        json!({"schemaVersion":2,"operationToken":self.token,"writerId":self.writer,"spreadsheetId":self.target.spreadsheet_id,"sheetGid":self.target.sheet_gid,"reportingEpoch":self.lock_epoch,"payloadHash":self.payload_hash,"publicationId":self.payload.as_ref().map(|p|p["publicationId"].clone()),"revision":self.payload.as_ref().map(|p|p["deliveryRevision"].clone()),"phase":if committed {"committed"}else{"acquired"}}).to_string()
    }
    fn transition(&mut self, db: &Database, phase: Phase) -> Result<()> {
        let prior = self.serialized();
        self.phase = phase;
        if !db
            .google_shared_journal_cas(&journal_key(&self.target), Some(&prior), &self.serialized())
            .map_err(|_| journal_error())?
        {
            return Err(busy(
                "Another local writer is reconciling this Sheet operation",
            ));
        }
        Ok(())
    }
    fn save_changes(&self, db: &Database, prior: &str) -> Result<()> {
        if !db
            .google_shared_journal_cas(&journal_key(&self.target), Some(prior), &self.serialized())
            .map_err(|_| journal_error())?
        {
            return Err(busy("Another local writer advanced this Sheet operation"));
        }
        Ok(())
    }
    fn filter(&self, committed: bool) -> Value {
        json!({"developerMetadataLookup":{"metadataId":lock_id(self.target.sheet_gid),"metadataKey":LOCK_KEY,"metadataValue":self.lock_value(committed),"metadataLocation":{"sheetId":self.target.sheet_gid},"visibility":"DOCUMENT"}})
    }
}

impl DirectSheetsClient {
    /// Upgrading v1 requires an operator-confirmed drain of every old writer.
    /// Metadata cannot fence a v1 HTTP write which was already in flight.
    pub async fn prepare_shared_target(
        &self,
        target: &SheetDeliveryTarget,
        writer_id: &str,
        upgrade_legacy_confirmed: bool,
        db: &Database,
    ) -> Result<DirectTargetCheck> {
        let gate = db
            .google_shared_operation_gate(&journal_key(target))
            .map_err(|_| journal_error())?;
        let result = tokio::time::timeout(Duration::from_secs(90), async {
            // Queue fairly inside the existing overall deadline. A separate short
            // lock timeout turns routine local contention into long outbox backoff.
            let _guard = gate.lock().await;
            self.shared_prepare(target, writer_id, upgrade_legacy_confirmed, db)
                .await
        })
        .await
        .map_err(|_| uncertain())?;
        protect_pending_error(result, target, db)
    }
    pub async fn deliver_shared(
        &self,
        target: &SheetDeliveryTarget,
        payload: &Value,
        writer_id: &str,
        db: &Database,
    ) -> Result<DeliveryReceipt> {
        let gate = db
            .google_shared_operation_gate(&journal_key(target))
            .map_err(|_| journal_error())?;
        let result = tokio::time::timeout(Duration::from_secs(90), async {
            let _guard = gate.lock().await;
            self.shared_deliver(target, payload, writer_id, db).await
        })
        .await
        .map_err(|_| uncertain())?;
        protect_pending_error(result, target, db)
    }
    async fn begin_shared(
        &self,
        target: &SheetDeliveryTarget,
        payload: Option<&Value>,
        writer: &str,
        confirmed: bool,
        db: &Database,
    ) -> Result<Operation> {
        target
            .validate()
            .map_err(|_| DirectSheetsError::invalid("Invalid shared Sheet target"))?;
        uuid::Uuid::parse_str(writer)
            .map_err(|_| DirectSheetsError::invalid("Installation writer ID must be UUID"))?;
        let hash=digest(serde_json::to_string(&json!({"target":target,"payload":payload,"writer":writer,"upgradeConfirmed":confirmed})).expect("intent JSON").as_bytes());
        let key = journal_key(target);
        let prior = db
            .google_shared_journal_read(&key)
            .map_err(|_| journal_error())?;
        if let Some(raw) = prior.as_deref() {
            let mut old: Operation = serde_json::from_str(raw).map_err(|_| journal_error())?;
            if old.phase == Phase::Abandoning {
                self.abandon_shared(&mut old, db).await?;
                return Box::pin(self.begin_shared(target, payload, writer, confirmed, db)).await;
            }
            if old.payload_hash == hash
                && (old.phase != Phase::Released || (payload.is_some() && old.receipt.is_some()))
            {
                return Ok(old);
            }
            if old.phase == Phase::Acquired {
                self.abandon_shared(&mut old, db).await?;
                return Box::pin(self.begin_shared(target, payload, writer, confirmed, db)).await;
            }
            if old.phase == Phase::MutationPending {
                // Outbox progress revisions may have coalesced since dispatch.
                // Reconcile the frozen previous payload; never replay its POST.
                self.reconcile_shared(&mut old, db).await?;
            }
            if old.phase == Phase::Settled {
                self.release_shared(&mut old, db).await?;
                return Box::pin(self.begin_shared(target, payload, writer, confirmed, db)).await;
            }
            if !matches!(old.phase, Phase::Released | Phase::Prepared) {
                return Err(busy("A prior local Sheet operation must finish before another publication or revision"));
            }
        }
        // Identity/owner only. No header scan, legacy upgrade or local migration
        // barrier is justified by this read-only preflight.
        let meta = self
            .metadata_only(&target.spreadsheet_id, target.sheet_gid)
            .await?;
        self.validate_shared_owner(&meta, target, payload.is_none(), confirmed)?;
        let operation = Operation {
            token: uuid::Uuid::new_v4().to_string(),
            writer: writer.into(),
            target: target.clone(),
            payload: payload.cloned(),
            payload_hash: hash,
            upgrade_confirmed: confirmed,
            phase: Phase::Prepared,
            epoch: meta
                .owner
                .as_ref()
                .map(|o| o.reporting_epoch.clone())
                .or_else(|| target.reporting_epoch.clone()),
            lock_epoch: meta
                .owner
                .as_ref()
                .map(|o| o.reporting_epoch.clone())
                .or_else(|| target.reporting_epoch.clone()),
            row: None,
            width: None,
            duplicate: false,
            receipt: None,
        };
        if !db
            .google_shared_journal_cas(&key, prior.as_deref(), &operation.serialized())
            .map_err(|_| journal_error())?
        {
            return Err(busy("Another local writer prepared this Sheet operation"));
        }
        Ok(operation)
    }
    fn validate_shared_owner(
        &self,
        meta: &Metadata,
        target: &SheetDeliveryTarget,
        prepare: bool,
        confirmed: bool,
    ) -> Result<()> {
        if let Some(owner) = &meta.owner {
            if owner.state != "ready"
                || target
                    .reporting_epoch
                    .as_ref()
                    .is_some_and(|e| e != &owner.reporting_epoch)
            {
                return Err(DirectSheetsError::conflict(
                    "Sheet reporting epoch or owner state changed",
                ));
            }
            if owner.schema_version == 1 && (!prepare || !confirmed) {
                return Err(DirectSheetsError {kind:DirectSheetsErrorKind::SharedUpgradeRequired,status:None,message:"Stop and drain every legacy writer, then explicitly confirm the one-time shared upgrade. Already in-flight legacy requests cannot be fenced".into()});
            }
        } else if !prepare {
            return Err(DirectSheetsError::conflict(
                "Prepare the shared Sheet target first",
            ));
        }
        Ok(())
    }
    async fn remote_lock(&self, op: &Operation) -> Result<Option<String>> {
        let value = self
            .request(
                reqwest::Method::GET,
                &op.target.spreadsheet_id,
                &[(
                    "fields",
                    "spreadsheetId,developerMetadata,sheets(developerMetadata)".into(),
                )],
                None,
            )
            .await?;
        if value["spreadsheetId"] != op.target.spreadsheet_id {
            return Err(DirectSheetsError::conflict(
                "Remote lock spreadsheet mismatch",
            ));
        }
        let mut found: Option<String> = None;
        let arrays = std::iter::once(value.get("developerMetadata")).chain(
            value["sheets"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|s| s.get("developerMetadata")),
        );
        for md in arrays.filter_map(|v| v.and_then(Value::as_array)).flatten() {
            if md["metadataId"].as_i64() != Some(lock_id(op.target.sheet_gid) as i64) {
                continue;
            }
            if md["metadataKey"] != LOCK_KEY
                || md["location"]["sheetId"].as_u64() != Some(op.target.sheet_gid)
            {
                return Err(DirectSheetsError::conflict(
                    "Shared mutex metadata ID collides with another metadata namespace or tab",
                ));
            }
            let raw = md["metadataValue"]
                .as_str()
                .ok_or_else(|| DirectSheetsError::conflict("Invalid shared mutex metadata"))?;
            if found.as_deref().is_some_and(|s| s != raw) {
                return Err(DirectSheetsError::conflict(
                    "Conflicting shared mutex metadata",
                ));
            }
            found = Some(raw.into());
        }
        Ok(found)
    }
    async fn acquire_shared(&self, op: &mut Operation, db: &Database) -> Result<()> {
        if !matches!(op.phase, Phase::Prepared | Phase::AcquirePending) {
            return Ok(());
        }
        let remote = self.remote_lock(op).await.map_err(|e| {
            if op.phase == Phase::AcquirePending {
                uncertain()
            } else {
                e
            }
        })?;
        if remote.as_deref() == Some(&op.lock_value(false)) {
            op.transition(db, Phase::Acquired)?;
            return Ok(());
        }
        if remote.is_some() {
            return Err(busy("Another computer is writing this shared Sheet; retry later. Locks are never stolen by age"));
        }
        if op.phase == Phase::AcquirePending {
            return Err(uncertain());
        }
        op.transition(db, Phase::AcquirePending)?;
        let result=self.batch(&op.target.spreadsheet_id,vec![json!({"createDeveloperMetadata":{"developerMetadata":{"metadataId":lock_id(op.target.sheet_gid),"metadataKey":LOCK_KEY,"metadataValue":op.lock_value(false),"location":{"sheetId":op.target.sheet_gid},"visibility":"DOCUMENT"}}})]).await;
        if let Err(error) = result {
            // Persist the definite no-effect result BEFORE any follow-up GET:
            // losing that GET must not turn a rejected create into uncertainty.
            if matches!(
                error.status,
                Some(400 | 401 | 403 | 404 | 409 | 412 | 422 | 429)
            ) {
                op.transition(db, Phase::Prepared)?;
                if error.status == Some(400) {
                    // The winner may already have released before this read.
                    // Still inspect ID/namespace collisions, never steal them.
                    self.remote_lock(op).await?;
                    return Err(busy("Shared Sheet acquire was rejected; another writer may have finished. Retry with backoff"));
                }
                return Err(error);
            }
        }
        let current = self.remote_lock(op).await.map_err(|_| uncertain())?;
        if current.as_deref() == Some(&op.lock_value(false)) {
            op.transition(db, Phase::Acquired)?;
            return Ok(());
        }
        Err(uncertain())
    }
    async fn dispatch_shared(
        &self,
        op: &mut Operation,
        db: &Database,
        mut requests: Vec<Value>,
    ) -> Result<()> {
        if self
            .remote_lock(op)
            .await
            .map_err(|_| uncertain())?
            .as_deref()
            != Some(&op.lock_value(false))
        {
            return Err(uncertain());
        }
        requests.push(json!({"updateDeveloperMetadata":{"dataFilters":[op.filter(false)],"developerMetadata":{"metadataValue":op.lock_value(true)},"fields":"metadataValue"}}));
        op.transition(db, Phase::MutationPending)?;
        // Exactly one dispatch. Even a successful HTTP response is insufficient
        // until the atomic marker AND exact row/owner receipt are read back.
        if let Err(error) = self.batch(&op.target.spreadsheet_id, requests).await {
            if matches!(
                error.status,
                Some(400 | 401 | 403 | 404 | 409 | 412 | 422 | 429)
            ) {
                // Only the sole dispatch's actual rejection proves no effect.
                // A missing marker, transport failure or 5xx never does.
                op.transition(db, Phase::Abandoning)?;
                self.abandon_shared(op, db).await?;
                return Err(error);
            }
        }
        Ok(())
    }
    async fn committed_shared(&self, op: &Operation) -> Result<()> {
        if self
            .remote_lock(op)
            .await
            .map_err(|_| uncertain())?
            .as_deref()
            != Some(&op.lock_value(true))
        {
            return Err(uncertain());
        }
        Ok(())
    }
    async fn reconcile_shared(&self, op: &mut Operation, db: &Database) -> Result<()> {
        if op.phase != Phase::MutationPending {
            return Err(uncertain());
        }
        self.committed_shared(op).await?;
        let meta = self
            .metadata(&op.target.spreadsheet_id, op.target.sheet_gid)
            .await
            .map_err(|_| uncertain())?;
        self.validate_shared_owner(&meta, &op.target, false, false)?;
        if meta
            .owner
            .as_ref()
            .is_none_or(|owner| Some(&owner.reporting_epoch) != op.epoch.as_ref())
            || Layout::from_header(&meta.header)?.internal != op.target.internal_reporting
        {
            return Err(uncertain());
        }
        let prior = op.serialized();
        if let Some(payload) = &op.payload {
            let row = self
                .rows(
                    &meta,
                    op.row.ok_or_else(journal_error)?,
                    1,
                    op.width.ok_or_else(journal_error)?,
                )
                .await
                .map_err(|_| uncertain())?
                .remove(0);
            op.receipt = Some(planner::receipt(&row, payload, op.duplicate)?);
        }
        op.phase = Phase::Settled;
        op.save_changes(db, &prior)
    }
    async fn abandon_shared(&self, op: &mut Operation, db: &Database) -> Result<()> {
        // This CAS is the local single-owner gate. A stale planner can no longer
        // transition Acquired -> MutationPending after cleanup starts.
        if op.phase == Phase::Acquired {
            op.transition(db, Phase::Abandoning)?;
        }
        if op.phase != Phase::Abandoning {
            return Err(uncertain());
        }
        let current = self.remote_lock(op).await.map_err(|_| uncertain())?;
        if current.as_deref() == Some(&op.lock_value(true)) {
            return Err(uncertain());
        }
        if current.as_deref() == Some(&op.lock_value(false)) {
            let _ = self
                .batch(
                    &op.target.spreadsheet_id,
                    vec![json!({"deleteDeveloperMetadata":{"dataFilter":op.filter(false)}})],
                )
                .await;
            let current = self.remote_lock(op).await.map_err(|_| uncertain())?;
            if current.as_deref() == Some(&op.lock_value(false))
                || current.as_deref() == Some(&op.lock_value(true))
            {
                return Err(uncertain());
            }
        }
        op.transition(db, Phase::Released)
    }
    async fn release_shared(&self, op: &mut Operation, db: &Database) -> Result<()> {
        if op.phase == Phase::Released {
            return Ok(());
        }
        if op.phase != Phase::Settled {
            return Err(uncertain());
        }
        let current = self.remote_lock(op).await.map_err(|_| uncertain())?;
        if current.as_deref() == Some(&op.lock_value(true)) {
            // Retrying this deletion is safe: an exact serialized operation
            // value can never match the next writer's token, even after delay.
            let _ = self
                .batch(
                    &op.target.spreadsheet_id,
                    vec![json!({"deleteDeveloperMetadata":{"dataFilter":op.filter(true)}})],
                )
                .await;
            if self
                .remote_lock(op)
                .await
                .map_err(|_| uncertain())?
                .as_deref()
                == Some(&op.lock_value(true))
            {
                return Err(uncertain());
            }
        } else if current.as_deref() == Some(&op.lock_value(false)) {
            return Err(uncertain());
        }
        op.transition(db, Phase::Released)
    }
    async fn shared_deliver(
        &self,
        target: &SheetDeliveryTarget,
        payload: &Value,
        writer: &str,
        db: &Database,
    ) -> Result<DeliveryReceipt> {
        let mut op = self
            .begin_shared(target, Some(payload), writer, false, db)
            .await?;
        self.acquire_shared(&mut op, db).await?;
        if op.phase == Phase::Acquired {
            let planning: Result<Vec<Value>> = async {
                let meta = self
                    .metadata(&target.spreadsheet_id, target.sheet_gid)
                    .await
                    .map_err(|_| uncertain())?;
                self.validate_shared_owner(&meta, target, false, false)?;
                let layout = Layout::from_header(&meta.header)?;
                if layout.internal != target.internal_reporting {
                    return Err(DirectSheetsError::conflict(
                        "Pinned reporting layout changed",
                    ));
                }
                planner::validate_payload(payload, target, &layout)?;
                let scan = self.scan(&meta, layout.width, Some(payload)).await?;
                let plan = planner::plan(&meta, &layout, &scan, payload)?;
                let before = self.metadata(&meta.spreadsheet_id, meta.gid).await?;
                self.validate_shared_owner(&before, target, false, false)?;
                if before.header != meta.header {
                    return Err(DirectSheetsError::conflict(
                        "Header changed during shared row planning",
                    ));
                }
                if plan.row < meta.row_count
                    && self
                        .rows(&before, plan.row, 1, layout.width as u32)
                        .await?
                        .remove(0)
                        .cells
                        != plan.before.cells
                {
                    return Err(DirectSheetsError::conflict(
                        "Row moved or changed before shared writing",
                    ));
                }
                let prior = op.serialized();
                op.row = Some(plan.row);
                op.width = Some(plan.width as u32);
                op.duplicate = plan.duplicate;
                op.save_changes(db, &prior)?;
                Ok(plan.requests)
            }
            .await;
            let requests = match planning {
                Ok(requests) => requests,
                Err(error) => {
                    self.abandon_shared(&mut op, db).await?;
                    return Err(error);
                }
            };
            self.dispatch_shared(&mut op, db, requests).await?;
        }
        if op.phase == Phase::MutationPending {
            self.reconcile_shared(&mut op, db).await?;
        }
        self.release_shared(&mut op, db).await?;
        op.receipt.ok_or_else(journal_error)
    }
    async fn shared_prepare(
        &self,
        target: &SheetDeliveryTarget,
        writer: &str,
        confirmed: bool,
        db: &Database,
    ) -> Result<DirectTargetCheck> {
        let mut op = self
            .begin_shared(target, None, writer, confirmed, db)
            .await?;
        let needs_fresh_permission_proof =
            matches!(op.phase, Phase::MutationPending | Phase::Settled);
        self.acquire_shared(&mut op, db).await?;
        if op.phase == Phase::Acquired {
            let planning:Result<Vec<Value>>=async {
            let meta=self.metadata(&target.spreadsheet_id,target.sheet_gid).await.map_err(|_|uncertain())?;
            self.validate_shared_owner(&meta,target,true,confirmed)?;
            let empty=meta.header.iter().all(Cell::empty);
            if empty && meta.column_count>MAX_COLUMNS {return Err(DirectSheetsError::invalid("Blank-header preparation exceeds inspected columns"))}
            let layout=if empty {Layout::standard(target.internal_reporting)} else {Layout::from_header(&meta.header)?};
            if layout.internal!=target.internal_reporting {return Err(DirectSheetsError::conflict("Configured reporting layout differs from the tab"))}
            let scan=self.scan(&meta,if empty {meta.column_count.min(MAX_COLUMNS) as usize}else{layout.width},None).await?;
            if empty && scan.last_nonempty.is_some() {return Err(DirectSheetsError::conflict("Blank header has existing data below it"))}
            let epoch=op.epoch.clone().or_else(||scan.epochs.iter().next().cloned()).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
            if scan.epochs.iter().any(|e|e!=&epoch) {return Err(DirectSheetsError::conflict("Existing row epoch differs from selected target"))}
            let before=self.metadata(&target.spreadsheet_id,target.sheet_gid).await?;
            if before.header!=meta.header || serde_json::to_value(&before.owner).ok()!=serde_json::to_value(&meta.owner).ok() {return Err(DirectSheetsError::conflict("Owner/header changed during shared upgrade"))}
            let owner=Owner{schema_version:2,writer_id:meta.owner.as_ref().map(|o|o.writer_id.clone()).unwrap_or_else(||writer.into()),reporting_epoch:epoch.clone(),state:"ready".into(),reset_id:None,backup_spreadsheet_id:None,backup_fingerprint:None};
            let mut requests=Vec::new();
            if let Some(old)=&meta.owner {
                if old.schema_version!=2 {requests.push(json!({"updateDeveloperMetadata":{"dataFilters":[{"developerMetadataLookup":{"metadataId":writer_metadata_id(meta.gid),"metadataKey":WRITER_METADATA_KEY,"metadataValue":meta.owner_raw.as_deref().ok_or_else(||DirectSheetsError::conflict("Owner metadata source is missing"))?}}],"developerMetadata":{"metadataValue":serde_json::to_string(&owner).expect("owner JSON")},"fields":"metadataValue"}}));}
            } else {requests.push(json!({"createDeveloperMetadata":{"developerMetadata":{"metadataId":writer_metadata_id(meta.gid),"metadataKey":WRITER_METADATA_KEY,"metadataValue":serde_json::to_string(&owner).expect("owner JSON"),"location":{"sheetId":meta.gid},"visibility":"DOCUMENT"}}}));}
            if empty {
                if meta.column_count<layout.width as u32 {requests.push(json!({"appendDimension":{"sheetId":meta.gid,"dimension":"COLUMNS","length":layout.width as u32-meta.column_count}}));}
                requests.push(json!({"updateCells":{"start":{"sheetId":meta.gid,"rowIndex":0,"columnIndex":0},"rows":[{"values":layout.headers.iter().map(|s|json!({"userEnteredValue":{"stringValue":s}})).collect::<Vec<_>>()}],"fields":"userEnteredValue"}}));
            }
            let prior=op.serialized();op.epoch=Some(epoch);op.save_changes(db,&prior)?;
            Ok(requests)
            }.await;
            let requests = match planning {
                Ok(requests) => requests,
                Err(error) => {
                    self.abandon_shared(&mut op, db).await?;
                    return Err(error);
                }
            };
            self.dispatch_shared(&mut op, db, requests).await?;
        }
        if op.phase == Phase::MutationPending {
            self.reconcile_shared(&mut op, db).await?;
        }
        self.release_shared(&mut op, db).await?;
        if needs_fresh_permission_proof {
            return Box::pin(self.shared_prepare(target, writer, confirmed, db)).await;
        }
        let meta = self
            .metadata(&target.spreadsheet_id, target.sheet_gid)
            .await
            .map_err(|_| uncertain())?;
        self.validate_shared_owner(&meta, target, false, false)?;
        let mut check = meta.check();
        check.writable = true;
        Ok(check)
    }
}
