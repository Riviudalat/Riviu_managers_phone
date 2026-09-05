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
    Migration {
        version: 15,
        name: "comment-cost-reported-by-gateway",
        apply: apply_migration_15,
        rebuilds_tables: false,
    },
    Migration {
        version: 16,
        name: "drop-tables-nothing-reads",
        apply: apply_migration_16,
        rebuilds_tables: false,
    },
    Migration {
        version: 17,
        name: "publish-sheet-outbox",
        apply: apply_migration_17,
        rebuilds_tables: false,
    },
    Migration {
        version: 18,
        name: "sheet-outbox-outlives-its-parents",
        apply: apply_migration_18,
        rebuilds_tables: true,
    },
    Migration {
        version: 19,
        name: "app-library-artifact-metadata",
        apply: apply_migration_19,
        rebuilds_tables: false,
    },
    Migration {
        version: 20,
        name: "versioned-automation-definitions",
        apply: apply_migration_20,
        rebuilds_tables: false,
    },
    Migration {
        version: 21,
        name: "interaction-public-action-runs",
        apply: apply_migration_21,
        rebuilds_tables: false,
    },
    Migration {
        version: 22,
        name: "orchestration-v1",
        apply: apply_migration_22,
        rebuilds_tables: false,
    },
    Migration {
        version: 23,
        name: "orchestration-cancelled-child-proof",
        apply: apply_migration_23,
        rebuilds_tables: true,
    },
    Migration {
        version: 24,
        name: "orchestration-confirmed-targets-and-nurture-children",
        apply: apply_migration_24,
        rebuilds_tables: false,
    },
    Migration {
        version: 25,
        name: "automation-schedule-occurrences",
        apply: apply_migration_25,
        rebuilds_tables: false,
    },
    Migration {
        version: 26,
        name: "interaction-action-only-message-count",
        apply: apply_migration_26,
        rebuilds_tables: true,
    },
    Migration {
        version: 27,
        name: "publish-execution-snapshots",
        apply: apply_migration_27,
        rebuilds_tables: false,
    },
    Migration {
        version: 28,
        name: "nurture-run-history",
        apply: apply_migration_28,
        rebuilds_tables: false,
    },
    Migration {
        version: 29,
        name: "public-effect-cleanup-journal",
        apply: apply_migration_29,
        rebuilds_tables: false,
    },
    Migration {
        version: 30,
        name: "nurture-follow-source-identities",
        apply: apply_migration_30,
        rebuilds_tables: false,
    },
    Migration {
        version: 31,
        name: "library-batch-operation-ledger",
        apply: apply_migration_31,
        rebuilds_tables: false,
    },
];

fn apply_migration_31(tx: &Transaction<'_>) -> anyhow::Result<()> {
    tx.execute_batch(
        "CREATE TABLE library_batches (
            id TEXT PRIMARY KEY,
            kind TEXT NOT NULL CHECK(kind IN ('appInstall','materialTransfer')),
            artifact_id TEXT NOT NULL,
            title TEXT NOT NULL,
            target_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
         );
         CREATE TABLE library_batch_items (
            batch_id TEXT NOT NULL REFERENCES library_batches(id),
            udid TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            label TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('queued','running','succeeded','failed','uncertain','cancelled')),
            error_code TEXT,
            detail TEXT,
            evidence TEXT,
            PRIMARY KEY(batch_id,udid)
         );
         CREATE INDEX library_batches_updated ON library_batches(updated_at);
         CREATE INDEX library_batch_items_state ON library_batch_items(state,batch_id);",
    )?;
    Ok(())
}

pub(super) fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}

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

/// Put a money column back, and this time let it say "I do not know".
///
/// **Migration 11 was right to drop the old one, and its reasoning is why this column is
/// nullable.** That `usd` was `tokens * a_price_table_typed_into_the_source`; three different
/// pairs of numbers existed at once, no UI could edit any of them, and after any model change
/// every figure was silently wrong. It was dropped rather than zeroed, because — in migration
/// 11's own words — a column reading `0.0` beside a real token count reads as *this comment was
/// free*, which is a worse lie than the one being removed.
///
/// That objection applies to a gateway that does not report a price, and it is answered by the
/// type rather than by argument: `REAL` with no `NOT NULL`, so "the biller did not say" is
/// `NULL` and is never confused with "it cost nothing". What goes in is
/// `usage.cost` out of the response — measured 25/08/2026 on OpenRouter, which returns it when
/// asked — so the number is a report, not a multiplication done here.
///
/// A new name as well as a new shape. `usd` is the name of the fabricated one, it is in old
/// backups and in this file's history, and reusing it would make the two indistinguishable to
/// anyone reading a database without reading these comments.
fn apply_migration_15(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch("ALTER TABLE nurture_comment_attempts ADD COLUMN cost_usd REAL;")?;
    Ok(())
}

fn apply_migration_16(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **Five tables and one column that no code reads, and two of them are worse than absent.**
    //
    // `rebuilds_tables: false` deliberately, and the reason is checked rather than assumed:
    // **not one of these tables is the parent of a cascade.** `proxies` is referenced by
    // nothing -- `device_meta.proxy_id` is a plain `TEXT` column with no `FOREIGN KEY` clause at
    // all. `tiktok_accounts`, `publish_tasks` and `nurture_comment_costs` have no foreign keys
    // in either direction. `publish_dispatch` has one, and it points *outward*
    // (`campaign_id REFERENCES publish_campaigns(id) ON DELETE CASCADE`), so it is a child:
    // cascades run parent to child, never back. So dropping these with enforcement **on** is
    // safe, and leaving it on is stricter than the window `rebuilds_tables` would open.
    //
    // Table by table:
    //
    // * **`proxies`** -- a whole dead vertical slice: four commands, three DB methods, a type on
    //   both sides of the wire and an icon, none of them reachable from any UI (`Sidebar.tsx`
    //   records the removal: "groups/proxy/team removed"). It also held a **plaintext password
    //   column** for a feature that does not exist, which is a live risk rather than harmless
    //   clutter. Confirmed with the operator before dropping.
    //
    // * **`publish_dispatch`** -- INSERT and UPDATE only, **no `SELECT` anywhere**, with `owner`
    //   and `claimed_at` written as hard-coded `NULL`. That is byte for byte the shape migration
    //   14 removed `interaction_dispatch` for, in its own words: *"shaped like a single-owner
    //   lease nothing ever claims, which is worse than absent -- a future reader could mistake
    //   the row for proof of an owner."* Same hazard, same argument, missed table.
    //
    // * **`tiktok_accounts`** -- never written, never read, and it survived migration 14 only
    //   because it was not named in the comment that swept its three siblings.
    //
    // * **`publish_tasks`** -- the legacy publish path, labelled "(legacy script compatibility)"
    //   at its commands and superseded by `publish_campaigns` / `publish_bundles` /
    //   `publish_assignments`.
    //
    // * **`nurture_comment_costs`** -- write-only in production: the app paid one INSERT per
    //   comment for rows whose only reader was a command the frontend never called. The number
    //   an operator actually wants comes from `nurture_comment_attempts`, which stays, and
    //   `nurture_cost_summary` reads it.
    //
    // * **`device_meta.proxy_id`** -- always `None`, because nothing could ever set it once the
    //   proxy UI was removed. `DROP COLUMN` needs SQLite 3.35+; migration 11 already relies on
    //   that and rusqlite 0.32 bundles well past it. The column is neither a key nor indexed,
    //   which is the other restriction `DROP COLUMN` has.
    //
    // Nothing readable is lost. Every one of these had no read path, so there is no view an
    // operator could open today that shows less tomorrow.
    transaction.execute_batch(
        r#"
DROP TABLE proxies;
DROP TABLE tiktok_accounts;
DROP TABLE publish_dispatch;
DROP TABLE publish_tasks;
DROP TABLE nurture_comment_costs;
ALTER TABLE device_meta DROP COLUMN proxy_id;
"#,
    )?;
    Ok(())
}

fn apply_migration_17(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **A table, because "post the link to a Sheet" is a second delivery and must be able to
    // fail on its own.**
    //
    // The operator's requirement is that every published carousel puts its link in column D of
    // a partners sheet, `bot` as the poster, and the partner names from the campaign's
    // `partners-setN.xlsx` from column K onward. The naive shape is an HTTP call at the end of
    // the post step. That shape is wrong in a way this project has already paid for once: a
    // network error would then make a **published** post look like a failed one, and a failed
    // post is the thing the operator retries.
    //
    // So the row is written here first, in the same transaction that records the post, and
    // pushed afterwards. `state` distinguishes the two deliveries, and the push can be retried
    // by itself without touching the post.
    //
    // # Why `assignment_id` is the primary key
    //
    // One assignment publishes at most one carousel, so it can owe the sheet at most one row.
    // Making that the key rather than a fresh id means a retry that re-inserts cannot create a
    // second row for the same post — the operator would see the same link twice in column D
    // and have no way to tell which one to delete.
    //
    // # `ON DELETE CASCADE`, deliberately
    //
    // A campaign deleted from the app takes its unsent outbox rows with it. A row already
    // `sent` is *history that lives in the sheet*, not here, so losing this copy loses nothing
    // an operator can act on.
    //
    // `partners_json` is a JSON array of the names read out of the workbook. Stored as text
    // rather than one row per name because the sheet wants them spread across columns K
    // onward in their original order, which is an array, not a set.
    transaction.execute_batch(
        r#"
CREATE TABLE publish_sheet_outbox (
  assignment_id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  post_url TEXT NOT NULL,
  poster TEXT NOT NULL,
  partners_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pending','sent','failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  last_error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (assignment_id) REFERENCES publish_assignments(id) ON DELETE CASCADE,
  FOREIGN KEY (campaign_id) REFERENCES publish_campaigns(id) ON DELETE CASCADE
);
CREATE INDEX publish_sheet_outbox_pending
  ON publish_sheet_outbox(state) WHERE state <> 'sent';
"#,
    )?;
    Ok(())
}

