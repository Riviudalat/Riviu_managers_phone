//! Read-only observation transport and projection; no action IDs escape this module.
use super::AndroidUiSession;
use crate::agent::{escape_java_regex, quote_java, AndroidObservationMode, Locator};
use anyhow::anyhow;
use riviu_core::driver::UiSession;
use riviu_core::ui_automation::{
    resolver::{resolve_observation, semantic_nodes},
    tree::Tree,
    ObservationCompleteness, ObservationRequest, ObservationSource, ObservedAppContext, Rect,
    UiObservation,
};
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

impl AndroidUiSession {
    pub(super) async fn observe_bounded(
        &self,
        request: &ObservationRequest,
    ) -> anyhow::Result<UiObservation> {
        self.observe_inner(request, true).await
    }

    pub(super) async fn observe_without_recovery_bounded(
        &self,
        request: &ObservationRequest,
    ) -> anyhow::Result<UiObservation> {
        self.observe_inner(request, false).await
    }

    async fn observe_inner(
        &self,
        request: &ObservationRequest,
        allow_recovery: bool,
    ) -> anyhow::Result<UiObservation> {
        let started = Instant::now();
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        request.validate()?;
        anyhow::ensure!(request.remaining_ms > 0, "observation_deadline_exceeded");
        let deadline = started
            .checked_add(Duration::from_millis(request.remaining_ms))
            .ok_or_else(|| anyhow!("observation_budget_invalid"))?;
        let session_epoch = self.gui_session_epoch();
        let package = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.observation_foreground(deadline),
        )
        .await
        .map_err(|_| anyhow!("observation_deadline_exceeded"))??;
        let generation = self
            .hierarchy_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .map_err(|_| anyhow!("observation_generation_exhausted"))?
            + 1;
        let locator = if self.agent.observation_mode() == AndroidObservationMode::Enriched {
            native_locator(request)
        } else {
            None
        };
        let response = if allow_recovery {
            self.agent
                .observation_read(locator.as_ref(), deadline)
                .await?
        } else {
            self.agent
                .observation_read_without_recovery(locator.as_ref(), deadline)
                .await?
        };
        let repaired_session = response.repaired_session;
        let response = response.value;
        let (xml, source, native_rects, native_count) = if locator.is_some() {
            let entries = response
                .get("value")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("observation_elements_invalid"))?;
            anyhow::ensure!(entries.len() < 32768, "observation_node_limit");
            let (xml, rects) = native_tree(entries)?;
            (
                xml,
                ObservationSource::NativeQuery,
                rects,
                Some(entries.len()),
            )
        } else {
            let xml = response
                .get("value")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("observation_source_invalid"))?
                .to_owned();
            (
                xml,
                ObservationSource::AccessibilityHierarchy,
                Vec::new(),
                None,
            )
        };
        let tree = Tree::parse(riviu_core::HierarchySourceSnapshot { generation, xml })
            .map_err(|_| anyhow!("observation_tree_invalid"))?;
        let nodes = semantic_nodes(&tree);
        let mut app = ObservedAppContext {
            package: Some(package.clone()),
            ..Default::default()
        };
        if request.fields.bounds {
            let size = self.agent.observation_window_size(deadline).await?;
            let dimension = |key| -> anyhow::Result<u32> {
                let value = size
                    .get("value")
                    .and_then(|v| v.get(key))
                    .and_then(Value::as_f64)
                    .ok_or_else(|| anyhow!("observation_window_size_invalid"))?;
                anyhow::ensure!(
                    value.is_finite()
                        && value > 0.0
                        && value <= u32::MAX as f64
                        && value.fract() == 0.0,
                    "observation_window_size_invalid"
                );
                Ok(value as u32)
            };
            app.width = Some(dimension("width")?);
            app.height = Some(dimension("height")?);
        }
        let mut resolution = resolve_observation(&tree, request)?;
        // A compact/malformed element object is not evidence of absence. The core
        // projection skips XML wrappers, so account explicitly for omitted elements.
        if let Some(count) = native_count {
            resolution.unknown_match_count += count.saturating_sub(nodes.len());
            if request.query.id.is_some() {
                resolution.unknown_match_count +=
                    nodes.iter().filter(|node| node.id.is_none()).count();
            }
            if request.fields.bounds {
                for node in &mut resolution.matches {
                    node.bounds = node
                        .node_id
                        .checked_sub(1)
                        .and_then(|index| native_rects.get(index))
                        .cloned()
                        .flatten();
                }
            }
        }
        // /source has no completeness marker. In particular, a shallow well-formed
        // response cannot certify that upstream snapshot pruning never occurred.
        // Pinned AndroidX UiAutomator 2.3.0 QueryController also skips invisible
        // descendants (and null children). A full native predicate is still only
        // partial evidence for this contract, which does not imply visible-only scope.
        let completeness = if native_count.is_some() {
            ObservationCompleteness::Partial
        } else {
            ObservationCompleteness::Unknown
        };
        anyhow::ensure!(Instant::now() < deadline, "observation_deadline_exceeded");
        let ended_package = tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            self.observation_foreground(deadline),
        )
        .await
        .map_err(|_| anyhow!("observation_deadline_exceeded"))??;
        anyhow::ensure!(package == ended_package, "observation_foreground_changed");
        let current_epoch = self.gui_session_epoch();
        let repaired = if session_epoch != current_epoch {
            anyhow::ensure!(
                repaired_session.is_some_and(|(previous, current)| {
                    session_epoch == format!("{}:{previous}", self.gui_epoch)
                        && current_epoch == format!("{}:{current}", self.gui_epoch)
                }),
                "observation_session_changed"
            );
            true
        } else {
            false
        };
        let observation = UiObservation {
            device_id: self.serial.clone(),
            app,
            session_epoch: current_epoch.clone(),
            observation_id: uuid::Uuid::new_v4().to_string(),
            generation,
            started_at_ms,
            ended_at_ms: chrono::Utc::now().timestamp_millis(),
            source,
            completeness,
            matches: resolution.matches,
            unknown_match_count: resolution.unknown_match_count,
        };
        // Synchronous parsing/projection can finish after the timer would have fired.
        anyhow::ensure!(Instant::now() < deadline, "observation_deadline_exceeded");
        if repaired {
            return Err(riviu_core::driver::SessionEpochChanged {
                previous_epoch: session_epoch,
                current_epoch,
                device_id: self.serial.clone(),
                package,
                fresh_observation: Box::new(observation),
            }
            .into());
        }
        Ok(observation)
    }

    async fn observation_foreground(&self, deadline: Instant) -> anyhow::Result<String> {
        crate::adb::read_foreground_package_with(|source| async move {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(!remaining.is_zero(), "observation_deadline_exceeded");
            tracing::debug!(serial = %self.serial, transport = "adb", command = source,
                "observation read command");
            let output = self
                .adb
                .shell_output(&self.serial, source, remaining)
                .await?;
            anyhow::ensure!(output.exit_code == 0, "observation_foreground_read_failed");
            Ok(output.stdout)
        })
        .await
    }
}

