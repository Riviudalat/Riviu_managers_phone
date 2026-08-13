use std::collections::{BTreeMap, BTreeSet};

use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

type ApplyMigration = fn(&Transaction<'_>) -> anyhow::Result<()>;

struct Migration {
    version: i64,
    name: &'static str,
    apply: ApplyMigration,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "legacy-schema-baseline",
        apply: apply_migration_1,
    },
    Migration {
        version: 2,
        name: "flow-v2-schema",
        apply: apply_migration_2,
    },
    Migration {
        version: 3,
        name: "nurture-comment-attempts",
        apply: apply_migration_3,
    },
    Migration {
        version: 4,
        name: "interaction-comment-threads",
        apply: apply_migration_4,
    },
    Migration {
        version: 5,
        name: "publish-campaigns",
        apply: apply_migration_5,
    },
    Migration {
        version: 6,
        name: "flow-ifvision-branch",
        apply: apply_migration_6,
    },
];

const LEDGER_SQL: &str = r#"
CREATE TABLE schema_migrations (
  version INTEGER PRIMARY KEY CHECK (version >= 1),
  name TEXT NOT NULL UNIQUE,
  applied_at TEXT NOT NULL
);
"#;

const V1_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
  id TEXT PRIMARY KEY,
  script_name TEXT NOT NULL,
  udids_json TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  steps_json TEXT NOT NULL,
  error TEXT
);