fn apply_migration_18(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // **The obligation is about a post that exists in the world, so it must not be a child of
    // rows in this database.**
    //
    // Migration 17 gave the outbox `ON DELETE CASCADE` on both `publish_campaigns` and
    // `publish_assignments`, with a comment arguing that losing an unsent row costs nothing.
    // A review took that apart and it is wrong in the one direction that matters: a carousel
    // publishes, the webhook is down, the row sits `failed` — and the operator deletes the
    // campaign. SQLite quietly removes the only record that a live, undeletable post still
    // owes the sheet a link. Nothing can ever add it, and its absence is exactly what would
    // encourage publishing it again.
    //
    // So the parents go. `campaign_id` stays as plain text, for grouping and for the
    // operator's eye; it no longer decides whether the row lives. That also removes the
    // mismatch a separate finding named — two independent keys let (assignment A, campaign B)
    // insert cleanly, so deleting B took A's obligation with it.
    //
    // # Three more things the rebuild is the moment to fix
    //
    // * **`post_url` becomes unique.** One URL is one post is one row. Without it, a restored
    //   campaign that hands a new assignment id to an already-captured link writes the same
    //   link into column D twice, and both rows look ordinary.
    //   And when a v17 database really does hold two rows for one URL, the survivor is
    //   **chosen, not left to `GROUP BY`'s whim**: a `sent` row beats an unsent one (keeping
    //   the `failed` twin would make the sweep re-deliver a link the sheet already has —
    //   the duplicate column D this index exists to prevent), then the newest `updated_at`,
    //   then the highest rowid. A bare-column `GROUP BY` let SQLite pick any of them.
    //   (Editing this migration in place was safe when it happened: the outbox is not yet
    //   called by the publish path, so no fleet database had rows for the old copy to have
    //   mangled — the only DBs past v18 were dev machines with empty outboxes.)
    // * **`revision`**, bumped whenever the row's content changes. A sweep that read version 3
    //   and delivered it must not mark version 4 sent — which is what "update by id" did, and
    //   the newer URL then never travelled.
    // * **Empty text is refused.** `NOT NULL` was doing nothing about `""`, and an empty
    //   `post_url` is a row that is eligible forever and rejected by the script every time.
    //
    // `rebuilds_tables: true`, and the window it opens is not needed for a cascade — this
    // table is nobody's parent — but it is the honest declaration for a `DROP TABLE`.
    transaction.execute_batch(
        r#"
CREATE TABLE publish_sheet_outbox_new (
  assignment_id TEXT PRIMARY KEY,
  campaign_id TEXT NOT NULL,
  post_url TEXT NOT NULL,
  poster TEXT NOT NULL,
  partners_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('pending','sent','failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  last_error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (length(trim(assignment_id)) > 0),
  CHECK (length(trim(campaign_id)) > 0),
  CHECK (length(trim(post_url)) > 0),
  CHECK (length(trim(poster)) > 0)
);
INSERT INTO publish_sheet_outbox_new(
  assignment_id,campaign_id,post_url,poster,partners_json,state,attempts,revision,
  last_error,created_at,updated_at
)
SELECT o.assignment_id,o.campaign_id,o.post_url,o.poster,o.partners_json,o.state,o.attempts,0,
       o.last_error,o.created_at,o.updated_at
FROM publish_sheet_outbox o
WHERE length(trim(o.assignment_id)) > 0
  AND length(trim(o.campaign_id)) > 0
  AND length(trim(o.post_url)) > 0
  AND length(trim(o.poster)) > 0
  AND o.rowid = (
    SELECT o2.rowid FROM publish_sheet_outbox o2
    WHERE o2.post_url = o.post_url
      AND length(trim(o2.assignment_id)) > 0
      AND length(trim(o2.campaign_id)) > 0
      AND length(trim(o2.post_url)) > 0
      AND length(trim(o2.poster)) > 0
    ORDER BY (o2.state='sent') DESC, o2.updated_at DESC, o2.rowid DESC
    LIMIT 1
  );
DROP TABLE publish_sheet_outbox;
ALTER TABLE publish_sheet_outbox_new RENAME TO publish_sheet_outbox;
CREATE UNIQUE INDEX publish_sheet_outbox_one_row_per_post
  ON publish_sheet_outbox(post_url);
CREATE INDEX publish_sheet_outbox_pending
  ON publish_sheet_outbox(created_at) WHERE state <> 'sent';
"#,
    )?;
    Ok(())
}

fn apply_migration_19(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // Existing rows are IPA files. Defaults preserve their old meaning instead of
    // classifying historical entries from a mutable filename extension.
    transaction.execute_batch(
        "ALTER TABLE apps_library ADD COLUMN platform TEXT NOT NULL DEFAULT 'ios' CHECK (platform IN ('ios','android'));
         ALTER TABLE apps_library ADD COLUMN package_format TEXT NOT NULL DEFAULT 'ipa' CHECK (package_format IN ('ipa','apk','xapk','apkm','apks'));
         ALTER TABLE apps_library ADD COLUMN artifact_kind TEXT NOT NULL DEFAULT 'ipa' CHECK (artifact_kind IN ('ipa','apk','xapk','apkm','apks'));
         ALTER TABLE apps_library ADD COLUMN application_id TEXT NOT NULL DEFAULT '';
         ALTER TABLE apps_library ADD COLUMN version_name TEXT NOT NULL DEFAULT '';
         ALTER TABLE apps_library ADD COLUMN version_code TEXT;
         ALTER TABLE apps_library ADD COLUMN sha256 TEXT NOT NULL DEFAULT '';
         ALTER TABLE apps_library ADD COLUMN size_bytes INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE apps_library ADD COLUMN signer_sha256 TEXT NOT NULL DEFAULT '';
         ALTER TABLE apps_library ADD COLUMN icon_png_base64 TEXT;
         ALTER TABLE apps_library ADD COLUMN metadata_status TEXT NOT NULL DEFAULT 'legacy';
         ALTER TABLE apps_library ADD COLUMN metadata_error TEXT;
         UPDATE apps_library SET application_id=bundle_id WHERE application_id='';
         UPDATE apps_library SET version_name=version WHERE version_name='';
         CREATE UNIQUE INDEX apps_library_verified_sha256_unique
           ON apps_library(sha256)
           WHERE sha256 <> '' AND metadata_status = 'verified';",
    )?;
    Ok(())
}

fn apply_migration_20(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE automation_definitions (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(trim(name)) > 0),
  kind TEXT NOT NULL CHECK (kind IN ('nurture','interaction','publish')),
  latest_revision INTEGER NOT NULL CHECK (latest_revision >= 1),
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE automation_definition_revisions (
  definition_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  target_json TEXT NOT NULL CHECK (json_valid(target_json)),
  config_json TEXT NOT NULL CHECK (json_valid(config_json)),
  created_at TEXT NOT NULL,
  PRIMARY KEY (definition_id, revision),
  FOREIGN KEY (definition_id) REFERENCES automation_definitions(id) ON DELETE RESTRICT
);

CREATE TABLE automation_schedules (
  id TEXT PRIMARY KEY,
  revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
  name TEXT NOT NULL CHECK (length(trim(name)) > 0),
  definition_id TEXT NOT NULL,
  definition_revision INTEGER NOT NULL CHECK (definition_revision >= 1),
  enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0,1)),
  schedule_json TEXT NOT NULL CHECK (json_valid(schedule_json)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (definition_id, definition_revision)
    REFERENCES automation_definition_revisions(definition_id, revision) ON DELETE RESTRICT
);

CREATE TRIGGER automation_definition_revisions_no_update
BEFORE UPDATE ON automation_definition_revisions
BEGIN
  SELECT RAISE(ABORT, 'automation definition revisions are immutable');
END;

CREATE TRIGGER automation_definition_revisions_no_delete
BEFORE DELETE ON automation_definition_revisions
BEGIN
  SELECT RAISE(ABORT, 'automation definition revisions are immutable');
END;

CREATE INDEX automation_definitions_active_updated
  ON automation_definitions(archived, updated_at DESC);
CREATE INDEX automation_schedules_enabled_updated
  ON automation_schedules(enabled, updated_at DESC);
"#,
    )?;
    Ok(())
}

fn apply_migration_21(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE tiktok_action_runs (
  id TEXT PRIMARY KEY,
  owner_kind TEXT NOT NULL CHECK (owner_kind IN ('interaction','nurture')),
  owner_id TEXT NOT NULL CHECK (length(trim(owner_id)) > 0),
  device_udid TEXT NOT NULL CHECK (length(trim(device_udid)) > 0),
  card_identity_json TEXT CHECK (
    card_identity_json IS NULL OR json_valid(card_identity_json)
  ),
  campaign_id TEXT,
  assignment_id TEXT,
  action_kind TEXT NOT NULL CHECK (action_kind IN ('like','save','comment','follow')),
  state TEXT NOT NULL DEFAULT 'planned'
    CHECK (state IN (
      'planned','preparing','armed','confirmed','no_op','failed_before_effect','uncertain'
    )),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  effect_intent TEXT CHECK (effect_intent IS NULL OR length(trim(effect_intent)) > 0),
  evidence_json TEXT CHECK (evidence_json IS NULL OR json_valid(evidence_json)),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (owner_kind, owner_id, device_udid, action_kind),
  CHECK (
    (owner_kind='interaction' AND campaign_id IS NOT NULL AND assignment_id IS NOT NULL
      AND owner_id=assignment_id)
    OR
    (owner_kind='nurture' AND campaign_id IS NULL AND assignment_id IS NULL)
  ),
  FOREIGN KEY (campaign_id) REFERENCES interaction_campaigns(id) ON DELETE CASCADE,
  FOREIGN KEY (assignment_id) REFERENCES interaction_assignments(id) ON DELETE CASCADE
);

CREATE INDEX tiktok_action_runs_campaign_assignment
  ON tiktok_action_runs(campaign_id, assignment_id, action_kind);
CREATE INDEX tiktok_action_runs_owner
  ON tiktok_action_runs(owner_kind, owner_id, device_udid, action_kind);
CREATE INDEX tiktok_action_runs_recovery
  ON tiktok_action_runs(state, updated_at)
  WHERE state IN ('preparing','armed');
"#,
    )?;
    Ok(())
}

fn apply_migration_22(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE orchestration_documents (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL CHECK (length(trim(name)) > 0),
  latest_revision INTEGER NOT NULL CHECK (latest_revision >= 1),
  archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0,1)),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE orchestration_revisions (
  document_id TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  compiled_json TEXT NOT NULL CHECK (json_valid(compiled_json)),
  canonical_json TEXT NOT NULL CHECK (json_valid(canonical_json)),
  document_sha256 TEXT NOT NULL CHECK (length(document_sha256) = 64),
  created_at TEXT NOT NULL,
  PRIMARY KEY (document_id,revision),
  FOREIGN KEY (document_id) REFERENCES orchestration_documents(id) ON DELETE RESTRICT
);

CREATE TRIGGER orchestration_revisions_no_update
BEFORE UPDATE ON orchestration_revisions
BEGIN
  SELECT RAISE(ABORT, 'orchestration revisions are immutable');
END;

