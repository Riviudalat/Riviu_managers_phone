use anyhow::Context;
use chrono::{DateTime, Duration as ChronoDuration, SecondsFormat, Utc};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::Database;
use crate::{
    validate_automation_config, validate_automation_profile_config, AutomationDefinition,
    AutomationDefinitionArchived, AutomationDefinitionNotFound, AutomationDefinitionRecord,
    AutomationDefinitionRevision, AutomationKind, AutomationProfileRef, AutomationRevisionConflict,
    AutomationSchedule, AutomationScheduleConflict, AutomationScheduleNotFound,
    AutomationScheduleOccurrence, AutomationScheduleOccurrenceState, AutomationScheduleV1,
    ChildCampaignOutcome, ResolvedTargetSnapshot, TargetRef,
};

impl Database {
    pub fn create_automation_definition(
        &self,
        name: &str,
        kind: AutomationKind,
        target_ref: &TargetRef,
        config: &serde_json::Value,
    ) -> anyhow::Result<AutomationDefinitionRecord> {
        let name = name.trim();
        anyhow::ensure!(
            !name.is_empty(),
            "automation definition name must not be empty"
        );
        validate_automation_profile_config(kind, config)?;
        let target_json = serde_json::to_string(target_ref).context("serialize target ref")?;
        let config_json = serde_json::to_string(config).context("serialize automation config")?;
        let id = Uuid::new_v4();
        let now = timestamp();
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO automation_definitions(
                id,name,kind,latest_revision,archived,created_at,updated_at
             ) VALUES(?1,?2,?3,1,0,?4,?4)",
            params![id.to_string(), name, kind.as_str(), now],
        )?;
        transaction.execute(
            "INSERT INTO automation_definition_revisions(
                definition_id,revision,target_json,config_json,created_at
             ) VALUES(?1,1,?2,?3,?4)",
            params![id.to_string(), target_json, config_json, now],
        )?;
        transaction.commit()?;
        Ok(AutomationDefinitionRecord {
            definition: AutomationDefinition {
                id,
                name: name.into(),
                kind,
                latest_revision: 1,
                archived: false,
                created_at: now.clone(),
                updated_at: now.clone(),
            },
            revision: AutomationDefinitionRevision {
                definition_id: id,
                revision: 1,
                target_ref: target_ref.clone(),
                config: config.clone(),
                created_at: now,
            },
        })
    }

    pub fn revise_automation_definition(
        &self,
        definition_id: Uuid,
        expected_revision: u64,
        target_ref: &TargetRef,
        config: &serde_json::Value,
    ) -> anyhow::Result<AutomationDefinitionRecord> {
        let target_json = serde_json::to_string(target_ref).context("serialize target ref")?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row: Option<(i64, bool, String)> = transaction
            .query_row(
                "SELECT latest_revision,archived,kind FROM automation_definitions WHERE id=?1",
                [definition_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((actual_sql, archived, kind)) = row else {
            return Err(AutomationDefinitionNotFound { definition_id }.into());
        };
        let kind = AutomationKind::from_str(&kind)?;
        validate_automation_profile_config(kind, config)?;
        let config_json = serde_json::to_string(config).context("serialize automation config")?;
        let actual = sql_to_revision(actual_sql, "automation_definitions.latest_revision")?;
        if archived {
            return Err(AutomationDefinitionArchived { definition_id }.into());
        }
        if expected_revision != actual {
            return Err(AutomationRevisionConflict {
                expected: expected_revision,
                actual,
            }
            .into());
        }
        let next = actual
            .checked_add(1)
            .context("automation definition revision overflow")?;
        let next_sql = revision_to_sql(next)?;
        let expected_sql = revision_to_sql(expected_revision)?;
        let now = timestamp();
        let changed = transaction.execute(
            "UPDATE automation_definitions
             SET latest_revision=?2,updated_at=?3
             WHERE id=?1 AND latest_revision=?4 AND archived=0",
            params![definition_id.to_string(), next_sql, now, expected_sql],
        )?;
        if changed != 1 {
            let current: i64 = transaction.query_row(
                "SELECT latest_revision FROM automation_definitions WHERE id=?1",
                [definition_id.to_string()],
                |row| row.get(0),
            )?;
            return Err(AutomationRevisionConflict {
                expected: expected_revision,
                actual: sql_to_revision(current, "automation_definitions.latest_revision")?,
            }
            .into());
        }
        transaction.execute(
            "INSERT INTO automation_definition_revisions(
                definition_id,revision,target_json,config_json,created_at
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                definition_id.to_string(),
                next_sql,
                target_json,
                config_json,
                now
            ],
        )?;
        transaction.commit()?;

        self.get_automation_definition_record(definition_id, next)?
            .context("saved automation definition disappeared")
    }

    pub fn get_automation_definition(
        &self,
        definition_id: Uuid,
    ) -> anyhow::Result<Option<AutomationDefinition>> {
        let connection = self.conn()?;
        connection
            .query_row(
                "SELECT id,name,kind,latest_revision,archived,created_at,updated_at
                 FROM automation_definitions WHERE id=?1",
                [definition_id.to_string()],
                definition_from_row,
            )
            .optional()?
            .map(DefinitionRow::into_definition)
            .transpose()
    }

    pub fn list_automation_definitions(
        &self,
        include_archived: bool,
    ) -> anyhow::Result<Vec<AutomationDefinition>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id,name,kind,latest_revision,archived,created_at,updated_at
             FROM automation_definitions
             WHERE (?1=1 OR archived=0)
             ORDER BY updated_at DESC,id ASC",
        )?;
        let rows = statement.query_map([i64::from(include_archived)], definition_from_row)?;
        rows.map(|row| row?.into_definition()).collect()
    }

    pub fn archive_automation_definition(&self, definition_id: Uuid) -> anyhow::Result<()> {
        let changed = self.conn()?.execute(
            "UPDATE automation_definitions SET archived=1,updated_at=?2 WHERE id=?1",
            params![definition_id.to_string(), timestamp()],
        )?;
        if changed == 0 {
            return Err(AutomationDefinitionNotFound { definition_id }.into());
        }
        Ok(())
    }

    pub fn get_automation_definition_revision(
        &self,
        definition_id: Uuid,
        revision: u64,
    ) -> anyhow::Result<Option<AutomationDefinitionRevision>> {
        let connection = self.conn()?;
        let row = connection
            .query_row(
                "SELECT definition_id,revision,target_json,config_json,created_at
                 FROM automation_definition_revisions
                 WHERE definition_id=?1 AND revision=?2",
                params![definition_id.to_string(), revision_to_sql(revision)?],
                |row| {
                    Ok(RevisionRow {
                        definition_id: row.get(0)?,
                        revision: row.get(1)?,
                        target_json: row.get(2)?,
                        config_json: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        row.map(RevisionRow::into_revision).transpose()
    }

    pub fn get_automation_definition_record(
        &self,
        definition_id: Uuid,
        revision: u64,
    ) -> anyhow::Result<Option<AutomationDefinitionRecord>> {
        let Some(definition) = self.get_automation_definition(definition_id)? else {
            return Ok(None);
        };
        let revision = self
            .get_automation_definition_revision(definition_id, revision)?
            .context("automation definition revision is missing")?;
        Ok(Some(AutomationDefinitionRecord {
            definition,
            revision,
        }))
    }

    pub fn create_automation_schedule(
        &self,
        name: &str,
        definition_id: Uuid,
        definition_revision: u64,
        enabled: bool,
        schedule: &AutomationScheduleV1,
    ) -> anyhow::Result<AutomationSchedule> {
        let name = name.trim();
        anyhow::ensure!(
            !name.is_empty(),
            "automation schedule name must not be empty"
        );
        schedule.validate()?;
        validate_schedule_profile(self, definition_id, definition_revision)?;
        let schedule_json =
            serde_json::to_string(schedule).context("serialize automation schedule")?;
        let id = Uuid::new_v4();
        let now = timestamp();
        let next_due_at = enabled
            .then(|| next_due_from(&now, schedule.every_minutes))
            .transpose()?;
        self.conn()?.execute(
            "INSERT INTO automation_schedules(
                id,revision,name,definition_id,definition_revision,enabled,
                schedule_json,next_due_at,last_error_code,created_at,updated_at
             ) VALUES(?1,1,?2,?3,?4,?5,?6,?7,NULL,?8,?8)",
            params![
                id.to_string(),
                name,
                definition_id.to_string(),
                revision_to_sql(definition_revision)?,
                i64::from(enabled),
                schedule_json,
                next_due_at,
                now,
            ],
        )?;
        Ok(AutomationSchedule {
            id,
            revision: 1,
            name: name.into(),
            definition_id,
            definition_revision,
            enabled,
            schedule: serde_json::to_value(schedule).context("serialize automation schedule")?,
            next_due_at,
            last_error_code: None,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_automation_schedule(
        &self,
        schedule_id: Uuid,
        expected_revision: u64,
        name: &str,
        definition_id: Uuid,
        definition_revision: u64,
        enabled: bool,
        schedule: &AutomationScheduleV1,
    ) -> anyhow::Result<AutomationSchedule> {
        let name = name.trim();
        anyhow::ensure!(
            !name.is_empty(),
            "automation schedule name must not be empty"
        );
        schedule.validate()?;
        validate_schedule_profile(self, definition_id, definition_revision)?;
        let schedule_json =
            serde_json::to_string(schedule).context("serialize automation schedule")?;
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual_sql: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM automation_schedules WHERE id=?1",
                [schedule_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(actual_sql) = actual_sql else {
            return Err(AutomationScheduleNotFound { schedule_id }.into());
        };
        let actual = sql_to_revision(actual_sql, "automation_schedules.revision")?;
        if expected_revision != actual {
            return Err(AutomationScheduleConflict {
                expected: expected_revision,
                actual,
            }
            .into());
        }
        let next = actual
            .checked_add(1)
            .context("automation schedule revision overflow")?;
        let updated_at = timestamp();
        let next_due_at = enabled
            .then(|| next_due_from(&updated_at, schedule.every_minutes))
            .transpose()?;
        let changed = transaction.execute(
            "UPDATE automation_schedules
             SET revision=?2,name=?3,definition_id=?4,definition_revision=?5,
                 enabled=?6,schedule_json=?7,next_due_at=?8,last_error_code=NULL,updated_at=?9
             WHERE id=?1 AND revision=?10",
            params![
                schedule_id.to_string(),
                revision_to_sql(next)?,
                name,
                definition_id.to_string(),
                revision_to_sql(definition_revision)?,
                i64::from(enabled),
                schedule_json,
                next_due_at,
                updated_at,
                revision_to_sql(expected_revision)?,
            ],
        )?;
        if changed != 1 {
            let current: i64 = transaction.query_row(
                "SELECT revision FROM automation_schedules WHERE id=?1",
                [schedule_id.to_string()],
                |row| row.get(0),
            )?;
            return Err(AutomationScheduleConflict {
                expected: expected_revision,
                actual: sql_to_revision(current, "automation_schedules.revision")?,
            }
            .into());
        }
        let row = query_schedule(&transaction, schedule_id)?
            .context("updated automation schedule disappeared")?;
        transaction.commit()?;
        Ok(row)
    }

    pub fn get_automation_schedule(
        &self,
        schedule_id: Uuid,
    ) -> anyhow::Result<Option<AutomationSchedule>> {
        query_schedule(&self.conn()?, schedule_id)
    }

    pub fn list_automation_schedules(&self) -> anyhow::Result<Vec<AutomationSchedule>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id,revision,name,definition_id,definition_revision,enabled,
                    schedule_json,next_due_at,last_error_code,created_at,updated_at
             FROM automation_schedules ORDER BY updated_at DESC,id ASC",
        )?;
        let rows = statement.query_map([], schedule_from_row)?;
        rows.map(|row| row?.into_schedule()).collect()
    }

    pub fn list_due_automation_schedules(
        &self,
        now: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<AutomationSchedule>> {
        DateTime::parse_from_rfc3339(now).context("parse automation schedule tick time")?;
        anyhow::ensure!(
            (1..=100).contains(&limit),
            "schedule due limit must be 1..=100"
        );
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id FROM automation_schedules
             WHERE enabled=1 AND next_due_at IS NOT NULL AND next_due_at<=?1
               AND NOT EXISTS (
                 SELECT 1 FROM automation_schedule_occurrences o
                 WHERE o.schedule_id=automation_schedules.id
                   AND o.state IN ('queued','dispatching','running')
               )
             ORDER BY next_due_at,id LIMIT ?2",
        )?;
        let ids = statement
            .query_map(params![now, i64::try_from(limit)?], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                query_schedule(
                    &connection,
                    Uuid::parse_str(&id).context("parse due automation schedule ID")?,
                )?
                .context("due automation schedule disappeared")
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn claim_automation_schedule_occurrence(
        &self,
        schedule_id: Uuid,
        expected_revision: u64,
        scheduled_for: &str,
        now: &str,
        target: Option<&ResolvedTargetSnapshot>,
        error_code: Option<&str>,
    ) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
        let scheduled_for_time = DateTime::parse_from_rfc3339(scheduled_for)
            .context("parse automation occurrence slot")?;
        let now_time = DateTime::parse_from_rfc3339(now).context("parse automation tick time")?;
        anyhow::ensure!(
            scheduled_for_time <= now_time,
            "automation occurrence cannot be claimed before it is due"
        );
        match (target, error_code) {
            (Some(target), None) => validate_occurrence_target(target)?,
            (None, Some(code)) => anyhow::ensure!(
                !code.trim().is_empty(),
                "failed automation occurrence needs an error code"
            ),
            _ => anyhow::bail!("automation occurrence target and error code are inconsistent"),
        }

        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(schedule) = query_schedule(&transaction, schedule_id)? else {
            return Err(AutomationScheduleNotFound { schedule_id }.into());
        };
        if schedule.revision != expected_revision
            || !schedule.enabled
            || schedule.next_due_at.as_deref() != Some(scheduled_for)
        {
            transaction.commit()?;
            return Ok(None);
        }
        let profile_row: Option<(String, bool)> = transaction
            .query_row(
                "SELECT d.kind,d.archived
                 FROM automation_definitions d
                 JOIN automation_definition_revisions r
                   ON r.definition_id=d.id AND r.revision=?2
                 WHERE d.id=?1",
                params![
                    schedule.definition_id.to_string(),
                    revision_to_sql(schedule.definition_revision)?
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let (kind, archived) = profile_row
            .context("scheduled automation profile revision does not exist during claim")?;
        let kind = AutomationKind::from_str(&kind)?;
        let (target, error_code) = if archived {
            (None, Some("profile_archived"))
        } else {
            (target, error_code)
        };
        let occurrence_id = Uuid::new_v4();
        let child_campaign_id = Uuid::new_v4();
        let key_material =
            format!("riviu-automation-schedule-occurrence-v1:{schedule_id}:{scheduled_for}");
        let idempotency_key = format!("{:x}", Sha256::digest(key_material.as_bytes()));
        let state = if target.is_some() { "queued" } else { "failed" };
        transaction.execute(
            "INSERT INTO automation_schedule_occurrences(
                id,schedule_id,schedule_revision,scheduled_for,definition_id,
                definition_revision,kind,target_json,child_campaign_id,idempotency_key,
                state,nurture_run_id,nurture_started_udids_json,error_code,created_at,updated_at
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,NULL,'[]',?12,?13,?13)",
            params![
                occurrence_id.to_string(),
                schedule_id.to_string(),
                revision_to_sql(schedule.revision)?,
                scheduled_for,
                schedule.definition_id.to_string(),
                revision_to_sql(schedule.definition_revision)?,
                kind.as_str(),
                target
                    .map(serde_json::to_string)
                    .transpose()
                    .context("serialize schedule occurrence target")?,
                child_campaign_id.to_string(),
                idempotency_key,
                state,
                error_code,
                now,
            ],
        )?;
        let schedule_v1: AutomationScheduleV1 = serde_json::from_value(schedule.schedule.clone())
            .context("parse due automation schedule")?;
        let next_due = next_slot_after(scheduled_for_time, now_time, schedule_v1.every_minutes)?;
        let changed = transaction.execute(
            "UPDATE automation_schedules
             SET next_due_at=?4,last_error_code=?5,updated_at=?6
             WHERE id=?1 AND revision=?2 AND enabled=1 AND next_due_at=?3",
            params![
                schedule_id.to_string(),
                revision_to_sql(expected_revision)?,
                scheduled_for,
                next_due,
                error_code,
                now,
            ],
        )?;
        anyhow::ensure!(
            changed == 1,
            "automation schedule changed while claiming due slot"
        );
        let occurrence = query_schedule_occurrence(&transaction, occurrence_id)?
            .context("claimed automation occurrence disappeared")?;
        transaction.commit()?;
        Ok(Some(occurrence))
    }

    pub fn mark_automation_schedule_occurrence_dispatching(
        &self,
        occurrence_id: Uuid,
    ) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
        self.transition_automation_schedule_occurrence(
            occurrence_id,
            AutomationScheduleOccurrenceState::Queued,
            AutomationScheduleOccurrenceState::Dispatching,
            None,
        )
    }

    pub fn mark_automation_schedule_occurrence_running(
        &self,
        occurrence_id: Uuid,
    ) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
        self.transition_automation_schedule_occurrence(
            occurrence_id,
            AutomationScheduleOccurrenceState::Dispatching,
            AutomationScheduleOccurrenceState::Running,
            None,
        )
    }

    pub fn record_scheduled_nurture_started(
        &self,
        occurrence_id: Uuid,
        child_campaign_id: Uuid,
        idempotency_key: &str,
        run_id: Uuid,
        started_udids: &[String],
    ) -> anyhow::Result<AutomationScheduleOccurrence> {
        anyhow::ensure!(
            !started_udids.is_empty(),
            "scheduled nurture started devices must not be empty"
        );
        let mut unique = std::collections::HashSet::new();
        anyhow::ensure!(
            started_udids
                .iter()
                .all(|udid| !udid.trim().is_empty() && unique.insert(udid.as_str())),
            "scheduled nurture started devices are empty or duplicated"
        );
        let connection = self.conn()?;
        let current = query_schedule_occurrence(&connection, occurrence_id)?
            .context("scheduled nurture occurrence does not exist")?;
        anyhow::ensure!(
            current.kind == AutomationKind::Nurture
                && current.child_campaign_id == child_campaign_id
                && current.idempotency_key == idempotency_key,
            "scheduled nurture child identity conflict"
        );
        let target = current
            .target
            .as_ref()
            .context("scheduled nurture occurrence has no target")?;
        let requested = target
            .included
            .iter()
            .map(|device| device.udid.as_str())
            .collect::<std::collections::HashSet<_>>();
        anyhow::ensure!(
            started_udids
                .iter()
                .all(|udid| requested.contains(udid.as_str())),
            "scheduled nurture child started outside its claimed target"
        );
        if current.state == AutomationScheduleOccurrenceState::Running {
            anyhow::ensure!(
                current.nurture_run_id == Some(run_id)
                    && current.nurture_started_udids == started_udids,
                "scheduled nurture runtime identity conflict"
            );
            return Ok(current);
        }
        let changed = connection.execute(
            "UPDATE automation_schedule_occurrences
             SET state='running',nurture_run_id=?4,nurture_started_udids_json=?5,updated_at=?6
             WHERE id=?1 AND child_campaign_id=?2 AND idempotency_key=?3
               AND kind='nurture' AND state='dispatching'",
            params![
                occurrence_id.to_string(),
                child_campaign_id.to_string(),
                idempotency_key,
                run_id.to_string(),
                serde_json::to_string(started_udids)
                    .context("serialize scheduled nurture started devices")?,
                timestamp(),
            ],
        )?;
        anyhow::ensure!(
            changed == 1,
            "scheduled nurture start acknowledgement raced"
        );
        query_schedule_occurrence(&connection, occurrence_id)?
            .context("scheduled nurture occurrence disappeared after start")
    }

    pub fn settle_automation_schedule_occurrence(
        &self,
        occurrence_id: Uuid,
        outcome: ChildCampaignOutcome,
        error_code: Option<&str>,
    ) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
        let next = occurrence_state_for_outcome(outcome);
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE automation_schedule_occurrences
             SET state=?2,error_code=?3,updated_at=?4
             WHERE id=?1 AND state IN ('queued','dispatching','running')",
            params![
                occurrence_id.to_string(),
                occurrence_state_label(next),
                error_code,
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_schedule_occurrence(&connection, occurrence_id)
    }

    pub fn get_automation_schedule_occurrence(
        &self,
        occurrence_id: Uuid,
    ) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
        query_schedule_occurrence(&self.conn()?, occurrence_id)
    }

    pub fn list_recoverable_automation_schedule_occurrences(
        &self,
    ) -> anyhow::Result<Vec<AutomationScheduleOccurrence>> {
        let connection = self.conn()?;
        let mut statement = connection.prepare(
            "SELECT id FROM automation_schedule_occurrences
             WHERE state IN ('queued','dispatching','running') ORDER BY created_at,id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                query_schedule_occurrence(
                    &connection,
                    Uuid::parse_str(&id).context("parse recoverable occurrence ID")?,
                )?
                .context("recoverable automation occurrence disappeared")
            })
            .collect()
    }

    fn transition_automation_schedule_occurrence(
        &self,
        occurrence_id: Uuid,
        expected: AutomationScheduleOccurrenceState,
        next: AutomationScheduleOccurrenceState,
        error_code: Option<&str>,
    ) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE automation_schedule_occurrences
             SET state=?3,error_code=?4,updated_at=?5 WHERE id=?1 AND state=?2",
            params![
                occurrence_id.to_string(),
                occurrence_state_label(expected),
                occurrence_state_label(next),
                error_code,
                timestamp(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        query_schedule_occurrence(&connection, occurrence_id)
    }
}

fn query_schedule(
    connection: &rusqlite::Connection,
    schedule_id: Uuid,
) -> anyhow::Result<Option<AutomationSchedule>> {
    connection
        .query_row(
            "SELECT id,revision,name,definition_id,definition_revision,enabled,
                    schedule_json,next_due_at,last_error_code,created_at,updated_at
             FROM automation_schedules WHERE id=?1",
            [schedule_id.to_string()],
            schedule_from_row,
        )
        .optional()?
        .map(ScheduleRow::into_schedule)
        .transpose()
}

fn query_schedule_occurrence(
    connection: &rusqlite::Connection,
    occurrence_id: Uuid,
) -> anyhow::Result<Option<AutomationScheduleOccurrence>> {
    connection
        .query_row(
            "SELECT id,schedule_id,schedule_revision,scheduled_for,definition_id,
                    definition_revision,kind,target_json,child_campaign_id,idempotency_key,
                    state,nurture_run_id,nurture_started_udids_json,error_code,created_at,updated_at
             FROM automation_schedule_occurrences WHERE id=?1",
            [occurrence_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                ))
            },
        )
        .optional()?
        .map(
            |(
                id,
                schedule_id,
                schedule_revision,
                scheduled_for,
                definition_id,
                definition_revision,
                kind,
                target_json,
                child_campaign_id,
                idempotency_key,
                state,
                nurture_run_id,
                nurture_started_json,
                error_code,
                created_at,
                updated_at,
            )| {
                let target: Option<ResolvedTargetSnapshot> = target_json
                    .map(|raw| {
                        serde_json::from_str(&raw).context("parse automation occurrence target")
                    })
                    .transpose()?;
                if let Some(target) = &target {
                    validate_occurrence_target(target)?;
                }
                let nurture_started_udids: Vec<String> =
                    serde_json::from_str(&nurture_started_json)
                        .context("parse scheduled nurture started devices")?;
                let mut unique = std::collections::HashSet::new();
                anyhow::ensure!(
                    nurture_started_udids
                        .iter()
                        .all(|udid| !udid.trim().is_empty() && unique.insert(udid.as_str())),
                    "scheduled nurture started devices are empty or duplicated"
                );
                Ok(AutomationScheduleOccurrence {
                    id: Uuid::parse_str(&id).context("parse automation occurrence ID")?,
                    schedule_id: Uuid::parse_str(&schedule_id)
                        .context("parse occurrence schedule ID")?,
                    schedule_revision: sql_to_revision(
                        schedule_revision,
                        "automation_schedule_occurrences.schedule_revision",
                    )?,
                    scheduled_for,
                    kind: AutomationKind::from_str(&kind)?,
                    profile: AutomationProfileRef {
                        definition_id: Uuid::parse_str(&definition_id)
                            .context("parse occurrence profile ID")?,
                        revision: sql_to_revision(
                            definition_revision,
                            "automation_schedule_occurrences.definition_revision",
                        )?,
                    },
                    target,
                    child_campaign_id: Uuid::parse_str(&child_campaign_id)
                        .context("parse occurrence child campaign ID")?,
                    idempotency_key,
                    state: occurrence_state_from_label(&state)?,
                    nurture_run_id: nurture_run_id
                        .map(|value| {
                            Uuid::parse_str(&value).context("parse scheduled nurture runtime ID")
                        })
                        .transpose()?,
                    nurture_started_udids,
                    error_code,
                    created_at,
                    updated_at,
                })
            },
        )
        .transpose()
}

struct ScheduleRow {
    id: String,
    revision: i64,
    name: String,
    definition_id: String,
    definition_revision: i64,
    enabled: bool,
    schedule_json: String,
    next_due_at: Option<String>,
    last_error_code: Option<String>,
    created_at: String,
    updated_at: String,
}

fn schedule_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScheduleRow> {
    Ok(ScheduleRow {
        id: row.get(0)?,
        revision: row.get(1)?,
        name: row.get(2)?,
        definition_id: row.get(3)?,
        definition_revision: row.get(4)?,
        enabled: row.get(5)?,
        schedule_json: row.get(6)?,
        next_due_at: row.get(7)?,
        last_error_code: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
    })
}

impl ScheduleRow {
    fn into_schedule(self) -> anyhow::Result<AutomationSchedule> {
        let schedule: serde_json::Value =
            serde_json::from_str(&self.schedule_json).context("parse automation schedule")?;
        Ok(AutomationSchedule {
            id: Uuid::parse_str(&self.id).context("invalid automation schedule id")?,
            revision: sql_to_revision(self.revision, "automation_schedules.revision")?,
            name: self.name,
            definition_id: Uuid::parse_str(&self.definition_id)
                .context("invalid scheduled automation definition id")?,
            definition_revision: sql_to_revision(
                self.definition_revision,
                "automation_schedules.definition_revision",
            )?,
            enabled: self.enabled,
            schedule,
            next_due_at: self.next_due_at,
            last_error_code: self.last_error_code,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

fn validate_schedule_profile(
    database: &Database,
    definition_id: Uuid,
    definition_revision: u64,
) -> anyhow::Result<()> {
    let record = database
        .get_automation_definition_record(definition_id, definition_revision)
        .context("scheduled automation profile revision does not exist")?
        .context("scheduled automation profile revision does not exist")?;
    anyhow::ensure!(
        !record.definition.archived,
        "scheduled automation profile is archived"
    );
    Ok(())
}

fn next_due_from(origin: &str, every_minutes: u16) -> anyhow::Result<String> {
    let origin = DateTime::parse_from_rfc3339(origin).context("parse schedule origin")?;
    Ok((origin + ChronoDuration::minutes(i64::from(every_minutes)))
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Nanos, true))
}

fn next_slot_after(
    scheduled_for: DateTime<chrono::FixedOffset>,
    now: DateTime<chrono::FixedOffset>,
    every_minutes: u16,
) -> anyhow::Result<String> {
    let interval_minutes = i64::from(every_minutes);
    let elapsed_minutes = now.signed_duration_since(scheduled_for).num_minutes();
    let intervals = elapsed_minutes
        .checked_div(interval_minutes)
        .and_then(|count| count.checked_add(1))
        .context("automation schedule interval overflow")?;
    let minutes = interval_minutes
        .checked_mul(intervals)
        .context("automation schedule next slot overflow")?;
    Ok((scheduled_for + ChronoDuration::minutes(minutes))
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Nanos, true))
}

fn validate_occurrence_target(target: &ResolvedTargetSnapshot) -> anyhow::Result<()> {
    anyhow::ensure!(
        target.roster_sha256.len() == 64
            && target
                .roster_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "automation occurrence target roster hash is invalid"
    );
    anyhow::ensure!(
        !target.included.is_empty(),
        "automation occurrence target has no eligible device"
    );
    let mut unique = std::collections::HashSet::new();
    anyhow::ensure!(
        target
            .included
            .iter()
            .all(|device| unique.insert(device.udid.as_str())),
        "automation occurrence target contains duplicate devices"
    );
    Ok(())
}

fn occurrence_state_from_label(value: &str) -> anyhow::Result<AutomationScheduleOccurrenceState> {
    Ok(match value {
        "queued" => AutomationScheduleOccurrenceState::Queued,
        "dispatching" => AutomationScheduleOccurrenceState::Dispatching,
        "running" => AutomationScheduleOccurrenceState::Running,
        "done" => AutomationScheduleOccurrenceState::Done,
        "partial" => AutomationScheduleOccurrenceState::Partial,
        "failed" => AutomationScheduleOccurrenceState::Failed,
        "uncertain" => AutomationScheduleOccurrenceState::Uncertain,
        other => anyhow::bail!("unknown automation occurrence state `{other}`"),
    })
}

fn occurrence_state_label(value: AutomationScheduleOccurrenceState) -> &'static str {
    match value {
        AutomationScheduleOccurrenceState::Queued => "queued",
        AutomationScheduleOccurrenceState::Dispatching => "dispatching",
        AutomationScheduleOccurrenceState::Running => "running",
        AutomationScheduleOccurrenceState::Done => "done",
        AutomationScheduleOccurrenceState::Partial => "partial",
        AutomationScheduleOccurrenceState::Failed => "failed",
        AutomationScheduleOccurrenceState::Uncertain => "uncertain",
    }
}

fn occurrence_state_for_outcome(
    outcome: ChildCampaignOutcome,
) -> AutomationScheduleOccurrenceState {
    match outcome {
        ChildCampaignOutcome::Done => AutomationScheduleOccurrenceState::Done,
        ChildCampaignOutcome::Partial => AutomationScheduleOccurrenceState::Partial,
        ChildCampaignOutcome::Failed => AutomationScheduleOccurrenceState::Failed,
        ChildCampaignOutcome::Uncertain => AutomationScheduleOccurrenceState::Uncertain,
    }
}

struct DefinitionRow {
    id: String,
    name: String,
    kind: String,
    latest_revision: i64,
    archived: bool,
    created_at: String,
    updated_at: String,
}

fn definition_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DefinitionRow> {
    Ok(DefinitionRow {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        latest_revision: row.get(3)?,
        archived: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

impl DefinitionRow {
    fn into_definition(self) -> anyhow::Result<AutomationDefinition> {
        Ok(AutomationDefinition {
            id: Uuid::parse_str(&self.id).context("invalid automation definition id")?,
            name: self.name,
            kind: AutomationKind::from_str(&self.kind)?,
            latest_revision: sql_to_revision(
                self.latest_revision,
                "automation_definitions.latest_revision",
            )?,
            archived: self.archived,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

struct RevisionRow {
    definition_id: String,
    revision: i64,
    target_json: String,
    config_json: String,
    created_at: String,
}

impl RevisionRow {
    fn into_revision(self) -> anyhow::Result<AutomationDefinitionRevision> {
        let config = serde_json::from_str(&self.config_json).context("parse automation config")?;
        validate_automation_config(&config)?;
        Ok(AutomationDefinitionRevision {
            definition_id: Uuid::parse_str(&self.definition_id)
                .context("invalid automation definition revision id")?,
            revision: sql_to_revision(self.revision, "automation_definition_revisions.revision")?,
            target_ref: serde_json::from_str(&self.target_json)
                .context("parse automation target ref")?,
            config,
            created_at: self.created_at,
        })
    }
}

fn revision_to_sql(revision: u64) -> anyhow::Result<i64> {
    i64::try_from(revision).context("automation revision does not fit SQLite INTEGER")
}

fn sql_to_revision(revision: i64, column: &'static str) -> anyhow::Result<u64> {
    u64::try_from(revision).with_context(|| format!("{column} must not be negative"))
}

fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use serde_json::json;
    use uuid::Uuid;

    use super::Database;
    use crate::{
        AutomationDefinitionArchived, AutomationDefinitionNotFound, AutomationKind,
        AutomationRevisionConflict, AutomationScheduleConflict, AutomationScheduleKind,
        AutomationScheduleOccurrenceState, AutomationScheduleV1, ResolvedTargetDevice,
        ResolvedTargetSnapshot, TargetRef,
    };

    fn database_fixture(label: &str) -> (Database, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-automation-{label}-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open database"), path)
    }

    fn cleanup(path: &std::path::Path) {
        for suffix in ["", "-wal", "-shm"] {
            std::fs::remove_file(format!("{}{suffix}", path.display())).ok();
        }
    }

    fn nurture_profile(like_prob: u32) -> serde_json::Value {
        json!({
            "schemaVersion": 1,
            "settings": { "likeProb": like_prob }
        })
    }

    fn interaction_profile(instruction: impl Into<String>) -> serde_json::Value {
        json!({
            "schemaVersion": 1,
            "request": {
                "targets": [],
                "messageCount": 1,
                "instruction": instruction.into(),
                "maxWords": 12,
                "actions": { "like": true, "comment": false, "save": false }
            }
        })
    }

    fn publish_profile(bundle_id: &str) -> serde_json::Value {
        json!({
            "schemaVersion": 1,
            "sourceRoot": "C:/fixture",
            "bundleIds": [bundle_id],
            "soundPolicy": { "kind": "default" },
            "executionConfirmed": true
        })
    }

    fn interval_schedule(every_minutes: u16) -> AutomationScheduleV1 {
        AutomationScheduleV1 {
            schema_version: 1,
            kind: AutomationScheduleKind::Interval,
            every_minutes,
        }
    }

    fn resolved_target(udid: &str) -> ResolvedTargetSnapshot {
        ResolvedTargetSnapshot {
            target_ref: TargetRef::All,
            included: vec![ResolvedTargetDevice {
                udid: udid.into(),
                alias: "May 1".into(),
                number: Some(1),
            }],
            excluded: Vec::new(),
            roster_sha256: "a".repeat(64),
        }
    }

    #[test]
    fn revisions_are_append_only_and_exact_revisions_remain_readable() {
        let (database, path) = database_fixture("immutable");
        let first = database
            .create_automation_definition(
                "Morning nurture",
                AutomationKind::Nurture,
                &TargetRef::All,
                &nurture_profile(20),
            )
            .expect("create definition");
        let second = database
            .revise_automation_definition(
                first.definition.id,
                1,
                &TargetRef::Explicit {
                    udids: vec!["phone-a".into()],
                },
                &nurture_profile(35),
            )
            .expect("revise definition");

        assert_eq!(first.revision.revision, 1);
        assert_eq!(second.revision.revision, 2);
        assert_eq!(second.definition.latest_revision, 2);
        assert_eq!(
            database
                .get_automation_definition_revision(first.definition.id, 1)
                .expect("read revision one")
                .expect("revision one exists"),
            first.revision
        );
        assert_eq!(
            database
                .get_automation_definition_revision(first.definition.id, 2)
                .expect("read revision two")
                .expect("revision two exists"),
            second.revision
        );

        let connection = database.conn().expect("inspect immutable table");
        assert!(connection
            .execute(
                "UPDATE automation_definition_revisions SET config_json='{}' \
                 WHERE definition_id=?1 AND revision=1",
                [first.definition.id.to_string()],
            )
            .is_err());
        assert!(connection
            .execute(
                "DELETE FROM automation_definition_revisions \
                 WHERE definition_id=?1 AND revision=1",
                [first.definition.id.to_string()],
            )
            .is_err());
        drop(connection);
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn two_writers_with_the_same_expected_revision_cannot_both_win() {
        let (database, path) = database_fixture("cas");
        let created = database
            .create_automation_definition(
                "Interaction",
                AutomationKind::Interaction,
                &TargetRef::All,
                &interaction_profile("base"),
            )
            .expect("create definition");
        drop(database);

        let barrier = Arc::new(Barrier::new(2));
        let handles = [1, 2].map(|value| {
            let database = Database::open(&path).expect("writer database");
            let barrier = Arc::clone(&barrier);
            let id = created.definition.id;
            std::thread::spawn(move || {
                barrier.wait();
                database.revise_automation_definition(
                    id,
                    1,
                    &TargetRef::All,
                    &interaction_profile(format!("writer-{value}")),
                )
            })
        });
        let results = handles.map(|handle| handle.join().expect("join writer"));
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let conflict = results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one conflict")
            .downcast_ref::<AutomationRevisionConflict>()
            .expect("typed conflict");
        assert_eq!((conflict.expected, conflict.actual), (1, 2));

        let database = Database::open(&path).expect("inspect database");
        assert!(database
            .get_automation_definition_revision(created.definition.id, 3)
            .expect("read absent revision")
            .is_none());
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn archived_definition_is_readable_but_cannot_be_revised() {
        let (database, path) = database_fixture("archive");
        let created = database
            .create_automation_definition(
                "Publish",
                AutomationKind::Publish,
                &TargetRef::All,
                &publish_profile("bundle-a"),
            )
            .expect("create definition");
        database
            .archive_automation_definition(created.definition.id)
            .expect("archive definition");

        let definition = database
            .get_automation_definition(created.definition.id)
            .expect("read archived definition")
            .expect("archived definition exists");
        assert!(definition.archived);
        assert!(database
            .list_automation_definitions(false)
            .expect("list active")
            .is_empty());
        assert_eq!(
            database
                .list_automation_definitions(true)
                .expect("list all")
                .len(),
            1
        );
        assert_eq!(
            database
                .get_automation_definition_revision(created.definition.id, 1)
                .expect("read archived revision"),
            Some(created.revision)
        );

        let error = database
            .revise_automation_definition(
                created.definition.id,
                1,
                &TargetRef::All,
                &publish_profile("bundle-b"),
            )
            .expect_err("archived definition cannot be revised");
        assert_eq!(
            error.downcast_ref::<AutomationDefinitionArchived>(),
            Some(&AutomationDefinitionArchived {
                definition_id: created.definition.id
            })
        );

        let missing_id = Uuid::new_v4();
        let error = database
            .archive_automation_definition(missing_id)
            .expect_err("missing archive");
        assert_eq!(
            error.downcast_ref::<AutomationDefinitionNotFound>(),
            Some(&AutomationDefinitionNotFound {
                definition_id: missing_id
            })
        );
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn create_and_revise_reject_secrets_before_any_row_is_written() {
        let (database, path) = database_fixture("secrets");
        let error = database
            .create_automation_definition(
                "Unsafe",
                AutomationKind::Nurture,
                &TargetRef::All,
                &json!({"nested": [{"PASSWORD": "do-not-store"}]}),
            )
            .expect_err("secret config");
        assert!(error.to_string().contains("$.nested[0].PASSWORD"));
        assert!(database
            .list_automation_definitions(true)
            .expect("list after rejected create")
            .is_empty());

        let created = database
            .create_automation_definition(
                "Safe",
                AutomationKind::Nurture,
                &TargetRef::All,
                &nurture_profile(20),
            )
            .expect("create safe definition");
        let error = database
            .revise_automation_definition(
                created.definition.id,
                1,
                &TargetRef::All,
                &json!({"items": [{"apiKEY": "do-not-store"}]}),
            )
            .expect_err("secret revision");
        assert!(error.to_string().contains("$.items[0].apiKEY"));
        assert_eq!(
            database
                .get_automation_definition(created.definition.id)
                .expect("read unchanged definition")
                .expect("definition exists")
                .latest_revision,
            1
        );
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn create_and_revise_reject_wrong_profile_kind_and_schema_before_writing() {
        let (database, path) = database_fixture("profile-schema");
        let interaction_config = json!({
            "schemaVersion": 1,
            "request": {
                "targets": [],
                "messageCount": 1,
                "instruction": "fixture",
                "maxWords": 12,
                "actions": { "like": true, "comment": false, "save": false }
            }
        });
        let wrong_kind = database
            .create_automation_definition(
                "Wrong kind",
                AutomationKind::Publish,
                &TargetRef::All,
                &interaction_config,
            )
            .expect_err("publish must reject interaction config");
        assert!(wrong_kind
            .to_string()
            .contains("invalid publish automation profile config"));
        assert!(database
            .list_automation_definitions(true)
            .expect("list after rejected create")
            .is_empty());

        let publish_config = json!({
            "schemaVersion": 1,
            "sourceRoot": "C:/fixture",
            "bundleIds": ["bundle-a"],
            "soundPolicy": { "kind": "default" },
            "executionConfirmed": true
        });
        let created = database
            .create_automation_definition(
                "Publish",
                AutomationKind::Publish,
                &TargetRef::All,
                &publish_config,
            )
            .expect("create valid publish profile");
        let wrong_schema = database
            .revise_automation_definition(
                created.definition.id,
                1,
                &TargetRef::All,
                &json!({
                    "schemaVersion": 2,
                    "sourceRoot": "C:/fixture",
                    "bundleIds": ["bundle-b"],
                    "soundPolicy": { "kind": "default" },
                    "executionConfirmed": true
                }),
            )
            .expect_err("schema v2 must be rejected");
        assert!(wrong_schema
            .to_string()
            .contains("automation profile schemaVersion must be 1"));
        assert_eq!(
            database
                .get_automation_definition(created.definition.id)
                .expect("read unchanged definition")
                .expect("definition exists")
                .latest_revision,
            1
        );
        assert!(database
            .get_automation_definition_revision(created.definition.id, 2)
            .expect("read absent revision")
            .is_none());
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn a_schedule_stays_pinned_until_an_explicit_cas_update_changes_its_revision() {
        let (database, path) = database_fixture("schedule-pin");
        let definition = database
            .create_automation_definition(
                "Nurture",
                AutomationKind::Nurture,
                &TargetRef::All,
                &nurture_profile(20),
            )
            .expect("create definition");
        let schedule = database
            .create_automation_schedule(
                "Every morning",
                definition.definition.id,
                1,
                true,
                &interval_schedule(60),
            )
            .expect("create schedule");

        database
            .revise_automation_definition(
                definition.definition.id,
                1,
                &TargetRef::All,
                &nurture_profile(30),
            )
            .expect("revise profile");
        assert_eq!(
            database
                .get_automation_schedule(schedule.id)
                .expect("read pinned schedule")
                .expect("schedule exists")
                .definition_revision,
            1
        );

        let updated = database
            .update_automation_schedule(
                schedule.id,
                1,
                "Every morning",
                definition.definition.id,
                2,
                true,
                &interval_schedule(60),
            )
            .expect("explicitly apply profile revision");
        assert_eq!(updated.definition_revision, 2);
        assert_eq!(updated.revision, 2);
        assert_eq!(
            database.list_automation_schedules().expect("list"),
            vec![updated]
        );

        let error = database
            .update_automation_schedule(
                schedule.id,
                1,
                "Stale writer",
                definition.definition.id,
                1,
                false,
                &interval_schedule(120),
            )
            .expect_err("stale schedule update");
        assert_eq!(
            error.downcast_ref::<AutomationScheduleConflict>(),
            Some(&AutomationScheduleConflict {
                expected: 1,
                actual: 2,
            })
        );
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn schedules_require_an_existing_exact_profile_revision() {
        let (database, path) = database_fixture("schedule-validation");
        let definition = database
            .create_automation_definition(
                "Interaction",
                AutomationKind::Interaction,
                &TargetRef::All,
                &interaction_profile("schedule"),
            )
            .expect("create definition");

        let missing_revision = database
            .create_automation_schedule(
                "Missing revision",
                definition.definition.id,
                2,
                true,
                &interval_schedule(60),
            )
            .expect_err("exact revision required");
        assert!(missing_revision
            .to_string()
            .contains("profile revision does not exist"));
        assert!(database
            .list_automation_schedules()
            .expect("list")
            .is_empty());
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn schedules_accept_only_v1_intervals_between_fifteen_minutes_and_one_day() {
        let (database, path) = database_fixture("schedule-schema");
        let definition = database
            .create_automation_definition(
                "Nurture",
                AutomationKind::Nurture,
                &TargetRef::All,
                &nurture_profile(20),
            )
            .expect("create definition");

        for invalid in [
            json!({"schemaVersion": 1, "kind": "interval", "everyMinutes": 14}),
            json!({"schemaVersion": 1, "kind": "interval", "everyMinutes": 1441}),
            json!({"schemaVersion": 2, "kind": "interval", "everyMinutes": 60}),
            json!({"schemaVersion": 1, "kind": "interval", "everyMinutes": 60, "cron": "*"}),
        ] {
            serde_json::from_value::<AutomationScheduleV1>(invalid)
                .expect_err("invalid interval schedule must be rejected");
        }
        for (index, minutes) in [15, 1_440].into_iter().enumerate() {
            database
                .create_automation_schedule(
                    &format!("Boundary {index}"),
                    definition.definition.id,
                    1,
                    false,
                    &interval_schedule(minutes),
                )
                .expect("valid boundary interval");
        }
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn duplicate_tick_and_restart_keep_one_pinned_occurrence_and_child_identity() {
        let (database, path) = database_fixture("schedule-occurrence");
        let definition = database
            .create_automation_definition(
                "Nurture",
                AutomationKind::Nurture,
                &TargetRef::All,
                &nurture_profile(20),
            )
            .expect("create definition");
        let schedule = database
            .create_automation_schedule(
                "Every hour",
                definition.definition.id,
                1,
                true,
                &interval_schedule(60),
            )
            .expect("create schedule");
        let scheduled_for = schedule.next_due_at.clone().expect("enabled due time");
        let occurrence = database
            .claim_automation_schedule_occurrence(
                schedule.id,
                schedule.revision,
                &scheduled_for,
                &scheduled_for,
                Some(&resolved_target("phone-1")),
                None,
            )
            .expect("claim due occurrence")
            .expect("claim owner");
        assert_eq!(occurrence.profile.revision, 1);
        assert_eq!(occurrence.state, AutomationScheduleOccurrenceState::Queued);
        let next_due = database
            .get_automation_schedule(schedule.id)
            .expect("reload schedule")
            .expect("schedule still exists")
            .next_due_at
            .expect("schedule is rearmed");
        assert!(database
            .list_due_automation_schedules(&next_due, 10)
            .expect("query next slot while occurrence remains active")
            .is_empty());

        let competing = Database::open(&path).expect("open competing scheduler");
        assert!(competing
            .claim_automation_schedule_occurrence(
                schedule.id,
                schedule.revision,
                &scheduled_for,
                &scheduled_for,
                Some(&resolved_target("phone-2")),
                None,
            )
            .expect("duplicate tick")
            .is_none());
        let dispatching = database
            .mark_automation_schedule_occurrence_dispatching(occurrence.id)
            .expect("arm occurrence")
            .expect("dispatch owner");
        assert_eq!(dispatching.child_campaign_id, occurrence.child_campaign_id);
        assert_eq!(dispatching.idempotency_key, occurrence.idempotency_key);
        drop(competing);
        drop(database);

        let restarted = Database::open(&path).expect("restart database");
        let recoverable = restarted
            .list_recoverable_automation_schedule_occurrences()
            .expect("list recoverable occurrences");
        assert_eq!(recoverable, vec![dispatching]);
        drop(restarted);
        cleanup(&path);
    }
}