CREATE TABLE IF NOT EXISTS scripts (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  body_json TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS device_meta (
  udid TEXT PRIMARY KEY,
  notes TEXT NOT NULL DEFAULT '',
  tags_json TEXT NOT NULL DEFAULT '[]',
  group_id TEXT,
  proxy_id TEXT
);

CREATE TABLE IF NOT EXISTS groups (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  color TEXT NOT NULL DEFAULT '#FF6A00',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS group_members (
  group_id TEXT NOT NULL,
  udid TEXT NOT NULL,
  PRIMARY KEY (group_id, udid)
);

CREATE TABLE IF NOT EXISTS proxies (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  proxy_type TEXT NOT NULL DEFAULT 'http',
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  username TEXT NOT NULL DEFAULT '',
  password TEXT NOT NULL DEFAULT '',
  notes TEXT NOT NULL DEFAULT ''
);

CREATE TABLE IF NOT EXISTS materials (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  kind TEXT NOT NULL,
  size INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS apps_library (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  path TEXT NOT NULL,
  bundle_id TEXT NOT NULL DEFAULT '',
  version TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS schedules (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  script_name TEXT NOT NULL,
  udids_json TEXT NOT NULL,
  every_minutes INTEGER NOT NULL DEFAULT 60,
  enabled INTEGER NOT NULL DEFAULT 1,
  last_run_at TEXT,
  next_run_at TEXT
);

CREATE TABLE IF NOT EXISTS publish_tasks (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  script_name TEXT NOT NULL,
  material_ids_json TEXT NOT NULL,
  udids_json TEXT NOT NULL,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS op_logs (
  id TEXT PRIMARY KEY,
  action TEXT NOT NULL,
  detail TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'admin',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS nurture_comment_costs (
  id TEXT PRIMARY KEY,
  udid TEXT NOT NULL,
  model TEXT NOT NULL,
  base_url_host TEXT NOT NULL,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  usd REAL NOT NULL DEFAULT 0,
  preview TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);
"#;

const FLOW_V2_SCHEMA_SQL: &str = r#"
CREATE TABLE flow_documents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  latest_revision INTEGER NOT NULL CHECK (latest_revision >= 1),
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE flow_revisions (
  flow_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  authoring_json TEXT NOT NULL,
  compiled_json TEXT NOT NULL,
  plan_sha256 TEXT NOT NULL CHECK (
    length(plan_sha256) = 64 AND plan_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  created_at TEXT NOT NULL,
  PRIMARY KEY (flow_id, revision),
  FOREIGN KEY (flow_id) REFERENCES flow_documents(id) ON DELETE RESTRICT
);

CREATE TABLE flow_runs (
  id TEXT PRIMARY KEY,
  flow_id TEXT NOT NULL,
  flow_revision INTEGER NOT NULL,
  plan_sha256 TEXT NOT NULL CHECK (
    length(plan_sha256) = 64 AND plan_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  selection_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (
    state IN ('queued','running','succeeded','partial','failed','cancelled')
  ),
  event_revision INTEGER NOT NULL DEFAULT 0 CHECK (event_revision >= 0),
  error_json TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (flow_id, flow_revision)
    REFERENCES flow_revisions(flow_id, revision) ON DELETE RESTRICT
);

CREATE TABLE flow_device_runs (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  udid TEXT NOT NULL,
  state TEXT NOT NULL CHECK (
    state IN ('queued','preflight','running','succeeded','failed','skipped','cancelled')
  ),
  capability_snapshot_json TEXT,
  release_proof_json TEXT,
  error_json TEXT,
  started_at TEXT,
  finished_at TEXT,
  UNIQUE (run_id, udid),
  FOREIGN KEY (run_id) REFERENCES flow_runs(id) ON DELETE RESTRICT
);

CREATE TABLE flow_node_attempts (
  id TEXT PRIMARY KEY,
  device_run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  action_kind TEXT NOT NULL,
  attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1),
  side_effect_class TEXT NOT NULL CHECK (
    side_effect_class IN ('none','idempotentSet','ambiguousUi','artifactWrite')
  ),
  state TEXT NOT NULL CHECK (
    state IN (
      'queued','intentCommitted','effectDispatched','verifying','succeeded',
      'failedBeforeDispatch','failedVerified','uncertain','cancelled','interrupted'
    )
  ),
  canonical_input_json TEXT,
  evidence_baseline_json TEXT,
  evidence_result_json TEXT,
  retry_safe INTEGER NOT NULL DEFAULT 0 CHECK (retry_safe IN (0, 1)),
  error_json TEXT,
  started_at TEXT,
  updated_at TEXT NOT NULL,
  finished_at TEXT,
  UNIQUE (device_run_id, node_id, attempt_no),
  FOREIGN KEY (device_run_id) REFERENCES flow_device_runs(id) ON DELETE RESTRICT
);

CREATE TABLE flow_artifacts (
  id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL,
  relative_path TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('jpeg','png')),
  size INTEGER NOT NULL CHECK (size > 0),
  sha256 TEXT NOT NULL CHECK (
    length(sha256) = 64 AND sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  created_at TEXT NOT NULL,
  FOREIGN KEY (attempt_id) REFERENCES flow_node_attempts(id) ON DELETE RESTRICT
);

CREATE TABLE flow_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (run_id, revision),
  FOREIGN KEY (run_id) REFERENCES flow_runs(id) ON DELETE RESTRICT
);

CREATE INDEX idx_flow_documents_updated ON flow_documents(updated_at DESC);
CREATE INDEX idx_flow_runs_updated ON flow_runs(updated_at DESC);
CREATE INDEX idx_flow_device_runs_state ON flow_device_runs(run_id, state);
CREATE INDEX idx_flow_attempts_state ON flow_node_attempts(device_run_id, state);
CREATE INDEX idx_flow_artifacts_attempt ON flow_artifacts(attempt_id);
CREATE INDEX idx_flow_events_revision ON flow_events(run_id, revision);
"#;

const NURTURE_COMMENT_ATTEMPTS_SCHEMA_SQL: &str = r#"
CREATE TABLE nurture_comment_attempts (
  id TEXT PRIMARY KEY,
  udid TEXT NOT NULL,
  outcome TEXT NOT NULL,
  source TEXT NOT NULL,
  model TEXT NOT NULL,
  base_url_host TEXT NOT NULL,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  usd REAL NOT NULL DEFAULT 0,
  preview TEXT NOT NULL DEFAULT '',
  caption_preview TEXT NOT NULL DEFAULT '',
  frame_sha256 TEXT NOT NULL DEFAULT '',
  context_confidence INTEGER,
  relevance INTEGER,
  evidence_support INTEGER,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_nurture_comment_attempts_created
  ON nurture_comment_attempts(created_at DESC);
"#;

const INTERACTION_COMMENT_THREADS_SCHEMA_SQL: &str = r#"
CREATE TABLE tiktok_accounts (
  id TEXT PRIMARY KEY,
  udid TEXT NOT NULL,
  slot_key TEXT NOT NULL,
  username TEXT,
  state TEXT NOT NULL DEFAULT 'active',
  created_at TEXT NOT NULL,
  UNIQUE (udid, slot_key)
);

CREATE TABLE interaction_campaigns (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  request_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('queued','running','succeeded','partial','failed','cancelled')),
  message_count INTEGER NOT NULL CHECK (message_count BETWEEN 2 AND 6),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE interaction_campaign_actors (
  campaign_id TEXT NOT NULL,
  actor_ordinal INTEGER NOT NULL CHECK (actor_ordinal >= 0),
  udid TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'planned',
  error_code TEXT,
  PRIMARY KEY (campaign_id, actor_ordinal),
  UNIQUE (campaign_id, udid),
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);

CREATE TABLE interaction_targets (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  line_no INTEGER NOT NULL CHECK (line_no >= 1),
  original_url TEXT NOT NULL,
  normalized_url TEXT NOT NULL,
  target_key TEXT NOT NULL,
  content_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('video','photo')),
  -- DEAD as of 13/08/2026: nothing writes this and nothing reads it. Measured on a live
  -- run, every row stayed 'queued' including a target whose assignment succeeded.
  -- Deliberately left unwritten rather than filled in: the per-assignment rows are the
  -- real record, and a target-level state maintained beside them is a second source of
  -- truth that can disagree with the first. Read `interaction_assignments.state` instead.
  state TEXT NOT NULL DEFAULT 'queued',
  context_json TEXT,
  error_code TEXT,
  created_at TEXT NOT NULL,
  UNIQUE (campaign_id, target_key),
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);

CREATE TABLE interaction_assignments (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  target_id TEXT NOT NULL,
  message_ordinal INTEGER NOT NULL CHECK (message_ordinal BETWEEN 0 AND 5),
  actor_udid TEXT NOT NULL,
  parent_assignment_id TEXT,
  prepared_json TEXT,
  state TEXT NOT NULL DEFAULT 'queued',
  effect_intent TEXT,
  evidence_json TEXT,
  error_code TEXT,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (campaign_id, target_id, message_ordinal),
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE,
  FOREIGN KEY (target_id) REFERENCES interaction_targets(id) ON DELETE CASCADE,
  FOREIGN KEY (parent_assignment_id) REFERENCES interaction_assignments(id) ON DELETE RESTRICT
);

CREATE TABLE interaction_artifacts (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  target_id TEXT NOT NULL,
  assignment_id TEXT,
  kind TEXT NOT NULL,
  metadata_json TEXT NOT NULL,
  relative_path TEXT,
  sha256 TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL,
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE,
  FOREIGN KEY (target_id) REFERENCES interaction_targets(id) ON DELETE CASCADE,
  FOREIGN KEY (assignment_id) REFERENCES interaction_assignments(id) ON DELETE SET NULL
);

-- DEAD as of 13/08/2026, and the one with a hazard attached. It was shaped as a
-- single-owner lease (`owner`, `claimed_at`) so two app instances could not run the same
-- campaign, but nothing ever claims it: the only write is the INSERT of 'queued' at
-- creation. So the row is not evidence of an owner, and any future reader must not treat
-- it as one. If two instances on one data directory ever becomes possible, this is where
-- the guard belongs -- it is not there yet.
CREATE TABLE interaction_dispatch (
  campaign_id TEXT PRIMARY KEY,
  state TEXT NOT NULL DEFAULT 'queued',
  owner TEXT,
  claimed_at TEXT,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);

CREATE TABLE interaction_retry_requests (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  request_id TEXT NOT NULL UNIQUE,
  assignment_ids_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);

-- NOTHING WRITES THIS TABLE, AND NOTHING READS IT. It is always empty.
--
-- Said here rather than only in a plan document, because this is where somebody meets it:
-- the shape copies `flow_events` exactly, so it reads like an audit trail of interaction
-- campaigns, and an empty table that looks like an audit trail is worse than no table —
-- it invites "there are no events, so nothing happened" when the truth is that nothing
-- ever recorded anything.
--
-- Deliberately left rather than resolved either way. Adding a writer is a feature nobody
-- asked for, and its value is small without a reader; dropping it is a migration against a
-- schema that is about to ship. Whoever decides: writing means one row per campaign state
-- transition keyed by the campaign's revision, where `UNIQUE (campaign_id, revision)` is
-- what makes a retried write idempotent — the same property `flow_events` relies on.
CREATE TABLE interaction_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  campaign_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (campaign_id, revision),
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE
);

CREATE INDEX idx_interaction_campaigns_updated ON interaction_campaigns(updated_at DESC);
CREATE INDEX idx_interaction_targets_campaign ON interaction_targets(campaign_id, line_no);
CREATE INDEX idx_interaction_assignments_target ON interaction_assignments(target_id, message_ordinal);
CREATE INDEX idx_interaction_assignments_state ON interaction_assignments(campaign_id, state);
CREATE INDEX idx_interaction_artifacts_assignment ON interaction_artifacts(assignment_id);
CREATE INDEX idx_interaction_events_revision ON interaction_events(campaign_id, revision);
"#;

const PUBLISH_CAMPAIGNS_SCHEMA_SQL: &str = r#"
CREATE TABLE publish_campaigns (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  source_root TEXT NOT NULL,
  request_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN (
    'queued','scheduled','preparing','ready','transferring','imported',
    'posting','verifying','succeeded','failed_before_dispatch','uncertain',
    'cancelled','missed'
  )),
  run_at TEXT,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE publish_bundles (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  name TEXT NOT NULL,
  source_path TEXT NOT NULL,
  caption TEXT NOT NULL,
  caption_sha256 TEXT NOT NULL CHECK (
    length(caption_sha256) = 64 AND caption_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  manifest_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (campaign_id, id),
  UNIQUE (campaign_id, ordinal),
  FOREIGN KEY (campaign_id) REFERENCES publish_campaigns(id) ON DELETE CASCADE
);

CREATE TABLE publish_assignments (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  bundle_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL CHECK (ordinal >= 0),
  udid TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN (
    'queued','scheduled','preparing','ready','transferring','imported',
    'posting','verifying','succeeded','failed_before_dispatch','uncertain',
    'cancelled','missed'
  )),
  effect_intent TEXT,
  evidence_json TEXT,
  error_code TEXT,
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (campaign_id, bundle_id),
  UNIQUE (campaign_id, udid),
  FOREIGN KEY (campaign_id) REFERENCES publish_campaigns(id) ON DELETE CASCADE,
  FOREIGN KEY (bundle_id) REFERENCES publish_bundles(id) ON DELETE RESTRICT
);

CREATE TABLE publish_dispatch (
  campaign_id TEXT PRIMARY KEY,
  state TEXT NOT NULL DEFAULT 'queued',
  owner TEXT,
  claimed_at TEXT,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (campaign_id) REFERENCES publish_campaigns(id) ON DELETE CASCADE
);

CREATE TABLE publish_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  campaign_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  kind TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  UNIQUE (campaign_id, revision),
  FOREIGN KEY (campaign_id) REFERENCES publish_campaigns(id) ON DELETE CASCADE
);

CREATE INDEX idx_publish_campaigns_updated ON publish_campaigns(updated_at DESC);
CREATE INDEX idx_publish_assignments_campaign ON publish_assignments(campaign_id, ordinal);
CREATE INDEX idx_publish_assignments_state ON publish_assignments(campaign_id, state);
CREATE INDEX idx_publish_events_revision ON publish_events(campaign_id, revision);
"#;

pub(super) fn run(connection: &mut Connection) -> anyhow::Result<()> {
    run_internal(connection, None)
}

#[cfg(test)]
fn run_with_failpoint(
    connection: &mut Connection,
    failed_version: Option<i64>,
) -> anyhow::Result<()> {
    run_internal(connection, failed_version)
}

fn run_internal(connection: &mut Connection, failed_version: Option<i64>) -> anyhow::Result<()> {
    bootstrap_ledger(connection, failed_version)?;
    validate_ledger(connection)?;

    for migration in MIGRATIONS {
        apply_one(connection, migration, failed_version)?;
    }
    Ok(())
}

fn bootstrap_ledger(
    connection: &mut Connection,
    failed_version: Option<i64>,
) -> anyhow::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if table_exists(&transaction, "schema_migrations")? {
        transaction.commit()?;
        return Ok(());
    }

    let actual = schema_fingerprint(&transaction)?;
    let empty = actual.tables.is_empty() && actual.other_objects.is_empty();
    let exact_legacy = !empty && actual == expected_v1_fingerprint()?;
    if !empty && !exact_legacy {
        anyhow::bail!("UnknownLegacySchema: database does not match the exact Riviu v1 schema");
    }

    transaction.execute_batch(LEDGER_SQL)?;
    if empty {
        (MIGRATIONS[0].apply)(&transaction)?;
    }
    fail_if_requested(MIGRATIONS[0].version, failed_version)?;
    insert_ledger_row(&transaction, &MIGRATIONS[0])?;
    transaction.commit()?;
    Ok(())
}

fn validate_ledger(connection: &Connection) -> anyhow::Result<Vec<(i64, String)>> {
    let rows: Vec<(i64, String)> = connection
        .prepare("SELECT version,name FROM schema_migrations ORDER BY version")
        .context("MigrationLedgerInvalid: cannot read schema_migrations")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .context("MigrationLedgerInvalid: cannot query schema_migrations")?
        .collect::<Result<_, _>>()
        .context("MigrationLedgerInvalid: malformed schema_migrations row")?;

    let mut names = BTreeSet::new();
    for (_, name) in &rows {
        if !names.insert(name) {
            anyhow::bail!("MigrationLedgerInvalid: duplicate logical migration name {name}");
        }
    }

    for (index, (version, name)) in rows.iter().enumerate() {
        let expected_version = index as i64 + 1;
        if *version != expected_version {
            anyhow::bail!(
                "MigrationLedgerInvalid: expected contiguous version {expected_version}, found {version}"
            );
        }
        let Some(migration) = MIGRATIONS.get(index) else {
            anyhow::bail!(
                "MigrationLedgerInvalid: database version {version} is newer than this binary"
            );
        };
        if migration.version != *version || migration.name != name {
            anyhow::bail!(
                "MigrationLedgerInvalid: version {version} must be named {}, found {name}",
                migration.name
            );
        }
    }
    Ok(rows)
}

fn apply_one(
    connection: &mut Connection,
    migration: &Migration,
    failed_version: Option<i64>,
) -> anyhow::Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let applied = validate_ledger(&transaction)?;
    let migration_index = usize::try_from(migration.version - 1)
        .context("MigrationLedgerInvalid: migration version must be positive")?;
    if applied.len() > migration_index {
        transaction.commit()?;
        return Ok(());
    }
    if applied.len() != migration_index {
        anyhow::bail!(
            "MigrationLedgerInvalid: migration {} cannot run after {} applied versions",
            migration.version,
            applied.len()
        );
    }
    (migration.apply)(&transaction)?;
    fail_if_requested(migration.version, failed_version)?;
    insert_ledger_row(&transaction, migration)?;
    transaction.commit()?;
    Ok(())
}

fn fail_if_requested(version: i64, failed_version: Option<i64>) -> anyhow::Result<()> {
    if failed_version == Some(version) {
        anyhow::bail!("InjectedMigrationFailure: version {version}");
    }
    Ok(())
}

fn insert_ledger_row(transaction: &Transaction<'_>, migration: &Migration) -> anyhow::Result<()> {
    transaction.execute(
        "INSERT INTO schema_migrations(version,name,applied_at) VALUES(?1,?2,?3)",
        params![migration.version, migration.name, Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

fn apply_migration_1(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    apply_v1_schema(transaction)?;
    let count: i64 = transaction.query_row("SELECT COUNT(*) FROM users", [], |row| row.get(0))?;
    if count == 0 {
        transaction.execute(
            "INSERT INTO users (id,email,password_hash,role,created_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                Uuid::new_v4().to_string(),
                "guest@local",
                "guest",
                "admin",
                Utc::now().to_rfc3339()
            ],
        )?;
    }
    Ok(())
}

fn apply_migration_2(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(FLOW_V2_SCHEMA_SQL)?;
    Ok(())
}

fn apply_migration_3(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(NURTURE_COMMENT_ATTEMPTS_SCHEMA_SQL)?;
    Ok(())
}

fn apply_migration_4(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(INTERACTION_COMMENT_THREADS_SCHEMA_SQL)?;
    Ok(())
}

fn apply_migration_5(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(PUBLISH_CAMPAIGNS_SCHEMA_SQL)?;
    Ok(())
}

fn apply_migration_6(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // First-class record of the port an IfVision branch selected at runtime, so
    // recovery can rebuild the taken path without re-running the vision predicate.
    // Nullable: every existing attempt (and every non-branch node) leaves it NULL.
    transaction.execute_batch("ALTER TABLE flow_node_attempts ADD COLUMN chosen_port TEXT;")?;
    Ok(())
}

fn apply_v1_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(V1_SCHEMA_SQL)?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaFingerprint {
    tables: BTreeMap<String, TableFingerprint>,
    other_objects: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
struct TableFingerprint {
    columns: Vec<ColumnFingerprint>,
    unique_constraints: Vec<UniqueConstraintFingerprint>,
}

#[derive(Debug, PartialEq, Eq)]
struct ColumnFingerprint {
    ordinal: i64,
    name: String,
    declared_type: String,
    not_null: bool,
    default_value: Option<String>,
    primary_key_position: i64,
    hidden: i64,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct UniqueConstraintFingerprint {
    origin: String,
    partial: bool,
    columns: Vec<IndexColumnFingerprint>,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct IndexColumnFingerprint {
    ordinal: i64,
    column_id: i64,
    name: Option<String>,
    descending: bool,
    collation: Option<String>,
    key: bool,
}

fn expected_v1_fingerprint() -> anyhow::Result<SchemaFingerprint> {
    let reference = Connection::open_in_memory()?;
    apply_v1_schema(&reference)?;
    schema_fingerprint(&reference)
}

fn schema_fingerprint(connection: &Connection) -> anyhow::Result<SchemaFingerprint> {
    let objects: Vec<(String, String)> = connection
        .prepare(
            "SELECT type,name FROM sqlite_master
             WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name",
        )?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;

    let mut tables = BTreeMap::new();
    let mut other_objects = Vec::new();
    for (object_type, name) in objects {
        if object_type == "table" {
            tables.insert(name.clone(), table_fingerprint(connection, &name)?);
        } else {
            other_objects.push((object_type, name));
        }
    }
    Ok(SchemaFingerprint {
        tables,
        other_objects,
    })
}

fn table_fingerprint(connection: &Connection, table: &str) -> anyhow::Result<TableFingerprint> {
    let quoted_table = quote_identifier(table);
    let columns = connection
        .prepare(&format!("PRAGMA table_xinfo({quoted_table})"))?
        .query_map([], |row| {
            Ok(ColumnFingerprint {
                ordinal: row.get(0)?,
                name: row.get(1)?,
                declared_type: row.get(2)?,
                not_null: row.get::<_, i64>(3)? != 0,
                default_value: row.get(4)?,
                primary_key_position: row.get(5)?,
                hidden: row.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    let indexes: Vec<(String, bool, String, bool)> = connection
        .prepare(&format!("PRAGMA index_list({quoted_table})"))?
        .query_map([], |row| {
            Ok((
                row.get(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })?
        .collect::<Result<_, _>>()?;
    let mut unique_constraints = Vec::new();
    for (name, unique, origin, partial) in indexes {
        if !unique {
            continue;
        }
        let quoted_index = quote_identifier(&name);
        let columns = connection
            .prepare(&format!("PRAGMA index_xinfo({quoted_index})"))?
            .query_map([], |row| {
                Ok(IndexColumnFingerprint {
                    ordinal: row.get(0)?,
                    column_id: row.get(1)?,
                    name: row.get(2)?,
                    descending: row.get::<_, i64>(3)? != 0,
                    collation: row.get(4)?,
                    key: row.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<Result<_, _>>()?;
        unique_constraints.push(UniqueConstraintFingerprint {
            origin,
            partial,
            columns,
        });
    }
    unique_constraints.sort();
    Ok(TableFingerprint {
        columns,
        unique_constraints,
    })
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn table_exists(connection: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT type FROM sqlite_master WHERE name=?1",
            [table],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some_and(|object_type| object_type == "table"))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Barrier};

    use rusqlite::{params, Connection, ErrorCode};
    use uuid::Uuid;

    use super::super::Database;
    use super::{apply_v1_schema, run, run_with_failpoint};

    type Blob = Vec<u8>;
    type ScriptRowBytes = (Blob, Blob, Blob, Blob);
    type JobRowBytes = (Blob, Blob, Blob, Blob, Blob, Blob, Blob, Option<Blob>);
    type SettingRowBytes = (Blob, Blob);
    type DeviceRowBytes = (Blob, Blob, Blob, Option<Blob>, Option<Blob>);
    type SchemaDriftArrange = fn(&Connection);

    #[derive(Debug, PartialEq, Eq)]
    struct LegacyRows {
        script: ScriptRowBytes,
        job: JobRowBytes,
        setting: SettingRowBytes,
        device: DeviceRowBytes,
    }

    fn temp_db_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("riviu-{label}-{}.db", Uuid::new_v4()))
    }

    fn cleanup(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{suffix}", path.display()));
            if candidate.exists() {
                std::fs::remove_file(candidate).expect("remove migration fixture");
            }
        }
    }

    fn insert_populated_legacy_rows(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO scripts (id,name,body_json,updated_at) VALUES (?1,?2,?3,?4)",
                params![
                    "script-1",
                    "fixture",
                    r#"{"version":1,"name":"fixture","steps":[{"action":"wait","milliseconds":1}]}"#,
                    "2026-07-30T00:00:00Z"
                ],
            )
            .expect("legacy script row");
        connection
            .execute(
                "INSERT INTO jobs (
                    id,script_name,udids_json,status,created_at,updated_at,steps_json,error
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,NULL)",
                params![
                    "job-1",
                    "fixture",
                    "[\"MOCK-IPHONE-01\"]",
                    r#""succeeded""#,
                    "2026-07-30T00:00:00Z",
                    "2026-07-30T00:00:01Z",
                    "[]"
                ],
            )
            .expect("legacy job row");
        connection
            .execute(
                "INSERT INTO settings (key,value) VALUES (?1,?2)",
                params!["fixture", "{\"enabled\":true}"],
            )
            .expect("legacy setting row");
        connection
            .execute(
                "INSERT INTO device_meta (udid,notes,tags_json) VALUES (?1,?2,?3)",
                params!["MOCK-IPHONE-01", "fixture", "[\"legacy\"]"],
            )
            .expect("legacy device row");
    }

    fn read_legacy_rows(connection: &Connection) -> LegacyRows {
        LegacyRows {
            script: connection
                .query_row(
                    "SELECT CAST(id AS BLOB), CAST(name AS BLOB), CAST(body_json AS BLOB),
                            CAST(updated_at AS BLOB)
                     FROM scripts WHERE id='script-1'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .expect("legacy script bytes"),
            job: connection
                .query_row(
                    "SELECT CAST(id AS BLOB), CAST(script_name AS BLOB), CAST(udids_json AS BLOB),
                            CAST(status AS BLOB), CAST(created_at AS BLOB),
                            CAST(updated_at AS BLOB), CAST(steps_json AS BLOB), CAST(error AS BLOB)
                     FROM jobs WHERE id='job-1'",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                            row.get(6)?,
                            row.get(7)?,
                        ))
                    },
                )
                .expect("legacy job bytes"),
            setting: connection
                .query_row(
                    "SELECT CAST(key AS BLOB), CAST(value AS BLOB)
                     FROM settings WHERE key='fixture'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .expect("legacy setting bytes"),
            device: connection
                .query_row(
                    "SELECT CAST(udid AS BLOB), CAST(notes AS BLOB), CAST(tags_json AS BLOB),
                            CAST(group_id AS BLOB), CAST(proxy_id AS BLOB)
                     FROM device_meta WHERE udid='MOCK-IPHONE-01'",
                    [],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .expect("legacy device bytes"),
        }
    }

    fn migration_rows(connection: &Connection) -> Vec<(i64, String)> {
        connection
            .prepare("SELECT version,name FROM schema_migrations ORDER BY version")
            .expect("prepare migration ledger")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query migration ledger")
            .collect::<Result<_, _>>()
            .expect("collect migration ledger")
    }

    fn user_objects(connection: &Connection) -> Vec<(String, String)> {
        connection
            .prepare(
                "SELECT type,name FROM sqlite_master
                 WHERE name NOT GLOB 'sqlite_*' ORDER BY type,name",
            )
            .expect("prepare schema objects")
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("query schema objects")
            .collect::<Result<_, _>>()
            .expect("collect schema objects")
    }

    fn table_exists(connection: &Connection, table: &str) -> bool {
        connection
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1
                 )",
                [table],
                |row| row.get(0),
            )
            .expect("table existence")
    }

    #[test]
    fn populated_legacy_database_upgrades_once_without_rewriting_rows() {
        let path = temp_db_path("flow-migration");
        let connection = Connection::open(&path).expect("legacy db");
        apply_v1_schema(&connection).expect("legacy schema");
        insert_populated_legacy_rows(&connection);
        let expected = read_legacy_rows(&connection);
        drop(connection);

        let database = Database::open(&path).expect("first migration");
        drop(database);
        let database = Database::open(&path).expect("idempotent reopen");
        let connection = database.conn().expect("inspect");
        assert_eq!(read_legacy_rows(&connection), expected);
        assert_eq!(
            migration_rows(&connection)
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
        assert!(table_exists(&connection, "flow_documents"));
        assert!(table_exists(&connection, "nurture_comment_attempts"));
        drop(connection);
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn unknown_or_drifted_legacy_schema_fails_before_creating_the_ledger() {
        let cases: &[(&str, SchemaDriftArrange)] = &[
            ("sqlite-prefix-lookalike", |connection| {
                connection
                    .execute_batch("CREATE TABLE sqliteXextra (id TEXT PRIMARY KEY)")
                    .expect("user table resembling sqlite prefix");
            }),
            ("partial", |connection| {
                connection
                    .execute_batch("CREATE TABLE scripts (id TEXT PRIMARY KEY)")
                    .expect("partial schema");
            }),
            ("column", |connection| {
                apply_v1_schema(connection).expect("legacy schema");
                connection
                    .execute("ALTER TABLE scripts ADD COLUMN drift TEXT", [])
                    .expect("column drift");
            }),
            ("generated-column", |connection| {
                apply_v1_schema(connection).expect("legacy schema");
                connection
                    .execute(
                        "ALTER TABLE scripts ADD COLUMN generated_name TEXT
                         GENERATED ALWAYS AS (name || '-copy') VIRTUAL",
                        [],
                    )
                    .expect("generated column drift");
            }),
            ("primary-key", |connection| {
                apply_v1_schema(connection).expect("legacy schema");
                connection
                    .execute_batch(
                        "DROP TABLE scripts;
                     CREATE TABLE scripts (
                       id TEXT NOT NULL,
                       name TEXT NOT NULL UNIQUE,
                       body_json TEXT NOT NULL,
                       updated_at TEXT NOT NULL
                     );",
                    )
                    .expect("primary-key drift");
            }),
            ("unique", |connection| {
                apply_v1_schema(connection).expect("legacy schema");
                connection
                    .execute_batch(
                        "DROP TABLE scripts;
                     CREATE TABLE scripts (
                       id TEXT PRIMARY KEY,
                       name TEXT NOT NULL,
                       body_json TEXT NOT NULL,
                       updated_at TEXT NOT NULL
                     );",
                    )
                    .expect("unique drift");
            }),
            ("unique-collation", |connection| {
                apply_v1_schema(connection).expect("legacy schema");
                connection
                    .execute_batch(
                        "DROP TABLE scripts;
                         CREATE TABLE scripts (
                           id TEXT PRIMARY KEY,
                           name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                           body_json TEXT NOT NULL,
                           updated_at TEXT NOT NULL
                         );",
                    )
                    .expect("unique collation drift");
            }),
        ];

        for (label, arrange) in cases {
            let path = temp_db_path(label);
            let connection = Connection::open(&path).expect("drift fixture");
            arrange(&connection);
            let before = user_objects(&connection);
            drop(connection);

            let error = Database::open(&path)
                .err()
                .expect("unknown schema must fail");
            assert!(
                error.to_string().contains("UnknownLegacySchema"),
                "{error:#}"
            );
            let connection = Connection::open(&path).expect("reopen drift fixture");
            assert_eq!(user_objects(&connection), before, "{label}");
            assert!(!table_exists(&connection, "schema_migrations"), "{label}");
            drop(connection);
            cleanup(&path);
        }
    }

    #[test]
    fn every_migration_rolls_back_its_schema_and_ledger_row_on_failure() {
        for failed_version in [1, 2, 3, 4, 5, 6] {
            let path = temp_db_path(&format!("migration-{failed_version}-rollback"));
            let mut connection = Connection::open(&path).expect("rollback fixture");
            let error = run_with_failpoint(&mut connection, Some(failed_version))
                .expect_err("injected migration failure");
            assert!(error.to_string().contains("InjectedMigrationFailure"));

            if failed_version == 1 {
                assert!(user_objects(&connection).is_empty());
            } else if failed_version == 2 {
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1]
                );
                assert!(table_exists(&connection, "scripts"));
                assert!(!table_exists(&connection, "flow_documents"));
            } else if failed_version == 3 {
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2]
                );
                assert!(table_exists(&connection, "flow_documents"));
                assert!(!table_exists(&connection, "nurture_comment_attempts"));
            } else if failed_version == 4 {
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3]
                );
                assert!(table_exists(&connection, "nurture_comment_attempts"));
                assert!(!table_exists(&connection, "publish_campaigns"));
                assert!(!table_exists(&connection, "interaction_campaigns"));
            } else if failed_version == 5 {
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3, 4]
                );
                assert!(table_exists(&connection, "interaction_campaigns"));
                assert!(!table_exists(&connection, "publish_campaigns"));
            } else {
                // Failed at 6: migrations 1..=5 applied (publish_campaigns
                // present), and the IfVision branch column migration rolled back.
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3, 4, 5]
                );
                assert!(table_exists(&connection, "publish_campaigns"));
            }

            run(&mut connection).expect("retry migrations");
            assert_eq!(
                migration_rows(&connection)
                    .iter()
                    .map(|(version, _)| *version)
                    .collect::<Vec<_>>(),
                vec![1, 2, 3, 4, 5, 6]
            );
            let guest_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM users WHERE email='guest@local'",
                    [],
                    |row| row.get(0),
                )
                .expect("guest seed count");
            assert_eq!(guest_count, 1);
            drop(connection);
            cleanup(&path);
        }
    }

    #[test]
    fn populated_legacy_failure_preserves_rows_and_retries_cleanly() {
        let path = temp_db_path("populated-rollback");
        let mut connection = Connection::open(&path).expect("populated rollback fixture");
        apply_v1_schema(&connection).expect("legacy schema");
        insert_populated_legacy_rows(&connection);
        let expected = read_legacy_rows(&connection);

        let error = run_with_failpoint(&mut connection, Some(2))
            .expect_err("inject Flow schema migration failure");
        assert!(error.to_string().contains("InjectedMigrationFailure"));
        assert_eq!(read_legacy_rows(&connection), expected);
        assert_eq!(
            migration_rows(&connection)
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            vec![1]
        );
        assert!(!table_exists(&connection, "flow_documents"));

        run(&mut connection).expect("retry Flow schema migration");
        assert_eq!(read_legacy_rows(&connection), expected);
        assert_eq!(
            migration_rows(&connection)
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn concurrent_openers_observe_or_apply_each_migration_once() {
        let path = temp_db_path("concurrent-migration");
        let mut connection = Connection::open(&path).expect("concurrency fixture");
        run_with_failpoint(&mut connection, Some(2)).expect_err("leave version one applied");
        drop(connection);

        const OPENERS: usize = 8;
        let barrier = Arc::new(Barrier::new(OPENERS));
        let handles = (0..OPENERS)
            .map(|_| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut connection =
                        Connection::open(path).map_err(|error| error.to_string())?;
                    connection
                        .busy_timeout(std::time::Duration::from_secs(5))
                        .map_err(|error| error.to_string())?;
                    barrier.wait();
                    run(&mut connection).map_err(|error| format!("{error:#}"))
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("migration opener thread"))
            .collect::<Vec<_>>();
        assert!(
            results.iter().all(Result::is_ok),
            "concurrent migration errors: {results:?}"
        );

        let connection = Connection::open(&path).expect("inspect concurrent migration");
        assert_eq!(
            migration_rows(&connection)
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn drifted_migration_ledgers_fail_closed_before_running_sql() {
        for case in ["gap", "renamed", "newer", "duplicate-name"] {
            let path = temp_db_path(&format!("ledger-{case}"));
            let mut connection = Connection::open(&path).expect("ledger fixture");
            match case {
                "duplicate-name" => {
                    connection
                        .execute_batch(
                            "CREATE TABLE schema_migrations (
                               version INTEGER PRIMARY KEY,
                               name TEXT NOT NULL,
                               applied_at TEXT NOT NULL
                             );
                             INSERT INTO schema_migrations VALUES
                               (1,'same','2026-07-30T00:00:00Z'),
                               (2,'same','2026-07-30T00:00:01Z');",
                        )
                        .expect("duplicate logical names");
                }
                _ => {
                    run(&mut connection).expect("baseline migrations");
                    match case {
                        "gap" => {
                            connection
                                .execute("DELETE FROM schema_migrations WHERE version=1", [])
                                .expect("create ledger gap");
                        }
                        "renamed" => {
                            connection
                                .execute(
                                    "UPDATE schema_migrations SET name='renamed' WHERE version=2",
                                    [],
                                )
                                .expect("rename migration");
                        }
                        "newer" => {
                            connection
                                .execute(
                                    "INSERT INTO schema_migrations(version,name,applied_at)
                                     VALUES(7,'future','2026-07-30T00:00:02Z')",
                                    [],
                                )
                                .expect("future migration");
                        }
                        _ => unreachable!(),
                    }
                }
            }
            let before = user_objects(&connection);
            let error = run(&mut connection).expect_err("ledger drift must fail");
            assert!(
                error.to_string().contains("MigrationLedgerInvalid"),
                "{error:#}"
            );
            assert_eq!(user_objects(&connection), before, "{case}");
            drop(connection);
            cleanup(&path);
        }
    }

    #[test]
    fn normal_database_connections_enforce_flow_foreign_keys() {
        let path = temp_db_path("foreign-key");
        let database = Database::open(&path).expect("migrated database");
        let connection = database.conn().expect("normal database connection");
        let error = connection
            .execute(
                "INSERT INTO flow_device_runs(id,run_id,udid,state)
                 VALUES('device-run-1','missing-run','MOCK-IPHONE-01','queued')",
                [],
            )
            .expect_err("missing parent run must fail");
        assert_eq!(
            error.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
        drop(connection);
        drop(database);
        cleanup(&path);
    }

    #[test]
    fn flow_schema_exposes_required_indexes_and_checks() {
        let path = temp_db_path("flow-schema");
        let database = Database::open(&path).expect("migrated database");
        let connection = database.conn().expect("inspect flow schema");
        let tables: Vec<String> = connection
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type='table' AND name LIKE 'flow_%' ORDER BY name",
            )
            .expect("prepare flow tables")
            .query_map([], |row| row.get(0))
            .expect("query flow tables")
            .collect::<Result<_, _>>()
            .expect("collect flow tables");
        assert_eq!(
            tables,
            vec![
                "flow_artifacts",
                "flow_device_runs",
                "flow_documents",
                "flow_events",
                "flow_node_attempts",
                "flow_revisions",
                "flow_runs",
            ]
        );
        let indexes: Vec<String> = connection
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type='index' AND name LIKE 'idx_flow_%' ORDER BY name",
            )
            .expect("prepare indexes")
            .query_map([], |row| row.get(0))
            .expect("query indexes")
            .collect::<Result<_, _>>()
            .expect("collect indexes");
        assert_eq!(
            indexes,
            vec![
                "idx_flow_artifacts_attempt",
                "idx_flow_attempts_state",
                "idx_flow_device_runs_state",
                "idx_flow_documents_updated",
                "idx_flow_events_revision",
                "idx_flow_runs_updated",
            ]
        );
        let error = connection
            .execute(
                "INSERT INTO flow_documents(
                    id,name,latest_revision,archived,created_at,updated_at
                 ) VALUES('flow-1','fixture',0,0,'now','now')",
                [],
            )
            .expect_err("invalid revision check");
        assert_eq!(
            error.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
        connection
            .execute(
                "INSERT INTO flow_documents(
                    id,name,latest_revision,archived,created_at,updated_at
                 ) VALUES('flow-1','fixture',1,0,'now','now')",
                [],
            )
            .expect("valid flow document");
        let error = connection
            .execute(
                "INSERT INTO flow_revisions(
                    flow_id,revision,authoring_json,compiled_json,plan_sha256,created_at
                 ) VALUES('flow-1',1,'{}','{}',?1,'now')",
                ["A".repeat(64)],
            )
            .expect_err("uppercase hash must fail");
        assert_eq!(
            error.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
        let valid_hash = "0".repeat(64);
        connection
            .execute(
                "INSERT INTO flow_revisions(
                    flow_id,revision,authoring_json,compiled_json,plan_sha256,created_at
                 ) VALUES('flow-1',1,'{}','{}',?1,'now')",
                [&valid_hash],
            )
            .expect("valid flow revision");
        let error = connection
            .execute(
                "INSERT INTO flow_runs(
                    id,flow_id,flow_revision,plan_sha256,selection_json,state,
                    created_at,updated_at
                 ) VALUES('run-1','flow-1',1,?1,'{}','unknown','now','now')",
                [&valid_hash],
            )
            .expect_err("unknown run state must fail");
        assert_eq!(
            error.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
        drop(connection);
        drop(database);
        cleanup(&path);
    }

    #[test]
    #[ignore = "writes the explicit rollback fixture path"]
    fn write_populated_legacy_fixture() {
        let path = PathBuf::from(
            std::env::var_os("RIVIU_LEGACY_FIXTURE_PATH").expect("RIVIU_LEGACY_FIXTURE_PATH"),
        );
        assert!(!path.exists(), "fixture path already exists");
        let mut connection = Connection::open(&path).expect("fixture database");
        apply_v1_schema(&connection).expect("legacy schema");
        let transaction = connection.transaction().expect("fixture transaction");
        insert_populated_legacy_rows(&transaction);
        transaction.commit().expect("fixture commit");
    }
}
