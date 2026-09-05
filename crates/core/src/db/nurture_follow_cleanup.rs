//! Durable provenance for a Nurture Follow that a later cleanup may own.

use super::*;
use crate::tiktok_follow_cleanup::{
    nurture_follow_owner_prefix, ConfirmedNurtureFollowSource, NurtureFollowCleanupOrigin,
    NurtureFollowPossibleEffectState, NurtureFollowReadback, NurtureFollowRecovery,
    NurtureFollowSourceIdentity, PossibleNurtureFollowSource,
};
use rusqlite::Transaction;

type FollowSourceRow = (String, String, String, String, i64, String, String);

fn parse_identity(raw: &str) -> anyhow::Result<NurtureFollowSourceIdentity> {
    NurtureFollowSourceIdentity::from_persisted_json(raw)
}

fn read_confirmed_source(
    connection: &Connection,
    action_run_id: &str,
) -> anyhow::Result<Option<ConfirmedNurtureFollowSource>> {
    let row = connection
        .query_row(
            "SELECT action.id,action.owner_id,action.device_udid,
                    source.identity_json,source.readback_generation,
                    source.readback_snapshot_sha256,source.confirmed_at
             FROM nurture_follow_source_identities AS source
             JOIN tiktok_action_runs AS action ON action.id=source.action_run_id
             JOIN nurture_follow_armed_witnesses AS witness
               ON witness.action_run_id=action.id
             WHERE source.action_run_id=?1 AND action.owner_kind='nurture'
               AND action.action_kind='follow' AND action.state='confirmed'
               AND action.revision=witness.armed_revision+1
               AND action.effect_intent=witness.effect_intent
               AND json(action.card_identity_json)=json(source.identity_json)
               AND json(witness.identity_json)=json(source.identity_json)
               AND source.readback_verdict='follow_absent'
               AND source.readback_generation>
                   json_extract(witness.identity_json,
                                '$.authorProfileProof.hierarchyGeneration')
               AND action.updated_at=source.confirmed_at",
            [action_run_id],
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
        .optional()?;
    row.map(
        |(
            action_run_id,
            owner_id,
            device_udid,
            identity_json,
            readback_generation,
            readback_snapshot_sha256,
            confirmed_at,
        ): FollowSourceRow| {
            Ok(ConfirmedNurtureFollowSource {
                action_run_id,
                owner_id,
                device_udid,
                identity: parse_identity(&identity_json)?,
                readback_hierarchy_generation: u64::try_from(readback_generation)
                    .context("negative Follow readback generation")?,
                readback_snapshot_sha256,
                confirmed_at,
            })
        },
    )
    .transpose()
}

fn read_possible_source(
    connection: &Connection,
    action_run_id: &str,
) -> anyhow::Result<Option<PossibleNurtureFollowSource>> {
    type PossibleSourceRow = (String, String, String, String, String, String, String);

    let row = connection
        .query_row(
            "SELECT action.id,action.owner_id,action.device_udid,witness.identity_json,
                    action.state,witness.armed_at,action.updated_at
             FROM tiktok_action_runs AS action
             JOIN nurture_follow_armed_witnesses AS witness
               ON witness.action_run_id=action.id
             LEFT JOIN nurture_follow_source_identities AS source
               ON source.action_run_id=action.id
             WHERE action.id=?1 AND action.owner_kind='nurture'
               AND action.action_kind='follow' AND source.action_run_id IS NULL
               AND action.effect_intent=witness.effect_intent
               AND json(action.card_identity_json)=json(witness.identity_json)
               AND (
                 (action.state='armed' AND action.revision=witness.armed_revision)
                 OR
                 (action.state='uncertain' AND action.revision=witness.armed_revision+1)
               )",
            [action_run_id],
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
        .optional()?;
    row.map(
        |(
            action_run_id,
            owner_id,
            device_udid,
            identity_json,
            state,
            armed_at,
            updated_at,
        ): PossibleSourceRow| {
            let state = match state.as_str() {
                "armed" => NurtureFollowPossibleEffectState::Armed,
                "uncertain" => NurtureFollowPossibleEffectState::Uncertain,
                _ => anyhow::bail!("invalid possible-effect Nurture Follow state"),
            };
            Ok(PossibleNurtureFollowSource {
                action_run_id,
                owner_id,
                device_udid,
                identity: parse_identity(&identity_json)?,
                state,
                armed_at,
                uncertain_at: (state == NurtureFollowPossibleEffectState::Uncertain)
                    .then_some(updated_at),
            })
        },
    )
    .transpose()
}

fn recover_actions(
    transaction: &Transaction<'_>,
    owner_prefix: Option<&str>,
    device_udid: Option<&str>,
) -> anyhow::Result<NurtureFollowRecovery> {
    let now = Utc::now().to_rfc3339();
    let failed_before_effect = transaction.execute(
        "UPDATE tiktok_action_runs
         SET state='failed_before_effect',revision=revision+1,updated_at=?1,
             error_code=COALESCE(error_code,'nurture_follow_worker_lost_before_effect')
         WHERE owner_kind='nurture' AND action_kind='follow' AND state='preparing'
           AND (?2 IS NULL OR device_udid=?2)
           AND (?3 IS NULL OR substr(owner_id,1,length(?3))=?3)",
        params![now, device_udid, owner_prefix],
    )?;
    let uncertain = transaction.execute(
        "UPDATE tiktok_action_runs
         SET state='uncertain',revision=revision+1,updated_at=?1,
             error_code=COALESCE(error_code,'nurture_follow_worker_lost_after_effect_intent')
         WHERE owner_kind='nurture' AND action_kind='follow' AND state='armed'
           AND (?2 IS NULL OR device_udid=?2)
           AND (?3 IS NULL OR substr(owner_id,1,length(?3))=?3)",
        params![now, device_udid, owner_prefix],
    )?;
    let possible_effects = transaction.query_row(
        "SELECT COUNT(*)
         FROM tiktok_action_runs AS action
         JOIN nurture_follow_armed_witnesses AS witness ON witness.action_run_id=action.id
         WHERE action.owner_kind='nurture' AND action.action_kind='follow'
           AND action.state IN ('armed','uncertain')
           AND action.effect_intent=witness.effect_intent
           AND json(action.card_identity_json)=json(witness.identity_json)
           AND (
             (action.state='armed' AND action.revision=witness.armed_revision)
             OR
             (action.state='uncertain' AND action.revision=witness.armed_revision+1)
           )
           AND (?1 IS NULL OR action.device_udid=?1)
           AND (?2 IS NULL OR substr(action.owner_id,1,length(?2))=?2)",
        params![device_udid, owner_prefix],
        |row| row.get(0),
    )?;
    Ok(NurtureFollowRecovery {
        failed_before_effect,
        uncertain,
        possible_effects,
    })
}

fn insert_source(
    transaction: &Transaction<'_>,
    action_run_id: &str,
    identity_json: &str,
    identity: &NurtureFollowSourceIdentity,
    readback: &NurtureFollowReadback,
    confirmed_at: &str,
) -> anyhow::Result<()> {
    transaction.execute(
        "INSERT OR IGNORE INTO nurture_follow_source_identities
         (action_run_id,identity_json,canonical_handle,card_key,author_profile_key,
          readback_generation,readback_snapshot_sha256,readback_verdict,confirmed_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,'follow_absent',?8)",
        params![
            action_run_id,
            identity_json,
            identity.canonical_handle,
            identity.card_key,
            identity.author_profile_key,
            i64::try_from(readback.hierarchy_generation())
                .context("Follow readback generation exceeds SQLite range")?,
            readback.snapshot_sha256(),
            confirmed_at,
        ],
    )?;
    Ok(())
}

impl Database {
    /// Settle an armed Follow when fresh local proof shows that no tap was dispatched.
    ///
    /// The arm witness remains immutable for audit, while cleanup-origin lookup deliberately
    /// excludes this retryable state because the caller proved the effect did not occur.
    pub fn settle_armed_nurture_follow_failed_before_effect(
        &self,
        action_run_id: &str,
        armed_revision: i64,
        evidence_json: Option<&str>,
        error_code: &str,
    ) -> anyhow::Result<bool> {
        anyhow::ensure!(
            !error_code.trim().is_empty(),
            "Nurture Follow failure code is empty"
        );
        if let Some(raw) = evidence_json {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|error| anyhow::anyhow!("invalid Follow evidence: {error}"))?;
        }
        let connection = self.conn()?;
        let changed = connection.execute(
            "UPDATE tiktok_action_runs AS action
             SET state='failed_before_effect',evidence_json=?1,error_code=?2,
                 revision=revision+1,updated_at=?3
             WHERE action.id=?4 AND action.owner_kind='nurture'
               AND action.action_kind='follow' AND action.state='armed'
               AND action.revision=?5 AND EXISTS (
                 SELECT 1 FROM nurture_follow_armed_witnesses AS witness
                 WHERE witness.action_run_id=action.id
                   AND witness.armed_revision=action.revision
                   AND witness.effect_intent=action.effect_intent
                   AND json(witness.identity_json)=json(action.card_identity_json)
               )",
            params![
                evidence_json,
                error_code,
                Utc::now().to_rfc3339(),
                action_run_id,
                armed_revision
            ],
        )?;
        Ok(changed == 1)
    }

    /// Recover every Nurture Follow action left owned by workers from the previous process.
    ///
    /// This global sweep is only valid during startup before command admission opens. It is not
    /// safe to run alongside a live worker because `armed` deliberately precedes the device tap.
    pub fn recover_all_orphaned_nurture_follow_actions(
        &self,
    ) -> anyhow::Result<NurtureFollowRecovery> {
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let recovered = recover_actions(&transaction, None, None)?;
        transaction.commit()?;
        Ok(recovered)
    }

    /// Recover one durable run/device scope at a worker boundary.
    pub fn recover_orphaned_nurture_follow_actions(
        &self,
        run_id: Uuid,
        device_udid: &str,
    ) -> anyhow::Result<NurtureFollowRecovery> {
        anyhow::ensure!(!device_udid.trim().is_empty(), "nurture device is empty");
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run_id_text = run_id.to_string();
        let owns_device: bool = transaction.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM nurture_runs AS run,json_each(run.target_udids_json) AS target
               WHERE run.id=?1 AND target.type='text' AND target.value=?2
             )",
            params![run_id_text, device_udid],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            owns_device,
            "nurture run does not own the requested recovery device"
        );
        let owner_prefix = nurture_follow_owner_prefix(run_id);
        let recovered =
            recover_actions(&transaction, Some(owner_prefix.as_str()), Some(device_udid))?;
        transaction.commit()?;
        Ok(recovered)
    }

    /// Atomically settle an armed Nurture Follow and append the exact source identity.
    ///
    /// The identity is read from immutable `card_identity_json`; callers cannot attach a new
    /// handle after the effect. Generic confirmation is rejected, so confirmation and source
    /// insertion either commit together or both roll back.
    pub fn settle_confirmed_nurture_follow_with_source(
        &self,
        action_run_id: &str,
        armed_revision: i64,
        readback: &NurtureFollowReadback,
        evidence_json: Option<&str>,
    ) -> anyhow::Result<Option<ConfirmedNurtureFollowSource>> {
        if let Some(raw) = evidence_json {
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|error| anyhow::anyhow!("invalid Follow evidence: {error}"))?;
        }
        let mut connection = self.conn()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction
            .query_row(
                "SELECT action.card_identity_json,action.effect_intent
                 FROM tiktok_action_runs AS action
                 JOIN nurture_follow_armed_witnesses AS witness
                   ON witness.action_run_id=action.id
                 WHERE action.id=?1 AND action.owner_kind='nurture'
                   AND action.action_kind='follow' AND witness.armed_revision=?2
                   AND action.state='armed' AND action.revision=?2
                   AND action.effect_intent=witness.effect_intent
                   AND json(action.card_identity_json)=json(witness.identity_json)",
                params![action_run_id, armed_revision],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((identity_json, _effect_intent)) = row else {
            transaction.rollback()?;
            return Ok(None);
        };
        let identity_json = identity_json
            .ok_or_else(|| anyhow::anyhow!("source Nurture Follow has no card identity"))?;
        let identity = parse_identity(&identity_json)?;
        readback.validate_confirmation(&identity)?;

        let confirmed_at = Utc::now().to_rfc3339();
        let changed = transaction.execute(
            "UPDATE tiktok_action_runs
             SET state='confirmed',evidence_json=?1,error_code=NULL,
                 revision=revision+1,updated_at=?2
             WHERE id=?3 AND owner_kind='nurture' AND action_kind='follow'
               AND state='armed' AND revision=?4",
            params![evidence_json, confirmed_at, action_run_id, armed_revision],
        )?;
        if changed != 1 {
            transaction.rollback()?;
            return Ok(None);
        }

        insert_source(
            &transaction,
            action_run_id,
            &identity_json,
            &identity,
            readback,
            &confirmed_at,
        )?;
        let source = read_confirmed_source(&transaction, action_run_id)?
            .ok_or_else(|| anyhow::anyhow!("Follow source identity disappeared after insertion"))?;
        anyhow::ensure!(
            source.identity == identity,
            "existing Follow source identity does not match its immutable action"
        );
        transaction.commit()?;
        Ok(Some(source))
    }

    /// Lookup for cleanup preflight; weak, unconfirmed and non-Nurture actions are invisible.
    pub fn nurture_follow_cleanup_source(
        &self,
        action_run_id: &str,
    ) -> anyhow::Result<Option<ConfirmedNurtureFollowSource>> {
        read_confirmed_source(&self.conn()?, action_run_id)
    }

    /// Read exact Follow provenance without upgrading a possible effect to a confirmed effect.
    pub fn nurture_follow_cleanup_origin(
        &self,
        action_run_id: &str,
    ) -> anyhow::Result<Option<NurtureFollowCleanupOrigin>> {
        let connection = self.conn()?;
        if let Some(source) = read_confirmed_source(&connection, action_run_id)? {
            return Ok(Some(NurtureFollowCleanupOrigin::Confirmed(source)));
        }
        Ok(read_possible_source(&connection, action_run_id)?
            .map(NurtureFollowCleanupOrigin::PossibleEffect))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::HierarchySourceSnapshot;
    use crate::interaction::{InteractionActionKind, TikTokActionOwner, TikTokActionOwnerKind};
    use crate::tiktok_follow_cleanup::{
        prove_nurture_follow_source, readback_nurture_follow_source, NurtureFollowCleanupOrigin,
        NurtureFollowPossibleEffectState, NurtureFollowReadback, NurtureFollowSourceIdentity,
        MEASURED_FOLLOW_LOCALE, MEASURED_FOLLOW_PACKAGE, MEASURED_FOLLOW_VERSION,
    };
    use crate::types::NurtureSessionStatus;
    use uuid::Uuid;

    fn fixture() -> (Database, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "riviu-nurture-follow-cleanup-test-{}.db",
            Uuid::new_v4()
        ));
        (Database::open(&path).expect("open fixture database"), path)
    }

    fn identity(handle: &str) -> NurtureFollowSourceIdentity {
        let snapshot = HierarchySourceSnapshot {
            generation: 41,
            xml: format!(
                r#"<hierarchy><node package="{package}" class="android.widget.FrameLayout" resource-id="root" text="" content-desc="" bounds="[0,0][1080,2160]" enabled="true" clickable="false" selected="false"><node package="{package}" class="androidx.viewpager.widget.ViewPager" resource-id="{package}:id/tod" text="" content-desc="" bounds="[0,0][1080,2000]" enabled="true" clickable="false" selected="false"><node package="{package}" class="android.widget.LinearLayout" resource-id="{package}:id/hfp" text="" content-desc="" bounds="[0,0][1080,1900]" enabled="true" clickable="true" selected="false"><node package="{package}" class="android.widget.FrameLayout" resource-id="{package}:id/cv2" text="" content-desc="" bounds="[0,100][1080,1900]" enabled="true" clickable="true" selected="false"><node package="{package}" class="android.widget.ImageView" resource-id="{package}:id/t40" text="" content-desc="{handle} profile" bounds="[24,1500][580,1560]" enabled="true" clickable="true" selected="false"/><node package="{package}" class="android.widget.Button" resource-id="{package}:id/fm1" text="" content-desc="Follow {handle}" bounds="[870,1450][1030,1530]" enabled="true" clickable="true" selected="false"/></node></node></node><node package="{package}" class="android.widget.LinearLayout" resource-id="" text="" content-desc="For You" bounds="[540,2020][1080,2160]" enabled="true" clickable="false" selected="true"/></node></hierarchy>"#,
                package = MEASURED_FOLLOW_PACKAGE,
            ),
        };
        prove_nurture_follow_source(
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &snapshot,
        )
        .expect("source identity")
        .into_parts()
        .0
    }

    fn confirmed_readback(identity: &NurtureFollowSourceIdentity) -> NurtureFollowReadback {
        let handle = identity.canonical_handle();
        let snapshot = HierarchySourceSnapshot {
            generation: 42,
            xml: format!(
                r#"<hierarchy><node package="{package}" class="android.widget.FrameLayout" resource-id="root" text="" content-desc="" bounds="[0,0][1080,2160]" enabled="true" clickable="false" selected="false"><node package="{package}" class="androidx.viewpager.widget.ViewPager" resource-id="{package}:id/tod" text="" content-desc="" bounds="[0,0][1080,2000]" enabled="true" clickable="false" selected="false"><node package="{package}" class="android.widget.LinearLayout" resource-id="{package}:id/hfp" text="" content-desc="" bounds="[0,0][1080,1900]" enabled="true" clickable="true" selected="false"><node package="{package}" class="android.widget.FrameLayout" resource-id="{package}:id/cv2" text="" content-desc="" bounds="[0,100][1080,1900]" enabled="true" clickable="true" selected="false"><node package="{package}" class="android.widget.ImageView" resource-id="{package}:id/t40" text="" content-desc="{handle} profile" bounds="[24,1500][580,1560]" enabled="true" clickable="true" selected="false"/></node></node></node><node package="{package}" class="android.widget.LinearLayout" resource-id="" text="" content-desc="For You" bounds="[540,2020][1080,2160]" enabled="true" clickable="false" selected="true"/></node></hierarchy>"#,
                package = MEASURED_FOLLOW_PACKAGE,
            ),
        };
        readback_nurture_follow_source(
            identity,
            MEASURED_FOLLOW_PACKAGE,
            MEASURED_FOLLOW_VERSION,
            MEASURED_FOLLOW_LOCALE,
            &snapshot,
        )
        .expect("confirmed readback")
    }

    fn owner(identity: &NurtureFollowSourceIdentity) -> TikTokActionOwner {
        TikTokActionOwner {
            kind: TikTokActionOwnerKind::Nurture,
            owner_id: "nurture-session:follow:card-a".into(),
            device_udid: "device-2".into(),
            card_identity: Some(serde_json::to_string(identity).expect("serialize identity")),
        }
    }

    fn scoped_owner(
        run_id: Uuid,
        device_udid: &str,
        identity: &NurtureFollowSourceIdentity,
    ) -> TikTokActionOwner {
        TikTokActionOwner {
            kind: TikTokActionOwnerKind::Nurture,
            owner_id: format!(
                "{}{}:{}",
                nurture_follow_owner_prefix(run_id),
                &identity.card_key()[..16],
                &identity.author_profile_key()[..16]
            ),
            device_udid: device_udid.to_owned(),
            card_identity: Some(serde_json::to_string(identity).expect("serialize identity")),
        }
    }

    fn create_run(db: &Database, run_id: Uuid, devices: &[&str]) {
        let targets = devices
            .iter()
            .map(|device| (*device).to_owned())
            .collect::<Vec<_>>();
        let statuses = targets
            .iter()
            .map(|device| {
                let mut status = NurtureSessionStatus::new(device);
                status.run_id = Some(run_id);
                status.run_size = targets.len() as u32;
                status
            })
            .collect::<Vec<_>>();
        db.create_nurture_run(run_id, &targets, &statuses)
            .expect("create durable nurture run");
    }

    fn arm(db: &Database, owner: &TikTokActionOwner) -> (String, i64) {
        let action = db
            .ensure_tiktok_action_run(owner, InteractionActionKind::Follow)
            .expect("action row");
        let preparing = db
            .claim_tiktok_action(owner, InteractionActionKind::Follow)
            .expect("claim")
            .expect("owner");
        let armed = db
            .arm_tiktok_action(
                owner,
                InteractionActionKind::Follow,
                preparing,
                "follow_exact_author",
            )
            .expect("arm")
            .expect("armed");
        (action.id, armed)
    }

    #[test]
    fn atomic_settle_has_no_confirmed_without_source_crash_gap() {
        let (db, path) = fixture();
        let identity = identity("@exact.author");
        let owner = owner(&identity);
        let (action_id, armed) = arm(&db, &owner);
        db.conn()
            .expect("connection")
            .execute_batch(
                "CREATE TRIGGER fail_follow_source BEFORE INSERT
                 ON nurture_follow_source_identities BEGIN
                   SELECT RAISE(ABORT, 'injected source write failure');
                 END;",
            )
            .expect("install source failpoint");

        assert!(db
            .settle_confirmed_nurture_follow_with_source(
                &action_id,
                armed,
                &confirmed_readback(&identity),
                Some(r#"{"sameAuthor":true,"followControlGone":true}"#),
            )
            .is_err());
        let action = db
            .get_tiktok_action_run(&owner, InteractionActionKind::Follow)
            .expect("read action")
            .expect("action exists");
        assert_eq!(
            action.state,
            crate::interaction::InteractionActionState::Armed
        );
        assert!(db
            .nurture_follow_cleanup_source(&action_id)
            .expect("source lookup")
            .is_none());

        db.conn()
            .expect("connection")
            .execute_batch("DROP TRIGGER fail_follow_source;")
            .expect("remove failpoint");
        let source = db
            .settle_confirmed_nurture_follow_with_source(
                &action_id,
                armed,
                &confirmed_readback(&identity),
                None,
            )
            .expect("atomic settle")
            .expect("owned action");
        assert_eq!(source.identity, identity);
        assert_eq!(
            db.get_tiktok_action_run(&owner, InteractionActionKind::Follow)
                .expect("read confirmed action")
                .expect("action exists")
                .state,
            crate::interaction::InteractionActionState::Confirmed
        );

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn generic_confirm_is_rejected_and_atomic_confirm_remains_the_only_path() {
        let (db, path) = fixture();
        let identity = identity("@exact.author");
        let owner = owner(&identity);
        let (action_id, armed) = arm(&db, &owner);
        assert!(db
            .settle_tiktok_action(
                &owner,
                InteractionActionKind::Follow,
                armed,
                crate::interaction::InteractionActionState::Confirmed,
                None,
                None,
            )
            .is_err());
        let action = db
            .get_tiktok_action_run(&owner, InteractionActionKind::Follow)
            .expect("read rejected generic settlement")
            .expect("armed Follow remains");
        assert_eq!(
            action.state,
            crate::interaction::InteractionActionState::Armed
        );
        assert!(db
            .nurture_follow_cleanup_source(&action_id)
            .expect("confirmed source lookup")
            .is_none());
        let source = db
            .settle_confirmed_nurture_follow_with_source(
                &action_id,
                armed,
                &confirmed_readback(&identity),
                None,
            )
            .expect("atomic confirmation")
            .expect("atomic source");
        assert_eq!(source.identity, identity);

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn armed_identity_is_immutable_and_non_follow_rows_have_no_cleanup_origin() {
        let (db, path) = fixture();
        let identity = identity("@exact.author");
        let mut like_owner = owner(&identity);
        like_owner.owner_id = "nurture-session:like:card-a".into();
        let like = db
            .ensure_tiktok_action_run(&like_owner, InteractionActionKind::Like)
            .expect("like row");
        assert!(db
            .nurture_follow_cleanup_origin(&like.id)
            .expect("ignore non-Follow")
            .is_none());

        let owner = owner(&identity);
        let (action_id, armed) = arm(&db, &owner);
        assert!(db
            .conn()
            .expect("connection")
            .execute(
                "UPDATE tiktok_action_runs SET card_identity_json='{}'
                 WHERE id=?1 AND revision=?2",
                params![action_id, armed],
            )
            .is_err());
        let source = db
            .settle_confirmed_nurture_follow_with_source(
                &action_id,
                armed,
                &confirmed_readback(&identity),
                None,
            )
            .expect("settle intact armed identity")
            .expect("source");
        assert_eq!(source.identity, identity);
        assert!(db
            .conn()
            .expect("connection")
            .execute(
                "UPDATE tiktok_action_runs SET owner_id='other-owner' WHERE id=?1",
                [action_id],
            )
            .is_err());

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn directly_inserted_confirmed_row_has_no_arm_witness_and_is_invisible() {
        let (db, path) = fixture();
        let identity = identity("@exact.author");
        let identity_json = serde_json::to_string(&identity).expect("identity JSON");
        db.conn()
            .expect("connection")
            .execute(
                "INSERT INTO tiktok_action_runs
                 (id,owner_kind,owner_id,device_udid,card_identity_json,action_kind,state,
                  revision,effect_intent,created_at,updated_at)
                 VALUES('forged-confirmed','nurture','forged-owner','device-2',?1,'follow',
                        'confirmed',3,'follow_exact_author','now','confirmed-at')",
                [identity_json.as_str()],
            )
            .expect("direct confirmed fixture");
        assert!(db
            .nurture_follow_cleanup_origin("forged-confirmed")
            .expect("ignore forged row")
            .is_none());
        assert!(db
            .conn()
            .expect("connection")
            .execute(
                "INSERT INTO nurture_follow_source_identities
                 (action_run_id,identity_json,canonical_handle,card_key,author_profile_key,
                  confirmed_at) VALUES('forged-confirmed',?1,?2,?3,?4,'confirmed-at')",
                params![
                    identity_json,
                    identity.canonical_handle(),
                    identity.card_key(),
                    identity.author_profile_key(),
                ],
            )
            .is_err());

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn restart_recovers_scoped_follow_actions_and_preserves_possible_effect_source() {
        let (db, path) = fixture();
        let run_id = Uuid::new_v4();
        let other_run_id = Uuid::new_v4();
        create_run(&db, run_id, &["device-a", "device-b"]);
        create_run(&db, other_run_id, &["device-a"]);

        let before_identity = identity("@before.effect");
        let before_owner = scoped_owner(run_id, "device-a", &before_identity);
        let _before_action = db
            .ensure_tiktok_action_run(&before_owner, InteractionActionKind::Follow)
            .expect("before-effect action");
        let before_revision = db
            .claim_tiktok_action(&before_owner, InteractionActionKind::Follow)
            .expect("claim before-effect action")
            .expect("before-effect ownership");

        let possible_identity = identity("@possible.effect");
        let possible_owner = scoped_owner(run_id, "device-a", &possible_identity);
        let (possible_action_id, armed_revision) = arm(&db, &possible_owner);
        let armed_origin = db
            .nurture_follow_cleanup_origin(&possible_action_id)
            .expect("read armed origin")
            .expect("armed origin");
        assert!(matches!(
            armed_origin,
            NurtureFollowCleanupOrigin::PossibleEffect(ref source)
                if source.state == NurtureFollowPossibleEffectState::Armed
                    && source.identity == possible_identity
                    && source.uncertain_at.is_none()
        ));

        let other_device_identity = identity("@other.device");
        let other_device_owner = scoped_owner(run_id, "device-b", &other_device_identity);
        let (_other_device_id, other_device_armed) = arm(&db, &other_device_owner);
        let other_run_identity = identity("@other.run");
        let other_run_owner = scoped_owner(other_run_id, "device-a", &other_run_identity);
        let (_other_run_id, other_run_armed) = arm(&db, &other_run_owner);

        drop(db);
        let reopened = Database::open(&path).expect("restart database");
        let recovered = reopened
            .recover_orphaned_nurture_follow_actions(run_id, "device-a")
            .expect("recover exact run/device");
        assert_eq!(
            recovered,
            NurtureFollowRecovery {
                failed_before_effect: 1,
                uncertain: 1,
                possible_effects: 1,
            }
        );
        let idempotent = reopened
            .recover_orphaned_nurture_follow_actions(run_id, "device-a")
            .expect("idempotent recovery");
        assert!(idempotent.is_empty());
        assert!(idempotent.has_possible_effect());

        let before = reopened
            .get_tiktok_action_run(&before_owner, InteractionActionKind::Follow)
            .expect("read before-effect action")
            .expect("before-effect action remains");
        assert_eq!(
            before.state,
            crate::interaction::InteractionActionState::FailedBeforeEffect
        );
        assert_eq!(before.revision, before_revision + 1);
        assert_eq!(before.card_identity, before_owner.card_identity);
        assert_eq!(
            reopened
                .claim_tiktok_action(&before_owner, InteractionActionKind::Follow)
                .expect("retry before-effect action"),
            Some(before_revision + 2)
        );

        let possible = reopened
            .get_tiktok_action_run(&possible_owner, InteractionActionKind::Follow)
            .expect("read possible-effect action")
            .expect("possible-effect action remains");
        assert_eq!(
            possible.state,
            crate::interaction::InteractionActionState::Uncertain
        );
        assert_eq!(possible.revision, armed_revision + 1);
        assert_eq!(possible.card_identity, possible_owner.card_identity);
        assert!(possible.effect_intent.is_some());
        assert_eq!(
            reopened
                .claim_tiktok_action(&possible_owner, InteractionActionKind::Follow)
                .expect("possible effect stays outside retry"),
            None
        );
        assert!(reopened
            .nurture_follow_cleanup_source(&possible_action_id)
            .expect("confirmed-only lookup")
            .is_none());
        let uncertain_origin = reopened
            .nurture_follow_cleanup_origin(&possible_action_id)
            .expect("read uncertain origin")
            .expect("uncertain origin");
        assert!(matches!(
            uncertain_origin,
            NurtureFollowCleanupOrigin::PossibleEffect(ref source)
                if source.state == NurtureFollowPossibleEffectState::Uncertain
                    && source.identity == possible_identity
                    && source.owner_id == possible_owner.owner_id
                    && source.device_udid == possible_owner.device_udid
                    && source.uncertain_at.is_some()
        ));

        for (owner, revision) in [
            (&other_device_owner, other_device_armed),
            (&other_run_owner, other_run_armed),
        ] {
            let action = reopened
                .get_tiktok_action_run(owner, InteractionActionKind::Follow)
                .expect("read out-of-scope action")
                .expect("out-of-scope action remains");
            assert_eq!(
                action.state,
                crate::interaction::InteractionActionState::Armed
            );
            assert_eq!(action.revision, revision);
        }
        drop(reopened);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn startup_sweep_recovers_follow_rows_without_a_running_status() {
        let (db, path) = fixture();
        let run_id = Uuid::new_v4();
        create_run(&db, run_id, &["device-a"]);
        let identity = identity("@terminal.status");
        let owner = scoped_owner(run_id, "device-a", &identity);
        let (action_id, armed_revision) = arm(&db, &owner);

        let recovered = db
            .recover_all_orphaned_nurture_follow_actions()
            .expect("startup sweep");
        assert_eq!(recovered.uncertain, 1);
        assert_eq!(recovered.failed_before_effect, 0);
        assert!(matches!(
            db.nurture_follow_cleanup_origin(&action_id)
                .expect("read startup origin"),
            Some(NurtureFollowCleanupOrigin::PossibleEffect(source))
                if source.state == NurtureFollowPossibleEffectState::Uncertain
                    && source.identity == identity
        ));
        let action = db
            .get_tiktok_action_run(&owner, InteractionActionKind::Follow)
            .expect("read recovered action")
            .expect("recovered action");
        assert_eq!(action.revision, armed_revision + 1);
        assert!(db
            .recover_all_orphaned_nurture_follow_actions()
            .expect("idempotent startup sweep")
            .is_empty());

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }

    #[test]
    fn stop_after_arm_can_settle_before_effect_without_exposing_cleanup_origin() {
        let (db, path) = fixture();
        let identity = identity("@stopped.before.tap");
        let owner = owner(&identity);
        let (action_id, armed_revision) = arm(&db, &owner);

        assert!(db
            .settle_armed_nurture_follow_failed_before_effect(
                &action_id,
                armed_revision,
                Some(r#"{"stopObserved":true,"tapDispatched":false}"#),
                "follow_stopped_before_tap",
            )
            .expect("settle before-effect Follow"));
        let action = db
            .get_tiktok_action_run(&owner, InteractionActionKind::Follow)
            .expect("read stopped Follow")
            .expect("stopped Follow action");
        assert_eq!(
            action.state,
            crate::interaction::InteractionActionState::FailedBeforeEffect
        );
        assert_eq!(action.revision, armed_revision + 1);
        assert!(db
            .nurture_follow_cleanup_origin(&action_id)
            .expect("cleanup origin lookup")
            .is_none());
        let witness_count: i64 = db
            .conn()
            .expect("read witness")
            .query_row(
                "SELECT COUNT(*) FROM nurture_follow_armed_witnesses WHERE action_run_id=?1",
                [action_id],
                |row| row.get(0),
            )
            .expect("witness count");
        assert_eq!(witness_count, 1);

        drop(db);
        std::fs::remove_file(path).expect("remove fixture database");
    }
}