/// Only compile predicates with identical semantics. Roles and inclusive subtrees
/// need the shared resolver; do not narrow first and then claim exhaustive absence.
fn native_locator(request: &ObservationRequest) -> Option<Locator> {
    let query = &request.query;
    let package = request
        .scope
        .as_ref()
        .and_then(|scope| scope.package.as_deref());
    if query.role.is_some()
        // Name fallback/precedence is owned by the shared resolver. A native
        // description-only predicate would omit nodes named by their text.
        || query.name.is_some()
        || request
            .scope
            .as_ref()
            .is_some_and(|scope| scope.root.is_some())
        || (query.id.is_none() && query.name.is_none() && query.text.is_none() && package.is_none())
    {
        return None;
    }
    let mut selector = String::from("new UiSelector()");
    for (method, value) in [
        ("resourceId", query.id.as_deref()),
        ("description", query.name.as_deref()),
        ("text", query.text.as_deref()),
        ("packageName", package),
    ] {
        if let Some(value) = value {
            // Control characters are left to the hierarchy resolver, not Java source.
            if value.chars().any(char::is_control) {
                return None;
            }
            if query.exact || method == "packageName" {
                selector.push_str(&format!(".{method}({})", quote_java(value)));
            } else {
                // UiSelector.descriptionContains is case-insensitive; a quoted
                // DOTALL literal preserves core's case-sensitive substring semantics.
                let pattern = format!("(?s).*{}.*", escape_java_regex(value));
                selector.push_str(&format!(".{method}Matches({})", quote_java(&pattern)));
            }
        }
    }
    Some(Locator::UiSelector(selector))
}

/// Normalize the pinned wire keys into the shared resolver's XML vocabulary.
/// Strings/booleans only; null, missing, and malformed values remain unknown.
fn native_tree(entries: &[Value]) -> anyhow::Result<(String, Vec<Option<Rect>>)> {
    let mut xml = String::from("<hierarchy>");
    let mut rects = Vec::with_capacity(entries.len());
    for entry in entries {
        anyhow::ensure!(entry.is_object(), "observation_element_invalid");
        xml.push_str("<node");
        for (wire, attribute) in [
            ("name", "content-desc"),
            ("text", "text"),
            ("enabled", "enabled"),
            ("displayed", "displayed"),
            ("selected", "selected"),
            ("attribute/checked", "checked"),
            ("attribute/checkable", "checkable"),
            ("attribute/focused", "focused"),
            ("attribute/password", "password"),
            ("attribute/showing-hint", "showing-hint"),
            ("attribute/scrollable", "scrollable"),
            ("attribute/class", "class"),
            ("attribute/resource-id", "resource-id"),
            ("attribute/package", "package"),
            ("attribute/clickable", "clickable"),
            ("attribute/long-clickable", "long-clickable"),
            ("attribute/focusable", "focusable"),
        ] {
            let value = match entry.get(wire) {
                Some(Value::String(value)) => value.clone(),
                Some(Value::Bool(value))
                    if !matches!(
                        attribute,
                        "content-desc" | "text" | "class" | "resource-id" | "package"
                    ) =>
                {
                    value.to_string()
                }
                _ => continue,
            };
            xml.push_str(&format!(" {attribute}=\"{}\"", escape_xml(&value)));
        }
        xml.push_str("/>");
        rects.push(entry.get("rect").and_then(native_rect));
    }
    xml.push_str("</hierarchy>");
    anyhow::ensure!(xml.len() <= 16 * 1024 * 1024, "observation_tree_size_limit");
    Ok((xml, rects))
}

fn native_rect(value: &Value) -> Option<Rect> {
    let rect = Rect {
        x: value.get("x")?.as_f64()?,
        y: value.get("y")?.as_f64()?,
        width: value.get("width")?.as_f64()?,
        height: value.get("height")?.as_f64()?,
    };
    ([rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        && rect.width >= 0.0
        && rect.height >= 0.0)
        .then_some(rect)
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;")
}