CREATE TRIGGER orchestration_revisions_no_delete
BEFORE DELETE ON orchestration_revisions
BEGIN
  SELECT RAISE(ABORT, 'orchestration revisions are immutable');
END;

CREATE TABLE orchestration_runs (
  id TEXT PRIMARY KEY,
  document_id TEXT NOT NULL,
  document_revision INTEGER NOT NULL CHECK (document_revision >= 1),
  document_sha256 TEXT NOT NULL CHECK (length(document_sha256) = 64),
  target_json TEXT NOT NULL CHECK (json_valid(target_json)),
  state TEXT NOT NULL CHECK (state IN (
    'queued','running','done','partial','failed','uncertain','cancelled'
  )),
  current_node_id TEXT,
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (document_id,document_revision)
    REFERENCES orchestration_revisions(document_id,revision) ON DELETE RESTRICT
);

CREATE TABLE orchestration_attempts (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1),
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) = 64),
  snapshot_json TEXT NOT NULL CHECK (json_valid(snapshot_json)),
  state TEXT NOT NULL CHECK (state IN (
    'queued','dispatching','waiting_child','done','partial','failed','uncertain','cancelled'
  )),
  child_kind TEXT CHECK (child_kind IS NULL OR child_kind IN ('nurture','interaction','publish')),
  child_campaign_id TEXT,
  branch TEXT CHECK (branch IS NULL OR branch IN ('done','partial','failed','uncertain')),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (run_id,node_id,attempt_no),
  CHECK ((child_kind IS NULL) = (child_campaign_id IS NULL)),
  CHECK (
    (state IN ('queued','cancelled') AND child_campaign_id IS NULL) OR
    (state IN ('dispatching','waiting_child') AND child_campaign_id IS NOT NULL AND branch IS NULL) OR
    (state IN ('done','partial','failed','uncertain') AND branch IS NOT NULL)
  ),
  FOREIGN KEY (run_id) REFERENCES orchestration_runs(id) ON DELETE CASCADE
);

CREATE INDEX orchestration_documents_active_updated
  ON orchestration_documents(archived,updated_at DESC);
CREATE INDEX orchestration_runs_state_updated
  ON orchestration_runs(state,updated_at DESC);
CREATE INDEX orchestration_attempts_recovery
  ON orchestration_attempts(state,updated_at)
  WHERE state IN ('dispatching','waiting_child');
"#,
    )?;
    Ok(())
}

fn apply_migration_23(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE orchestration_attempts_v23 (
  id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  attempt_no INTEGER NOT NULL CHECK (attempt_no >= 1),
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) = 64),
  snapshot_json TEXT NOT NULL CHECK (json_valid(snapshot_json)),
  state TEXT NOT NULL CHECK (state IN (
    'queued','dispatching','waiting_child','done','partial','failed','uncertain','cancelled'
  )),
  child_kind TEXT CHECK (child_kind IS NULL OR child_kind IN ('nurture','interaction','publish')),
  child_campaign_id TEXT,
  branch TEXT CHECK (branch IS NULL OR branch IN ('done','partial','failed','uncertain')),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (run_id,node_id,attempt_no),
  CHECK ((child_kind IS NULL) = (child_campaign_id IS NULL)),
  CHECK (
    (state='queued' AND child_campaign_id IS NULL AND branch IS NULL) OR
    (state='cancelled' AND branch IS NULL) OR
    (state IN ('dispatching','waiting_child') AND child_campaign_id IS NOT NULL AND branch IS NULL) OR
    (state IN ('done','partial','failed','uncertain') AND branch IS NOT NULL)
  ),
  FOREIGN KEY (run_id) REFERENCES orchestration_runs(id) ON DELETE CASCADE
);

INSERT INTO orchestration_attempts_v23(
  id,run_id,node_id,attempt_no,idempotency_key,snapshot_json,state,
  child_kind,child_campaign_id,branch,error_code,created_at,updated_at
)
SELECT
  id,run_id,node_id,attempt_no,idempotency_key,snapshot_json,state,
  child_kind,child_campaign_id,branch,error_code,created_at,updated_at
FROM orchestration_attempts;

DROP TABLE orchestration_attempts;
ALTER TABLE orchestration_attempts_v23 RENAME TO orchestration_attempts;
CREATE INDEX orchestration_attempts_recovery
  ON orchestration_attempts(state,updated_at)
  WHERE state IN ('dispatching','waiting_child');
"#,
    )?;
    Ok(())
}

fn apply_migration_24(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
ALTER TABLE orchestration_runs
  ADD COLUMN node_targets_json TEXT NOT NULL DEFAULT '{}'
  CHECK (json_valid(node_targets_json) AND json_type(node_targets_json)='object');

CREATE TABLE orchestration_nurture_children (
  id TEXT PRIMARY KEY,
  attempt_id TEXT NOT NULL UNIQUE,
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key) = 64),
  run_id TEXT,
  requested_udids_json TEXT NOT NULL
    CHECK (json_valid(requested_udids_json) AND json_type(requested_udids_json)='array'),
  started_udids_json TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(started_udids_json) AND json_type(started_udids_json)='array'),
  state TEXT NOT NULL CHECK (state IN (
    'dispatching','running','done','partial','failed','uncertain'
  )),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  CHECK (
    (state='dispatching' AND run_id IS NULL) OR
    (state='running' AND run_id IS NOT NULL) OR
    state IN ('done','partial','failed','uncertain')
  ),
  FOREIGN KEY (attempt_id) REFERENCES orchestration_attempts(id) ON DELETE CASCADE
);

CREATE INDEX orchestration_nurture_children_recovery
  ON orchestration_nurture_children(state,updated_at)
  WHERE state IN ('dispatching','running');
"#,
    )?;
    Ok(())
}

fn apply_migration_25(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
ALTER TABLE automation_schedules ADD COLUMN next_due_at TEXT;
ALTER TABLE automation_schedules ADD COLUMN last_error_code TEXT;

UPDATE automation_schedules
SET enabled=0,next_due_at=NULL,last_error_code='unsupported_schedule_schema'
WHERE NOT (
  json_type(schedule_json)='object' AND
  json_extract(schedule_json,'$.schemaVersion')=1 AND
  json_extract(schedule_json,'$.kind')='interval' AND
  json_type(schedule_json,'$.everyMinutes')='integer' AND
  json_extract(schedule_json,'$.everyMinutes') BETWEEN 15 AND 1440 AND
  (SELECT COUNT(*) FROM json_each(schedule_json))=3
);

UPDATE automation_schedules
SET next_due_at=strftime(
      '%Y-%m-%dT%H:%M:%fZ',
      'now',
      '+' || json_extract(schedule_json,'$.everyMinutes') || ' minutes'
    ),
    last_error_code=NULL
WHERE enabled=1 AND last_error_code IS NULL;

DROP INDEX automation_schedules_enabled_updated;
CREATE INDEX automation_schedules_due
  ON automation_schedules(enabled,next_due_at,id);

CREATE TABLE automation_schedule_occurrences (
  id TEXT PRIMARY KEY,
  schedule_id TEXT NOT NULL,
  schedule_revision INTEGER NOT NULL CHECK (schedule_revision >= 1),
  scheduled_for TEXT NOT NULL,
  definition_id TEXT NOT NULL,
  definition_revision INTEGER NOT NULL CHECK (definition_revision >= 1),
  kind TEXT NOT NULL CHECK (kind IN ('nurture','interaction','publish')),
  target_json TEXT CHECK (target_json IS NULL OR json_valid(target_json)),
  child_campaign_id TEXT NOT NULL UNIQUE,
  idempotency_key TEXT NOT NULL UNIQUE CHECK (length(idempotency_key)=64),
  state TEXT NOT NULL CHECK (state IN (
    'queued','dispatching','running','done','partial','failed','uncertain'
  )),
  nurture_run_id TEXT,
  nurture_started_udids_json TEXT NOT NULL DEFAULT '[]'
    CHECK (
      json_valid(nurture_started_udids_json) AND
      json_type(nurture_started_udids_json)='array'
    ),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (schedule_id,scheduled_for),
  CHECK (state='failed' OR target_json IS NOT NULL),
  CHECK (kind='nurture' OR (nurture_run_id IS NULL AND nurture_started_udids_json='[]')),
  FOREIGN KEY (schedule_id) REFERENCES automation_schedules(id) ON DELETE RESTRICT,
  FOREIGN KEY (definition_id,definition_revision)
    REFERENCES automation_definition_revisions(definition_id,revision) ON DELETE RESTRICT
);

CREATE UNIQUE INDEX automation_schedule_occurrences_one_active
  ON automation_schedule_occurrences(schedule_id)
  WHERE state IN ('queued','dispatching','running');
CREATE INDEX automation_schedule_occurrences_recovery
  ON automation_schedule_occurrences(state,updated_at)
  WHERE state IN ('queued','dispatching','running');
"#,
    )?;
    Ok(())
}

fn apply_migration_26(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    // Comment campaigns still validate at 2..=64. Like/Save-only campaigns deliberately
    // carry zero comments, though, and the immutable request stores that as `message_count=0`.
    // Migration 14's table CHECK predated action-only Interaction and rejected those otherwise
    // valid requests at persistence time. SQLite cannot alter a CHECK in place, so preserve the
    // exact row and rebuild only its parent table under the guarded foreign-key window.
    transaction.execute_batch(
        r#"
CREATE TABLE interaction_campaigns_new (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE,
  request_json TEXT NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('queued','running','succeeded','partial','failed','cancelled')),
  message_count INTEGER NOT NULL CHECK (message_count BETWEEN 0 AND 64),
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
"#,
    )?;

    for table in [
        "interaction_campaign_actors",
        "interaction_targets",
        "interaction_assignments",
        "interaction_artifacts",
        "tiktok_action_runs",
    ] {
        let orphans = transaction
            .prepare(&format!("PRAGMA foreign_key_check({table})"))?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .len();
        if orphans > 0 {
            anyhow::bail!(
                "InteractionActionOnlyRebuildLostReferences: {orphans} row(s) in {table} no longer resolve to a parent"
            );
        }
    }
    Ok(())
}

fn apply_migration_27(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE publish_execution_snapshots (
  campaign_id TEXT PRIMARY KEY REFERENCES publish_campaigns(id) ON DELETE CASCADE,
  input_digest TEXT NOT NULL CHECK (
    length(input_digest) = 64 AND
    input_digest NOT GLOB '*[^0-9a-f]*'
  ),
  status TEXT NOT NULL CHECK (status IN ('complete','partial','uncertain')),
  retry_scope TEXT NOT NULL CHECK (
    retry_scope IN ('fullPipeline','linkAndSheet','sheetOnly','none')
  ),
  report_json TEXT NOT NULL CHECK (
    json_valid(report_json) AND json_type(report_json) = 'object'
  ),
  updated_at TEXT NOT NULL
);
"#,
    )?;
    Ok(())
}

