//! The app-exit fence for a new public Post.
//!
//! `pause_publish_dispatch` only parks *queued* dispatch jobs; a composer already inside its
//! session could still claim its assignment and tap Post while the exit drain waited for it.
//! The fence is process-local on purpose: it describes this process going away, not a durable
//! campaign decision, so the next launch must not inherit it.
//!
//! Both posting claims read the flag inside their `IMMEDIATE` transaction. Raising it and then
//! taking one empty `IMMEDIATE` transaction is the barrier: a claim that read the flag before it
//! was raised still holds the write lock, so the barrier returns only after that claim committed.
//! Every claim that commits after the barrier refuses. A claim that committed before it is an
//! already-dispatched effect and is left to drain and settle — never replayed.
use super::*;
use std::sync::atomic::Ordering;

impl Database {
    /// Refuse every new Post claim from now on, and wait out one that is mid-transaction.
    pub fn begin_publish_shutdown(&self) -> anyhow::Result<()> {
        self.publish_shutdown.store(true, Ordering::SeqCst);
        let mut conn = self.conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.commit()?;
        Ok(())
    }

    /// Whether this process is closing. Composers poll it to stop before their Post boundary.
    pub fn publish_shutdown_started(&self) -> bool {
        self.publish_shutdown.load(Ordering::SeqCst)
    }
}
