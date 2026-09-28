//! A proposal is returned only after a second observation proves the target still exists.
//! This module never taps, writes text, marks an assignment done, or retries a public effect.
use super::{model::*, resolver, tree::Tree};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

/// Wait using only `UiSession::observe`, one monotonic total deadline and cancellation.
/// Unknown and ambiguous observations never satisfy an expectation. Closed cancellation
/// channels cancel too. Transport failures propagate without recovery or public actions.
pub async fn wait_for_observation(
    session: &dyn crate::UiSession,
    request: &super::ObservationRequest,
    expected: &super::ObservationExpectation,
    deadline: tokio::time::Instant,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    poll_interval: Duration,
) -> anyhow::Result<super::ObservationWaitResult> {
    use super::{ExpectationVerdict, ObservationWaitResult, ObservationWaitStatus};
    request.validate()?;
    // The explicit deadline may shorten, but never extend, the request's remaining budget.
    let request_deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_millis(request.remaining_ms))
        .unwrap_or(deadline);
    let deadline = deadline.min(request_deadline);
    let mut result = ObservationWaitResult {
        status: ObservationWaitStatus::DeadlineExceeded,
        verdict: ExpectationVerdict::Unknown,
        observation: None,
    };
    loop {
        if *cancel.borrow() || cancel.has_changed().is_err() {
            result.status = ObservationWaitStatus::Cancelled;
            return Ok(result);
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let remaining_ms = remaining.as_millis().min(u128::from(u64::MAX)) as u64;
        if remaining_ms == 0 {
            return Ok(result);
        }
        let mut current = request.clone();
        current.remaining_ms = remaining_ms;
        let read = tokio::select! {
            biased;
            _ = observation_cancelled(cancel) => {
                result.status = ObservationWaitStatus::Cancelled;
                return Ok(result);
            }
            _ = tokio::time::sleep_until(deadline) => return Ok(result),
            read = session.observe(&current) => read,
        };
        // Completion guards apply to errors too: a read may finish within one poll
        // after cancellation or the deadline, before select can poll its timers again.
        if *cancel.borrow() || cancel.has_changed().is_err() {
            result.status = ObservationWaitStatus::Cancelled;
            return Ok(result);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(result);
        }
        let observation = match read {
            Ok(observation) => observation,
            Err(error)
                if error
                    .downcast_ref::<crate::driver::UnsupportedCapability>()
                    .is_some() =>
            {
                result.status = ObservationWaitStatus::Unsupported;
                result.verdict = ExpectationVerdict::Unknown;
                return Ok(result);
            }
            Err(error) => return Err(error),
        };
        anyhow::ensure!(
            !observation.device_id.is_empty()
                && !observation.session_epoch.is_empty()
                && !observation.observation_id.is_empty()
                && observation.generation > 0
                && observation.ended_at_ms >= observation.started_at_ms,
            "observation_binding_invalid"
        );
        if let Some(package) = request.scope.as_ref().and_then(|s| s.package.as_ref()) {
            anyhow::ensure!(
                observation.app.package.as_ref() == Some(package),
                "observation_app_changed"
            );
        }
        if let Some(previous) = &result.observation {
            anyhow::ensure!(
                previous.device_id == observation.device_id
                    && previous.session_epoch == observation.session_epoch
                    && previous.app.package == observation.app.package
                    && observation.generation > previous.generation
                    && observation.observation_id != previous.observation_id,
                "observation_binding_changed_or_stale"
            );
        }
        result.verdict = super::expect_observation(&observation, expected);
        result.observation = Some(observation);
        if result.verdict == ExpectationVerdict::Satisfied {
            result.status = ObservationWaitStatus::Satisfied;
            return Ok(result);
        }
        let delay = poll_interval
            .max(Duration::from_millis(1))
            .min(deadline.saturating_duration_since(tokio::time::Instant::now()));
        tokio::select! {
            biased;
            _ = observation_cancelled(cancel) => {
                result.status = ObservationWaitStatus::Cancelled;
                return Ok(result);
            }
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

async fn observation_cancelled(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        let cancelled = *cancel.borrow_and_update();
        if cancelled || cancel.changed().await.is_err() {
            return;
        }
    }
}

pub async fn resolve_navigation(
    session: &dyn crate::UiSession,
    target: &str,
    budget: Duration,
) -> anyhow::Result<Option<crate::ElementBox>> {
    let budget = if let Some(end) = session.gui_scope().and_then(|s| s.deadline_ms) {
        budget.min(Duration::from_millis(
            (end - chrono::Utc::now().timestamp_millis()).max(0) as u64,
        ))
    } else {
        budget
    };
    if !session.supports_element_bounds()
        || session.gui_session_epoch().is_empty()
        || budget.is_zero()
    {
        return Ok(None);
    }
    tokio::time::timeout(budget, resolve_inner(session, target, budget))
        .await
        .map_err(|_| anyhow::anyhow!("gui_deadline: hết thời gian nhận diện"))?
}
async fn resolve_inner(
    session: &dyn crate::UiSession,
    target: &str,
    budget: Duration,
) -> anyhow::Result<Option<crate::ElementBox>> {
    let started = Instant::now();
    let package = session.active_app_bundle().await?;
    let Some(adapter) = crate::app_automation::adapter(&package) else {
        return Ok(None);
    };
    let reasoner = session.gui_reasoner();
    let pack = session
        .gui_compatibility_pack(&package)
        .unwrap_or_else(|| adapter.pack());
    let Some(spec) = pack.target(target) else {
        return Ok(None);
    };
    anyhow::ensure!(!spec.effectful, "gui_effect_requires_engine");
    let epoch = session.gui_session_epoch();
    let (width, height) = session.window_size().await?;
    anyhow::ensure!(
        width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
            && width <= 16384.0
            && height <= 16384.0,
        "gui_dimensions_invalid"
    );
    let mut app = AppContext {
        package: package.clone(),
        version: session.app_version(&package).await.unwrap_or_default(),
        system_locale: session.ui_language().await.unwrap_or_default(),
        observed_language: None,
        width: width as u32,
        height: height as u32,
    };
    let tree = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    app.observed_language = crate::app_automation::observed_language(&tree, &app);
    let screen = adapter.classify(&tree, &app);
    if screen == "login" || (screen != "unknown" && screen != spec.screen) {
        return Ok(None);
    }
    let nodes = resolver::visible_nodes(&tree, &app);
    let mut request = GuiRequest {
        scope: session.gui_scope(),
        protocol_version: GUI_PROTOCOL_VERSION,
        request_id: uuid::Uuid::new_v4().to_string(),
        observation_id: uuid::Uuid::new_v4().to_string(),
        session_epoch: epoch.clone(),
        generation: tree.generation,
        app: app.clone(),
        target: target.into(),
        expected_screen: screen,
        remaining_ms: budget.saturating_sub(started.elapsed()).as_millis() as u64,
        nodes,
        screenshot: String::new(),
        screenshot_sha256: String::new(),
    };
    let candidates = resolver::resolve(&tree, &app, spec);
    let candidate = match candidates.as_slice() {
        [candidate] => candidate.clone(),
        [] => {
            let Some(reasoner) = reasoner.as_ref() else {
                return Ok(None);
            };
            if epoch.is_empty() || request.remaining_ms < 1000 {
                return Ok(None);
            }
            let bytes = session.screenshot_png().await?;
            request.screenshot_sha256 = format!("{:x}", Sha256::digest(&bytes));
            request.screenshot = base64::engine::general_purpose::STANDARD.encode(bytes);
            request.remaining_ms = budget.saturating_sub(started.elapsed()).as_millis() as u64;
            let response = reasoner.resolve(request.clone()).await?;
            response.validate_binding(&request)?;
            reasoner.record(&request, Some(&response), "proposal");
            if response.status != ResolutionStatus::Resolved {
                return Ok(None);
            }
            response.candidates[0].clone()
        }
        _ => {
            if let Some(r) = reasoner.as_ref() {
                r.record(&request, None, "gui_target_ambiguous");
            }
            return Ok(None);
        }
    };
    // An image-only guess may help diagnosis but cannot become a device target here.
    let Some(id) = candidate.node_id else {
        return Ok(None);
    };
    let original = request
        .nodes
        .iter()
        .find(|n| n.id == id)
        .ok_or_else(|| anyhow::anyhow!("gui_node_missing"))?;
    if is_public_effect_label(&original.text) || is_public_effect_label(&original.description) {
        return Ok(None);
    }
    let fresh = Tree::parse(session.hierarchy_source_snapshot().await?)?;
    let fresh_screen = adapter.classify(&fresh, &app);
    if fresh.generation <= request.generation || fresh_screen != request.expected_screen {
        return Ok(None);
    }
    if session.gui_session_epoch() != epoch
        || session.active_app_bundle().await? != package
        || started.elapsed() >= budget
    {
        return Ok(None);
    }
    let matches: Vec<_> = resolver::visible_nodes(&fresh, &app)
        .into_iter()
        .filter(|n| {
            n.text == original.text
                && n.description == original.description
                && n.resource_id == original.resource_id
                && n.class_name == original.class_name
                && n.bounds == original.bounds
                && n.enabled == Some(true)
                && n.clickable == Some(true)
        })
        .collect();
    let [node] = matches.as_slice() else {
        return Ok(None);
    };
    let b = &node.bounds;
    if let Some(r) = reasoner.as_ref() {
        r.record(&request, None, "target_revalidated");
    }
    Ok(Some(crate::ElementBox {
        x: b.x,
        y: b.y,
        width: b.width,
        height: b.height,
        description: Some(node.text.clone()),
        enabled: true,
        clickable: true,
    }))
}

fn is_public_effect_label(value: &str) -> bool {
    let text = value.trim().to_lowercase();
    [
        "post",
        "đăng",
        "send",
        "gửi",
        "delete",
        "xóa",
        "xoá",
        "follow",
        "theo dõi",
        "buy",
        "mua",
        "allow",
        "cho phép",
        "log in",
        "đăng nhập",
    ]
    .iter()
    .any(|s| text == *s)
}
