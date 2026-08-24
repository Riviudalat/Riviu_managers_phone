use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use crate::types::{JobRecord, JobStatus, JobStepRecord, StepStatus};

mod fleet;
mod flow_runs;
mod flows;
mod interaction;
mod inventory;
mod jobs;
mod migrations;
mod nurture;
mod publish;

pub use flow_runs::{AttemptTransitionPatch, FlowStateConflict};
pub(crate) use flow_runs::{FlowAttemptExecutionContext, FlowRecoveryRunContext};

/// Somewhere to keep a secret that is **not** the SQLite file.
///
/// The database is the app's SQLite file under `%APPDATA%`, opened with a plain
/// `Connection::open` — no SQLCipher, no key, and nothing hardening its ACL. Anything written
/// there is readable by any process running as the operator. That is fine for campaign rows and
/// device aliases; it is not fine for an API key that can spend money.
///
/// A trait rather than a direct dependency on `riviu-signing`: `crates/core` deliberately does
/// not know about the OS credential store, the same way it deliberately does not know about the
/// driver crates. The desktop supplies the keyring-backed implementation; tests supply an
/// in-memory one; a `Database` with no store at all keeps the old behaviour, which is what the
/// 38 test constructors rely on.
pub trait SecretStore: Send + Sync {
    fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>>;
    fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()>;
}

/// Name under which the AI API key lives in the secret store.
pub const SECRET_AI_API_KEY: &str = "nurture-ai-api-key";

pub struct Database {
    path: PathBuf,
    secrets: Option<std::sync::Arc<dyn SecretStore>>,
}

const NURTURE_SETTINGS_MIGRATION_V2: &str = "nurture.settings.migration.v2";
const NURTURE_SETTINGS_MIGRATION_V3: &str = "nurture.settings.migration.v3";
const NURTURE_SETTINGS_MIGRATION_V3_VALUE: &str = "2026-08-14-openrouter-luna";

impl Database {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Self {
            path,
            secrets: None,
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn default_path() -> anyhow::Result<PathBuf> {
        let base = dirs::data_dir().context("no data dir")?;
        Ok(base.join("riviu-managers-phone").join("riviu.db"))
    }

    /// Attach the place secrets live. Without one, secrets stay in the SQLite blob as before.
    pub fn with_secrets(mut self, store: std::sync::Arc<dyn SecretStore>) -> Self {
        self.secrets = Some(store);
        self
    }

    fn conn(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(&self.path)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(connection)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let mut conn = self.conn()?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        migrations::run(&mut conn)
    }
}

/// A value in the database does not fit the type the row is read into.
#[derive(Debug)]
struct ColumnOutOfRange {
    column: &'static str,
    value: i64,
    target: &'static str,
}

impl std::fmt::Display for ColumnOutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "column `{}` holds {}, which does not fit {}",
            self.column, self.value, self.target
        )
    }
}

impl std::error::Error for ColumnOutOfRange {}

/// Read an `i64` column into a narrower integer, refusing rather than wrapping.
///
/// Every call site here used to be `as u8`, `as u16` or `as u32`. That is not a conversion,
/// it is a truncation with no signal: a proxy port stored as 70000 came back as 4464 and the
/// app then dialled 4464; a thread of 256 messages came back with `message_count` 0 and read
/// as empty; an AI score above 255 wrapped into a plausible small number and was charted.
///
/// None of those can be caught downstream, because the wrapped value is a perfectly ordinary
/// value of the target type. The only place the truth still exists is right here, so this is
/// where it has to be checked — and a row that cannot be read honestly is an error, not a
/// number someone invented.
fn narrow<T>(value: i64, column: &'static str) -> rusqlite::Result<T>
where
    T: TryFrom<i64>,
{
    T::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(ColumnOutOfRange {
                column,
                value,
                target: std::any::type_name::<T>(),
            }),
        )
    })
}

fn publish_state_from_str(value: &str) -> crate::publish::PublishCampaignState {
    match value {
        "queued" => crate::publish::PublishCampaignState::Queued,
        "scheduled" => crate::publish::PublishCampaignState::Scheduled,
        "preparing" => crate::publish::PublishCampaignState::Preparing,
        "ready" => crate::publish::PublishCampaignState::Ready,
        "transferring" => crate::publish::PublishCampaignState::Transferring,
        "imported" => crate::publish::PublishCampaignState::Imported,
        "posting" => crate::publish::PublishCampaignState::Posting,
        "verifying" => crate::publish::PublishCampaignState::Verifying,
        "succeeded" => crate::publish::PublishCampaignState::Succeeded,
        "failed_before_dispatch" => crate::publish::PublishCampaignState::FailedBeforeDispatch,
        "uncertain" => crate::publish::PublishCampaignState::Uncertain,
        "cancelled" => crate::publish::PublishCampaignState::Cancelled,
        "missed" => crate::publish::PublishCampaignState::Missed,
        _ => crate::publish::PublishCampaignState::Uncertain,
    }
}

/// The group write itself, separated from the transaction that makes it atomic.
///
/// Split out so a test can hold the transaction open, drop it without committing, and prove
/// the old membership is still there — which on the previous autocommit version it was not.
fn write_group(
    transaction: &rusqlite::Transaction<'_>,
    group: &crate::types::DeviceGroup,
) -> anyhow::Result<()> {
    transaction.execute(
        r#"INSERT INTO groups (id, name, color, created_at) VALUES (?1,?2,?3,?4)
           ON CONFLICT(id) DO UPDATE SET name=excluded.name, color=excluded.color"#,
        params![group.id, group.name, group.color, group.created_at],
    )?;
    transaction.execute(
        "DELETE FROM group_members WHERE group_id = ?1",
        params![group.id],
    )?;
    for udid in &group.udids {
        transaction.execute(
            "INSERT OR IGNORE INTO group_members (group_id, udid) VALUES (?1,?2)",
            params![group.id, udid],
        )?;
    }
    Ok(())
}

/// Both deletes, which genuinely both have to happen: `group_members` has no foreign key to
/// `groups`, so removing the group alone would orphan its rows.
fn erase_group(transaction: &rusqlite::Transaction<'_>, id: &str) -> anyhow::Result<()> {
    transaction.execute("DELETE FROM group_members WHERE group_id = ?1", params![id])?;
    transaction.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
    Ok(())
}

fn interaction_campaign_state_label(
    state: crate::interaction::ThreadCampaignState,
) -> &'static str {
    match state {
        crate::interaction::ThreadCampaignState::Queued => "queued",
        crate::interaction::ThreadCampaignState::Running => "running",
        crate::interaction::ThreadCampaignState::Succeeded => "succeeded",
        crate::interaction::ThreadCampaignState::Partial => "partial",
        crate::interaction::ThreadCampaignState::Failed => "failed",
        crate::interaction::ThreadCampaignState::Cancelled => "cancelled",
    }
}

