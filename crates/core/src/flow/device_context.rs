use std::sync::Arc;

use crate::device_control::FailedStreamStartContext;

use crate::{
    AppProcessState, ContextReleaseProof, DeviceControlError, DeviceControlPlane,
    DeviceExclusiveContext, DeviceWorkOwner, InteractionSessionKind, ProcessAbsenceProof,
    UiCapacityReservation, UiSession, UiSessionContext, UiWithStreamContext,
};

pub(crate) enum FlowDeviceContext {
    NoDeviceResources { udid: String },
    Exclusive(DeviceExclusiveContext),
    Session(UiSessionContext),
    Streaming(UiWithStreamContext),
    Closed,
}

pub(crate) struct FlowDeviceUpgradeFailure {
    pub(crate) error: DeviceControlError,
    pub(crate) failed_stream: Option<FailedStreamStartContext>,
}

impl FlowDeviceUpgradeFailure {
    pub(crate) fn reconciliation_session(
        &self,
        control: &DeviceControlPlane,
    ) -> Result<Option<Arc<dyn UiSession>>, DeviceControlError> {
        self.failed_stream
            .as_ref()
            .map(|context| control.failed_stream_session(context))
            .transpose()
    }

    pub(crate) async fn release_failed_stream(
        &mut self,
        control: &DeviceControlPlane,
    ) -> Result<Option<ContextReleaseProof>, DeviceControlError> {
        match self.failed_stream.take() {
            Some(context) => control.close_failed_stream_start(context).await.map(Some),
            None => Ok(None),
        }
    }
}

impl FlowDeviceContext {
    pub(crate) fn level(&self) -> u8 {
        match self {
            Self::NoDeviceResources { .. } => 0,
            Self::Exclusive(_) => 1,
            Self::Session(_) => 2,
            Self::Streaming(_) => 3,
            Self::Closed => 4,
        }
    }

    pub(crate) fn no_device_resources(udid: impl Into<String>) -> Self {
        Self::NoDeviceResources { udid: udid.into() }
    }

    pub(crate) fn exclusive(&self) -> Result<&DeviceExclusiveContext, DeviceControlError> {
        match self {
            Self::Exclusive(context) => Ok(context),
            _ => Err(DeviceControlError::InvalidContext {
                reason: "Flow context is no longer exclusive",
            }),
        }
    }

