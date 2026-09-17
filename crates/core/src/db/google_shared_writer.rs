//! Durable shared-Sheets intent journal. Values contain no OAuth credentials.
//! A single atomic settings slot per tab prevents a new intent from bypassing an
//! unresolved operation; compare-and-swap also protects independent DB handles.
use super::*;

type SharedWriterGate = std::sync::Arc<tokio::sync::Mutex<()>>;
type SharedWriterGates =
    std::collections::HashMap<(PathBuf, String), std::sync::Weak<tokio::sync::Mutex<()>>>;

impl Database {
    /// Local tasks must not reconcile or abandon another task's live operation.
    /// Distinct databases still contend through the remote metadata lock.
    pub(crate) fn google_shared_operation_gate(
        &self,
        key: &str,
    ) -> anyhow::Result<SharedWriterGate> {
        static GATES: std::sync::OnceLock<parking_lot::Mutex<SharedWriterGates>> =
            std::sync::OnceLock::new();
        let path = std::fs::canonicalize(&self.path)?;
        let mut gates = GATES.get_or_init(Default::default).lock();
        gates.retain(|_, gate| gate.strong_count() > 0);
        let slot = gates.entry((path, key.to_owned())).or_default();
        if let Some(gate) = slot.upgrade() {
            return Ok(gate);
        }
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        *slot = std::sync::Arc::downgrade(&gate);
        Ok(gate)
    }

    pub(crate) fn google_shared_journal_read(&self, key: &str) -> anyhow::Result<Option<String>> {
        self.get_setting(key)
    }

    pub(crate) fn google_shared_journal_cas(
        &self,
        key: &str,
        expected: Option<&str>,
        next: &str,
    ) -> anyhow::Result<bool> {
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<String> = tx
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        if current.as_deref() != expected {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, next],
        )?;
        tx.commit()?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Break caught: unconditional settings writes permit two local processes to
    // both dispatch the same operation after observing the same prior phase.
    #[test]
    fn journal_cas_survives_reopen_and_refuses_stale_phase() {
        let path = std::env::temp_dir().join(format!("shared-journal-{}.db", Uuid::new_v4()));
        let db = Database::open(&path).unwrap();
        let second = Database::open(&path).unwrap();
        let key = "google.sheets.shared-journal.fixture";
        assert!(db.google_shared_journal_cas(key, None, "prepared").unwrap());
        assert!(!second
            .google_shared_journal_cas(key, None, "different")
            .unwrap());
        assert!(db
            .google_shared_journal_cas(key, Some("prepared"), "mutationPending")
            .unwrap());
        assert!(!second
            .google_shared_journal_cas(key, Some("prepared"), "dispatchAgain")
            .unwrap());
        drop(db);
        drop(second);
        let restarted = Database::open(&path).unwrap();
        assert_eq!(
            restarted
                .google_shared_journal_read(key)
                .unwrap()
                .as_deref(),
            Some("mutationPending")
        );
        drop(restarted);
        let _ = std::fs::remove_file(path);
    }
}