fn interaction_message_state_label(state: crate::interaction::ThreadMessageState) -> &'static str {
    match state {
        crate::interaction::ThreadMessageState::Queued => "queued",
        crate::interaction::ThreadMessageState::Preparing => "preparing",
        crate::interaction::ThreadMessageState::Ready => "ready",
        crate::interaction::ThreadMessageState::Sending => "sending",
        crate::interaction::ThreadMessageState::Succeeded => "succeeded",
        crate::interaction::ThreadMessageState::Failed => "failed",
        crate::interaction::ThreadMessageState::Uncertain => "uncertain",
        crate::interaction::ThreadMessageState::SkippedParent => "skipped_parent",
    }
}

fn interaction_message_state(value: &str) -> crate::interaction::ThreadMessageState {
    match value {
        "preparing" => crate::interaction::ThreadMessageState::Preparing,
        "ready" => crate::interaction::ThreadMessageState::Ready,
        "sending" => crate::interaction::ThreadMessageState::Sending,
        "succeeded" => crate::interaction::ThreadMessageState::Succeeded,
        "failed" => crate::interaction::ThreadMessageState::Failed,
        "uncertain" => crate::interaction::ThreadMessageState::Uncertain,
        "skipped_parent" => crate::interaction::ThreadMessageState::SkippedParent,
        _ => crate::interaction::ThreadMessageState::Queued,
    }
}

fn interaction_summary_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<crate::interaction::InteractionCampaignSummary> {
    let state: String = row.get(2)?;
    Ok(crate::interaction::InteractionCampaignSummary {
        id: row.get(0)?,
        request_id: row.get(1)?,
        state: match state.as_str() {
            "running" => crate::interaction::ThreadCampaignState::Running,
            "succeeded" => crate::interaction::ThreadCampaignState::Succeeded,
            "partial" => crate::interaction::ThreadCampaignState::Partial,
            "failed" => crate::interaction::ThreadCampaignState::Failed,
            "cancelled" => crate::interaction::ThreadCampaignState::Cancelled,
            _ => crate::interaction::ThreadCampaignState::Queued,
        },
        message_count: narrow(row.get::<_, i64>(3)?, "message_count")?,
        updated_at: row.get(4)?,
        // Index 5 is `c.error_code`, which sits between the plain columns and the three
        // counting subqueries in both SELECTs — so adding it shifted every index after it.
        // A test caught that; the shift was silent otherwise, because the counts and the
        // reason are all readable as the wrong type only sometimes.
        error_code: row.get(5)?,
        target_count: narrow(row.get::<_, i64>(6)?, "target_count")?,
        succeeded_messages: narrow(row.get::<_, i64>(7)?, "succeeded_messages")?,
        failed_messages: narrow(row.get::<_, i64>(8)?, "failed_messages")?,
        // Index 9, appended last on purpose — see the note above about what inserting a
        // column mid-list did the first time. Parsed leniently: a request blob this build
        // cannot read is a row that shows without a name, never a list that refuses to load.
        brief: row
            .get::<_, String>(9)
            .ok()
            .and_then(|json| {
                serde_json::from_str::<crate::interaction::ThreadCampaignRequest>(&json).ok()
            })
            .map(|request| crate::interaction::InteractionCampaignBrief::from_request(&request)),
    })
}

struct JobRow {
    id: String,
    script_name: String,
    udids_json: String,
    status: String,
    created_at: String,
    updated_at: String,
    steps_json: String,
    error: Option<String>,
}

impl JobRow {
    fn into_job(self) -> anyhow::Result<JobRecord> {
        Ok(JobRecord {
            id: Uuid::parse_str(&self.id)?,
            script_name: self.script_name,
            udids: serde_json::from_str(&self.udids_json)?,
            status: serde_json::from_str(&self.status).unwrap_or(JobStatus::Failed),
            created_at: DateTime::parse_from_rfc3339(&self.created_at)?.with_timezone(&Utc),
            updated_at: DateTime::parse_from_rfc3339(&self.updated_at)?.with_timezone(&Utc),
            steps: serde_json::from_str::<Vec<JobStepRecord>>(&self.steps_json).unwrap_or_default(),
            error: self.error,
        })
    }
}

pub fn step_label(status: &StepStatus) -> &'static str {
    match status {
        StepStatus::Pending => "pending",
        StepStatus::Running => "running",
        StepStatus::Succeeded => "succeeded",
        StepStatus::Failed => "failed",
        StepStatus::Skipped => "skipped",
    }
}

#[cfg(test)]
mod narrowing_tests {
    use super::*;

    /// A stored value that does not fit is an error, not a different number.
    ///
    /// Every one of these columns used to be read with `as u16` / `as u8` / `as u32`. The
    /// failure that produces is the worst kind available: the wrapped result is a perfectly
    /// ordinary value of the target type, so nothing downstream can tell it apart from a real
    /// one, and the only place the truth still exists is the row itself.
    #[test]
    fn a_value_too_large_for_its_column_type_is_refused_not_wrapped() {
        // 70000 as u16 is 4464 — a real port number, and the wrong one. The app would have
        // dialled it and reported a connection failure against a port nobody configured.
        assert_eq!(70000_i64 as u16, 4464, "the wrap this replaces");
        assert!(narrow::<u16>(70000, "port").is_err());
        assert_eq!(narrow::<u16>(8080, "port").ok(), Some(8080_u16));

        // 256 as u8 is 0 — a thread of 256 messages read back as an empty thread.
        assert_eq!(256_i64 as u8, 0, "the wrap this replaces");
        assert!(narrow::<u8>(256, "message_count").is_err());
        assert_eq!(narrow::<u8>(255, "message_count").ok(), Some(255_u8));

        // And negatives, which `as` turns into very large numbers rather than refusing.
        assert!(narrow::<u32>(-1, "target_count").is_err());
        assert!(narrow::<u16>(-1, "port").is_err());
    }

    #[test]
    fn the_refusal_says_which_column_and_what_it_held() {
        // A row that cannot be read is only actionable if the message names the column: the
        // operator sees it as "could not load proxies", and the cause is one row's port.
        let message = narrow::<u16>(70000, "port").unwrap_err().to_string();
        assert!(message.contains("port"), "{message}");
        assert!(message.contains("70000"), "{message}");
    }