fn apply_migration_28(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE nurture_runs (
  id TEXT PRIMARY KEY,
  target_udids_json TEXT NOT NULL CHECK (
    json_valid(target_udids_json) AND
    json_type(target_udids_json) = 'array'
  ),
  target_count INTEGER NOT NULL CHECK (
    target_count > 0 AND
    target_count = json_array_length(target_udids_json)
  ),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX nurture_runs_updated ON nurture_runs(updated_at DESC);

CREATE TABLE nurture_run_status_events (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id TEXT NOT NULL REFERENCES nurture_runs(id) ON DELETE CASCADE,
  udid TEXT NOT NULL CHECK (length(udid) > 0),
  status_json TEXT NOT NULL CHECK (
    json_valid(status_json) AND
    json_type(status_json) = 'object' AND
    json_extract(status_json, '$.udid') = udid AND
    json_extract(status_json, '$.runId') = run_id AND
    json_type(status_json, '$.running') IN ('true', 'false')
  ),
  recorded_at TEXT NOT NULL
);
CREATE INDEX nurture_run_status_events_latest
  ON nurture_run_status_events(run_id, udid, sequence DESC);
"#,
    )?;
    Ok(())
}

fn apply_migration_29(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE public_cleanup_runs (
  id TEXT PRIMARY KEY,
  request_id TEXT NOT NULL UNIQUE CHECK (length(trim(request_id)) > 0),
  source_action_run_id TEXT NOT NULL UNIQUE
    REFERENCES tiktok_action_runs(id) ON DELETE RESTRICT,
  campaign_id TEXT NOT NULL REFERENCES interaction_campaigns(id) ON DELETE RESTRICT,
  assignment_id TEXT NOT NULL REFERENCES interaction_assignments(id) ON DELETE RESTRICT,
  device_udid TEXT NOT NULL CHECK (length(trim(device_udid)) > 0),
  action_kind TEXT NOT NULL CHECK (action_kind IN ('like','save')),
  target_json TEXT NOT NULL CHECK (
    json_valid(target_json) AND
    json_type(target_json) = 'object' AND
    length(trim(json_extract(target_json, '$.normalizedUrl'))) > 0 AND
    length(trim(json_extract(target_json, '$.contentId'))) > 0 AND
    length(trim(json_extract(target_json, '$.author'))) > 0
  ),
  state TEXT NOT NULL DEFAULT 'planned' CHECK (state IN (
    'planned','preparing','armed','cleared','already_clear','failed_before_effect','uncertain'
  )),
  revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
  effect_intent TEXT CHECK (effect_intent IS NULL OR length(trim(effect_intent)) > 0),
  evidence_json TEXT CHECK (evidence_json IS NULL OR json_valid(evidence_json)),
  error_code TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE INDEX public_cleanup_runs_recovery
  ON public_cleanup_runs(state, updated_at)
  WHERE state IN ('preparing','armed');
CREATE INDEX public_cleanup_runs_assignment
  ON public_cleanup_runs(campaign_id, assignment_id, action_kind);
"#,
    )?;
    Ok(())
}

fn apply_migration_30(transaction: &Transaction<'_>) -> anyhow::Result<()> {
    transaction.execute_batch(
        r#"
CREATE TABLE nurture_follow_armed_witnesses (
  action_run_id TEXT PRIMARY KEY
    REFERENCES tiktok_action_runs(id) ON DELETE RESTRICT,
  armed_revision INTEGER NOT NULL CHECK (armed_revision >= 2),
  identity_json TEXT NOT NULL CHECK (json_valid(identity_json)),
  effect_intent TEXT NOT NULL CHECK (length(trim(effect_intent)) > 0),
  armed_at TEXT NOT NULL CHECK (length(trim(armed_at)) > 0)
);

CREATE TRIGGER nurture_follow_arm_transition_valid
BEFORE UPDATE ON tiktok_action_runs
WHEN OLD.owner_kind='nurture' AND OLD.action_kind='follow'
  AND OLD.state='preparing' AND NEW.state='armed'
BEGIN
  SELECT CASE WHEN NOT (
    NEW.owner_kind=OLD.owner_kind AND NEW.owner_id=OLD.owner_id
    AND NEW.device_udid=OLD.device_udid AND NEW.action_kind=OLD.action_kind
    AND NEW.revision=OLD.revision+1 AND NEW.revision >= 2
    AND OLD.effect_intent IS NULL AND NEW.effect_intent IS NOT NULL
    AND length(trim(NEW.effect_intent)) > 0
    AND OLD.card_identity_json IS NOT NULL AND NEW.card_identity_json IS NOT NULL
    AND json(OLD.card_identity_json)=json(NEW.card_identity_json)
  ) THEN RAISE(ABORT, 'invalid Nurture Follow arm transition') END;
END;

CREATE TRIGGER nurture_follow_arm_transition_capture
AFTER UPDATE ON tiktok_action_runs
WHEN OLD.owner_kind='nurture' AND OLD.action_kind='follow'
  AND OLD.state='preparing' AND NEW.state='armed'
BEGIN
  INSERT INTO nurture_follow_armed_witnesses
    (action_run_id,armed_revision,identity_json,effect_intent,armed_at)
  VALUES(NEW.id,NEW.revision,NEW.card_identity_json,NEW.effect_intent,NEW.updated_at);
END;

CREATE TRIGGER nurture_follow_armed_witnesses_no_update
BEFORE UPDATE ON nurture_follow_armed_witnesses
BEGIN
  SELECT RAISE(ABORT, 'nurture Follow arm witnesses are immutable');
END;

CREATE TRIGGER nurture_follow_armed_witnesses_no_delete
BEFORE DELETE ON nurture_follow_armed_witnesses
BEGIN
  SELECT RAISE(ABORT, 'nurture Follow arm witnesses are immutable');
END;

CREATE TRIGGER nurture_follow_witness_parent_identity_no_update
BEFORE UPDATE OF owner_kind,owner_id,device_udid,card_identity_json,action_kind,effect_intent
ON tiktok_action_runs
WHEN EXISTS (
  SELECT 1 FROM nurture_follow_armed_witnesses AS witness
  WHERE witness.action_run_id=OLD.id
)
AND (
  NEW.owner_kind IS NOT OLD.owner_kind OR NEW.owner_id IS NOT OLD.owner_id
  OR NEW.device_udid IS NOT OLD.device_udid
  OR NEW.card_identity_json IS NOT OLD.card_identity_json
  OR NEW.action_kind IS NOT OLD.action_kind
  OR NEW.effect_intent IS NOT OLD.effect_intent
)
BEGIN
  SELECT RAISE(ABORT, 'armed Nurture Follow identity is immutable');
END;

CREATE TABLE nurture_follow_source_identities (
  action_run_id TEXT PRIMARY KEY
    REFERENCES tiktok_action_runs(id) ON DELETE RESTRICT,
  identity_json TEXT NOT NULL CHECK (json_valid(identity_json)),
  canonical_handle TEXT NOT NULL CHECK (
    length(canonical_handle) BETWEEN 3 AND 33 AND
    substr(canonical_handle, 1, 1) = '@' AND
    lower(canonical_handle) = canonical_handle
  ),
  card_key TEXT NOT NULL CHECK (
    length(card_key) = 64 AND
    lower(card_key) = card_key AND
    card_key NOT GLOB '*[^0-9a-f]*'
  ),
  author_profile_key TEXT NOT NULL CHECK (
    length(author_profile_key) = 64 AND
    lower(author_profile_key) = author_profile_key AND
    author_profile_key NOT GLOB '*[^0-9a-f]*'
  ),
  readback_generation INTEGER NOT NULL CHECK (readback_generation > 0),
  readback_snapshot_sha256 TEXT NOT NULL CHECK (
    length(readback_snapshot_sha256) = 64 AND
    lower(readback_snapshot_sha256) = readback_snapshot_sha256 AND
    readback_snapshot_sha256 NOT GLOB '*[^0-9a-f]*'
  ),
  readback_verdict TEXT NOT NULL CHECK (readback_verdict = 'follow_absent'),
  confirmed_at TEXT NOT NULL CHECK (length(trim(confirmed_at)) > 0)
);

CREATE TRIGGER nurture_follow_source_identities_valid_source
BEFORE INSERT ON nurture_follow_source_identities
BEGIN
  SELECT CASE WHEN NOT EXISTS (
    SELECT 1
    FROM tiktok_action_runs AS action
    JOIN nurture_follow_armed_witnesses AS witness
      ON witness.action_run_id=action.id
    WHERE action.id=NEW.action_run_id
      AND action.owner_kind='nurture'
      AND action.action_kind='follow'
      AND action.state='confirmed'
      AND action.revision=witness.armed_revision+1
      AND action.effect_intent=witness.effect_intent
      AND action.card_identity_json IS NOT NULL
      AND json(action.card_identity_json)=json(NEW.identity_json)
      AND json(witness.identity_json)=json(NEW.identity_json)
      AND json_type(NEW.identity_json)='object'
      AND (SELECT COUNT(*) FROM json_each(NEW.identity_json))=4
      AND json_type(NEW.identity_json,'$.canonicalHandle')='text'
      AND json_extract(NEW.identity_json,'$.canonicalHandle')=NEW.canonical_handle
      AND json_type(NEW.identity_json,'$.cardKey')='text'
      AND json_extract(NEW.identity_json,'$.cardKey')=NEW.card_key
      AND json_type(NEW.identity_json,'$.authorProfileKey')='text'
      AND json_extract(NEW.identity_json,'$.authorProfileKey')=NEW.author_profile_key
      AND json_type(NEW.identity_json,'$.authorProfileProof')='object'
      AND (SELECT COUNT(*) FROM json_each(
            json_extract(NEW.identity_json,'$.authorProfileProof')))=39
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.hierarchyGeneration')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.hierarchyGeneration') > 0
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.snapshotSha256')='text'
      AND length(json_extract(NEW.identity_json,
            '$.authorProfileProof.snapshotSha256'))=64
      AND lower(json_extract(NEW.identity_json,
            '$.authorProfileProof.snapshotSha256'))=
          json_extract(NEW.identity_json,'$.authorProfileProof.snapshotSha256')
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.snapshotSha256') NOT GLOB '*[^0-9a-f]*'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.profileNodeIndex')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.profileNodeIndex') >= 0
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.profileResourceId')='text'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.profileResourceId')='com.ss.android.ugc.trill:id/t40'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.profileClassName')='android.widget.ImageView'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.profileContentDescription')=
          NEW.canonical_handle || ' profile'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.profileEnabled')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.profileClickable')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.followNodeIndex')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.followNodeIndex') >= 0
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.followNodeIndex') !=
          json_extract(NEW.identity_json,'$.authorProfileProof.profileNodeIndex')
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.followResourceId')='com.ss.android.ugc.trill:id/fm1'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.followClassName')='android.widget.Button'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.followContentDescription')=
          'Follow ' || NEW.canonical_handle
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.followEnabled')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.followClickable')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.cardNodeIndex')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.cardResourceId')='com.ss.android.ugc.trill:id/cv2'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.cardClassName')='android.widget.FrameLayout'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.cardEnabled')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.cardClickable')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.railNodeIndex')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.railResourceId')='com.ss.android.ugc.trill:id/hfp'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.railClassName')='android.widget.LinearLayout'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.railEnabled')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.railClickable')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.pagerNodeIndex')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.pagerResourceId')='com.ss.android.ugc.trill:id/tod'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.pagerClassName')='androidx.viewpager.widget.ViewPager'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.pagerEnabled')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.pagerClickable')='false'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.feedTabNodeIndex')='integer'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.feedTabClassName')='android.widget.LinearLayout'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.feedTabContentDescription')='For You'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.feedTabEnabled')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.feedTabClickable')='false'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.feedTabSelected')='true'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.parentChain')='array'
      AND json_array_length(json_extract(NEW.identity_json,
            '$.authorProfileProof.parentChain')) >= 3
      AND NOT EXISTS (
        SELECT 1 FROM json_each(json_extract(
          NEW.identity_json,'$.authorProfileProof.parentChain')) AS parent
        WHERE parent.type != 'integer' OR parent.value < 0
      )
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.parentChain[0]')=
          json_extract(NEW.identity_json,'$.authorProfileProof.cardNodeIndex')
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.parentChain[1]')=
          json_extract(NEW.identity_json,'$.authorProfileProof.railNodeIndex')
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.parentChain[2]')=
          json_extract(NEW.identity_json,'$.authorProfileProof.pagerNodeIndex')
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.canonicalHandle')='text'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.canonicalHandle')=NEW.canonical_handle
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.cardContinuityKey')='text'
      AND length(json_extract(NEW.identity_json,
            '$.authorProfileProof.cardContinuityKey'))=64
      AND lower(json_extract(NEW.identity_json,
            '$.authorProfileProof.cardContinuityKey'))=
          json_extract(NEW.identity_json,'$.authorProfileProof.cardContinuityKey')
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.cardContinuityKey') NOT GLOB '*[^0-9a-f]*'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.cardContinuityKey')=NEW.card_key
      AND NEW.readback_generation>
          json_extract(NEW.identity_json,
                       '$.authorProfileProof.hierarchyGeneration')
      AND NEW.readback_verdict='follow_absent'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.tuple')='object'
      AND (SELECT COUNT(*) FROM json_each(
            json_extract(NEW.identity_json,'$.authorProfileProof.tuple')))=3
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.tuple.package')='text'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.tuple.package')='com.ss.android.ugc.trill'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.tuple.versionName')='text'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.tuple.versionName')='38.3.2'
      AND json_type(NEW.identity_json,
            '$.authorProfileProof.tuple.locale')='text'
      AND json_extract(NEW.identity_json,
            '$.authorProfileProof.tuple.locale')='en'
      AND NEW.confirmed_at=action.updated_at
  ) THEN RAISE(ABORT, 'invalid Nurture Follow source identity') END;
