//! Semantic Inspector is a borrower of the existing manual owner, never a controller.
use crate::{command_error::CommandError, state::AppState};
use base64::{engine::general_purpose::STANDARD, Engine};
use riviu_core::{
    ui_automation::{
        runtime::{resolve_navigation, wait_for_observation},
        *,
    },
    HardwareKey, SwipeGesture, TapPoint, UiSession,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const TTL: Duration = Duration::from_secs(60);
type Slot = Arc<Mutex<References>>;
static REFERENCES: LazyLock<Mutex<HashMap<String, Slot>>> = LazyLock::new(Mutex::default);

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Begin,
    End,
    Observe,
    Tap,
    Type,
    Swipe,
    Press,
    WaitFor,
    Expect,
    Screenshot,
}

/// No coordinates, caller identity, or externally supplied observation binding are accepted.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticRequest {
    pub operation: Operation,
    pub udid: String,
    pub session_token: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub query: SemanticLocator,
    pub scope: Option<ObservationScope>,
    #[serde(default)]
    pub fields: ObservationFieldMask,
    #[serde(rename = "ref")]
    pub reference: Option<String>,
    pub navigation_target: Option<String>,
    pub text: Option<String>,
    pub direction: Option<Direction>,
    pub key: Option<PressKey>,
    pub expected: Option<ObservationExpectation>,
}
fn default_timeout() -> u64 {
    10_000
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressKey {
    Back,
    Home,
}

#[derive(Clone)]
struct Reference {
    request: ObservationRequest,
    node: SemanticNode,
    observation: UiObservation,
}
struct References {
    udid: String,
    caller: String,
    expires: Instant,
    last_used: Instant,
    entries: HashMap<String, Reference>,
}

fn error(code: &str) -> CommandError {
    CommandError::code(
        code,
        "Không thể xác nhận thao tác Inspector; hãy đọc lại trạng thái.",
    )
}
fn driver_error(cause: anyhow::Error) -> CommandError {
    // Transport errors may include typed text. Never expose them at this boundary.
    if cause
        .downcast_ref::<riviu_core::driver::UnsupportedCapability>()
        .is_some()
    {
        error("InspectorUnsupported")
    } else {
        error("InspectorReadFailed")
    }
}

/// Called by the existing owner on end, handoff, session replacement, and legacy mutation.
pub async fn invalidate(udid: &str) {
    let slots: Vec<_> = REFERENCES.lock().await.values().cloned().collect();
    for slot in slots {
        let mut refs = slot.lock().await;
        if refs.udid == udid {
            refs.entries.clear();
        }
    }
}

pub async fn execute(
    state: &AppState,
    caller: &str,
    request: SemanticRequest,
) -> Result<Value, CommandError> {
    let _admission = state.ensure_accepting_work()?;
    if request.udid.is_empty() || request.timeout_ms == 0 || request.timeout_ms > 120_000 {
        return Err(error("InvalidArgument"));
    }
    let deadline = tokio::time::Instant::now() + Duration::from_millis(request.timeout_ms);
    if matches!(request.operation, Operation::Begin) {
        // The owner must bound open and clean up a cancelled/failed begin itself.
        let token = state
            .begin_semantic_session(&request.udid, caller, request.timeout_ms)
            .await?;
        let slot = Arc::new(Mutex::new(References {
            udid: request.udid,
            caller: caller.into(),
            expires: Instant::now() + TTL,
            last_used: Instant::now(),
            entries: HashMap::new(),
        }));
        REFERENCES
            .lock()
            .await
            .insert(token.clone(), Arc::clone(&slot));
        let cleanup_token = token.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(TTL).await;
                // An executing request holds this lock, so cleanup waits for its drain.
                let mut refs = slot.lock().await;
                if Instant::now() >= refs.expires {
                    refs.entries.clear();
                }
                if refs.last_used.elapsed() >= TTL {
                    REFERENCES.lock().await.remove(&cleanup_token);
                    break;
                }
                if !REFERENCES.lock().await.contains_key(&cleanup_token) {
                    break;
                }
            }
        });
        return Ok(json!({"status":"begun", "sessionToken":token, "idleTimeoutMs":60000}));
    }
    let token = request
        .session_token
        .as_deref()
        .ok_or_else(|| error("InspectorSessionRequired"))?;
    let slot = REFERENCES
        .lock()
        .await
        .get(token)
        .cloned()
        .ok_or_else(|| error("InspectorSessionExpired"))?;
    if matches!(request.operation, Operation::End) {
        // Revoke through the caller-bound owner before waiting on a read's lock.
        // Its borrowers retain the lease until any dispatched primitive drains.
        state
            .end_semantic_session(&request.udid, caller, token)
            .await?;
    }
    let mut refs = tokio::time::timeout_at(deadline, slot.lock())
        .await
        .map_err(|_| error("InspectorDeadlineExceeded"))?;
    if refs.caller != caller || refs.udid != request.udid {
        return Err(error("InspectorOwnerMismatch"));
    }
    if matches!(request.operation, Operation::End) {
        refs.entries.clear();
        drop(refs);
        REFERENCES.lock().await.remove(token);
        return Ok(json!({"status":"ended"}));
    }
    if Instant::now() >= refs.expires {
        refs.entries.clear();
    }
    // Owner validates token, idle expiry, and live context; the hold protects graceful close.
    let hold = state
        .hold_semantic_session(&request.udid, caller, token)
        .await?;
    let session = hold.session();
    let mut cancel = hold.cancellation();
    // Never cancel a dispatched effect and release its lease while the backend still acts.
    // Only reads are deadline-cancelled; transport timeout is an uncertain action outcome.
    let result = run(session.as_ref(), &request, &mut refs, deadline, &mut cancel).await;
    refs.last_used = Instant::now();
    if result.is_err() {
        refs.entries.clear();
    }
    result
}

