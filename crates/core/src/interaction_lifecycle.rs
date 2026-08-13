//! Driver-local proof that the interaction handoff is being followed in order.
//!
//! The control plane demands a strict sequence for any device it is about to
//! drive: the old producer is **stopped**, then a **session** is created, then a
//! **stream** is started — all at one stream generation, so a frame from before
//! the session can never be mistaken for evidence of an action taken after it.
//! The plane checks the *outcome* (`StreamStartProof::generation` must equal the
//! handoff generation, `crate::device_control`), but by then a violation has
//! already reached core as an opaque mismatch.
//!
//! This registry is what lets a driver catch it *first*, at the step that broke,
//! and say which one. It holds no device state and does no I/O; the token never
//! crosses the driver boundary.
//!
//! It lives in core rather than in one driver because **both** backends have to
//! follow the same rule. It started out private to `riviu-ios-driver`; a copy in
//! `riviu-android-driver` would be a second source of truth for the handoff
//! order, and the two would drift.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::types::InteractionSessionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionLifecyclePhase {
    Stopped,
    SessionStarting {
        token: u64,
        kind: InteractionSessionKind,
    },
    SessionReady {
        token: u64,
        kind: InteractionSessionKind,
    },
    StreamStarting {
        token: u64,
        kind: InteractionSessionKind,
    },
    Streaming {
        token: u64,
        kind: InteractionSessionKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InteractionLifecycleState {
    generation: u64,
    phase: InteractionLifecyclePhase,
}

#[derive(Default)]
struct InteractionLifecycleMap {
    next_token: u64,
    devices: HashMap<String, InteractionLifecycleState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionSessionReservation {
    udid: String,
    generation: u64,
    token: u64,
    kind: InteractionSessionKind,
}

impl InteractionSessionReservation {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Which session flavour was asked for. A backend that treats both the same
    /// still carries this so the choice is visible in a log rather than lost.
    pub fn kind(&self) -> InteractionSessionKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionStreamReservation {
    udid: String,
    generation: u64,
    token: u64,
    kind: InteractionSessionKind,
}

impl InteractionStreamReservation {
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// Per-device lifecycle phase for one driver's devices.
///
/// Synchronous on purpose: `invalidate_ui_session` on the `DeviceDriver` trait is
/// not `async`, and the control plane calls it from cleanup paths that cannot
/// await.
#[derive(Clone, Default)]
pub struct InteractionLifecycleRegistry {
    state: Arc<Mutex<InteractionLifecycleMap>>,
}

impl InteractionLifecycleRegistry {
    /// Record that `udid` owns no producer, at `generation`.
    ///
    /// Called after a confirmed stop **and** by the handoff read, which is why it
    /// overwrites rather than requiring a prior phase: a handoff may legitimately
    /// be confirmed twice (the plane re-confirms on a failed stream start).
    pub fn record_stopped(&self, udid: &str, generation: u64) {
        self.state.lock().devices.insert(
            udid.to_string(),
            InteractionLifecycleState {
                generation,
                phase: InteractionLifecyclePhase::Stopped,
            },
        );
    }

    pub fn begin_session(
        &self,
        udid: &str,
        generation: u64,
        kind: InteractionSessionKind,
    ) -> anyhow::Result<InteractionSessionReservation> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get(udid).copied() else {
            anyhow::bail!(
                "interaction session requires a stop_owned_stream reservation for {udid}"
            );
        };
        if current.generation != generation || current.phase != InteractionLifecyclePhase::Stopped {
            anyhow::bail!(
                "interaction session requires the current stop_owned_stream reservation for {udid}"
            );
        }

        state.next_token = state
            .next_token
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("interaction lifecycle token space exhausted"))?;
        let token = state.next_token;
        state.devices.insert(
            udid.to_string(),
            InteractionLifecycleState {
                generation,
                phase: InteractionLifecyclePhase::SessionStarting { token, kind },
            },
        );
        Ok(InteractionSessionReservation {
            udid: udid.to_string(),
            generation,
            token,
            kind,
        })
    }

    pub fn complete_session(
        &self,
        reservation: &InteractionSessionReservation,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get_mut(&reservation.udid) else {
            anyhow::bail!("interaction session reservation is no longer active");
        };
        let expected = InteractionLifecyclePhase::SessionStarting {
            token: reservation.token,
            kind: reservation.kind,
        };
        if current.generation != reservation.generation || current.phase != expected {
            anyhow::bail!("interaction session reservation is stale");
        }
        current.phase = InteractionLifecyclePhase::SessionReady {
            token: reservation.token,
            kind: reservation.kind,
        };
        Ok(())
    }

    pub fn reserve_stream(
        &self,
        udid: &str,
        generation: u64,
    ) -> anyhow::Result<InteractionStreamReservation> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get_mut(udid) else {
            anyhow::bail!("interaction stream requires an approved session reservation");
        };
        let InteractionLifecyclePhase::SessionReady { token, kind } = current.phase else {
            anyhow::bail!("interaction stream requires an approved session reservation");
        };
        if current.generation != generation {
            anyhow::bail!("interaction stream session reservation has a stale generation");
        }
        current.phase = InteractionLifecyclePhase::StreamStarting { token, kind };
        Ok(InteractionStreamReservation {
            udid: udid.to_string(),
            generation,
            token,
            kind,
        })
    }

    pub fn complete_stream(
        &self,
        reservation: &InteractionStreamReservation,
    ) -> anyhow::Result<()> {
        let mut state = self.state.lock();
        let Some(current) = state.devices.get_mut(&reservation.udid) else {
            anyhow::bail!("interaction stream reservation is no longer active");
        };
        let expected = InteractionLifecyclePhase::StreamStarting {
            token: reservation.token,
            kind: reservation.kind,
        };
        if current.generation != reservation.generation || current.phase != expected {
            anyhow::bail!("interaction stream reservation is stale");
        }
        current.phase = InteractionLifecyclePhase::Streaming {
            token: reservation.token,
            kind: reservation.kind,
        };
        Ok(())
    }

    pub fn clear(&self, udid: &str) {
        self.state.lock().devices.remove(udid);
    }

    /// Whether a session reservation for `udid` has been approved and not cleared.
    ///
    /// `pub` rather than `#[cfg(test)]`: the registry now lives in another crate
    /// from the drivers that use it, so a test-only method would be invisible to
    /// exactly the tests that need it.
    pub fn has_session_reservation(&self, udid: &str) -> bool {
        self.state.lock().devices.get(udid).is_some_and(|state| {
            matches!(
                state.phase,
                InteractionLifecyclePhase::SessionReady { .. }
                    | InteractionLifecyclePhase::StreamStarting { .. }
                    | InteractionLifecyclePhase::Streaming { .. }
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_lifecycle_requires_stop_then_session_then_stream() {
        let lifecycle = InteractionLifecycleRegistry::default();

        assert!(lifecycle
            .begin_session("fixture", 1, InteractionSessionKind::Ordinary)
            .unwrap_err()
            .to_string()
            .contains("stop_owned_stream"));

        lifecycle.record_stopped("fixture", 1);
        let session = lifecycle
            .begin_session("fixture", 1, InteractionSessionKind::Ordinary)
            .expect("session reservation");
        assert!(lifecycle.reserve_stream("fixture", 1).is_err());
        lifecycle
            .complete_session(&session)
            .expect("approved session");
        let stream = lifecycle
            .reserve_stream("fixture", 1)
            .expect("stream reservation");
        assert_eq!(stream.generation(), 1);
        lifecycle.complete_stream(&stream).expect("running stream");
    }

    #[test]
    fn interaction_lifecycle_rejects_stale_generation_and_clears_failed_transition() {
        let lifecycle = InteractionLifecycleRegistry::default();
        lifecycle.record_stopped("fixture", 4);
        let session = lifecycle
            .begin_session("fixture", 4, InteractionSessionKind::FreshText)
            .expect("session reservation");
        lifecycle
            .complete_session(&session)
            .expect("approved session");

        assert!(lifecycle.reserve_stream("fixture", 5).is_err());
        assert!(lifecycle.has_session_reservation("fixture"));
        lifecycle.clear("fixture");
        assert!(!lifecycle.has_session_reservation("fixture"));
        assert!(lifecycle.reserve_stream("fixture", 4).is_err());
    }

    #[test]
    fn a_second_confirm_at_the_same_generation_is_accepted() {
        // The plane re-confirms the handoff when a stream start fails
        // (`device_control.rs` start_reserved_stream error path), so recording a
        // stop twice must not wedge the device.
        let lifecycle = InteractionLifecycleRegistry::default();
        lifecycle.record_stopped("fixture", 7);
        lifecycle.record_stopped("fixture", 7);
        let session = lifecycle
            .begin_session("fixture", 7, InteractionSessionKind::Ordinary)
            .expect("session reservation after a repeated confirm");
        lifecycle
            .complete_session(&session)
            .expect("approved session");
        assert_eq!(
            lifecycle
                .reserve_stream("fixture", 7)
                .expect("stream reservation")
                .generation(),
            7
        );
    }

    #[test]
    fn the_session_kind_survives_the_reservation() {
        // A backend may treat both kinds identically, but the choice must stay
        // readable rather than being silently dropped at the seam.
        let lifecycle = InteractionLifecycleRegistry::default();
        lifecycle.record_stopped("fixture", 2);
        let session = lifecycle
            .begin_session("fixture", 2, InteractionSessionKind::FreshText)
            .expect("session reservation");
        assert_eq!(session.kind(), InteractionSessionKind::FreshText);
        assert_eq!(session.generation(), 2);
    }
}
