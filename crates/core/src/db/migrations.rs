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
    /// Whether this migration rebuilds a table — create-copy-drop-rename, the only way
    /// SQLite lets a `CHECK` constraint change.
    ///
    /// It exists because a rebuild is unsafe under the enforcement every production
    /// connection opens with (`foreign_keys=ON`, [`super::Database::conn`]). With foreign
    /// keys on, `DROP TABLE` runs an implicit `DELETE FROM` first, so dropping a parent
    /// cascades through every `ON DELETE CASCADE` child — for `interaction_campaigns` that
    /// is the entire campaign history — and `RENAME` rewrites the `REFERENCES` clauses of
    /// other tables. `PRAGMA foreign_keys` is a no-op inside a transaction, so the window
    /// has to be opened around the one [`apply_one`] opens, which is what this flag buys.
    ///
    /// A field rather than a list of versions kept elsewhere: a new migration that rebuilds
    /// a table and forgets to say so would silently erase rows, and the question being asked
    /// once per migration is cheaper than finding that out from a user's database.
    ///
    /// The compiler only forces a `bool` to be *written*, not the right one, so the
    /// correspondence is pinned by `a_migration_that_drops_or_renames_a_table_declares_it`
    /// instead. It has to be: `PRAGMA foreign_key_check` cannot catch the mistake, because a
    /// cascade **deletes** children rather than orphaning them — the check finds nothing wrong
    /// and the migration commits the loss.
    rebuilds_tables: bool,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "legacy-schema-baseline",
        apply: apply_migration_1,
        rebuilds_tables: false,
    },
    Migration {
        version: 2,
        name: "flow-v2-schema",
        apply: apply_migration_2,
        rebuilds_tables: false,
    },
    Migration {
        version: 3,
        name: "nurture-comment-attempts",
        apply: apply_migration_3,
        rebuilds_tables: false,
    },
    Migration {
        version: 4,
        name: "interaction-comment-threads",
        apply: apply_migration_4,
        rebuilds_tables: false,
    },
    Migration {
        version: 5,
        name: "publish-campaigns",
        apply: apply_migration_5,
        rebuilds_tables: false,
    },
    Migration {
        version: 6,
        name: "flow-ifvision-branch",
        apply: apply_migration_6,
        rebuilds_tables: false,
    },
    Migration {
        version: 7,
        name: "drop-local-users",
        apply: apply_migration_7,
        rebuilds_tables: false,
    },
    Migration {
        version: 8,
        name: "schedule-last-error",
        apply: apply_migration_8,
        rebuilds_tables: false,
    },
    Migration {
        version: 9,
        name: "device-account-handle",
        apply: apply_migration_9,
        rebuilds_tables: false,
    },
    Migration {
        version: 10,
        name: "device-alias-and-number",
        apply: apply_migration_10,
        rebuilds_tables: false,
    },
    Migration {
        version: 11,
        name: "drop-fabricated-comment-usd",
        apply: apply_migration_11,
        rebuilds_tables: false,
    },
    Migration {
        version: 12,
        name: "comment-distinct-frames",
        apply: apply_migration_12,
        rebuilds_tables: false,
    },
    Migration {
        version: 13,
        name: "comment-carousel-slides",
        apply: apply_migration_13,
        rebuilds_tables: false,
    },
    Migration {
        version: 14,
        name: "interaction-64-message-rebuild",
        apply: apply_migration_14,
        rebuilds_tables: true,
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
    if !migration.rebuilds_tables {
        return apply_one_transaction(connection, migration, failed_version);
    }

    // See [`Migration::rebuilds_tables`] for why this window has to exist, and why it has
    // to be out here rather than inside the transaction.
    let was_on: bool = connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
    if !was_on {
        return apply_one_transaction(connection, migration, failed_version);
    }
    connection.pragma_update(None, "foreign_keys", "OFF")?;
    let applied = apply_one_transaction(connection, migration, failed_version);
    // Restored whether the migration committed or rolled back, and the return value carries
    // a failure to restore rather than swallowing it.
    //
    // Not because the caller keeps this connection — it does not. `Database::migrate` owns a
    // local `Connection` and drops it on return, and `foreign_keys` is per-connection and not
    // persisted, so enforcement cannot actually leak out of here. The reason is narrower: a
    // pragma that refuses to go back on means this connection is in a state nothing else in
    // the file reasons about, and the rest of `migrate` still runs on it. There is no RAII
    // guard, so a panic inside the transaction skips this line — harmless for the same reason,
    // and `Transaction`'s own `Drop` still rolls the migration back.
    let restored = connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(anyhow::Error::from);
    applied.and(restored)
}