END;

CREATE TRIGGER nurture_follow_source_identities_no_update
BEFORE UPDATE ON nurture_follow_source_identities
BEGIN
  SELECT RAISE(ABORT, 'nurture Follow source identities are immutable');
END;

CREATE TRIGGER nurture_follow_source_identities_no_delete
BEFORE DELETE ON nurture_follow_source_identities
BEGIN
  SELECT RAISE(ABORT, 'nurture Follow source identities are immutable');
END;

CREATE TRIGGER nurture_follow_source_parent_no_update
BEFORE UPDATE ON tiktok_action_runs
WHEN EXISTS (
  SELECT 1 FROM nurture_follow_source_identities AS source
  WHERE source.action_run_id=OLD.id
)
BEGIN
  SELECT RAISE(ABORT, 'confirmed Nurture Follow source parent is immutable');
END;
"#,
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
    use super::{apply_v1_schema, run, run_with_failpoint, LEDGER_SQL, MIGRATIONS};

    #[test]
    fn app_library_metadata_migration_is_version_nineteen() {
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == 19)
            .expect("app-library migration");
        assert_eq!(migration.version, 19);
        assert_eq!(migration.name, "app-library-artifact-metadata");
    }

    #[test]
    fn app_library_migration_limits_sha_uniqueness_to_new_verified_rows() {
        let path = temp_db_path("app-library-sha-index");
        let database = Database::open(&path).expect("migrated database");
        let connection = database.conn().expect("inspect app-library schema");
        let index_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type='index' AND name='apps_library_verified_sha256_unique'",
                [],
                |row| row.get(0),
            )
            .expect("verified SHA index");
        assert!(index_sql.contains("metadata_status = 'verified'"));

        for id in ["legacy-a", "legacy-b"] {
            connection
                .execute(
                    "INSERT INTO apps_library(
                        id,name,path,bundle_id,version,sha256,metadata_status,created_at
                     ) VALUES(?1,?1,?1,'com.example.legacy','1','same-sha','legacy','now')",
                    [id],
                )
                .expect("historical duplicates remain valid");
        }
        connection
            .execute(
                "INSERT INTO apps_library(
                    id,name,path,bundle_id,version,sha256,metadata_status,created_at
                 ) VALUES('verified-a','A','A','com.example.app','1','verified-sha','verified','now')",
                [],
            )
            .expect("first verified row");
        let duplicate = connection
            .execute(
                "INSERT INTO apps_library(
                    id,name,path,bundle_id,version,sha256,metadata_status,created_at
                 ) VALUES('verified-b','B','B','com.example.app','1','verified-sha','verified','now')",
                [],
            )
            .expect_err("new verified duplicate must fail closed");
        assert_eq!(
            duplicate.sqlite_error_code(),
            Some(ErrorCode::ConstraintViolation)
        );
        drop(connection);
        drop(database);
        cleanup(&path);
    }

    type Blob = Vec<u8>;
    type ScriptRowBytes = (Blob, Blob, Blob, Blob);
    type JobRowBytes = (Blob, Blob, Blob, Blob, Blob, Blob, Blob, Option<Blob>);
    type SettingRowBytes = (Blob, Blob);
    /// Four columns since migration 16 dropped `proxy_id`, which was always `None` because
    /// nothing could set it once the proxy UI was removed. The property is unchanged -- an
    /// upgrade must not rewrite a surviving row -- and the surviving columns still prove it.
    type DeviceRowBytes = (Blob, Blob, Blob, Option<Blob>);
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
                            CAST(group_id AS BLOB)
                     FROM device_meta WHERE udid='MOCK-IPHONE-01'",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
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
    fn migration_from_v19_adds_empty_versioned_automation_tables() {
        let path = temp_db_path("automation-v19-upgrade");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(20)).expect_err("stop at v19");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 19);
        assert!(!table_exists(&connection, "automation_definitions"));

        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('v19-fixture','preserved')",
                [],
            )
            .expect("seed v19 row");
        run_with_failpoint(&mut connection, None).expect("upgrade to latest");

        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );
        assert!(table_exists(&connection, "automation_definitions"));
        assert!(table_exists(&connection, "automation_definition_revisions"));
        assert!(table_exists(&connection, "automation_schedules"));
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM settings WHERE key='v19-fixture'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("preserved fixture row"),
            "preserved"
        );
        let counts: (i64, i64, i64) = (
            connection
                .query_row("SELECT COUNT(*) FROM automation_definitions", [], |row| {
                    row.get(0)
                })
                .expect("definition count"),
            connection
                .query_row(
                    "SELECT COUNT(*) FROM automation_definition_revisions",
                    [],
                    |row| row.get(0),
                )
                .expect("revision count"),
            connection
                .query_row("SELECT COUNT(*) FROM automation_schedules", [], |row| {
                    row.get(0)
                })
                .expect("schedule count"),
        );
        assert_eq!(
            counts,
            (0, 0, 0),
            "migration must not backfill a profile or schedule"
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_from_v20_adds_an_empty_interaction_action_ledger() {
        let path = temp_db_path("interaction-actions-v20-upgrade");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(21)).expect_err("stop at v20");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 20);
        assert!(!table_exists(&connection, "tiktok_action_runs"));
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('v20-fixture','preserved')",
                [],
            )
            .expect("seed v20 row");

        run_with_failpoint(&mut connection, None).expect("upgrade to latest");

        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );
        assert!(table_exists(&connection, "tiktok_action_runs"));
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM settings WHERE key='v20-fixture'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("preserved fixture row"),
            "preserved"
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM tiktok_action_runs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("action count"),
            0
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_from_v21_adds_empty_orchestration_tables_without_backfill() {
        let path = temp_db_path("orchestration-v21-upgrade");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(22)).expect_err("stop at v21");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 21);
        assert!(!table_exists(&connection, "orchestration_documents"));
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('v21-fixture','preserved')",
                [],
            )
            .expect("seed v21 row");

        run_with_failpoint(&mut connection, None).expect("upgrade to latest");

        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );
        for table in [
            "orchestration_documents",
            "orchestration_revisions",
            "orchestration_runs",
            "orchestration_attempts",
        ] {
            assert!(table_exists(&connection, table), "missing {table}");
            assert_eq!(
                connection
                    .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .expect("orchestration table count"),
                0,
                "migration must not fabricate {table} rows"
            );
        }
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM settings WHERE key='v21-fixture'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("preserved fixture"),
            "preserved"
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_from_v22_preserves_an_armed_child_id_when_cancellation_is_proven() {
        let path = temp_db_path("orchestration-v22-cancel-proof");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(23)).expect_err("stop at v22");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 22);

        let document_id = Uuid::from_u128(10).to_string();
        let run_id = Uuid::from_u128(2).to_string();
        connection
            .execute(
                "INSERT INTO orchestration_documents(
                    id,name,latest_revision,archived,created_at,updated_at
                 ) VALUES(?1,'fixture',1,0,'now','now')",
                [&document_id],
            )
            .expect("seed v22 document");
        connection
            .execute(
                "INSERT INTO orchestration_revisions(
                    document_id,revision,compiled_json,canonical_json,document_sha256,created_at
                 ) VALUES(?1,1,'{}','{}',?2,'now')",
                params![document_id, "b".repeat(64)],
            )
            .expect("seed v22 revision");
        connection
            .execute(
                "INSERT INTO orchestration_runs(
                    id,document_id,document_revision,document_sha256,target_json,state,
                    current_node_id,created_at,updated_at
                 ) VALUES(?1,?2,1,?3,'{}','running',?4,'now','now')",
                params![
                    run_id,
                    Uuid::from_u128(10).to_string(),
                    "b".repeat(64),
                    Uuid::from_u128(3).to_string(),
                ],
            )
            .expect("seed v22 run");

        connection
            .execute(
                "INSERT INTO orchestration_attempts(
                    id,run_id,node_id,attempt_no,idempotency_key,snapshot_json,state,
                    child_kind,child_campaign_id,created_at,updated_at
                 ) VALUES(?1,?2,?3,1,?4,'{}','waiting_child','interaction',?5,'now','now')",
                params![
                    Uuid::from_u128(1).to_string(),
                    Uuid::from_u128(2).to_string(),
                    Uuid::from_u128(3).to_string(),
                    "a".repeat(64),
                    Uuid::from_u128(4).to_string(),
                ],
            )
            .expect("seed an armed v22 attempt");
        assert!(connection
            .execute(
                "UPDATE orchestration_attempts SET state='cancelled' WHERE id=?1",
                [Uuid::from_u128(1).to_string()],
            )
            .is_err());

        run_with_failpoint(&mut connection, None).expect("upgrade to latest");
        connection
            .execute(
                "UPDATE orchestration_attempts SET state='cancelled' WHERE id=?1",
                [Uuid::from_u128(1).to_string()],
            )
            .expect("cancel with durable child proof");
        assert_eq!(
            connection
                .query_row(
                    "SELECT state,child_campaign_id FROM orchestration_attempts WHERE id=?1",
                    [Uuid::from_u128(1).to_string()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .expect("read upgraded attempt"),
            ("cancelled".into(), Uuid::from_u128(4).to_string())
        );
        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );
        let violations = connection
            .prepare("PRAGMA foreign_key_check")
            .expect("prepare foreign-key check")
            .query_map([], |_| Ok(()))
            .expect("run foreign-key check")
            .collect::<Result<Vec<_>, _>>()
            .expect("read foreign-key check");
        assert!(
            violations.is_empty(),
            "migration left foreign-key violations"
        );
        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_from_v23_pins_node_targets_and_owns_nurture_children() {
        let path = temp_db_path("orchestration-v23-runtime-proof");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(24)).expect_err("stop at v23");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 23);

        run_with_failpoint(&mut connection, None).expect("upgrade to latest");

        let run_columns = connection
            .prepare("PRAGMA table_info(orchestration_runs)")
            .expect("prepare run columns")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query run columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect run columns");
        assert!(
            run_columns
                .iter()
                .any(|column| column == "node_targets_json"),
            "run confirmation must persist every node target snapshot"
        );
        assert!(table_exists(&connection, "orchestration_nurture_children"));
        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_from_v24_adds_durable_schedule_occurrences() {
        let path = temp_db_path("automation-schedule-v24-runtime");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(25)).expect_err("stop at v24");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 24);

        run_with_failpoint(&mut connection, None).expect("upgrade to latest");

        assert!(table_exists(&connection, "automation_schedule_occurrences"));
        let schedule_columns = connection
            .prepare("PRAGMA table_info(automation_schedules)")
            .expect("prepare schedule columns")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query schedule columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect schedule columns");
        assert!(schedule_columns
            .iter()
            .any(|column| column == "next_due_at"));
        assert!(schedule_columns
            .iter()
            .any(|column| column == "last_error_code"));
        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_26_allows_action_only_zero_without_rewriting_interaction_rows() {
        let path = temp_db_path("interaction-action-only-message-count");
        let mut connection = Connection::open(&path).expect("fixture");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("production foreign-key posture");
        run_with_failpoint(&mut connection, Some(26)).expect_err("stop at v25");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 25);

        connection
            .execute_batch(
                r#"
INSERT INTO interaction_campaigns
  (id,request_id,request_json,state,message_count,revision,error_code,created_at,updated_at)
  VALUES('camp-v25','req-v25','{"requestId":"req-v25"}','partial',2,7,'kept','t0','t1');
INSERT INTO interaction_campaign_actors(campaign_id,actor_ordinal,udid,state,error_code)
  VALUES('camp-v25',0,'phone-a','active','kept-actor');
INSERT INTO interaction_targets
  (id,campaign_id,line_no,original_url,normalized_url,target_key,content_id,kind,state,
   context_json,error_code,created_at)
  VALUES('target-v25','camp-v25',1,'https://x/1','https://x/1','content:1','1','video',
         'ready','{"kept":true}','kept-target','t0');
INSERT INTO interaction_assignments
  (id,campaign_id,target_id,message_ordinal,actor_udid,prepared_json,state,effect_intent,
   evidence_json,error_code,revision,created_at,updated_at)
  VALUES('assignment-v25','camp-v25','target-v25',0,'phone-a','{"text":"kept"}',
         'sending','post_comment','{"kept":true}','kept-assignment',9,'t0','t1');
INSERT INTO interaction_artifacts
  (id,campaign_id,target_id,assignment_id,kind,metadata_json,relative_path,sha256,created_at)
  VALUES('artifact-v25','camp-v25','target-v25','assignment-v25','comment-root-evidence',
         '{"kept":true}','a/b.jpg','beef','t1');
INSERT INTO tiktok_action_runs
  (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,action_kind,state,
   revision,effect_intent,evidence_json,error_code,created_at,updated_at)
  VALUES('action-v25','interaction','assignment-v25','phone-a','camp-v25','assignment-v25',
         'like','armed',4,'like_desired_state','{"kept":true}','kept-action','t0','t1');
"#,
            )
            .expect("seed populated v25 interaction graph");
        let before = read_interaction_rows(&connection);
        let read_action = |connection: &Connection| {
            connection
                .query_row(
                    "SELECT CAST(id AS BLOB),CAST(owner_id AS BLOB),CAST(campaign_id AS BLOB),
                            CAST(assignment_id AS BLOB),CAST(action_kind AS BLOB),CAST(state AS BLOB),
                            CAST(revision AS BLOB),CAST(effect_intent AS BLOB),CAST(evidence_json AS BLOB)
                     FROM tiktok_action_runs WHERE id='action-v25'",
                    [],
                    |row| {
                        (0..9)
                            .map(|index| row.get::<_, Option<Vec<u8>>>(index))
                            .collect::<Result<Vec<_>, _>>()
                    },
                )
                .expect("read action row")
        };
        let action_before = read_action(&connection);

        run(&mut connection).expect("apply migrations from version 26 onward");

        assert_eq!(read_interaction_rows(&connection), before);
        assert_eq!(read_action(&connection), action_before);
        assert_eq!(
            migration_rows(&connection).last().unwrap().0,
            super::latest_version()
        );
        let index_sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='index' AND name='idx_interaction_campaigns_updated'",
                [],
                |row| row.get(0),
            )
            .expect("campaign updated index survives rebuild");
        assert!(index_sql.contains("updated_at DESC"));

        for (label, message_count, allowed) in [
            ("negative", -1_i64, false),
            ("zero", 0, true),
            ("max", 64, true),
            ("above-max", 65, false),
        ] {
            let inserted = connection.execute(
                "INSERT INTO interaction_campaigns
                 (id,request_id,request_json,state,message_count,created_at,updated_at)
                 VALUES(?1,?2,'{}','queued',?3,'t','t')",
                params![
                    format!("campaign-{label}"),
                    format!("request-{label}"),
                    message_count
                ],
            );
            assert_eq!(inserted.is_ok(), allowed, "message_count {message_count}");
        }

        connection
            .execute("DELETE FROM interaction_campaigns WHERE id='camp-v25'", [])
            .expect("campaign cascade remains enforced");
        for table in [
            "interaction_campaign_actors",
            "interaction_targets",
            "interaction_assignments",
            "interaction_artifacts",
            "tiktok_action_runs",
        ] {
            let count: i64 = connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("count cascade survivors");
            assert_eq!(count, 0, "{table} did not cascade after rebuild");
        }
        let violations = connection
            .prepare("PRAGMA foreign_key_check")
            .expect("prepare foreign-key check")
            .query_map([], |_| Ok(()))
            .expect("run foreign-key check")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect foreign-key violations");
        assert!(violations.is_empty());

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_27_adds_publish_execution_snapshots_without_rewriting_campaigns() {
        let path = temp_db_path("publish-execution-snapshots");
        let mut connection = Connection::open(&path).expect("fixture");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("production foreign-key posture");
        run_with_failpoint(&mut connection, Some(27)).expect_err("stop at v26");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 26);
        assert!(!table_exists(&connection, "publish_execution_snapshots"));

        connection
            .execute(
                "INSERT INTO publish_campaigns
                 (id,request_id,source_root,request_json,state,revision,error_code,created_at,updated_at)
                 VALUES('campaign-v26','request-v26','C:/fixture','{\"kept\":true}',
                        'uncertain',9,'kept','t0','t1')",
                [],
            )
            .expect("seed v26 publish campaign");
        let before: (String, String, String, i64, Option<String>, String, String) = connection
            .query_row(
                "SELECT request_id,source_root,request_json,revision,error_code,created_at,updated_at
                 FROM publish_campaigns WHERE id='campaign-v26'",
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
                    ))
                },
            )
            .expect("read v26 campaign");

        run_with_failpoint(&mut connection, Some(28)).expect_err("stop after migration 27");

        assert_eq!(migration_rows(&connection).last().unwrap().0, 27);
        assert!(table_exists(&connection, "publish_execution_snapshots"));
        let after = connection
            .query_row(
                "SELECT request_id,source_root,request_json,revision,error_code,created_at,updated_at
                 FROM publish_campaigns WHERE id='campaign-v26'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )
            .expect("read migrated campaign");
        assert_eq!(
            after, before,
            "the append-only migration changed its parent row"
        );

        connection
            .execute(
                "INSERT INTO publish_execution_snapshots
                 (campaign_id,input_digest,status,retry_scope,report_json,updated_at)
                 VALUES('campaign-v26',?1,'partial','linkAndSheet','{}','t2')",
                ["a".repeat(64)],
            )
            .expect("valid public enum values and report");
        for (column, value) in [
            ("input_digest", "A".repeat(64)),
            ("status", "failed".to_string()),
            ("retry_scope", "link_and_sheet".to_string()),
            ("report_json", "[]".to_string()),
        ] {
            let result = connection.execute(
                &format!(
                    "UPDATE publish_execution_snapshots SET {column}=?1 WHERE campaign_id='campaign-v26'"
                ),
                [value],
            );
            assert!(result.is_err(), "invalid {column} passed its schema check");
        }

        connection
            .execute("DELETE FROM publish_campaigns WHERE id='campaign-v26'", [])
            .expect("delete parent");
        let snapshots: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM publish_execution_snapshots",
                [],
                |row| row.get(0),
            )
            .expect("count cascade survivors");
        assert_eq!(
            snapshots, 0,
            "execution snapshot did not follow its campaign"
        );

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_28_adds_append_only_nurture_history_and_preserves_existing_rows() {
        let path = temp_db_path("nurture-run-history");
        let mut connection = Connection::open(&path).expect("fixture");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("production foreign-key posture");
        run_with_failpoint(&mut connection, Some(28)).expect_err("stop at v27");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 27);
        assert!(!table_exists(&connection, "nurture_runs"));
        connection
            .execute(
                "INSERT INTO settings(key,value) VALUES('kept-at-v27','yes')",
                [],
            )
            .expect("seed existing row");

        run_with_failpoint(&mut connection, Some(29)).expect_err("stop after migration 28");

        assert_eq!(migration_rows(&connection).last().unwrap().0, 28);
        assert_eq!(
            connection
                .query_row(
                    "SELECT value FROM settings WHERE key='kept-at-v27'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .expect("read existing row"),
            "yes"
        );
        assert!(table_exists(&connection, "nurture_runs"));
        assert!(table_exists(&connection, "nurture_run_status_events"));

        connection
            .execute(
                "INSERT INTO nurture_runs
                 (id,target_udids_json,target_count,created_at,updated_at)
                 VALUES('run-a','[\"phone-a\"]',1,'t0','t0')",
                [],
            )
            .expect("valid run");
        let queued = serde_json::json!({
            "udid": "phone-a",
            "runId": "run-a",
            "running": true
        })
        .to_string();
        connection
            .execute(
                "INSERT INTO nurture_run_status_events
                 (run_id,udid,status_json,recorded_at) VALUES('run-a','phone-a',?1,'t0')",
                [queued],
            )
            .expect("first transition");
        let finished = serde_json::json!({
            "udid": "phone-a",
            "runId": "run-a",
            "running": false
        })
        .to_string();
        connection
            .execute(
                "INSERT INTO nurture_run_status_events
                 (run_id,udid,status_json,recorded_at) VALUES('run-a','phone-a',?1,'t1')",
                [finished],
            )
            .expect("second transition");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM nurture_run_status_events WHERE run_id='run-a'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .expect("count transitions"),
            2,
            "a later state must append rather than overwrite its predecessor"
        );
        connection
            .execute("DELETE FROM nurture_runs WHERE id='run-a'", [])
            .expect("delete run");
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM nurture_run_status_events",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .expect("count cascade survivors"),
            0
        );

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_29_adds_a_constrained_public_cleanup_journal() {
        let path = temp_db_path("public-cleanup-journal");
        let mut connection = Connection::open(&path).expect("fixture");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("production foreign-key posture");
        run_with_failpoint(&mut connection, Some(29)).expect_err("stop at v28");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 28);
        assert!(!table_exists(&connection, "public_cleanup_runs"));

        run_with_failpoint(&mut connection, Some(30)).expect_err("stop after migration 29");

        assert_eq!(migration_rows(&connection).last().unwrap().0, 29);
        assert!(table_exists(&connection, "public_cleanup_runs"));
        let schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='table' AND name='public_cleanup_runs'",
                [],
                |row| row.get(0),
            )
            .expect("read cleanup schema");
        for invariant in [
            "source_action_run_id TEXT NOT NULL UNIQUE",
            "action_kind IN ('like','save')",
            "'planned','preparing','armed','cleared','already_clear','failed_before_effect','uncertain'",
            "json_extract(target_json, '$.normalizedUrl')",
        ] {
            assert!(schema.contains(invariant), "cleanup schema lost `{invariant}`");
        }

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn migration_30_adds_immutable_nurture_follow_provenance_without_rebuilding_v29() {
        let path = temp_db_path("nurture-follow-source-identity");
        let mut connection = Connection::open(&path).expect("fixture");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("production foreign-key posture");
        run_with_failpoint(&mut connection, Some(30)).expect_err("stop at v29");
        assert_eq!(migration_rows(&connection).last().unwrap().0, 29);
        assert!(table_exists(&connection, "public_cleanup_runs"));
        assert!(!table_exists(
            &connection,
            "nurture_follow_source_identities"
        ));

        run(&mut connection).expect("apply migration 30");

        assert_eq!(migration_rows(&connection).last().unwrap().0, 31);
        assert!(table_exists(
            &connection,
            "nurture_follow_source_identities"
        ));
        assert!(table_exists(&connection, "nurture_follow_armed_witnesses"));
        let schema: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master
                 WHERE type='table' AND name='nurture_follow_source_identities'",
                [],
                |row| row.get(0),
            )
            .expect("read Follow provenance schema");
        for invariant in [
            "action_run_id TEXT PRIMARY KEY",
            "identity_json TEXT NOT NULL CHECK (json_valid(identity_json))",
            "substr(canonical_handle, 1, 1) = '@'",
            "card_key NOT GLOB '*[^0-9a-f]*'",
            "author_profile_key NOT GLOB '*[^0-9a-f]*'",
            "readback_generation INTEGER NOT NULL CHECK (readback_generation > 0)",
            "readback_snapshot_sha256 TEXT NOT NULL CHECK",
            "readback_verdict TEXT NOT NULL CHECK (readback_verdict = 'follow_absent')",
        ] {
            assert!(
                schema.contains(invariant),
                "Follow schema lost `{invariant}`"
            );
        }
        let trigger_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type='trigger'
                   AND name LIKE 'nurture_follow_source_identities_%'",
                [],
                |row| row.get(0),
            )
            .expect("count immutable triggers");
        assert_eq!(trigger_count, 3);

        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("schema fixture isolates the source trigger");
        let mut identity_value = serde_json::json!({
            "canonicalHandle": "@exact.author",
            "cardKey": "a".repeat(64),
            "authorProfileKey": "b".repeat(64),
            "authorProfileProof": {
                "tuple": {
                    "package": "com.ss.android.ugc.trill",
                    "versionName": "38.3.2",
                    "locale": "en"
                },
                "hierarchyGeneration": 41,
                "snapshotSha256": "c".repeat(64),
                "profileNodeIndex": 4,
                "profileResourceId": "com.ss.android.ugc.trill:id/t40",
                "profileClassName": "android.widget.ImageView",
                "profileContentDescription": "@exact.author profile",
                "profileEnabled": true,
                "profileClickable": true,
                "followNodeIndex": 5,
                "followResourceId": "com.ss.android.ugc.trill:id/fm1",
                "followClassName": "android.widget.Button",
                "followContentDescription": "Follow @exact.author",
                "followEnabled": true,
                "followClickable": true,
                "cardNodeIndex": 3,
                "railNodeIndex": 2,
                "pagerNodeIndex": 1,
                "feedTabNodeIndex": 6,
                "parentChain": [3, 2, 1, 0],
                "canonicalHandle": "@exact.author",
                "cardContinuityKey": "a".repeat(64)
            }
        });
        let proof = identity_value["authorProfileProof"]
            .as_object_mut()
            .expect("proof object");
        for (name, value) in [
            (
                "cardResourceId",
                serde_json::json!("com.ss.android.ugc.trill:id/cv2"),
            ),
            (
                "cardClassName",
                serde_json::json!("android.widget.FrameLayout"),
            ),
            ("cardEnabled", serde_json::json!(true)),
            ("cardClickable", serde_json::json!(true)),
            (
                "railResourceId",
                serde_json::json!("com.ss.android.ugc.trill:id/hfp"),
            ),
            (
                "railClassName",
                serde_json::json!("android.widget.LinearLayout"),
            ),
            ("railEnabled", serde_json::json!(true)),
            ("railClickable", serde_json::json!(true)),
            (
                "pagerResourceId",
                serde_json::json!("com.ss.android.ugc.trill:id/tod"),
            ),
            (
                "pagerClassName",
                serde_json::json!("androidx.viewpager.widget.ViewPager"),
            ),
            ("pagerEnabled", serde_json::json!(true)),
            ("pagerClickable", serde_json::json!(false)),
            (
                "feedTabClassName",
                serde_json::json!("android.widget.LinearLayout"),
            ),
            ("feedTabContentDescription", serde_json::json!("For You")),
            ("feedTabEnabled", serde_json::json!(true)),
            ("feedTabClickable", serde_json::json!(false)),
            ("feedTabSelected", serde_json::json!(true)),
        ] {
            proof.insert(name.to_owned(), value);
        }
        let identity = identity_value.to_string();
        let mut untyped_value: serde_json::Value =
            serde_json::from_str(&identity).expect("identity");
        untyped_value["unexpected"] = serde_json::json!(true);
        let untyped = untyped_value.to_string();
        let wrong_tuple = identity.replace("38.3.2", "38.3.3");
        for (id, owner_kind, action_kind, card_identity) in [
            ("valid", "nurture", "follow", identity.as_str()),
            ("direct-confirmed", "nurture", "follow", identity.as_str()),
            ("planned", "nurture", "follow", identity.as_str()),
            ("like", "nurture", "like", identity.as_str()),
            ("interaction", "interaction", "follow", identity.as_str()),
            ("untyped", "nurture", "follow", untyped.as_str()),
            ("wrong-tuple", "nurture", "follow", wrong_tuple.as_str()),
        ] {
            let (campaign, assignment) = if owner_kind == "interaction" {
                (Some("campaign"), Some("assignment"))
            } else {
                (None, None)
            };
            connection
                .execute(
                    "INSERT INTO tiktok_action_runs
                     (id,owner_kind,owner_id,device_udid,card_identity_json,campaign_id,
                      assignment_id,action_kind,state,revision,effect_intent,created_at,updated_at)
                     VALUES(?1,?2,COALESCE(?5,?1),'device-2',?3,?4,?5,?6,
                            CASE WHEN ?1='direct-confirmed' THEN 'confirmed' ELSE 'planned' END,
                            CASE WHEN ?1='direct-confirmed' THEN 3 ELSE 0 END,
                            CASE WHEN ?1='direct-confirmed' THEN 'follow_exact_author' ELSE NULL END,
                            'now',CASE WHEN ?1='direct-confirmed' THEN 'confirmed-at' ELSE 'now' END)",
                    params![
                        id,
                        owner_kind,
                        card_identity,
                        campaign,
                        assignment,
                        action_kind
                    ],
                )
                .expect("source action fixture");
        }
        for id in ["valid", "untyped", "wrong-tuple"] {
            assert_eq!(
                connection
                    .execute(
                        "UPDATE tiktok_action_runs SET state='preparing',revision=1,
                                updated_at='preparing-at' WHERE id=?1 AND state='planned'",
                        [id],
                    )
                    .expect("claim Follow fixture"),
                1
            );
            assert_eq!(
                connection
                    .execute(
                        "UPDATE tiktok_action_runs SET state='armed',revision=2,
                                effect_intent='follow_exact_author',updated_at='armed-at'
                         WHERE id=?1 AND state='preparing' AND revision=1",
                        [id],
                    )
                    .expect("arm Follow fixture"),
                1
            );
            assert_eq!(
                connection
                    .execute(
                        "UPDATE tiktok_action_runs SET state='confirmed',revision=3,
                                updated_at='confirmed-at'
                         WHERE id=?1 AND state='armed' AND revision=2",
                        [id],
                    )
                    .expect("confirm Follow fixture"),
                1
            );
        }
        let insert_source = |action_run_id: &str, raw_identity: &str, handle: &str| {
            connection.execute(
                "INSERT INTO nurture_follow_source_identities
                 (action_run_id,identity_json,canonical_handle,card_key,author_profile_key,
                  readback_generation,readback_snapshot_sha256,readback_verdict,confirmed_at)
                 VALUES(?1,?2,?3,?4,?5,42,?6,'follow_absent','confirmed-at')",
                params![
                    action_run_id,
                    raw_identity,
                    handle,
                    "a".repeat(64),
                    "b".repeat(64),
                    "d".repeat(64)
                ],
            )
        };
        assert!(insert_source("planned", &identity, "@exact.author").is_err());
        assert!(insert_source("like", &identity, "@exact.author").is_err());
        assert!(insert_source("interaction", &identity, "@exact.author").is_err());
        assert!(insert_source("direct-confirmed", &identity, "@exact.author").is_err());
        assert!(insert_source("valid", &identity, "@other.author").is_err());
        assert!(insert_source("untyped", &untyped, "@exact.author").is_err());
        assert!(insert_source("wrong-tuple", &wrong_tuple, "@exact.author").is_err());
        assert_eq!(
            insert_source("valid", &identity, "@exact.author").expect("valid exact source"),
            1
        );
        assert!(connection
            .execute(
                "UPDATE tiktok_action_runs SET card_identity_json='{}' WHERE id='valid'",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                "UPDATE nurture_follow_armed_witnesses SET effect_intent='other'
                 WHERE action_run_id='valid'",
                [],
            )
            .is_err());

        drop(connection);
        cleanup(&path);
    }

    #[test]
    fn interaction_action_ledger_has_one_row_per_assignment_and_kind() {
        let mut connection = Connection::open_in_memory().expect("fixture");
        connection.execute_batch(LEDGER_SQL).expect("ledger");
        {
            let transaction = connection.transaction().expect("tx");
            for migration in MIGRATIONS {
                (migration.apply)(&transaction).expect("apply migration");
            }
            transaction.commit().expect("commit schema");
        }
        // Foreign keys are intentionally off for this schema-only fixture; this test pins the
        // uniqueness and enum checks without constructing an entire campaign graph.
        connection
            .execute_batch("PRAGMA foreign_keys = OFF;")
            .expect("disable foreign keys for schema-only fixture");
        connection
            .execute(
                "INSERT INTO tiktok_action_runs
                 (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,
                  action_kind,state,created_at,updated_at)
                 VALUES('run-1','interaction','assignment','device','campaign','assignment',
                        'save','planned','now','now')",
                [],
            )
            .expect("first action");
        assert!(connection
            .execute(
                "INSERT INTO tiktok_action_runs
                 (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,
                  action_kind,state,created_at,updated_at)
                 VALUES('run-2','interaction','assignment','device','campaign','assignment',
                        'save','planned','now','now')",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                "INSERT INTO tiktok_action_runs
                 (id,owner_kind,owner_id,device_udid,campaign_id,assignment_id,
                  action_kind,state,created_at,updated_at)
                 VALUES('run-3','interaction','assignment','device','campaign','assignment',
                        'share','planned','now','now')",
                [],
            )
            .is_err());
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
        const EXEMPT: &[(i64, &str)] = &[
            (
                7,
                "drops `users`, which no table references — no cascade, so nothing to lose",
            ),
            (
                16,
                "drops five tables none of which is a cascade parent: `proxies` is referenced \
                 by nothing (`device_meta.proxy_id` has no FOREIGN KEY clause), \
                 `tiktok_accounts`/`publish_tasks`/`nurture_comment_costs` have no foreign keys \
                 at all, and `publish_dispatch`'s only key points outward at \
                 `publish_campaigns` — so it is a child, and cascades never run child to parent",
            ),
        ];

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
    fn migration_18_keeps_the_sent_twin_when_a_url_was_owed_twice() {
        let path = temp_db_path("outbox-dedupe");
        let mut connection = Connection::open(&path).expect("fixture");
        run_with_failpoint(&mut connection, Some(18)).expect_err("stop before the rebuild");
        // The v17 rows reference publish parents through CASCADE FKs; the parents are not
        // this test's subject, so enforcement is off while seeding — the rebuild itself
        // drops those FKs, which is the whole point of migration 18.
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .expect("seed without parents");
        // Seed order is part of the assertion: group t/1 puts its correct survivor LAST and
        // group t/2 puts its correct survivor FIRST, so no positional picker — "GROUP BY
        // takes the first row it visits", or the last — can satisfy both groups. The first
        // version of this test had both answers in the lucky position, and the reverted
        // GROUP BY passed it.
        connection
            .execute_batch(
                "INSERT INTO publish_sheet_outbox
                   (assignment_id,campaign_id,post_url,poster,partners_json,state,attempts,last_error,created_at,updated_at)
                 VALUES
                   ('asg-failed','camp-1','https://t/1','poster-a','[]','failed',3,'webhook 500','2026-08-29T00:00:00Z','2026-08-29T02:00:00Z'),
                   ('asg-sent','camp-1','https://t/1','poster-a','[]','sent',1,NULL,'2026-08-29T00:00:00Z','2026-08-29T01:00:00Z'),
                   ('asg-newer','camp-2','https://t/2','poster-b','[]','pending',0,NULL,'2026-08-29T00:00:00Z','2026-08-29T03:00:00Z'),
                   ('asg-older','camp-2','https://t/2','poster-b','[]','pending',0,NULL,'2026-08-29T00:00:00Z','2026-08-29T00:30:00Z');",
            )
            .expect("seed v17 duplicates");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("back to production posture");
        run_with_failpoint(&mut connection, None).expect("finish the migrations");

        // **A `sent` twin survives even when the `failed` one is newer.** Keeping the failed
        // copy makes the sweep re-deliver a link the sheet already has — the duplicate
        // column D the new unique index exists to prevent. A bare-column GROUP BY left this
        // choice to SQLite.
        let survivor: (String, String) = connection
            .query_row(
                "SELECT assignment_id,state FROM publish_sheet_outbox WHERE post_url='https://t/1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("exactly one row per url");
        assert_eq!(
            survivor,
            ("asg-sent".to_string(), "sent".to_string()),
            "keeping the failed twin re-delivers a link the sheet already has"
        );
        // Two unsent twins: the newer write wins, deterministically.
        let newest: String = connection
            .query_row(
                "SELECT assignment_id FROM publish_sheet_outbox WHERE post_url='https://t/2'",
                [],
                |row| row.get(0),
            )
            .expect("exactly one row per url");
        assert_eq!(newest, "asg-newer");
        let _ = std::fs::remove_file(path);
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
            MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
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
            MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
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
        // **Every version, not a hand-picked eight.** The list was `[1..=7, 14]`, so 8 through
        // 13 were never shown to roll back at all — and three of them create tables. A
        // migration that half-applies leaves a database no later run can repair, which is the
        // one failure mode this test exists for, so the set has to be the whole ledger rather
        // than the versions somebody happened to write an assertion for.
        for failed_version in crate::db::migrations::MIGRATIONS
            .iter()
            .map(|migration| migration.version)
        {
            let path = temp_db_path(&format!("migration-{failed_version}-rollback"));
            let mut connection = Connection::open(&path).expect("rollback fixture");
            let error = run_with_failpoint(&mut connection, Some(failed_version))
                .expect_err("injected migration failure");
            assert!(error.to_string().contains("InjectedMigrationFailure"));

            // The invariant that holds for all of them, whether or not anyone wrote a
            // table-level assertion below: the ledger stops one short, and the version that
            // failed left no row claiming it applied.
            //
            // Except at 1, which creates the ledger itself — a failure there leaves no table to
            // read, and the branch below asserts the stronger thing instead: no user objects at
            // all.
            if failed_version > 1 {
                assert_eq!(
                    migration_rows(&connection)
                        .iter()
                        .map(|(version, _)| *version)
                        .collect::<Vec<_>>(),
                    (1..failed_version).collect::<Vec<_>>(),
                    "migration {failed_version} rolled back but the ledger disagrees"
                );
            }

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
            } else if failed_version == 8 {
                // Failed at 8: the schedules table is back to the shape it had before the
                // column was added. A rolled-back `ALTER TABLE` that left the column behind
                // would make the retry fail with "duplicate column name" forever.
                assert!(!table_exists(&connection, "users"));
                assert!(!column_exists(&connection, "schedules", "last_error"));
            }
            // 9 through 13 carry no table-level assertion of their own; the ledger check above
            // is what they are here for, plus the retry below.

            run(&mut connection).expect("retry migrations");
            assert_eq!(
                migration_rows(&connection)
                    .iter()
                    .map(|(version, _)| *version)
                    .collect::<Vec<_>>(),
                MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
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
            MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
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
            MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>()
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
                                    &format!(
                                        "INSERT INTO schema_migrations(version,name,applied_at)
                                         VALUES({},'future','2026-07-30T00:00:02Z')",
                                        MIGRATIONS.last().expect("a migration").version + 1
                                    ),
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