    pub(crate) fn session(
        &self,
        control: &DeviceControlPlane,
    ) -> Result<Arc<dyn UiSession>, DeviceControlError> {
        match self {
            Self::Session(context) => control.session(context),
            Self::Streaming(context) => control.streaming_session(context),
            _ => Err(DeviceControlError::InvalidContext {
                reason: "Flow context has no UI session",
            }),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        match self {
            Self::Streaming(context) => context.stream_proof().generation,
            _ => 0,
        }
    }

    pub(crate) async fn upgrade_session(
        &mut self,
        control: &DeviceControlPlane,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<(), DeviceControlError> {
        let Self::Exclusive(context) = std::mem::replace(self, Self::Closed) else {
            return Err(DeviceControlError::InvalidContext {
                reason: "Flow session upgrade requires an exclusive context",
            });
        };
        let session = match control
            .try_foreground_target_app_and_start_interaction_session(context, bundle_id, kind)
            .await
        {
            Ok((session, _proof)) => session,
            Err(failure) => {
                *self = Self::Exclusive(failure.context);
                return Err(failure.error);
            }
        };
        *self = Self::Session(session);
        Ok(())
    }

    pub(crate) async fn upgrade_existing_session(
        &mut self,
        control: &DeviceControlPlane,
        bundle_id: &str,
        kind: InteractionSessionKind,
    ) -> Result<(), DeviceControlError> {
        let Self::Exclusive(context) = std::mem::replace(self, Self::Closed) else {
            return Err(DeviceControlError::InvalidContext {
                reason: "Flow session attach requires an exclusive context",
            });
        };
        let session = match control
            .try_start_interaction_session(context, bundle_id, kind)
            .await
        {
            Ok(session) => session,
            Err(failure) => {
                *self = Self::Exclusive(failure.context);
                return Err(failure.error);
            }
        };
        *self = Self::Session(session);
        Ok(())
    }

    pub(crate) async fn reserve_capacity(
        &mut self,
        control: &DeviceControlPlane,
    ) -> Result<UiCapacityReservation, FlowDeviceUpgradeFailure> {
        let Self::Exclusive(context) = std::mem::replace(self, Self::Closed) else {
            return Err(FlowDeviceUpgradeFailure {
                error: DeviceControlError::InvalidContext {
                    reason: "Flow capacity reservation requires an exclusive context",
                },
                failed_stream: None,
            });
        };
        match control.try_reserve_ui_capacity(context).await {
            Ok((context, reservation)) => {
                *self = Self::Exclusive(context);
                Ok(reservation)
            }
            Err(failure) => {
                if let Some(context) = failure.context {
                    *self = Self::Exclusive(context);
                }
                Err(FlowDeviceUpgradeFailure {
                    error: failure.error,
                    failed_stream: None,
                })
            }
        }
    }

    pub(crate) async fn upgrade_stream(
        &mut self,
        control: &DeviceControlPlane,
        reservation: crate::UiCapacityReservation,
    ) -> Result<(), FlowDeviceUpgradeFailure> {
        let Self::Session(context) = std::mem::replace(self, Self::Closed) else {
            return Err(FlowDeviceUpgradeFailure {
                error: DeviceControlError::InvalidContext {
                    reason: "Flow stream upgrade requires a session context",
                },
                failed_stream: None,
            });
        };
        let streaming = match control
            .try_start_reserved_stream(context, reservation)
            .await
        {
            Ok(streaming) => streaming,
            Err(failure) => {
                if let Some(context) = failure.context {
                    *self = Self::Session(context);
                }
                return Err(FlowDeviceUpgradeFailure {
                    error: failure.error,
                    failed_stream: failure.failed_start,
                });
            }
        };
        *self = Self::Streaming(streaming);
        Ok(())
    }

    pub(crate) async fn active_app_bundle(
        &self,
        control: &DeviceControlPlane,
    ) -> Result<String, DeviceControlError> {
        match self {
            Self::Exclusive(context) => control.read_active_app_bundle(context).await,
            Self::Session(context) => {
                let udid = context.udid().to_string();
                control
                    .session(context)?
                    .active_app_bundle()
                    .await
                    .map_err(|error| DeviceControlError::Driver {
                        udid,
                        operation: "readActiveAppBundle",
                        message: error.to_string(),
                    })
            }
            Self::Streaming(context) => {
                let udid = context.udid().to_string();
                control
                    .streaming_session(context)?
                    .active_app_bundle()
                    .await
                    .map_err(|error| DeviceControlError::Driver {
                        udid,
                        operation: "readActiveAppBundle",
                        message: error.to_string(),
                    })
            }
            Self::NoDeviceResources { .. } | Self::Closed => {
                Err(DeviceControlError::InvalidContext {
                    reason: "Flow context has no active-app channel",
                })
            }
        }
    }

    pub(crate) async fn foreground_app(
        &self,
        control: &DeviceControlPlane,
        bundle_id: &str,
    ) -> Result<(), DeviceControlError> {
        match self {
            Self::Exclusive(context) => {
                control.foreground_target_app(context, bundle_id).await?;
            }
            Self::Session(context) => {
                control.foreground_session_app(context, bundle_id).await?;
            }
            Self::Streaming(context) => {
                control.foreground_streaming_app(context, bundle_id).await?;
            }
            Self::NoDeviceResources { .. } | Self::Closed => {
                return Err(DeviceControlError::InvalidContext {
                    reason: "Flow context is closed",
                });
            }
        }
        Ok(())
    }

    pub(crate) async fn terminate_app(
        &self,
        control: &DeviceControlPlane,
        bundle_id: &str,
    ) -> Result<ProcessAbsenceProof, DeviceControlError> {
        match self {
            Self::Exclusive(context) => control.terminate_app(context, bundle_id).await,
            Self::Session(context) => control.terminate_session_app(context, bundle_id).await,
            Self::Streaming(context) => control.terminate_streaming_app(context, bundle_id).await,
            Self::NoDeviceResources { .. } | Self::Closed => {
                Err(DeviceControlError::InvalidContext {
                    reason: "Flow context has no device-control channel",
                })
            }
        }
    }

    pub(crate) async fn inspect_process(
        &self,
        control: &DeviceControlPlane,
        bundle_id: &str,
    ) -> Result<AppProcessState, DeviceControlError> {
        match self {
            Self::Exclusive(context) => control.inspect_app_process(context, bundle_id).await,
            Self::Session(context) => {
                control
                    .inspect_session_app_process(context, bundle_id)
                    .await
            }
            Self::Streaming(context) => {
                control
                    .inspect_streaming_app_process(context, bundle_id)
                    .await
            }
            Self::NoDeviceResources { .. } | Self::Closed => {
                Err(DeviceControlError::InvalidContext {
                    reason: "Flow context has no device-control channel",
                })
            }
        }
    }

    pub(crate) async fn close(
        &mut self,
        control: &DeviceControlPlane,
    ) -> Result<ContextReleaseProof, DeviceControlError> {
        match std::mem::replace(self, Self::Closed) {
            Self::NoDeviceResources { udid } => Ok(ContextReleaseProof {
                udid,
                owner: DeviceWorkOwner::Script,
                had_session: false,
                had_stream: false,
            }),
            Self::Exclusive(context) => control.close_exclusive_context(context),
            Self::Session(context) => control.close_session_context(context),
            Self::Streaming(context) => control.close_ui_context(context).await.map(Into::into),
            Self::Closed => Err(DeviceControlError::InvalidContext {
                reason: "Flow context was already closed",
            }),
        }
    }
}

#[cfg(test)]
mod take_and_restore_tests {
    /// **Taking the context out and not putting anything back releases the device, silently.**
    ///
    /// Every upgrade here starts with `std::mem::replace(self, Self::Closed)` so the owned context
    /// can be handed to the control plane by value. If the upgrade fails and the error arm forgets
    /// `*self = Self::Exclusive(failure.context)`, that context is **dropped** -- and
    /// `DeviceExclusiveContext` releases its lease on `Drop`, with no release method to grep for.
    /// The device goes back to the pool mid-run while `self` reads `Closed`, so nothing downstream
    /// can tell "the upgrade failed" from "this never held the device".
    ///
    /// Nothing else catches it: this file has no behavioural tests of its own, and every runtime
    /// test drives a mock driver that succeeds, so not one of them reaches an error arm. A source
    /// scan is the honest instrument -- the invariant is structural, and the cost of breaking it is
    /// invisible at runtime.
    #[test]
    fn every_context_take_puts_something_back() {
        let source = include_str!("device_context.rs").replace("\r\n", "\n");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(before, _)| before)
            .unwrap_or(&source);
        // **Comments are not code, and this gate learnt that the expensive way.** A review
        // pointed out that `// *self = Self::Exclusive(failure.context);` satisfied the
        // restore check while the arm below it dropped the context -- the same
        // comment-counts-as-code shape `code_of()` strips for the scanners in `lib.rs`. The
        // doc comments in this very file quote the shapes being hunted, so scanning them is
        // scanning the description instead of the thing described. `://` is spared so a URL
        // in a string is not mistaken for the start of a comment.
        let production: String = production
            .lines()
            .map(|line| {
                let mut search_from = 0;
                loop {
                    let Some(pos) = line[search_from..].find("//") else {
                        return line;
                    };
                    let at = search_from + pos;
                    if at > 0 && line.as_bytes()[at - 1] == b':' {
                        search_from = at + 2;
                        continue;
                    }
                    return &line[..at];
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let production = production.as_str();

        // Split into `impl`-level functions: a line starting with exactly four spaces and a
        // `fn`/`pub(crate) fn`/`pub(crate) async fn` declaration begins a new one.
        let mut blocks: Vec<(String, String)> = Vec::new();
        for line in production.lines() {
            let is_declaration = line.starts_with("    ")
                && !line.starts_with("     ")
                && (line.trim_start().starts_with("fn ")
                    || line.trim_start().starts_with("pub fn ")
                    || line.trim_start().starts_with("pub async fn ")
                    || line.trim_start().starts_with("pub(crate) fn ")
                    || line.trim_start().starts_with("pub(crate) async fn "));
            if is_declaration {
                let name = line
                    .trim_start()
                    .rsplit_once("fn ")
                    .map(|(_, rest)| rest)
                    .unwrap_or(line)
                    .split(['(', '<', ' '])
                    .next()
                    .unwrap_or("")
                    .to_string();
                blocks.push((name, String::new()));
            }
            if let Some(last) = blocks.last_mut() {
                last.1.push_str(line);
                last.1.push('\n');
            }
        }

        let takers: Vec<&(String, String)> = blocks
            .iter()
            .filter(|(_, body)| body.contains("mem::replace(self, Self::Closed)"))
            .collect();

        // A parser that found nothing passes every assertion below it. Four functions took the
        // context when this was written, plus `close`, which consumes it on purpose.
        assert!(
            takers.len() >= 5,
            "only {} functions take the context; the block parser has stopped reading this file \
             (blocks seen: {})",
            takers.len(),
            blocks.len()
        );

        /// The body of every `Err(<binding>) => { .. }` arm, by matching its braces.
        ///
        /// Asking whether the *function* restores anywhere is not enough, and that is not a
        /// hypothetical: the first version of this gate stayed green with `upgrade_session`'s
        /// error-arm restore deleted, because the success path's `*self = Self::Session(session)`
        /// satisfied it. The arm is what has to be read.
        ///
        /// And the arm has to be found however its binding is spelt. A review constructed the
        /// blind spot the first matcher had: it looked for the literal `Err(failure) => {`, so
        /// renaming one binding — `Err(refused)`, `Err(e)`, even `Err(_)`, which drops the
        /// context by construction — made that arm invisible, and the only thing left catching
        /// it was an exact-count floor sitting at exactly the current count.
        ///
        /// **An arm this parser cannot read is reported, not skipped**, which is the third
        /// bypass a review built: `Err(failure) if failure.context.is_none() => {` has a guard
        /// between the binding and the arrow, so the old matcher walked past it. Added *beside*
        /// a conforming arm it changed no count and the gate stayed green while the new arm
        /// leaked. Anything arm-shaped that does not parse now fails the test by name, so a
        /// shape this gate has never seen can never be silently absent.
        fn error_arms(body: &str) -> (Vec<String>, Vec<String>) {
            fn is_binding(pattern: &str) -> bool {
                let pattern = pattern.trim().trim_start_matches("ref ").trim();
                !pattern.is_empty()
                    && pattern
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            let mut arms = Vec::new();
            let mut unreadable = Vec::new();
            let mut rest = body;
            while let Some(at) = rest.find("Err(") {
                let after_paren = &rest[at + "Err(".len()..];
                let Some(close) = after_paren.find(')') else {
                    break;
                };
                let binding = &after_paren[..close];
                let tail = &after_paren[close + 1..];
                let trimmed = tail.trim_start();
                // `Err(failure.error)` and `Err(SomeType { .. })` are values, not arms; and a
                // bare `Err(failure)` in a `return` is a value too — only an arrow or a guard
                // after the binding makes this a match arm.
                let is_arm = is_binding(binding)
                    && (trimmed.starts_with("=>") || trimmed.starts_with("if "));
                if !is_arm {
                    rest = &rest[at + "Err(".len()..];
                    continue;
                }
                // Whitespace-tolerant on purpose: `cargo fmt` is free to wrap between the
                // binding and the brace, and a gate that a reformat can blind is one of the
                // five bypass shapes this repo has already paid for.
                if !trimmed.starts_with("=> {") {
                    unreadable.push(format!(
                        "Err({binding}){}",
                        trimmed.chars().take(48).collect::<String>()
                    ));
                    rest = &rest[at + "Err(".len()..];
                    continue;
                }
                let Some(brace_in_tail) = tail.find('{') else {
                    break;
                };
                let open_offset = at + "Err(".len() + close + 1 + brace_in_tail;
                let mut depth = 0usize;
                let mut end = None;
                for (offset, character) in rest[open_offset..].char_indices() {
                    match character {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(open_offset + offset + 1);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let Some(end) = end else { break };
                arms.push(rest[open_offset..end].to_string());
                rest = &rest[end..];
            }
            (arms, unreadable)
        }

        /// Whether an arm puts a context back, rather than merely assigning *some* variant.
        ///
        /// **`Self::Closed` is not a restore, and it was the likeliest accident.** The old
        /// check asked for the substring `*self = Self::`, which `*self = Self::Closed;`
        /// satisfies — while restoring nothing and releasing the lease exactly as a missing
        /// arm would. `Closed` is not an exotic spelling either: it is the value
        /// `mem::replace(self, Self::Closed)` writes four lines above every one of these
        /// arms, so an edit that "kept the assignment" would have reached for it first. Only
        /// the three variants that carry a context count.
        fn restores_a_context(arm: &str) -> bool {
            ["Exclusive(", "Session(", "Streaming("]
                .iter()
                .any(|variant| arm.contains(&format!("*self = Self::{variant}")))
        }

        let mut leaking = Vec::new();
        let mut arms_seen = 0usize;
        // The restore comes in exactly two shapes, and the gate reads them apart rather than
        // blind-counting `*self = Self::`. The unconditional shape holds a failure whose
        // context is not optional; the conditional shape (`if let Some(context) =
        // <binding>.context`) belongs to the two capacity/stream upgrades whose failure may
        // arrive with the context already consumed — its `None` branch legitimately restores
        // nothing, because there is nothing left to restore. A conditional restore appearing
        // where an unconditional one stood would still satisfy a blind count while quietly
        // making the restore optional; these two floors are what notice.
        let mut unconditional_restores = 0usize;
        let mut conditional_restores = 0usize;
        let mut unreadable_arms: Vec<String> = Vec::new();
        for (name, body) in takers {
            // `close` is the one place `Closed` is the intended end state.
            if name == "close" {
                continue;
            }
            let (arms, unreadable) = error_arms(body);
            assert!(
                !arms.is_empty(),
                "{name} takes the context but has no `Err(..)` arm; either the control-plane \
                 call stopped being fallible or this parser has stopped finding the arm"
            );
            unreadable_arms.extend(unreadable.into_iter().map(|arm| format!("{name}: {arm}")));
            arms_seen += arms.len();
            for arm in arms {
                if !restores_a_context(&arm) {
                    leaking.push(name.clone());
                } else if arm.contains("if let Some(") && arm.contains(".context") {
                    conditional_restores += 1;
                } else {
                    unconditional_restores += 1;
                }
            }
        }
        // **An arm shape this parser cannot read fails the test rather than lowering a
        // count.** The floor below still catches a parser that has gone entirely blind; this
        // catches the narrower and more likely case — one new arm, written in a shape the
        // matcher does not know — which used to pass by leaving the old arms' count intact.
        assert!(
            unreadable_arms.is_empty(),
            "these look like `Err(..)` arms but this gate could not read them, so it cannot \
             say whether they restore anything — teach the matcher the shape or write the arm \
             the way the others are written: {}",
            unreadable_arms.join(", ")
        );
        assert!(
            arms_seen >= 4,
            "only {arms_seen} error arms were read; the parser is not seeing this file"
        );
        assert!(
            unconditional_restores >= 2,
            "only {unconditional_restores} unconditional restores were read; the session \
             upgrades' failures carry a context that is not optional, and restoring it must \
             not have become conditional"
        );
        assert!(
            conditional_restores >= 2,
            "only {conditional_restores} conditional restores were read; the capacity and \
             stream upgrades restore through `if let Some(context) = failure.context`, and \
             this gate has to keep seeing that shape as itself"
        );
        assert!(
            leaking.is_empty(),
            "these take the context out and their failure arm puts nothing back, so a failed \
             upgrade releases the device mid-run: {}",
            leaking.join(", ")
        );
    }
}