fn apply_one_transaction(
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

fn apply_migration_7(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // The local login is gone, so the table that held its credentials goes with it.
    //
    // **This is the point of the change, not a tidy-up.** `register_user` wrote the password
    // verbatim into a column named `password_hash` and `login_user` compared it as plaintext,
    // so anyone who could read `riviu.db` — a synced folder, a backup, a support bundle,
    // another account on the machine — could read every operator password. Removing the login
    // surface while leaving the rows behind would leave the exposure exactly where it was.
    //
    // One-way on purpose. Nothing reads this table any more, the login UI is gone, and there
    // is no path that could restore a row and still have somewhere to use it.
    transaction.execute_batch("DROP TABLE IF EXISTS users;")?;
    Ok(())
}

/// Give a schedule somewhere to say why it did not run.
///
/// A schedule whose script has been renamed or deleted used to advance `last_run_at` and
/// `next_run_at` on every tick while enqueueing nothing — the two `if let Ok(...)` guards
/// around the lookup and the parse both fell through in silence. On the schedules page it
/// read as a job that had run two minutes ago and would run again in an hour, forever.
///
/// Nullable and added rather than backfilled: an existing schedule has no failure to
/// describe until the next tick decides one way or the other.
fn apply_migration_8(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch("ALTER TABLE schedules ADD COLUMN last_error TEXT;")?;
    Ok(())
}

fn apply_migration_9(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // The TikTok @handle a phone is logged into, so an interaction @-mention can resolve to
    // the owning phone. Kept out of the V1 baseline schema on purpose, so legacy-DB
    // detection (`expected_v1_fingerprint`) still recognises pre-ledger databases.
    transaction
        .execute_batch("ALTER TABLE device_meta ADD COLUMN handle TEXT NOT NULL DEFAULT '';")?;
    Ok(())
}

fn apply_migration_10(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // What the operator calls this phone and the number written on it (xiaowei "Change Name"
    // / "Change Number"). Two columns rather than one, because they answer different
    // questions: the alias is how a tile is labelled, the number is how the fleet is ordered
    // and what goes on the sticker. `number` is nullable — unnumbered is a real state, and a
    // default of 0 would put every phone in the fleet at position zero.
    transaction.execute_batch(
        "ALTER TABLE device_meta ADD COLUMN alias TEXT NOT NULL DEFAULT '';\n\
         ALTER TABLE device_meta ADD COLUMN number INTEGER;",
    )?;
    Ok(())
}

fn apply_migration_11(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **Dropping a column that was always a guess.** The `usd` in both comment tables was
    // `prompt_tokens * input_price_per_1m + completion_tokens * output_price_per_1m`, over two
    // numbers typed into the settings blob by hand and never sent to the API. Three different
    // pairs of them existed in the codebase at once — `types.rs` said $0.10/$0.60, `db.rs`
    // said $1.25/$10.00, and a migration rewrote the second back to the first — and no UI
    // could edit any of them, so after any model change every figure in this column was
    // silently wrong.
    //
    // Dropped rather than left in place and ignored. A column reading 0.0 next to a real
    // token count reads as "this comment was free", which is a worse lie than the one being
    // removed: `prompt_tokens` and `completion_tokens` stay, they come from the API's own
    // `usage` object, and they are true of whatever model is configured. Multiply by the
    // provider's real rate outside the app.
    //
    // `DROP COLUMN` needs SQLite 3.35+; rusqlite 0.32 bundles well past that.
    transaction.execute_batch(
        "ALTER TABLE nurture_comment_attempts DROP COLUMN usd;
         ALTER TABLE nurture_comment_costs DROP COLUMN usd;",
    )?;
    Ok(())
}

fn apply_migration_12(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **How many different frames the model was actually shown.** The contact sheet always
    // carried three thumbnails, and on a photo post all three were the same picture — a still
    // card publishes byte-identical frames (measured: 0 of 2,170,800 picture pixels changed
    // over 13 s untouched, and the repo's own `card_is_still` found 4 of 40 cards still).
    // `evidence_support` therefore had two very different meanings that looked identical in
    // this table: "the model read the post badly" and "there was only ever one frame to read".
    //
    // Nullable on purpose, with no default. Every row written before this build was grounded
    // on an unknown number of distinct frames, and writing `3` into them would invent a
    // measurement — exactly the mistake migration 11 removed. NULL reads as "not recorded".
    transaction.execute_batch(
        "ALTER TABLE nurture_comment_attempts ADD COLUMN distinct_frames INTEGER;",
    )?;
    Ok(())
}

fn apply_migration_13(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **The other half of migration 12's number.** `distinct_frames = 1` is ambiguous on its
    // own: it is what a photo post looks like when the stream is parked on one picture, and
    // also what a still video looks like, and also what a post the pager never turned looks
    // like. Paired with the slides the traversal actually paged, each of those reads
    // differently — and the deferral that grounds a comment on more than image one cannot be
    // told apart from the old behaviour without it.
    //
    // Nullable, no default, same discipline as 12: rows written before this know nothing
    // about how many slides they saw, and `0` would be a claim rather than an absence.
    transaction.execute_batch(
        "ALTER TABLE nurture_comment_attempts ADD COLUMN carousel_slides INTEGER;",
    )?;
    Ok(())
}

fn apply_migration_14(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **The interaction schema said a thread could hold six messages; the engine has said
    // sixty-four since the cohort work landed.** `MIN_MESSAGE_COUNT`/`MAX_MESSAGE_COUNT` are
    // `2..=64` and validation demands `message_count >= the largest cohort`, so a single
    // cohort over the fourteen phones attached to this box needs fourteen messages — and
    // `CREATE TABLE interaction_campaigns` still carried `CHECK (message_count BETWEEN 2 AND
    // 6)` from migration 4. Every whole-fleet campaign therefore died inside
    // `create_interaction_campaign` as a SQLite CHECK violation, surfaced as `OperationFailed`
    // with no hint that the number the operator typed was the problem. The UI offered 2..=64
    // the whole time.
    //
    // A CHECK cannot be altered, so both tables are rebuilt. Read
    // [`Migration::rebuilds_tables`] before touching this: it only runs with foreign keys
    // off, and it must stay that way.
    //
    // The three dead tables go with it, because a rebuild is the one moment their absence is
    // free. All three are documented in migration 4 as never read: `interaction_events` has
    // no writer at all and its shape invites "no events, so nothing happened";
    // `interaction_retry_requests` is never touched by anything; `interaction_dispatch` is
    // INSERT-only and shaped like a single-owner lease nothing ever claims, which is worse
    // than absent — a future reader could mistake the row for proof of an owner. Its one
    // writer in `create_interaction_campaign` goes in the same commit as this migration.
    //
    // `interaction_campaign_actors` and `interaction_targets.state` are deliberately kept:
    // the first has a real writer and reads as provenance, the second is a documented
    // default-only column, and neither is worth widening this migration's blast radius for.
    transaction.execute_batch(
        r#"
DROP TABLE interaction_events;
DROP TABLE interaction_retry_requests;
DROP TABLE interaction_dispatch;

CREATE TABLE interaction_campaigns_new (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  request_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('queued','running','succeeded','partial','failed','cancelled')),
  message_count INTEGER NOT NULL CHECK (message_count BETWEEN 2 AND 64),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

INSERT INTO interaction_campaigns_new
  (id,request_id,request_json,state,message_count,revision,error_code,created_at,updated_at)
  SELECT id,request_id,request_json,state,message_count,revision,error_code,created_at,updated_at
  FROM interaction_campaigns;

DROP TABLE interaction_campaigns;
ALTER TABLE interaction_campaigns_new RENAME TO interaction_campaigns;
CREATE INDEX idx_interaction_campaigns_updated ON interaction_campaigns(updated_at DESC);

CREATE TABLE interaction_assignments_new (
  id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  target_id TEXT NOT NULL,
  message_ordinal INTEGER NOT NULL CHECK (message_ordinal BETWEEN 0 AND 63),
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

INSERT INTO interaction_assignments_new
  (id,campaign_id,target_id,message_ordinal,actor_udid,parent_assignment_id,prepared_json,
   state,effect_intent,evidence_json,error_code,revision,created_at,updated_at)
  SELECT id,campaign_id,target_id,message_ordinal,actor_udid,parent_assignment_id,prepared_json,
         state,effect_intent,evidence_json,error_code,revision,created_at,updated_at
  FROM interaction_assignments;

DROP TABLE interaction_assignments;
ALTER TABLE interaction_assignments_new RENAME TO interaction_assignments;
CREATE INDEX idx_interaction_assignments_target ON interaction_assignments(target_id, message_ordinal);
CREATE INDEX idx_interaction_assignments_state ON interaction_assignments(campaign_id, state);
"#,
    )?;

    // Enforcement is off for the whole rebuild, so nothing above would have complained about
    // a child left pointing at a row the copy dropped. Checked here, inside the transaction,
    // so a bad copy rolls the whole thing back instead of shipping a database whose children
    // are orphans. Scoped to the interaction tables on purpose: a pre-existing violation
    // somewhere else in the file is not this migration's to refuse.
    for table in [
        "interaction_campaign_actors",
        "interaction_targets",
        "interaction_assignments",
        "interaction_artifacts",
    ] {
        let orphans = transaction
            .prepare(&format!("PRAGMA foreign_key_check({table})"))?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .len();
        if orphans > 0 {
            anyhow::bail!(
                "InteractionRebuildLostReferences: {orphans} row(s) in {table} no longer resolve to a parent"
            );
        }
    }
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

    fn column_exists(connection: &Connection, table: &str, column: &str) -> bool {
        connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .and_then(|mut statement| {
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    if row.get::<_, String>(1)? == column {
                        return Ok(true);
                    }
                }
                Ok(false)
            })
            .expect("column existence")
    }

    /// The interaction rebuild has to change two CHECK constraints without touching a single
    /// row — and it runs with foreign keys off, which is exactly when a wrong `DROP` order
    /// erases a campaign's children in silence.
    ///
    /// So this stops at 13, fills every interaction table by hand, and then proves four
    /// separate things about the rebuild: the rows come out byte-identical, the three dead
    /// tables are gone, the new bounds are the ones in force at both ends, and enforcement
    /// still works afterwards (a campaign delete still cascades). The pragma is set ON here
    /// deliberately — `Database::conn` does that in production, and it is the state the
    /// rebuild is dangerous in.
    /// A migration whose body drops or renames a table has to declare `rebuilds_tables`.
    ///
    /// The flag is a correspondence the compiler cannot check: it forces a `bool` to be
    /// *written*, not the right one. And the `PRAGMA foreign_key_check` in `apply_migration_14`
    /// cannot catch the mistake either, because a cascade **deletes** children rather than
    /// orphaning them — the check finds zero bad rows and the migration commits the loss. The
    /// only thing that can catch it before a user's database does is the source.
    ///
    /// Exemptions are written down one line each, the way `ALLOWED_HELPERS` is. Dropping a
    /// table nothing references is genuinely safe; naming which table and why is the price of
    /// skipping the window.
    #[test]
    fn a_migration_that_drops_or_renames_a_table_declares_it() {
        const EXEMPT: &[(i64, &str)] = &[(
            7,
            "drops `users`, which no table references — no cascade, so nothing to lose",
        )];

        let source = include_str!("migrations.rs");
        // What the array says.
        let declared: Vec<(i64, bool)> = source
            .split("Migration {")
            .skip(1)
            .filter_map(|block| {
                let head = &block[..block.find('}')?];
                let version = head
                    .lines()
                    .find_map(|line| line.trim().strip_prefix("version: "))?
                    .trim_end_matches(',')
                    .parse::<i64>()
                    .ok()?;
                Some((version, head.contains("rebuilds_tables: true")))
            })
            .collect();
        assert_eq!(
            declared.len(),
            crate::db::migrations::MIGRATIONS.len(),
            "the scan did not find every entry in MIGRATIONS, so it proves nothing"
        );

        // What the bodies do. Each function is a top-level item, so it ends at the first `}`
        // in the first column.
        let mut rebuilds = Vec::new();
        for chunk in source.split("\nfn apply_migration_").skip(1) {
            let Some((number, rest)) = chunk.split_once('(') else {
                continue;
            };
            let Ok(version) = number.parse::<i64>() else {
                continue;
            };
            let body = rest.split("\n}\n").next().unwrap_or(rest);
            if body.contains("DROP TABLE") || body.contains("RENAME TO") {
                rebuilds.push(version);
            }
        }
        assert!(
            rebuilds.contains(&14),
            "migration 14 rebuilds two tables; a scan that cannot see it cannot see anything"
        );

        let undeclared: Vec<i64> = rebuilds
            .iter()
            .copied()
            .filter(|version| !EXEMPT.iter().any(|(exempt, _)| exempt == version))
            .filter(|version| !declared.iter().any(|(v, flag)| v == version && *flag))
            .collect();
        assert!(
            undeclared.is_empty(),
            "migration(s) {undeclared:?} drop or rename a table without `rebuilds_tables: true`. \
             Under the `foreign_keys=ON` every production connection opens with, that cascades \
             children away, and `PRAGMA foreign_key_check` cannot see it because the rows are \
             deleted rather than orphaned. Declare the flag, or add a written exemption."
        );

        // Stale exemptions are their own hazard: one that no longer describes anything is a
        // hole standing open for the next migration to fall into.
        for (version, reason) in EXEMPT {
            assert!(
                rebuilds.contains(version),
                "migration {version} is exempted ({reason}) but no longer drops or renames \
                 anything — remove the exemption"
            );
        }
    }

    #[test]
    fn migration_14_relaxes_the_check_without_rewriting_interaction_rows() {
        let path = temp_db_path("interaction-64-rebuild");
        let mut connection = Connection::open(&path).expect("rebuild fixture");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("enforce like production does");
        run_with_failpoint(&mut connection, Some(14)).expect_err("stop before the rebuild");
        connection
            .execute_batch(
                "INSERT INTO interaction_campaigns
                   (id,request_id,request_json,state,message_count,revision,error_code,created_at,updated_at)
                   VALUES('camp-1','req-1','{\"requestId\":\"req-1\"}','partial',2,3,'lỗi cũ','2026-08-01T00:00:00Z','2026-08-01T01:00:00Z');
                 INSERT INTO interaction_campaign_actors(campaign_id,actor_ordinal,udid)
                   VALUES('camp-1',0,'udid-a');
                 INSERT INTO interaction_targets
                   (id,campaign_id,line_no,original_url,normalized_url,target_key,content_id,kind,created_at)
                   VALUES('tgt-1','camp-1',1,'https://x/1','https://y/1','content:1','1','photo','2026-08-01T00:00:00Z');
                 INSERT INTO interaction_assignments
                   (id,campaign_id,target_id,message_ordinal,actor_udid,prepared_json,state,
                    effect_intent,evidence_json,error_code,revision,created_at,updated_at)
                   VALUES('asg-1','camp-1','tgt-1',0,'udid-a','{\"text\":\"chào\"}','succeeded',
                          'post_comment','{\"reader\":\"hierarchy\"}',NULL,5,
                          '2026-08-01T00:00:00Z','2026-08-01T00:30:00Z');
                 INSERT INTO interaction_assignments
                   (id,campaign_id,target_id,message_ordinal,actor_udid,parent_assignment_id,
                    state,error_code,created_at,updated_at)
                   VALUES('asg-2','camp-1','tgt-1',1,'udid-b','asg-1','skipped_parent',
                          'parent_identity_not_confirmed_at_ordinal_0',
                          '2026-08-01T00:00:00Z','2026-08-01T00:31:00Z');
                 INSERT INTO interaction_artifacts
                   (id,campaign_id,target_id,assignment_id,kind,metadata_json,relative_path,sha256,created_at)
                   VALUES('art-1','camp-1','tgt-1','asg-1','comment-root-evidence','{}','a/b.jpg','beef','2026-08-01T00:32:00Z');",
            )
            .expect("seed the pre-rebuild interaction rows");
        let before = read_interaction_rows(&connection);

        run(&mut connection).expect("apply the rebuild");

        assert_eq!(
            read_interaction_rows(&connection),
            before,
            "the rebuild copied rows, it must not have rewritten them"
        );
        assert_eq!(
            migration_rows(&connection)
                .iter()
                .map(|(version, _)| *version)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
        );
        assert!(!table_exists(&connection, "interaction_events"));
        assert!(!table_exists(&connection, "interaction_dispatch"));
        assert!(!table_exists(&connection, "interaction_retry_requests"));

        // Both ends of the new range, on both rebuilt tables. Sixty-four is the number the
        // engine has always allowed; sixty-five must still be refused, or the CHECK stopped
        // being a bound and became decoration.
        for (message_count, allowed) in [(64_i64, true), (65, false)] {
            let attempt = connection.execute(
                "INSERT INTO interaction_campaigns
                 (id,request_id,request_json,state,message_count,created_at,updated_at)
                 VALUES(?1,?1,'{}','queued',?2,'t','t')",
                params![format!("camp-{message_count}"), message_count],
            );
            assert_eq!(attempt.is_ok(), allowed, "message_count {message_count}");
        }
        for (ordinal, allowed) in [(63_i64, true), (64, false)] {
            let attempt = connection.execute(
                "INSERT INTO interaction_assignments
                 (id,campaign_id,target_id,message_ordinal,actor_udid,created_at,updated_at)
                 VALUES(?1,'camp-1','tgt-1',?2,'udid-z','t','t')",
                params![format!("asg-{ordinal}"), ordinal],
            );
            assert_eq!(attempt.is_ok(), allowed, "message_ordinal {ordinal}");
        }

        // The rebuild dropped and renamed two tables with enforcement off, and
        // `interaction_assignments` references *itself* — the one clause that had to keep
        // resolving to the renamed table rather than the one that was dropped. Proof it did:
        // deleting the campaign is refused while a reply still points at its parent, because
        // the cascade reaches `asg-1` and the self-FK is `ON DELETE RESTRICT`. A clause left
        // dangling would have let this through.
        let restricted = connection
            .execute("DELETE FROM interaction_campaigns WHERE id='camp-1'", [])
            .expect_err("the self-reference must still restrict");
        assert!(
            restricted.to_string().contains("FOREIGN KEY"),
            "expected the parent reference to refuse the delete, got {restricted}"
        );

        // With the reply gone the same delete must now reach every child, which is the other
        // half: the cascades survived too.
        connection
            .execute("DELETE FROM interaction_assignments WHERE id='asg-2'", [])
            .expect("drop the reply that pinned its parent");
        connection
            .execute("DELETE FROM interaction_campaigns WHERE id='camp-1'", [])
            .expect("delete the campaign");
        for (table, remaining) in [
            ("interaction_campaign_actors", 0_i64),
            ("interaction_targets", 0),
            ("interaction_assignments", 0),
            ("interaction_artifacts", 0),
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count survivors");
            assert_eq!(count, remaining, "{table} did not cascade");
        }

        drop(connection);
        cleanup(&path);
    }

    /// Every interaction row as raw bytes, so a rebuild that "helpfully" reformats a value
    /// fails the comparison instead of passing it.
    fn read_interaction_rows(connection: &Connection) -> Vec<Vec<Option<Vec<u8>>>> {
        let mut rows = Vec::new();
        for query in [
            "SELECT CAST(id AS BLOB),CAST(request_id AS BLOB),CAST(request_json AS BLOB),
                    CAST(state AS BLOB),CAST(message_count AS BLOB),CAST(revision AS BLOB),
                    CAST(error_code AS BLOB),CAST(created_at AS BLOB),CAST(updated_at AS BLOB)
             FROM interaction_campaigns ORDER BY id",
            "SELECT CAST(campaign_id AS BLOB),CAST(actor_ordinal AS BLOB),CAST(udid AS BLOB),
                    CAST(state AS BLOB),CAST(error_code AS BLOB),NULL,NULL,NULL,NULL
             FROM interaction_campaign_actors ORDER BY campaign_id,actor_ordinal",
            "SELECT CAST(id AS BLOB),CAST(campaign_id AS BLOB),CAST(target_key AS BLOB),
                    CAST(content_id AS BLOB),CAST(kind AS BLOB),CAST(state AS BLOB),
                    CAST(normalized_url AS BLOB),CAST(created_at AS BLOB),NULL
             FROM interaction_targets ORDER BY id",
            "SELECT CAST(id AS BLOB),CAST(campaign_id AS BLOB),CAST(message_ordinal AS BLOB),
                    CAST(actor_udid AS BLOB),CAST(parent_assignment_id AS BLOB),
                    CAST(prepared_json AS BLOB),CAST(state AS BLOB),CAST(evidence_json AS BLOB),
                    CAST(error_code AS BLOB)
             FROM interaction_assignments ORDER BY id",
            "SELECT CAST(id AS BLOB),CAST(campaign_id AS BLOB),CAST(assignment_id AS BLOB),
                    CAST(kind AS BLOB),CAST(relative_path AS BLOB),CAST(sha256 AS BLOB),
                    CAST(created_at AS BLOB),NULL,NULL
             FROM interaction_artifacts ORDER BY id",
        ] {
            let mut statement = connection.prepare(query).expect("prepare interaction read");
            let mapped = statement
                .query_map([], |row| {
                    (0..9)
                        .map(|index| row.get::<_, Option<Vec<u8>>>(index))
                        .collect::<Result<Vec<_>, _>>()
                })
                .expect("read interaction rows")
                .collect::<Result<Vec<_>, _>>()
                .expect("collect interaction rows");
            rows.extend(mapped);
        }
        rows
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
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
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
        for failed_version in [1, 2, 3, 4, 5, 6, 7, 14] {
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
            } else if failed_version == 6 {
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
            } else if failed_version == 7 {
                // Failed at 7: everything before it applied, and the drop rolled back --
                // so the credentials table is still there. That is the assertion that
                // matters: a half-applied removal must not look like a completed one.
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3, 4, 5, 6]
                );
                assert!(table_exists(&connection, "users"));
            } else if failed_version == 14 {
                // Failed at 14: the interaction rebuild rolled back whole. This is the one
                // rollback in the file that can destroy data rather than just leave work
                // undone — it drops three tables and two more out from under their children
                // — so the assertion is that the *old* schema is byte-for-byte still in
                // charge: the dead tables are back, and the old CHECK still refuses the
                // seventh message the rebuild exists to allow.
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13]
                );
                assert!(table_exists(&connection, "interaction_events"));
                assert!(table_exists(&connection, "interaction_dispatch"));
                assert!(table_exists(&connection, "interaction_retry_requests"));
                let refused = connection
                    .execute(
                        "INSERT INTO interaction_campaigns
                         (id,request_id,request_json,state,message_count,created_at,updated_at)
                         VALUES('c','r','{}','queued',7,'t','t')",
                        [],
                    )
                    .expect_err("the pre-rebuild CHECK is still the one in force");
                assert!(
                    refused.to_string().contains("CHECK"),
                    "expected the old CHECK to refuse 7 messages, got {refused}"
                );
            } else {
                // Failed at 8: the schedules table is back to the shape it had before the
                // column was added. A rolled-back `ALTER TABLE` that left the column behind
                // would make the retry fail with "duplicate column name" forever.
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    vec![1, 2, 3, 4, 5, 6, 7]
                );
                assert!(!table_exists(&connection, "users"));
                assert!(!column_exists(&connection, "schedules", "last_error"));
            }

            run(&mut connection).expect("retry migrations");
            assert_eq!(
                migration_rows(&connection)
                    .iter()
                    .map(|(version, _)| *version)
                    .collect::<Vec<_>>(),
                vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
            );
            // The local login is gone and migration 7 takes its credentials with it. This
            // used to assert the seeded `guest@local` row existed; the point of the change
            // is that it does not, and neither does the table that stored passwords in
            // plaintext under a column named `password_hash`.
            assert!(!table_exists(&connection, "users"));
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
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
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
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
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
                                    // One past the highest real migration: the point is a
                                    // ledger from a NEWER build, so this has to move
                                    // whenever a migration is added.
                                    "INSERT INTO schema_migrations(version,name,applied_at)
                                     VALUES(15,'future','2026-07-30T00:00:02Z')",
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
