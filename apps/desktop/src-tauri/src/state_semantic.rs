//! Caller-bound borrowers of the existing control plane, isolated from the desktop overlay.
use super::*;
use std::time::{Duration, Instant};

const IDLE: Duration = Duration::from_secs(60);
pub(super) struct SemanticOwner {
    caller: String,
    token: String,
    context: Option<Arc<UiSessionContext>>,
    control: Arc<DeviceControlPlane>,
    activity: parking_lot::Mutex<(Instant, usize)>,
    cancelled: tokio::sync::watch::Sender<bool>,
}
impl Drop for SemanticOwner {
    fn drop(&mut self) {
        if let Some(context) = self.context.take().and_then(Arc::into_inner) {
            if let Err(error) = self.control.close_manual_session(context) {
                log::warn!("semantic session release failed: {error}");
            }
        }
    }
}
pub struct SemanticHold {
    owner: Arc<SemanticOwner>,
    session: Arc<dyn UiSession>,
}
impl SemanticHold {
    pub fn session(&self) -> Arc<dyn UiSession> {
        Arc::clone(&self.session)
    }
    pub fn cancellation(&self) -> tokio::sync::watch::Receiver<bool> {
        self.owner.cancelled.subscribe()
    }
}
impl Drop for SemanticHold {
    fn drop(&mut self) {
        let mut activity = self.owner.activity.lock();
        activity.0 = Instant::now();
        activity.1 = activity.1.saturating_sub(1);
    }
}
fn denied() -> CommandError {
    CommandError::code(
        "InspectorSessionUnavailable",
        "Phiên Inspector đã hết hạn hoặc thiết bị đang được điều khiển bởi phiên khác.",
    )
}
impl AppState {
    pub async fn begin_semantic_session(
        &self,
        udid: &str,
        caller: &str,
        timeout_ms: u64,
    ) -> Result<String, CommandError> {
        if udid.is_empty() || caller.is_empty() || timeout_ms == 0 || timeout_ms > 120_000 {
            return Err(denied());
        }
        let gate = {
            let mut gates = self.overlay_gates.lock().await;
            Arc::clone(
                gates
                    .entry(udid.into())
                    .or_insert_with(|| Arc::new(AsyncMutex::new(()))),
            )
        };
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let opening = tokio::time::timeout_at(deadline, gate.clone().lock_owned())
            .await
            .map_err(|_| {
                CommandError::code(
                    "InspectorAdmissionTimeout",
                    "Hết thời gian chờ quyền mở phiên thiết bị; chưa mở phiên mới.",
                )
            })?;
        if self.overlay_sessions.lock().await.contains_key(udid)
            || self.semantic_sessions.lock().contains_key(udid)
        {
            return Err(denied());
        }
        let control = Arc::clone(&self.control);
        let owners = Arc::clone(&self.semantic_sessions);
        let udid = udid.to_owned();
        let caller = caller.to_owned();
        let admission = self.ensure_accepting_work()?;
        let (send, receive) = tokio::sync::oneshot::channel();
        // The owner drains an admitted open even if its HTTP caller disappears.
        tauri::async_runtime::spawn(async move {
            let admission = admission;
            let context = match control
                .open_manual_session(&udid, DeviceWorkOwner::ManualControl)
                .await
            {
                Ok(context) => context,
                Err(error) => {
                    let _ = send.send(Err(CommandError::from(error)));
                    return;
                }
            };
            if send.is_closed() || tokio::time::Instant::now() >= deadline {
                let _ = control.close_manual_session(context);
                return;
            }
            let token = uuid::Uuid::new_v4().to_string();
            let owner = Arc::new(SemanticOwner {
                caller,
                token: token.clone(),
                context: Some(Arc::new(context)),
                control: Arc::clone(&control),
                activity: parking_lot::Mutex::new((Instant::now(), 0)),
                cancelled: tokio::sync::watch::channel(false).0,
            });
            owners.lock().insert(udid.clone(), Arc::clone(&owner));
            if send.send(Ok(token.clone())).is_err() {
                owners.lock().remove(&udid);
            }
            drop(opening);
            drop(owner);
            drop(admission);
            loop {
                tokio::time::sleep(IDLE).await;
                let _gate = gate.lock().await;
                let expired = {
                    let map = owners.lock();
                    let Some(owner) = map.get(&udid) else {
                        break;
                    };
                    if owner.token != token {
                        break;
                    }
                    let activity = owner.activity.lock();
                    activity.1 == 0 && activity.0.elapsed() >= IDLE
                };
                if expired {
                    owners.lock().remove(&udid);
                    break;
                }
            }
        });
        tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| CommandError::code("InspectorStartupTimeout", "Chuẩn bị thiết bị vượt thời gian chờ. Tiến trình mở phiên đang hoàn tất và thu hồi; hãy kiểm tra lại khi thiết bị sẵn sàng."))?
            .map_err(|_| CommandError::code("InspectorStartupInterrupted", "Tiến trình chuẩn bị thiết bị bị gián đoạn; chưa xác nhận mở phiên thành công."))?
    }
    pub async fn hold_semantic_session(
        &self,
        udid: &str,
        caller: &str,
        token: &str,
    ) -> Result<SemanticHold, CommandError> {
        let map = self.semantic_sessions.lock();
        let owner = map.get(udid).ok_or_else(denied)?;
        if owner.caller != caller || owner.token != token {
            return Err(denied());
        }
        let mut activity = owner.activity.lock();
        if activity.1 == 0 && activity.0.elapsed() >= IDLE {
            return Err(denied());
        }
        let session = self
            .control
            .session(owner.context.as_deref().ok_or_else(denied)?)
            .map_err(CommandError::from)?;
        activity.1 += 1;
        activity.0 = Instant::now();
        Ok(SemanticHold {
            owner: Arc::clone(owner),
            session,
        })
    }
    pub async fn end_semantic_session(
        &self,
        udid: &str,
        caller: &str,
        token: &str,
    ) -> Result<(), CommandError> {
        let mut map = self.semantic_sessions.lock();
        let Some(owner) = map.get(udid) else {
            return Ok(());
        };
        if owner.caller != caller || owner.token != token {
            return Err(denied());
        }
        owner.cancelled.send_replace(true);
        map.remove(udid);
        Ok(())
    }
    pub(super) fn release_semantic_owner(&self, udid: &str) {
        // Existing borrowers keep their context/lease until the admitted operation drains.
        if let Some(owner) = self.semantic_sessions.lock().remove(udid) {
            owner.cancelled.send_replace(true);
        }
    }
}