fn observation_request(
    request: &SemanticRequest,
    deadline: tokio::time::Instant,
) -> ObservationRequest {
    ObservationRequest {
        query: request.query.clone(),
        scope: request.scope.clone(),
        fields: ObservationFieldMask::default(),
        remaining_ms: remaining(deadline),
    }
}
fn remaining(deadline: tokio::time::Instant) -> u64 {
    deadline
        .saturating_duration_since(tokio::time::Instant::now())
        .as_millis() as u64
}
fn validate_binding(
    observation: &UiObservation,
    udid: &str,
    session: &dyn UiSession,
) -> Result<(), CommandError> {
    if observation.device_id != udid
        || observation.session_epoch.is_empty()
        || observation.session_epoch != session.gui_session_epoch()
        || observation.observation_id.is_empty()
        || observation.generation == 0
        || observation.app.package.as_deref().is_none_or(str::is_empty)
    {
        return Err(error("InspectorStaleObservation"));
    }
    Ok(())
}

fn store_observation(
    refs: &mut References,
    mut observation: UiObservation,
    request: &ObservationRequest,
    fields: &ObservationFieldMask,
) -> Value {
    refs.entries.clear();
    refs.expires = Instant::now() + TTL;
    for node in &mut observation.matches {
        node.redact_sensitive();
    }
    let mut handles = HashMap::new();
    let mut identity = observation.clone();
    identity.matches.clear();
    for node in &observation.matches {
        let handle = uuid::Uuid::new_v4().to_string();
        let query = SemanticLocator {
            role: node.role.clone(),
            name: node.name.clone(),
            text: node.text.clone(),
            id: node.id.clone(),
            exact: true,
        };
        if query.role.is_some()
            || query.name.is_some()
            || query.text.is_some()
            || query.id.is_some()
        {
            let mut binding = request.clone();
            binding.query = query;
            let scope = binding.scope.get_or_insert_with(ObservationScope::default);
            scope.package = observation.app.package.clone();
            refs.entries.insert(
                handle.clone(),
                Reference {
                    request: binding,
                    node: node.clone(),
                    observation: identity.clone(),
                },
            );
            handles.insert(node.node_id.to_string(), handle);
        }
    }
    for node in &mut observation.matches {
        node.project(fields);
    }
    json!({"status":"observed", "observation":observation, "refs":handles, "refExpiryMs":60000})
}