    #[test]
    fn a_proxy_row_with_an_impossible_port_fails_the_read_instead_of_inventing_one() {
        let path = std::env::temp_dir().join(format!("riviu-narrow-test-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).expect("open fixture database");
        db.conn()
            .expect("connection")
            .execute(
                "INSERT INTO proxies (id,name,proxy_type,host,port,username,password,notes)
                 VALUES ('p1','bad','http','127.0.0.1',70000,'','','')",
                [],
            )
            .expect("insert a row no UI would produce but a migration or a hand edit could");

        let read = db.list_proxies();
        assert!(
            read.is_err(),
            "70000 came back as {:?}",
            read.map(|v| v.len())
        );
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-group-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn group(id: &str, udids: &[&str]) -> crate::types::DeviceGroup {
        crate::types::DeviceGroup {
            id: id.into(),
            name: format!("nhóm {id}"),
            color: "#ff6a00".into(),
            udids: udids.iter().map(|udid| (*udid).to_string()).collect(),
            created_at: "2026-08-17T00:00:00Z".into(),
        }
    }

    #[test]
    fn a_membership_write_that_does_not_commit_leaves_the_group_as_it_was() {
        // The defect: the membership rewrite is delete-everything-then-rebuild, and it ran
        // in autocommit -- so the DELETE was durable the moment it returned and anything
        // going wrong in the insert loop left the group saved as empty. Adding one phone to
        // a group could erase it.
        let (db, path) = fixture();
        db.upsert_group(&group("g1", &["a", "b", "c"]))
            .expect("seed the group");

        {
            let mut conn = db.conn().expect("connection");
            let transaction = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("begin");
            write_group(&transaction, &group("g1", &["z"])).expect("write inside the txn");
            // Dropped without committing -- the failure mid-loop, modelled exactly.
        }

        let groups = db.list_groups().expect("list");
        let found = groups
            .iter()
            .find(|g| g.id == "g1")
            .expect("group survives");
        assert_eq!(found.udids.len(), 3, "membership must be untouched");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_delete_that_does_not_commit_leaves_both_the_group_and_its_members() {
        let (db, path) = fixture();
        db.upsert_group(&group("g2", &["a", "b"]))
            .expect("seed the group");

        {
            let mut conn = db.conn().expect("connection");
            let transaction = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("begin");
            erase_group(&transaction, "g2").expect("erase inside the txn");
        }

        let groups = db.list_groups().expect("list");
        let found = groups
            .iter()
            .find(|g| g.id == "g2")
            .expect("group survives");
        assert_eq!(found.udids.len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_saved_group_replaces_its_membership_exactly() {
        // The happy path, so the refactor cannot quietly stop replacing.
        let (db, path) = fixture();
        db.upsert_group(&group("g3", &["a", "b", "c"]))
            .expect("seed");
        db.upsert_group(&group("g3", &["b", "d"])).expect("replace");

        let groups = db.list_groups().expect("list");
        let found = groups.iter().find(|g| g.id == "g3").expect("group");
        let mut udids = found.udids.clone();
        udids.sort();
        assert_eq!(udids, vec!["b".to_string(), "d".to_string()]);

        db.delete_group("g3").expect("delete");
        assert!(db.list_groups().expect("list").iter().all(|g| g.id != "g3"));
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod device_meta_tests {
    use super::*;

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-device-meta-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn meta(udid: &str) -> crate::types::DeviceMeta {
        crate::types::DeviceMeta {
            udid: udid.into(),
            notes: String::new(),
            tags: vec![],
            group_id: None,
            proxy_id: None,
            handle: String::new(),
            alias: String::new(),
            number: None,
        }
    }

    #[test]
    fn an_alias_and_a_number_survive_a_write_and_a_reopen() {
        let (db, path) = fixture();
        db.upsert_device_meta(&crate::types::DeviceMeta {
            alias: "Máy kệ trên, cột 3".into(),
            number: Some(21),
            handle: "riviu.demo".into(),
            ..meta("10969614")
        })
        .expect("write");

        let read = db.get_device_meta("10969614").expect("read");
        assert_eq!(read.alias, "Máy kệ trên, cột 3");
        assert_eq!(read.number, Some(21));
        // The neighbouring column, because the update statement lists every column by hand
        // and the way that breaks is by overwriting the one nobody looked at.
        assert_eq!(read.handle, "riviu.demo");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn an_unnumbered_phone_reads_back_as_unnumbered_rather_than_zero() {
        // `None` and `Some(0)` are different facts: the grid falls back to a tile's position
        // for the first and would print "0" for the second.
        let (db, path) = fixture();
        db.upsert_device_meta(&meta("ce0417145199e0490c"))
            .expect("write");
        assert_eq!(
            db.get_device_meta("ce0417145199e0490c")
                .expect("read")
                .number,
            None
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_phone_with_no_record_answers_with_an_empty_one_instead_of_failing() {
        let (db, path) = fixture();
        let read = db.get_device_meta("never-seen").expect("read");
        assert_eq!(read.udid, "never-seen");
        assert_eq!(read.alias, "");
        assert_eq!(read.number, None);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn listing_returns_only_the_phones_that_have_a_record() {
        // What the grid reads once per refresh. Rows exist only for edited phones, so a fleet
        // nobody has renamed answers empty and every tile keeps the name the phone reports.
        let (db, path) = fixture();
        assert!(db.list_device_metas().expect("empty list").is_empty());
        db.upsert_device_meta(&crate::types::DeviceMeta {
            number: Some(2),
            ..meta("b")
        })
        .expect("write b");
        db.upsert_device_meta(&crate::types::DeviceMeta {
            number: Some(1),
            ..meta("a")
        })
        .expect("write a");

        let listed = db.list_device_metas().expect("list");
        assert_eq!(listed.len(), 2);
        let mut numbered: Vec<(String, Option<u32>)> = listed
            .into_iter()
            .map(|row| (row.udid, row.number))
            .collect();
        numbered.sort();
        assert_eq!(
            numbered,
            vec![("a".to_string(), Some(1)), ("b".to_string(), Some(2))]
        );
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod agent_settings_tests {
    use super::*;
    use crate::types::AgentSettings;

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-agent-settings-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    #[test]
    fn agent_settings_round_trip_without_secret_fields() {
        let (db, path) = fixture();
        let settings = AgentSettings { auto_repair: false };

        db.save_agent_settings(&settings).expect("save settings");

        assert_eq!(db.get_agent_settings().expect("load settings"), settings);
        let raw = db
            .get_setting("agent.settings.v1")
            .expect("read raw setting")
            .expect("stored setting");
        assert_eq!(raw, r#"{"autoRepair":false}"#);
        assert!(!raw.to_ascii_lowercase().contains("token"));
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn invalid_agent_settings_json_is_not_silently_defaulted() {
        let (db, path) = fixture();
        db.set_setting("agent.settings.v1", "{not-json")
            .expect("store malformed fixture");

        let error = db
            .get_agent_settings()
            .expect_err("malformed settings must fail");

        assert!(error.to_string().contains("agent.settings.v1"));
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod stream_settings_tests {
    use super::*;
    use crate::types::{StreamQuality, StreamSettings};

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-stream-settings-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    #[test]
    fn an_untouched_install_reads_the_defaults_rather_than_failing() {
        let (db, path) = fixture();
        assert_eq!(
            db.get_stream_settings()
                .expect("absent key is not an error"),
            StreamSettings::default()
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn stream_settings_survive_a_restart() {
        // The whole point of the change: quality and frame rate used to live only in an
        // `Arc<RwLock<_>>` built from `Default` at bootstrap, so every setting the operator
        // chose was gone the next time the app opened.
        let (db, path) = fixture();
        let chosen = StreamSettings {
            fps: 18,
            grid_quality: StreamQuality::Extra,
            focus_quality: StreamQuality::Low,
        };

        db.save_stream_settings(&chosen).expect("save");

        assert_eq!(db.get_stream_settings().expect("load"), chosen);
        let raw = db
            .get_setting("stream.settings.v1")
            .expect("read raw setting")
            .expect("stored setting");
        assert_eq!(
            raw,
            r#"{"fps":18,"gridQuality":"extra","focusQuality":"low"}"#
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn a_blob_missing_a_field_still_loads() {
        // `#[serde(default)]` earns its place here rather than in review: the load runs at
        // startup, so without it the first field ever added to this struct would turn every
        // existing install's stored row into a failure to boot.
        let (db, path) = fixture();
        db.set_setting("stream.settings.v1", r#"{"fps":12}"#)
            .expect("store a blob from an older build");

        let loaded = db
            .get_stream_settings()
            .expect("a partial blob still loads");

        assert_eq!(loaded.fps, 12);
        assert_eq!(loaded.grid_quality, StreamSettings::default().grid_quality);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn invalid_stream_settings_json_is_not_silently_defaulted() {
        let (db, path) = fixture();
        db.set_setting("stream.settings.v1", "{not-json")
            .expect("store malformed fixture");

        let error = db
            .get_stream_settings()
            .expect_err("malformed settings must fail");

        assert!(error.to_string().contains("stream.settings.v1"));
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod nurture_settings_migration_tests {
    use super::*;
    use crate::types::NurtureSettings;

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-nurture-settings-migration-test-{}.db",
            Uuid::new_v4()
        ));
        (Database::open(&path).expect("open fixture database"), path)
    }

    #[test]
    fn stored_legacy_profile_is_migrated_once_and_obsolete_keys_are_removed() {
        let (db, path) = fixture();
        let legacy = serde_json::json!({
            "baseUrl": "https://api.deepseek.com",
            "model": "custom-model",
            "apiKey": "fixture-key",
            // Kept deliberately after the prices were removed from the struct: this is the
            // proof that a blob stored by an older build still loads. `NurtureSettings` has
            // no `deny_unknown_fields`, so serde drops them.
            "inputPricePer1m": 1.25,
            "outputPricePer1m": 10.0,
            "bundleId": "com.ss.iphone.ugc.Ame",
            "numVideos": 50,
            "numRounds": 1,
            "likeProb": 40,
            "commentProb": 25,
            "followProb": 5,
            "frenzyProb": 8,
            "watchMin": 5.0,
            "watchMax": 20.0,
            "persona": "custom-persona",
            "fatigue": true,
            "timeOfDay": true,
            "pauseSwipe": true,
            "nightStart": 0,
            "nightEnd": 0,
            "recoverDelayMin": 2,
            "recoverDelayMax": 4,
            "staggerDelayMin": 5,
            "staggerDelayMax": 15,
            "commentLang": "vi",
            "aiDirections": "custom",
            "maxCommentWords": 12,
            "riskGuardEnabled": true,
            "riskMaxLikes": 10,
            "scheduleEnabled": false,
            "scheduleEveryMinutes": 60,
            "scheduleDurationMinutes": 20,
            "scheduleUdids": ["fixture-device"]
        });
        db.set_setting("nurture.settings", &legacy.to_string())
            .expect("store legacy profile");

        let migrated = db.get_nurture_settings().expect("load migrated profile");
        assert_eq!(migrated.num_videos, 120);
        assert_eq!(migrated.like_prob, 35);
        assert_eq!(migrated.comment_prob, 0);
        assert_eq!(migrated.follow_prob, 3);
        assert_eq!(migrated.frenzy_prob, 6);
        assert_eq!((migrated.watch_min, migrated.watch_max), (3.0, 18.0));
        assert_eq!(migrated.schedule_every_minutes, 240);
        assert_eq!(migrated.schedule_duration_minutes, 150);
        assert_eq!(migrated.api_key, "fixture-key");
        assert_eq!(migrated.model, "custom-model");
        assert_eq!(migrated.persona, "custom-persona");
        assert_eq!(migrated.schedule_udids, vec!["fixture-device"]);

        let raw = db
            .get_setting("nurture.settings")
            .expect("read normalized profile")
            .expect("normalized profile exists");
        assert!(!raw.contains("riskGuard"));
        assert_eq!(
            db.get_setting(NURTURE_SETTINGS_MIGRATION_V2)
                .expect("read migration marker")
                .as_deref(),
            Some("2026-08-06-human-v2")
        );

        // A second read does not reapply migration or alter the normalized
        // profile, which keeps manual edits stable after the first launch.
        let second = db
            .get_nurture_settings()
            .expect("reload normalized profile");
        assert_eq!(second.num_videos, migrated.num_videos);
        assert_eq!(second.comment_prob, migrated.comment_prob);
        assert_eq!(second.api_key, migrated.api_key);
        assert_eq!(second.schedule_udids, migrated.schedule_udids);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn saving_a_new_profile_marks_it_as_already_migrated() {
        let (db, path) = fixture();
        let settings = NurtureSettings::default();
        db.save_nurture_settings(&settings).expect("save profile");
        assert_eq!(
            db.get_setting(NURTURE_SETTINGS_MIGRATION_V2)
                .expect("read migration marker")
                .as_deref(),
            Some("2026-08-06-human-v2")
        );
        assert_eq!(
            db.get_setting(NURTURE_SETTINGS_MIGRATION_V3)
                .expect("read v3 marker")
                .as_deref(),
            Some(NURTURE_SETTINGS_MIGRATION_V3_VALUE)
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn a_v2_row_still_on_shipped_deepseek_moves_to_openrouter_luna_once() {
        let (db, path) = fixture();
        let shipped = NurtureSettings {
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-v4-flash".into(),
            api_key: "replace-me".into(),
            ..Default::default()
        };
        db.set_setting(
            "nurture.settings",
            &serde_json::to_string(&shipped).unwrap(),
        )
        .expect("store shipped DeepSeek row");
        db.set_setting(NURTURE_SETTINGS_MIGRATION_V2, "2026-08-06-human-v2")
            .expect("mark v2 already applied");

        let migrated = db.get_nurture_settings().expect("load remapped profile");
        assert_eq!(migrated.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(migrated.model, "openai/gpt-5.6-luna");
        assert_eq!(migrated.api_key, "replace-me");
        assert_eq!(
            db.get_setting(NURTURE_SETTINGS_MIGRATION_V3)
                .expect("read v3 marker")
                .as_deref(),
            Some(NURTURE_SETTINGS_MIGRATION_V3_VALUE)
        );

        let mut stayed = migrated.clone();
        stayed.base_url = "https://api.deepseek.com".into();
        stayed.model = "deepseek-v4-flash".into();
        db.save_nurture_settings(&stayed)
            .expect("operator put DeepSeek back");
        let second = db
            .get_nurture_settings()
            .expect("reload after manual revert");
        assert_eq!(second.base_url, "https://api.deepseek.com");
        assert_eq!(second.model, "deepseek-v4-flash");
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::interaction::{
        plan_threads, PreparedThreadMessage, ResolvedTikTokTarget, ThreadCampaignRequest,
        ThreadCampaignState, ThreadMessageState, ThreadMode, TikTokPostKind,
    };

    fn fixture() -> (Database, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("riviu-interaction-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn request() -> ThreadCampaignRequest {
        ThreadCampaignRequest {
            request_id: "interaction-db-1".into(),
            targets: vec![ResolvedTikTokTarget {
                original_url: "https://www.tiktok.com/@creator/video/123".into(),
                normalized_url: "https://www.tiktok.com/@creator/video/123".into(),
                target_key: "content:123".into(),
                content_id: "123".into(),
                author: "creator".into(),
                kind: TikTokPostKind::Video,
            }],
            actor_udids: vec!["actor-a".into(), "actor-b".into()],
            message_count: 2,
            instruction: "tự nhiên".into(),
            max_words: 12,
            // Both default on the wire; a fixture spells them out so the shape stays
            // visible and a new field cannot be forgotten silently.
            manual_comments: Vec::new(),
            like_target: false,

            mode: ThreadMode::Threaded,
            shape: crate::interaction::ThreadShape::Chain,
            cohort_size: None,
            mentions: Vec::new(),
        }
    }

    /// A thread across the whole farm has to fit the schema, and until migration 14 it did
    /// not.
    ///
    /// Validation demands `message_count >= the largest cohort`, so one cohort over the
    /// fourteen phones on this box needs fourteen messages — and the table carried
    /// `CHECK (message_count BETWEEN 2 AND 6)`, so every whole-fleet campaign died in this
    /// function as a CHECK violation the operator saw as `OperationFailed`. The engine and
    /// the UI had allowed 2..=64 for months, which is why nothing above this layer caught it.
    ///
    /// Both the real fleet size and the engine's own ceiling are pinned: a bound that only
    /// holds for the number of phones plugged in today is not a bound.
    #[test]
    fn a_whole_fleet_thread_fits_the_relaxed_schema() {
        for actor_count in [14_usize, crate::interaction::MAX_ACTOR_COUNT] {
            let (db, path) = fixture();
            let mut request = request();
            request.request_id = format!("fleet-{actor_count}");
            request.actor_udids = (0..actor_count)
                .map(|index| format!("udid-{index}"))
                .collect();
            request.message_count = u8::try_from(actor_count).expect("fleet fits a u8");
            let plan = plan_threads(&request).expect("plan a single cohort over the fleet");

            let campaign_id = db
                .create_interaction_campaign(&request, &plan)
                .unwrap_or_else(|error| panic!("{actor_count} actors must persist, got {error:#}"));
            let detail = db
                .get_interaction_campaign(&campaign_id)
                .expect("read back")
                .expect("campaign exists");
            assert_eq!(detail.summary.message_count as usize, actor_count);
            assert_eq!(detail.assignments.len(), actor_count);
            // The last ordinal is the one the old `message_ordinal BETWEEN 0 AND 5` refused.
            assert_eq!(
                detail
                    .assignments
                    .iter()
                    .map(|assignment| assignment.ordinal)
                    .max(),
                Some(u8::try_from(actor_count - 1).expect("fits a u8"))
            );
            drop(db);
            let _ = std::fs::remove_file(&path);
        }
    }

    /// A campaign row has to say what it was, on both read paths.
    ///
    /// The Monitor tab could name a campaign only by a slice of its UUID, so a list of runs
    /// against three different posts read as three identical rows. Everything needed was in
    /// `request_json` and no query selected it.
    #[test]
    fn the_list_row_names_the_campaign_it_summarises() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");

        for (source, summary) in [
            (
                "list",
                db.list_interaction_campaigns(10)
                    .expect("list")
                    .into_iter()
                    .next()
                    .expect("one campaign"),
            ),
            (
                "get",
                db.get_interaction_campaign(&campaign_id)
                    .expect("get")
                    .expect("exists")
                    .summary,
            ),
        ] {
            let brief = summary
                .brief
                .unwrap_or_else(|| panic!("{source} must carry a brief"));
            assert_eq!(brief.first_author.as_deref(), Some("creator"), "{source}");
            assert_eq!(brief.first_content_id.as_deref(), Some("123"), "{source}");
            assert_eq!(brief.actor_count, 2, "{source}");
            assert_eq!(brief.mode, ThreadMode::Threaded, "{source}");
            assert!(!brief.manual, "{source}");
        }
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// A request blob this build cannot read must cost the name, not the row.
    ///
    /// The brief is parsed at read time, so a payload from a future build — or a corrupted
    /// one — reaches the same code path as a good one. Refusing there would make the whole
    /// Monitor list fail to load over a single bad campaign.
    #[test]
    fn a_summary_with_unreadable_request_json_still_lists() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        db.conn()
            .expect("connection")
            .execute(
                "UPDATE interaction_campaigns SET request_json='hỏng' WHERE id=?1",
                params![campaign_id],
            )
            .expect("corrupt the stored request");

        let listed = db.list_interaction_campaigns(10).expect("list");
        assert_eq!(listed.len(), 1, "the row must still be listed");
        assert!(
            listed[0].brief.is_none(),
            "an unreadable request has no name to show"
        );
        assert_eq!(listed[0].id, campaign_id);
        assert_eq!(listed[0].message_count, 2, "the real columns still read");
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// A campaign the app was killed in the middle of has to stop calling itself running.
    ///
    /// The worker is a `tokio::spawn` in the app process: once the app is gone there is
    /// nothing to finish it, and the Monitor draws `running` with a Dừng button and no Retry.
    /// Measured 24/08/2026 — a `tauri dev` rebuild restarted the app mid-campaign and left
    /// exactly this: a row frozen at "Đang chạy", rows frozen at "Đang gửi".
    #[test]
    fn a_campaign_whose_worker_died_is_closed_out_and_stays_safe_to_retry() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        db.update_interaction_campaign_state(&campaign_id, ThreadCampaignState::Running, None)
            .expect("start it");
        let detail = db
            .get_interaction_campaign(&campaign_id)
            .expect("get")
            .expect("exists");
        let (sending, waiting) = (&detail.assignments[0], &detail.assignments[1]);
        db.update_interaction_assignment_state(
            &sending.id,
            ThreadMessageState::Sending,
            None,
            Some("post_comment"),
            None,
        )
        .expect("one message was in flight");

        assert_eq!(
            db.interrupt_orphaned_interaction_campaigns()
                .expect("sweep"),
            1
        );

        let after = db
            .get_interaction_campaign(&campaign_id)
            .expect("get")
            .expect("exists");
        assert_eq!(after.summary.state, ThreadCampaignState::Cancelled);
        assert!(
            after
                .summary
                .error_code
                .as_deref()
                .is_some_and(|reason| reason.contains("interaction_worker_lost")),
            "the row has to say why it stopped, or it reads as an operator cancelling it"
        );

        let by_id = |id: &str| {
            after
                .assignments
                .iter()
                .find(|assignment| assignment.id == id)
                .expect("assignment survives")
                .state
        };
        // The safety-critical half: a message whose Send tap went out with no confirmation is
        // `Uncertain`, and `Uncertain` is permanently excluded from retry — so a comment that
        // may already be public can never be posted a second time.
        assert_eq!(by_id(&sending.id), ThreadMessageState::Uncertain);
        assert!(
            !crate::interaction_campaign::retryable_assignments(&after.assignments, None)
                .contains(&sending.id),
            "an interrupted send must never become retryable"
        );
        // A message that never touched the device is untouched, and still repairable.
        assert_eq!(by_id(&waiting.id), ThreadMessageState::Queued);
        assert!(
            crate::interaction_campaign::retryable_assignments(&after.assignments, None)
                .contains(&waiting.id)
        );

        // Idempotent: a second start-up finds nothing left to close.
        assert_eq!(
            db.interrupt_orphaned_interaction_campaigns()
                .expect("sweep again"),
            0
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    /// A campaign with live comments under it must never be filed as a total loss.
    ///
    /// The final state used to be decided from the counters of the pass that had just run,
    /// and on a retry those count only the messages the retry touched. Measured 24/08/2026 on
    /// the fleet: five comments were already public, the retry's eight all failed, so
    /// `succeeded == 0` for the pass and the row was written `Failed`. The summary the row is
    /// drawn from said `5/14` at the same moment.
    ///
    /// Pinned here rather than in the runner because the runner needs a device; what the
    /// runner now reads is exactly this projection of the assignment states.
    #[test]
    fn a_retry_that_lands_nothing_still_reports_the_comments_that_are_live() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        let detail = db
            .get_interaction_campaign(&campaign_id)
            .expect("get")
            .expect("exists");
        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Succeeded,
            None,
            None,
            None,
        )
        .expect("one comment is live");
        db.update_interaction_assignment_state(
            &detail.assignments[1].id,
            ThreadMessageState::Failed,
            Some("reply_parent_not_found: …"),
            None,
            None,
        )
        .expect("the other is not");

        let after = db
            .get_interaction_campaign(&campaign_id)
            .expect("get")
            .expect("exists");
        assert_eq!(after.summary.succeeded_messages, 1);
        assert_eq!(after.summary.failed_messages, 1);
        // "Some of it worked" is the only honest reading of that pair, whichever pass
        // produced it.
        assert!(
            after.summary.succeeded_messages > 0 && after.summary.failed_messages > 0,
            "the projection the runner reads has to see both, or Partial is unreachable"
        );
        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_campaign_summary_carries_the_error_code_that_ended_it() {
        // The column was written from the start and selected by nobody, so a live AI failure
        // put the whole reason in the row and the operator's only signal was the word
        // "Lỗi" (AGENTS.md 9.33). Both read paths are pinned: the list and the detail, since
        // they are two separate SELECTs and fixing one would have left the other blind.
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");

        assert_eq!(
            db.get_interaction_campaign(&campaign_id)
                .expect("get")
                .expect("exists")
                .summary
                .error_code,
            None,
            "a campaign that has not failed must not invent a reason"
        );

        let reason = "ai_comment_unavailable: ordinal 0 — HTTP 400: Model Not Exist";
        db.update_interaction_campaign_state(
            &campaign_id,
            crate::interaction::ThreadCampaignState::Failed,
            Some(reason),
        )
        .expect("fail the campaign");

        assert_eq!(
            db.get_interaction_campaign(&campaign_id)
                .expect("get")
                .expect("exists")
                .summary
                .error_code
                .as_deref(),
            Some(reason)
        );
        let listed = db.list_interaction_campaigns(10).expect("list");
        let found = listed
            .iter()
            .find(|summary| summary.id == campaign_id)
            .expect("the failed campaign is listed");
        assert_eq!(found.error_code.as_deref(), Some(reason));
        drop(path);
    }

    #[test]
    fn interaction_campaign_persists_plan_prepared_text_and_evidence_atomically() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        let detail = db
            .get_interaction_campaign(&campaign_id)
            .expect("get campaign")
            .expect("campaign exists");
        assert_eq!(detail.assignments.len(), 2);
        let first = &plan.assignments[0];
        let prepared = PreparedThreadMessage::new(first, "  món này   nhìn ngon quá ");
        db.prepare_interaction_assignment(&detail.assignments[0].id, &prepared)
            .expect("prepare");
        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Sending,
            None,
            Some("post_comment"),
            None,
        )
        .expect("intent");
        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Succeeded,
            None,
            None,
            Some(r#"{"armedFrameSha256":"a","clearedFrameSha256":"b"}"#),
        )
        .expect("evidence");
        db.add_interaction_artifact(
            &campaign_id,
            "content:123",
            Some(&detail.assignments[0].id),
            "comment-root-evidence",
            r#"{"fixture":true}"#,
            "fixture-sha",
            Some("campaign/assignment/attempt/artifact.jpeg"),
        )
        .expect("artifact");
        let loaded = db
            .get_interaction_campaign_request(&campaign_id)
            .expect("request")
            .expect("request exists");
        assert_eq!(loaded.0.request_id, request.request_id);
        let updated = db
            .get_interaction_campaign(&campaign_id)
            .expect("updated")
            .expect("updated exists");
        assert_eq!(updated.assignments[0].state, ThreadMessageState::Succeeded);
        assert_eq!(
            updated.assignments[0].prepared_text.as_deref(),
            Some("món này nhìn ngon quá")
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }

    /// A message that was meant to post and did not has to show up in the
    /// campaign totals.
    ///
    /// `skipped_parent` was counted in neither bucket. It is not a rare state:
    /// one message whose identity cannot be read makes every later message in
    /// that thread `skipped_parent`, so a six-message campaign could report
    /// "1 succeeded, 0 failed" with five silently dropped — the one number an
    /// operator reads to decide whether anything went wrong.
    #[test]
    fn a_skipped_parent_message_is_counted_as_not_delivered() {
        let (db, path) = fixture();
        let request = request();
        let plan = plan_threads(&request).expect("plan");
        let campaign_id = db
            .create_interaction_campaign(&request, &plan)
            .expect("create campaign");
        let detail = db
            .get_interaction_campaign(&campaign_id)
            .expect("detail")
            .expect("campaign exists");

        db.update_interaction_assignment_state(
            &detail.assignments[0].id,
            ThreadMessageState::Succeeded,
            None,
            None,
            None,
        )
        .expect("mark succeeded");
        db.update_interaction_assignment_state(
            &detail.assignments[1].id,
            ThreadMessageState::SkippedParent,
            Some("parent_identity_not_confirmed"),
            None,
            None,
        )
        .expect("mark skipped");

        let summary = db
            .get_interaction_campaign(&campaign_id)
            .expect("summary")
            .expect("campaign exists")
            .summary;
        assert_eq!(summary.succeeded_messages, 1);
        assert_eq!(
            summary.failed_messages, 1,
            "a skipped message is a message that did not post"
        );

        let listed = db
            .list_interaction_campaigns(10)
            .expect("list")
            .into_iter()
            .find(|item| item.id == campaign_id)
            .expect("campaign listed");
        assert_eq!(
            listed.failed_messages, 1,
            "the list view must agree with the detail view"
        );
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod publish_tests {
    use super::*;
    use crate::publish::{
        PublishBundle, PublishCampaignRequest, PublishCleanupPolicy, PublishImage,
        PublishMediaKind, PublishVisibility,
    };

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-publish-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn bundle(id: &str, ordinal: usize) -> PublishBundle {
        PublishBundle {
            id: id.into(),
            source_path: format!("/fixture/{id}"),
            name: format!("bundle-{ordinal}"),
            media_kind: PublishMediaKind::Image,
            images: vec![PublishImage {
                path: format!("/fixture/{id}/01.png"),
                file_name: "01.png".into(),
                order: 1,
                sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                byte_len: 3,
                width: 1,
                height: 1,
            }],
            caption_path: format!("/fixture/{id}/caption.txt"),
            caption: format!("caption {ordinal}"),
            caption_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into(),
            total_bytes: 3,
        }
    }

    #[test]
    fn publish_campaign_persists_mapping_hash_manifest_and_revision_events() {
        let (db, path) = fixture();
        let request = PublishCampaignRequest {
            request_id: "publish-db-1".into(),
            source_root: "/fixture/root".into(),
            bundle_ids: vec!["bundle-a".into(), "bundle-b".into()],
            udids: vec!["phone-a".into(), "phone-b".into()],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        };
        let campaign = db
            .create_publish_campaign(&request, &[bundle("bundle-a", 0), bundle("bundle-b", 1)])
            .expect("create campaign");
        assert_eq!(campaign.state, crate::publish::PublishCampaignState::Queued);
        assert_eq!(campaign.assignments[1].udid, "phone-b");
        assert_eq!(
            db.list_publish_campaigns(10).expect("list")[0]
                .assignments
                .len(),
            2
        );

        let detail = db
            .get_publish_campaign(&campaign.id)
            .expect("get campaign")
            .expect("campaign exists");
        assert_eq!(detail.bundles.len(), 2);
        assert_eq!(detail.bundles[0].caption_sha256.len(), 64);
        assert_eq!(detail.assignments.len(), 2);
        assert_eq!(detail.events.len(), 1);

        db.update_publish_campaign_state(
            &campaign.id,
            crate::publish::PublishCampaignState::Ready,
            None,
        )
        .expect("state event");
        let updated = db
            .get_publish_campaign(&campaign.id)
            .expect("reload")
            .expect("campaign exists");
        assert_eq!(
            updated.campaign.state,
            crate::publish::PublishCampaignState::Ready
        );
        assert_eq!(updated.events.len(), 2);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn publish_campaign_rejects_duplicate_device_mapping() {
        let (db, path) = fixture();
        let request = PublishCampaignRequest {
            request_id: "publish-db-duplicate".into(),
            source_root: "/fixture/root".into(),
            bundle_ids: vec!["bundle-a".into(), "bundle-b".into()],
            udids: vec!["phone-a".into(), "phone-a".into()],
            run_at: None,
            visibility: PublishVisibility::Public,
            cleanup_policy: PublishCleanupPolicy::DeleteImportedAssetsAfterVerified,
        };
        let error = db
            .create_publish_campaign(&request, &[bundle("bundle-a", 0), bundle("bundle-b", 1)])
            .expect_err("duplicate UDID must be rejected");
        assert!(error.to_string().contains("duplicate UDID"));
        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod schedule_tests {
    use super::*;

    fn fixture() -> (Database, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-schedule-test-{}.db", Uuid::new_v4()));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn schedule(script: &str) -> crate::types::ScheduleItem {
        crate::types::ScheduleItem {
            id: "sched-1".into(),
            name: "hourly".into(),
            script_name: script.into(),
            udids: vec!["phone-a".into()],
            every_minutes: 60,
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            last_error: None,
        }
    }

    #[test]
    fn a_schedule_can_record_why_it_did_not_run() {
        // The runner used to advance `last_run_at` on every tick whether or not anything
        // was enqueued, and a missing script produced no record anywhere -- both guards
        // around the lookup and the parse fell through in silence. There was nowhere to
        // write the reason even if someone had wanted to; migration 8 makes the column,
        // and this is the round trip that keeps it wired.
        let (db, path) = fixture();
        let mut item = schedule("đã-bị-xoá");
        item.last_error = Some("không còn kịch bản tên `đã-bị-xoá`".into());
        db.upsert_schedule(&item).expect("save the failed schedule");

        let stored = db.list_schedules().expect("list")[0].clone();
        assert_eq!(
            stored.last_error.as_deref(),
            Some("không còn kịch bản tên `đã-bị-xoá`")
        );
        // And `last_run_at` stays empty, because nothing ran.
        assert_eq!(stored.last_run_at, None);

        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn a_schedule_that_runs_clears_the_reason_it_used_to_fail_for() {
        // Otherwise a schedule fixed by restoring its script would keep explaining a
        // failure that is over -- the same immortal-error shape as `merge_scanned_device`.
        let (db, path) = fixture();
        let mut item = schedule("có-thật");
        item.last_error = Some("không còn kịch bản".into());
        db.upsert_schedule(&item).expect("save the failed schedule");

        item.last_error = None;
        item.last_run_at = Some("2026-08-17T12:00:00Z".into());
        db.upsert_schedule(&item)
            .expect("save the recovered schedule");

        let stored = db.list_schedules().expect("list")[0].clone();
        assert_eq!(stored.last_error, None);
        assert_eq!(stored.last_run_at.as_deref(), Some("2026-08-17T12:00:00Z"));

        std::fs::remove_file(path).expect("remove fixture database");
    }
}

#[cfg(test)]
mod secret_store_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// In-memory stand-in for the OS credential store.
    #[derive(Default)]
    struct MemoryStore {
        entries: Mutex<std::collections::HashMap<String, String>>,
    }

    impl SecretStore for MemoryStore {
        fn get_secret(&self, name: &str) -> anyhow::Result<Option<String>> {
            Ok(self.entries.lock().unwrap().get(name).cloned())
        }
        fn set_secret(&self, name: &str, value: &str) -> anyhow::Result<()> {
            self.entries
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
            Ok(())
        }
    }

    fn fixture() -> (Database, Arc<MemoryStore>, PathBuf) {
        let path = std::env::temp_dir().join(format!("riviu-secret-test-{}.db", Uuid::new_v4()));
        let store = Arc::new(MemoryStore::default());
        let db = Database::open(&path)
            .expect("open fixture database")
            .with_secrets(store.clone());
        (db, store, path)
    }

    /// What the whole seam is for: the key must not be in the SQLite file.
    #[test]
    fn the_api_key_goes_to_the_store_and_not_into_the_settings_blob() {
        let (db, store, path) = fixture();
        let settings = crate::types::NurtureSettings {
            api_key: "sk-secret-value".into(),
            ..Default::default()
        };
        db.save_nurture_settings(&settings).expect("save");

        let blob = db
            .get_setting("nurture.settings")
            .expect("read blob")
            .expect("blob exists");
        assert!(
            !blob.contains("sk-secret-value"),
            "the key is still in the SQLite blob: {blob}"
        );
        assert_eq!(
            store.get_secret(SECRET_AI_API_KEY).unwrap().as_deref(),
            Some("sk-secret-value")
        );

        // And it comes back on read, because the engine needs the real value.
        let loaded = db.get_nurture_settings().expect("load");
        assert_eq!(loaded.api_key, "sk-secret-value");
        let _ = std::fs::remove_file(path);
    }

    /// The engine re-reads settings mid-session; every read must carry the key.
    ///
    /// This is the failure a command-layer-only fix would have shipped: the first read looks
    /// right, and the *second* one — the live refresh `nurture::run_session` does on every post
    /// — comes back empty, so commenting stops part way through a run.
    #[test]
    fn every_read_carries_the_key_not_just_the_first() {
        let (db, _store, path) = fixture();
        let settings = crate::types::NurtureSettings {
            api_key: "sk-live-refresh".into(),
            ..Default::default()
        };
        db.save_nurture_settings(&settings).expect("save");

        for read in 1..=3 {
            let loaded = db.get_nurture_settings().expect("load");
            assert_eq!(
                loaded.api_key, "sk-live-refresh",
                "read #{read} lost the key"
            );
        }
        let _ = std::fs::remove_file(path);
    }

    /// A database written before the store existed still has the key in its blob.
    #[test]
    fn a_legacy_blob_is_migrated_into_the_store_and_blanked() {
        let path = std::env::temp_dir().join(format!("riviu-secret-legacy-{}.db", Uuid::new_v4()));
        // Written by a build with no secret store: the key lands in the blob.
        {
            let plain = Database::open(&path).expect("open plain");
            let settings = crate::types::NurtureSettings {
                api_key: "sk-legacy".into(),
                ..Default::default()
            };
            plain.save_nurture_settings(&settings).expect("save plain");
            let blob = plain.get_setting("nurture.settings").unwrap().unwrap();
            assert!(
                blob.contains("sk-legacy"),
                "fixture must start in the old shape"
            );
        }

        let store = Arc::new(MemoryStore::default());
        let db = Database::open(&path)
            .expect("reopen")
            .with_secrets(store.clone());

        // First read migrates and still answers with the key.
        let loaded = db.get_nurture_settings().expect("load");
        assert_eq!(loaded.api_key, "sk-legacy");
        assert_eq!(
            store.get_secret(SECRET_AI_API_KEY).unwrap().as_deref(),
            Some("sk-legacy")
        );
        let blob = db.get_setting("nurture.settings").unwrap().unwrap();
        assert!(
            !blob.contains("sk-legacy"),
            "the legacy key was left in the blob: {blob}"
        );

        // Second read is a plain store read, and still correct.
        assert_eq!(db.get_nurture_settings().unwrap().api_key, "sk-legacy");
        let _ = std::fs::remove_file(path);
    }

    /// Clearing means clearing. The database layer is faithful; "leave it unchanged" is a
    /// decision for whoever owns the form, not a rule hidden down here.
    #[test]
    fn an_empty_key_clears_the_stored_one() {
        let (db, store, path) = fixture();
        let mut settings = crate::types::NurtureSettings {
            api_key: "sk-first".into(),
            ..Default::default()
        };
        db.save_nurture_settings(&settings).expect("save");
        settings.api_key = String::new();
        db.save_nurture_settings(&settings).expect("clear");

        assert_eq!(
            store.get_secret(SECRET_AI_API_KEY).unwrap().as_deref(),
            Some("")
        );
        assert_eq!(db.get_nurture_settings().unwrap().api_key, "");
        let _ = std::fs::remove_file(path);
    }

    /// No store attached: unchanged behaviour, which is what the other test fixtures rely on.
    #[test]
    fn without_a_store_the_key_stays_in_the_blob_as_before() {
        let path = std::env::temp_dir().join(format!("riviu-secret-none-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).expect("open");
        let settings = crate::types::NurtureSettings {
            api_key: "sk-plain".into(),
            ..Default::default()
        };
        db.save_nurture_settings(&settings).expect("save");
        assert!(db
            .get_setting("nurture.settings")
            .unwrap()
            .unwrap()
            .contains("sk-plain"));
        assert_eq!(db.get_nurture_settings().unwrap().api_key, "sk-plain");
        let _ = std::fs::remove_file(path);
    }
}