async fn fresh_target(
    session: &dyn UiSession,
    reference: &Reference,
    deadline: tokio::time::Instant,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(UiObservation, SemanticNode), CommandError> {
    let mut request = reference.request.clone();
    request.remaining_ms = remaining(deadline);
    let observation = read_observation(session, &request, deadline, cancel).await?;
    validate_binding(&observation, &reference.observation.device_id, session)?;
    if observation.session_epoch != reference.observation.session_epoch
        || observation.app.package != reference.observation.app.package
        || observation.generation <= reference.observation.generation
        || observation.observation_id == reference.observation.observation_id
    {
        return Err(error("InspectorStaleRef"));
    }
    if observation.matches.len() != 1 || observation.unknown_match_count != 0 {
        return Err(error("InspectorAmbiguousTarget"));
    }
    let node = observation.matches[0].clone();
    if node.id != reference.node.id
        || node.role != reference.node.role
        || node.name != reference.node.name
        || node.text != reference.node.text
        || node.class_name != reference.node.class_name
        || node.package != reference.node.package
    {
        return Err(error("InspectorStaleRef"));
    }
    Ok((observation, node))
}

fn actionable(observation: &UiObservation, node: &SemanticNode) -> Result<Rect, CommandError> {
    let bounds = node
        .bounds
        .clone()
        .ok_or_else(|| error("InspectorNotActionable"))?;
    if node.enabled != Some(true)
        || node.visible != Some(true)
        || !bounds.valid(
            observation.app.width.unwrap_or(0),
            observation.app.height.unwrap_or(0),
        )
    {
        return Err(error("InspectorNotActionable"));
    }
    Ok(bounds)
}

async fn run(
    session: &dyn UiSession,
    request: &SemanticRequest,
    refs: &mut References,
    deadline: tokio::time::Instant,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Value, CommandError> {
    let read = observation_request(request, deadline);
    match request.operation {
        Operation::Observe => {
            refs.entries.clear();
            let observation = read_observation(session, &read, deadline, cancel).await?;
            validate_binding(&observation, &request.udid, session)?;
            Ok(store_observation(refs, observation, &read, &request.fields))
        }
        Operation::WaitFor | Operation::Expect => {
            refs.entries.clear();
            let expected = request
                .expected
                .as_ref()
                .ok_or_else(|| error("InvalidArgument"))?;
            if matches!(request.operation, Operation::Expect) {
                let observation = read_observation(session, &read, deadline, cancel).await?;
                validate_binding(&observation, &request.udid, session)?;
                let verdict = expect_observation(&observation, expected);
                let mut result = store_observation(refs, observation, &read, &request.fields);
                result["status"] = json!("checked");
                result["verdict"] = json!(verdict);
                return Ok(result);
            }
            let result = wait_for_observation(
                session,
                &read,
                expected,
                deadline,
                cancel,
                Duration::from_millis(150),
            )
            .await
            .map_err(driver_error)?;
            if let Some(observation) = &result.observation {
                validate_binding(observation, &request.udid, session)?;
            }
            // Wait output contains evidence but does not mint action refs.
            let mut result = result;
            if let Some(observation) = &mut result.observation {
                for node in &mut observation.matches {
                    node.project(&request.fields);
                }
            }
            Ok(json!(result))
        }
        Operation::Screenshot => {
            refs.entries.clear();
            let before = read_observation(session, &read, deadline, cancel).await?;
            validate_binding(&before, &request.udid, session)?;
            let png = cancellable_read(session.screenshot_png(), deadline, cancel).await?;
            if cancellable_read(session.active_app_bundle(), deadline, cancel).await?
                != before.app.package.clone().unwrap_or_default()
                || session.gui_session_epoch() != before.session_epoch
            {
                return Err(error("InspectorStaleObservation"));
            }
            Ok(
                json!({"status":"captured", "captureOrder":["semanticObservation","screenshot"], "atomic":false,
                "observationId":before.observation_id, "generation":before.generation, "sessionEpoch":before.session_epoch,
                "pngBase64":STANDARD.encode(png)}),
            )
        }
        Operation::Press => {
            refs.entries.clear();
            ensure_active(deadline, cancel)?;
            let key = match request.key.ok_or_else(|| error("InvalidArgument"))? {
                PressKey::Back => HardwareKey::Back,
                PressKey::Home => HardwareKey::Home,
            };
            session
                .press_hardware_key(key)
                .await
                .map_err(|_| error("InspectorActionUncertain"))?;
            Ok(json!({"status":"dispatched", "verified":false}))
        }
        Operation::Tap | Operation::Type | Operation::Swipe => {
            let handle = request
                .reference
                .as_deref()
                .ok_or_else(|| error("InspectorRefRequired"))?;
            let reference = refs
                .entries
                .get(handle)
                .cloned()
                .ok_or_else(|| error("InspectorStaleRef"))?;
            // Invalidate before dispatch, including uncertain and timeout outcomes.
            refs.entries.clear();
            let (observation, node) = fresh_target(session, &reference, deadline, cancel).await?;
            let bounds = actionable(&observation, &node)?;
            match request.operation {
                Operation::Tap => {
                    if node.clickable != Some(true) {
                        return Err(error("InspectorNotActionable"));
                    }
                    let target = request
                        .navigation_target
                        .as_deref()
                        .ok_or_else(|| error("InspectorEngineRequired"))?;
                    let approved = resolve_navigation(
                        session,
                        target,
                        Duration::from_millis(remaining(deadline)),
                    )
                    .await
                    .map_err(|_| error("InspectorEngineRequired"))?
                    .ok_or_else(|| error("InspectorEngineRequired"))?;
                    let mut latest = reference.clone();
                    latest.observation = observation;
                    let (fresh, node) = fresh_target(session, &latest, deadline, cancel).await?;
                    let bounds = actionable(&fresh, &node)?;
                    if node.clickable != Some(true) || bounds != Rect::from(&approved) {
                        return Err(error("InspectorStaleRef"));
                    }
                    ensure_active(deadline, cancel)?;
                    session
                        .tap(approved.centre())
                        .await
                        .map_err(|_| error("InspectorActionUncertain"))?;
                    Ok(json!({"status":"dispatched", "verified":false}))
                }
                Operation::Type => {
                    let text = request
                        .text
                        .as_deref()
                        .ok_or_else(|| error("InvalidArgument"))?;
                    // Never focus by coordinates or expose password input/readback in evidence.
                    if !session.supports_text_input()
                        || node.focused != Some(true)
                        || node.password != Some(false)
                        || node.role.as_deref() != Some("textbox")
                        || node.id.as_deref().is_none_or(str::is_empty)
                        || text.contains(['\n', '\r'])
                    {
                        return Err(error("InspectorExactFocusRequired"));
                    }
                    let mut readback = reference.request.clone();
                    // Text changes during type, so identity must survive without the old text predicate.
                    readback.query = SemanticLocator {
                        id: node.id.clone(),
                        role: node.role.clone(),
                        ..SemanticLocator::default()
                    };
                    readback.remaining_ms = remaining(deadline);
                    // Prove the stable post-type locator is unique before changing any text.
                    let focused = read_observation(session, &readback, deadline, cancel).await?;
                    validate_binding(&focused, &request.udid, session)?;
                    if focused.session_epoch != observation.session_epoch
                        || focused.app.package != observation.app.package
                        || focused.generation <= observation.generation
                        || focused.unknown_match_count != 0
                        || focused.matches.len() != 1
                        || focused.matches[0].focused != Some(true)
                        || focused.matches[0].password != Some(false)
                        || focused.matches[0].bounds != node.bounds
                    {
                        return Err(error("InspectorExactFocusRequired"));
                    }
                    actionable(&focused, &focused.matches[0])?;
                    ensure_active(deadline, cancel)?;
                    session
                        .type_text(text)
                        .await
                        .map_err(|_| error("InspectorActionUncertain"))?;
                    readback.remaining_ms = remaining(deadline);
                    let result = wait_for_observation(
                        session,
                        &readback,
                        &ObservationExpectation::Value {
                            value: text.into(),
                            exact: true,
                        },
                        deadline,
                        cancel,
                        Duration::from_millis(150),
                    )
                    .await
                    .map_err(|_| error("InspectorActionUncertain"))?;
                    let observed = result
                        .observation
                        .as_ref()
                        .ok_or_else(|| error("InspectorActionUncertain"))?;
                    validate_binding(observed, &request.udid, session)?;
                    if result.status != ObservationWaitStatus::Satisfied
                        || observed.session_epoch != observation.session_epoch
                        || observed.generation <= focused.generation
                        || observed.app.package != observation.app.package
                        || observed
                            .matches
                            .first()
                            .is_none_or(|n| n.focused != Some(true))
                    {
                        return Err(error("InspectorActionUncertain"));
                    }
                    Ok(
                        json!({"status":"verified", "verified":true, "readback":"exactValueAndFocus"}),
                    )
                }
                Operation::Swipe => {
                    if node.scrollable != Some(true) {
                        return Err(error("InspectorNotActionable"));
                    }
                    let direction = request.direction.ok_or_else(|| error("InvalidArgument"))?;
                    // Proportions are relative to fresh measured scroll-container bounds, not screen guesses.
                    let point = |x, y| TapPoint {
                        x: bounds.x + bounds.width * x,
                        y: bounds.y + bounds.height * y,
                    };
                    let (from, to) = match direction {
                        Direction::Up => (point(0.5, 0.75), point(0.5, 0.25)),
                        Direction::Down => (point(0.5, 0.25), point(0.5, 0.75)),
                        Direction::Left => (point(0.75, 0.5), point(0.25, 0.5)),
                        Direction::Right => (point(0.25, 0.5), point(0.75, 0.5)),
                    };
                    ensure_active(deadline, cancel)?;
                    session
                        .swipe(SwipeGesture {
                            from,
                            to,
                            duration_ms: 350,
                        })
                        .await
                        .map_err(|_| error("InspectorActionUncertain"))?;
                    Ok(json!({"status":"dispatched", "verified":false}))
                }
                _ => unreachable!(),
            }
        }
        Operation::Begin | Operation::End => unreachable!("session lifecycle is owned above"),
    }
}

fn ensure_time(deadline: tokio::time::Instant) -> Result<(), CommandError> {
    if remaining(deadline) == 0 {
        Err(error("InspectorDeadlineExceeded"))
    } else {
        Ok(())
    }
}

fn ensure_active(
    deadline: tokio::time::Instant,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> Result<(), CommandError> {
    if *cancel.borrow() || cancel.has_changed().is_err() {
        return Err(error("InspectorCancelled"));
    }
    ensure_time(deadline)
}

async fn read_observation(
    session: &dyn UiSession,
    request: &ObservationRequest,
    deadline: tokio::time::Instant,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<UiObservation, CommandError> {
    cancellable_read(session.observe(request), deadline, cancel).await
}

/// Read-only futures may be dropped on revocation; device effects must drain instead.
async fn cancellable_read<T>(
    read: impl std::future::Future<Output = anyhow::Result<T>>,
    deadline: tokio::time::Instant,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<T, CommandError> {
    ensure_active(deadline, cancel)?;
    tokio::select! {
        biased;
        _ = async {
            while !*cancel.borrow_and_update() {
                if cancel.changed().await.is_err() { break; }
            }
        } => Err(error("InspectorCancelled")),
        result = tokio::time::timeout_at(deadline, read) => {
            // A ready future can win the timeout poll after a synchronous parse.
            ensure_active(deadline, cancel)?;
            result.map_err(|_| error("InspectorDeadlineExceeded"))?.map_err(driver_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Phone {
        reading: tokio::sync::Notify,
        pressed: tokio::sync::Notify,
        release: tokio::sync::Notify,
        presses: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UiSession for Phone {
        async fn observe(&self, _: &ObservationRequest) -> anyhow::Result<UiObservation> {
            self.reading.notify_one();
            std::future::pending().await
        }
        async fn press_hardware_key(&self, _: HardwareKey) -> anyhow::Result<()> {
            self.presses.fetch_add(1, Ordering::SeqCst);
            self.pressed.notify_one();
            self.release.notified().await;
            Ok(())
        }
        async fn tap(&self, _: TapPoint) -> anyhow::Result<()> {
            panic!("unexpected tap")
        }
        async fn swipe(&self, _: SwipeGesture) -> anyhow::Result<()> {
            panic!("unexpected swipe")
        }
        async fn type_text(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected type")
        }
        async fn home(&self) -> anyhow::Result<()> {
            panic!("unexpected home")
        }
        async fn find_and_tap(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected input")
        }
        async fn assert_visible(&self, _: &str) -> anyhow::Result<()> {
            panic!("unexpected assertion")
        }
        fn stream_url(&self) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn owner_cancellation_stops_wait_but_drains_dispatched_input() {
        let phone = Phone::default();
        let mut refs = References {
            udid: "fixture".into(),
            caller: "fixture".into(),
            expires: Instant::now() + TTL,
            last_used: Instant::now(),
            entries: HashMap::new(),
        };
        let request: SemanticRequest = serde_json::from_value(json!({
            "operation":"wait_for", "udid":"fixture", "expected":{"kind":"exists"}
        }))
        .unwrap();
        let (owner, mut cancel) = tokio::sync::watch::channel(false);
        let wait = run(
            &phone,
            &request,
            &mut refs,
            tokio::time::Instant::now() + Duration::from_secs(10),
            &mut cancel,
        );
        let revoke = async {
            phone.reading.notified().await;
            owner.send_replace(true);
        };
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(wait, revoke).0
        })
        .await
        .expect("owner revocation must interrupt the read, not wait for its deadline")
        .unwrap();
        assert_eq!(result["status"], "cancelled");

        let (owner, mut cancel) = tokio::sync::watch::channel(false);
        let request: SemanticRequest = serde_json::from_value(json!({
            "operation":"observe", "udid":"fixture"
        }))
        .unwrap();
        let read = run(
            &phone,
            &request,
            &mut refs,
            tokio::time::Instant::now() + Duration::from_secs(10),
            &mut cancel,
        );
        let revoke = async {
            phone.reading.notified().await;
            owner.send_replace(true);
        };
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(read, revoke).0
        })
        .await
        .expect("owner revocation must interrupt a one-shot read");
        assert_eq!(result.unwrap_err().code, "InspectorCancelled");

        let request: SemanticRequest = serde_json::from_value(json!({
            "operation":"press", "udid":"fixture", "key":"back"
        }))
        .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        assert_eq!(
            run(&phone, &request, &mut refs, deadline, &mut cancel)
                .await
                .unwrap_err()
                .code,
            "InspectorCancelled"
        );
        assert_eq!(phone.presses.load(Ordering::SeqCst), 0);

        let (owner, mut cancel) = tokio::sync::watch::channel(false);
        let input = run(&phone, &request, &mut refs, deadline, &mut cancel);
        let revoke_in_flight = async {
            phone.pressed.notified().await;
            owner.send_replace(true);
            phone.release.notify_one();
        };
        let result = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(input, revoke_in_flight).0
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result["status"], "dispatched");
        assert_eq!(phone.presses.load(Ordering::SeqCst), 1);
    }
}
