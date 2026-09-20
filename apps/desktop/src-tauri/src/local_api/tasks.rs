//! Typed local access to existing Flow services. This module owns no scheduler or device.

use std::collections::{BTreeMap, HashSet};

use riviu_core::FlowTargetSelection;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

use super::ApiError;
use crate::{commands, farm_commands, flow_commands};

#[derive(Debug, Clone, PartialEq)]
pub enum TaskCommand {
    ListApps,
    GetApp {
        id: Uuid,
        revision: Option<u64>,
    },
    RunApp {
        id: Uuid,
        revision: u64,
        target: riviu_core::TargetRef,
    },
    Catalog,
    ListFlows {
        include_archived: bool,
    },
    GetFlow {
        id: Uuid,
        revision: Option<u64>,
    },
    ListRuns {
        limit: usize,
    },
    GetRun {
        id: Uuid,
    },
    Run {
        id: Uuid,
        request: RunRequest,
    },
    Cancel {
        id: Uuid,
    },
    ListGroups,
    ListJobs,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunRequest {
    pub revision: Option<u64>,
    pub selection: FlowTargetSelection,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new(400, message)
}

fn parse_id(raw: &str) -> Result<Uuid, ApiError> {
    let id = Uuid::parse_str(raw).map_err(|_| invalid("id must be a canonical UUID"))?;
    if id.to_string() != raw {
        return Err(invalid("id must be a canonical UUID"));
    }
    Ok(id)
}

fn positive_revision(revision: Option<u64>) -> Result<Option<u64>, ApiError> {
    if revision == Some(0) {
        return Err(invalid("revision must be a positive integer"));
    }
    Ok(revision)
}

fn validate_selection_fields(body: &Value) -> Result<(), ApiError> {
    let selection = body
        .get("selection")
        .and_then(Value::as_object)
        .ok_or_else(|| invalid("selection must be an object with an explicit mode"))?;
    // Serde's internally tagged unit variant can discard fields despite
    // deny_unknown_fields. Check the wire shape before AllEligible can erase a
    // caller's device restriction and expand work to the fleet.
    let allowed: &[&str] = match selection.get("mode").and_then(Value::as_str) {
        Some("one") => &["mode", "udid"],
        Some("selected") => &["mode", "udids"],
        Some("allEligible") => &["mode"],
        _ => {
            return Err(invalid(
                "selection.mode must be one, selected or allEligible",
            ))
        }
    };
    if selection.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid(
            "selection contains fields not allowed for its mode",
        ));
    }
    Ok(())
}

fn validate_selection(selection: &FlowTargetSelection) -> Result<(), ApiError> {
    let exact = |id: &str| {
        !id.is_empty() && id == id.trim() && id.len() <= 256 && !id.chars().any(char::is_control)
    };
    match selection {
        FlowTargetSelection::One { udid } if !exact(udid) => {
            Err(invalid("selection.udid must be an exact device identifier"))
        }
        FlowTargetSelection::Selected { udids } => {
            if udids.is_empty() || udids.len() > 200 {
                return Err(invalid("selection.udids must contain 1..=200 devices"));
            }
            let mut unique = HashSet::new();
            if udids.iter().any(|id| !exact(id) || !unique.insert(id)) {
                return Err(invalid("selection.udids must be exact, unique identifiers"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn query(raw_path: &str, allowed: &[&str]) -> Result<BTreeMap<String, String>, ApiError> {
    if raw_path.contains('#') {
        return Err(invalid("request target must not contain a fragment"));
    }
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{raw_path}"))
        .map_err(|_| invalid("invalid request target"))?;
    let mut values = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if !allowed.contains(&key.as_ref()) || values.contains_key(key.as_ref()) {
            return Err(invalid(format!(
                "unknown or duplicate query parameter: {key}"
            )));
        }
        values.insert(key.into_owned(), value.into_owned());
    }
    Ok(values)
}

fn empty_body(body: &Value) -> Result<(), ApiError> {
    if body.as_object().is_some_and(|object| object.is_empty()) {
        Ok(())
    } else {
        Err(invalid("this endpoint does not accept body fields"))
    }
}

/// Return None only when the route belongs to another API family.
pub fn route(method: &str, raw_path: &str, body: &Value) -> Option<Result<TaskCommand, ApiError>> {
    let path = raw_path.split('?').next().unwrap_or(raw_path);
    let pieces: Vec<_> = path.split('/').collect();
    let (expected_method, allowed_query) = match pieces.as_slice() {
        ["", "v1", "apps"] => ("GET", &[][..]),
        ["", "v1", "apps", _] => ("GET", &["revision"][..]),
        ["", "v1", "apps", _, "runs"] => ("POST", &[][..]),
        ["", "v1", "flows"] => ("GET", &["includeArchived"][..]),
        ["", "v1", "flows", "catalog"] => ("GET", &[][..]),
        ["", "v1", "flows", _] => ("GET", &["revision"][..]),
        ["", "v1", "flows", _, "runs"] => ("POST", &[][..]),
        ["", "v1", "flow-runs"] => ("GET", &["limit"][..]),
        ["", "v1", "flow-runs", _] => ("GET", &[][..]),
        ["", "v1", "flow-runs", _, "cancel"] => ("POST", &[][..]),
        ["", "v1", "groups" | "jobs"] => ("GET", &[][..]),
        _ => return None,
    };
    Some((|| {
        if method != expected_method {
            return Err(ApiError::new(
                405,
                format!("use {expected_method} for this endpoint"),
            ));
        }
        let query = query(raw_path, allowed_query)?;
        if !matches!(pieces.as_slice(), ["", "v1", "flows" | "apps", _, "runs"]) {
            empty_body(body)?;
        }
        match pieces.as_slice() {
            ["", "v1", "apps"] => Ok(TaskCommand::ListApps),
            ["", "v1", "apps", id] => {
                let revision = query
                    .get("revision")
                    .map(|raw| {
                        raw.parse::<u64>()
                            .map_err(|_| invalid("revision must be positive"))
                    })
                    .transpose()?;
                Ok(TaskCommand::GetApp {
                    id: parse_id(id)?,
                    revision: positive_revision(revision)?,
                })
            }
            ["", "v1", "apps", id, "runs"] => {
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct AppRun {
                    revision: u64,
                    target: riviu_core::TargetRef,
                }
                let request: AppRun = serde_json::from_value(body.clone()).map_err(|_| {
                    invalid("App run requires a pinned revision and explicit target")
                })?;
                positive_revision(Some(request.revision))?;
                if let riviu_core::TargetRef::Explicit { ref udids } = request.target {
                    if udids.is_empty()
                        || udids.len() > 200
                        || udids.iter().any(|id| id.trim().is_empty())
                    {
                        return Err(invalid("Select 1-200 devices"));
                    }
                }
                Ok(TaskCommand::RunApp {
                    id: parse_id(id)?,
                    revision: request.revision,
                    target: request.target,
                })
            }
            ["", "v1", "flows"] => {
                let include_archived = match query.get("includeArchived").map(String::as_str) {
                    None | Some("false") => false,
                    Some("true") => true,
                    _ => return Err(invalid("includeArchived must be true or false")),
                };
                Ok(TaskCommand::ListFlows { include_archived })
            }
            ["", "v1", "flows", "catalog"] => Ok(TaskCommand::Catalog),
            ["", "v1", "flows", id] => {
                let revision = query
                    .get("revision")
                    .map(|raw| {
                        raw.parse::<u64>()
                            .map_err(|_| invalid("revision must be a positive integer"))
                    })
                    .transpose()?;
                Ok(TaskCommand::GetFlow {
                    id: parse_id(id)?,
                    revision: positive_revision(revision)?,
                })
            }
            ["", "v1", "flow-runs"] => {
                let limit = query
                    .get("limit")
                    .map(|raw| {
                        raw.parse::<usize>()
                            .map_err(|_| invalid("limit must be an integer in 1..=200"))
                    })
                    .transpose()?
                    .unwrap_or(100);
                if !(1..=200).contains(&limit) {
                    return Err(invalid("limit must be an integer in 1..=200"));
                }
                Ok(TaskCommand::ListRuns { limit })
            }
            ["", "v1", "flow-runs", id] => Ok(TaskCommand::GetRun { id: parse_id(id)? }),
            ["", "v1", "flows", id, "runs"] => {
                validate_selection_fields(body)?;
                let request: RunRequest = serde_json::from_value(body.clone())
                    .map_err(|error| invalid(format!("invalid Flow run request: {error}")))?;
                positive_revision(request.revision)?;
                validate_selection(&request.selection)?;
                Ok(TaskCommand::Run {
                    id: parse_id(id)?,
                    request,
                })
            }
            ["", "v1", "flow-runs", id, "cancel"] => Ok(TaskCommand::Cancel { id: parse_id(id)? }),
            ["", "v1", "groups"] => Ok(TaskCommand::ListGroups),
            ["", "v1", "jobs"] => Ok(TaskCommand::ListJobs),
            _ => Err(ApiError::new(404, "route not found")),
        }
    })())
}

fn serialized<T: serde::Serialize>(value: T) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|_| ApiError::new(500, "response serialization failed"))
}

fn found<T>(value: Option<T>, code: &str, message: &str) -> Result<T, ApiError> {
    value.ok_or_else(|| crate::command_error::CommandError::code(code, message).into())
}

/// Delegate to the same command handlers the UI calls. In particular run/cancel retain
/// their command admission guard and use the sole durable Flow runtime in AppState.
pub async fn execute(app: &AppHandle, command: TaskCommand) -> Result<Value, ApiError> {
    match command {
        TaskCommand::ListApps => serialized(crate::app_workflow_commands::app_workflow_list(
            app.state(),
        )?),
        TaskCommand::GetApp { id, revision } => serialized(found(
            crate::app_workflow_commands::app_workflow_get(app.state(), id, revision)?,
            "AppNotFound",
            "App revision does not exist",
        )?),
        TaskCommand::RunApp {
            id,
            revision,
            target,
        } => serialized(
            crate::app_workflow_commands::app_workflow_run(
                app.clone(),
                app.state(),
                id,
                revision,
                target,
            )
            .await?,
        ),
        TaskCommand::Catalog => serialized(flow_commands::flow_action_catalog()),
        TaskCommand::ListFlows { include_archived } => {
            serialized(flow_commands::flow_list(app.state(), include_archived)?)
        }
        TaskCommand::GetFlow { id, revision } => serialized(found(
            flow_commands::flow_get(app.state(), id.to_string(), revision)?,
            "FlowNotFound",
            "Flow revision does not exist",
        )?),
        TaskCommand::ListRuns { limit } => {
            serialized(flow_commands::flow_list_runs(app.state(), limit)?)
        }
        TaskCommand::GetRun { id } => serialized(found(
            flow_commands::flow_get_run(app.state(), id.to_string())?,
            "FlowRunNotFound",
            "Flow run does not exist",
        )?),
        TaskCommand::Run { id, request } => serialized(
            flow_commands::flow_run(
                app.state(),
                id.to_string(),
                request.revision,
                request.selection,
            )
            .await?,
        ),
        TaskCommand::Cancel { id } => {
            flow_commands::flow_cancel_run(app.state(), id.to_string())?;
            // A cancellation request does not assert that in-flight device work has ended.
            serialized(json!({ "runId": id, "cancellationRequested": true }))
        }
        TaskCommand::ListGroups => serialized(farm_commands::list_groups(app.state())?),
        TaskCommand::ListJobs => serialized(commands::list_jobs(app.state()).await?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_routes_require_pinned_revision_and_preserve_device_selection() {
        let id = "d491cbe7-00a0-4651-ac30-7373f23ba0b8";
        assert!(matches!(
            route("GET", "/v1/apps", &json!({})).unwrap().unwrap(),
            TaskCommand::ListApps
        ));
        assert!(route(
            "POST",
            &format!("/v1/apps/{id}/runs"),
            &json!({"revision":0,"target":{"type":"all"}})
        )
        .unwrap()
        .is_err());
        assert!(route(
            "POST",
            &format!("/v1/apps/{id}/runs"),
            &json!({"revision":2,"target":{"type":"explicit","udids":[]}})
        )
        .unwrap()
        .is_err());
        let command = route(
            "POST",
            &format!("/v1/apps/{id}/runs"),
            &json!({"revision":2,"target":{"type":"explicit","udids":["phone-3"]}}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            command,
            TaskCommand::RunApp {
                id: Uuid::parse_str(id).unwrap(),
                revision: 2,
                target: riviu_core::TargetRef::Explicit {
                    udids: vec!["phone-3".into()]
                }
            }
        );
    }

    const ID: &str = "d491cbe7-00a0-4651-ac30-7373f23ba0b8";

    fn routed(method: &str, path: &str, body: Value) -> Result<TaskCommand, ApiError> {
        route(method, path, &body).expect("task route")
    }

    #[test]
    fn read_routes_preserve_revision_archive_and_run_limit() {
        assert_eq!(
            routed("GET", "/v1/flows?includeArchived=true", json!({})).unwrap(),
            TaskCommand::ListFlows {
                include_archived: true
            }
        );
        assert_eq!(
            routed("GET", &format!("/v1/flows/{ID}?revision=3"), json!({})).unwrap(),
            TaskCommand::GetFlow {
                id: Uuid::parse_str(ID).unwrap(),
                revision: Some(3)
            }
        );
        assert_eq!(
            routed("GET", "/v1/flow-runs?limit=17", json!({})).unwrap(),
            TaskCommand::ListRuns { limit: 17 }
        );
        for (path, expected) in [
            ("/v1/flows/catalog", TaskCommand::Catalog),
            ("/v1/groups", TaskCommand::ListGroups),
            ("/v1/jobs", TaskCommand::ListJobs),
        ] {
            assert_eq!(routed("GET", path, json!({})).unwrap(), expected);
        }
    }

    #[test]
    fn queries_reject_typos_duplicates_and_unbounded_limits() {
        for path in [
            "/v1/flows?includeArchive=true",
            "/v1/flows?includeArchived=1",
            "/v1/flows?includeArchived=false&includeArchived=true",
            "/v1/flow-runs?limit=0",
            "/v1/flow-runs?limit=201",
            "/v1/flow-runs?limit=-1",
            "/v1/flow-runs?limit=1.5",
            "/v1/groups?all=true",
            "/v1/flows?includeArchived=true#ignored",
        ] {
            assert_eq!(
                routed("GET", path, json!({})).unwrap_err().status,
                400,
                "{path}"
            );
        }
        for revision in ["0", "-1", "x", "18446744073709551616"] {
            let path = format!("/v1/flows/{ID}?revision={revision}");
            assert_eq!(routed("GET", &path, json!({})).unwrap_err().status, 400);
        }
    }

    #[test]
    fn run_requires_explicit_selection_and_keeps_the_requested_devices() {
        let path = format!("/v1/flows/{ID}/runs");
        for selection in [
            json!({"mode":"one","udid":"A"}),
            json!({"mode":"selected","udids":["A","B"]}),
            json!({"mode":"allEligible"}),
        ] {
            let command =
                routed("POST", &path, json!({"revision": 4,"selection":selection})).unwrap();
            let TaskCommand::Run { request, .. } = command else {
                panic!("run command")
            };
            assert_eq!(request.revision, Some(4));
            assert_eq!(serde_json::to_value(request.selection).unwrap(), selection);
        }
        for body in [
            json!({}),
            json!({"selection":{"mode":"selected","udids":[]}}),
            json!({"selection":{"mode":"selected","udids":["A","A"]}}),
            json!({"selection":{"mode":"one","udid":" A "}}),
            json!({"selection":{"mode":"allEligible","udids":["A"]}}),
            json!({"selection":{"mode":"allEligible","udid":"A"}}),
            json!({"selection":{"mode":"allEligible","groupId":"group-a"}}),
            json!({"selection":{"mode":"one","udid":"A","udids":["B"]}}),
            json!({"selection":{"mode":"selected","udids":["A"],"udid":"B"}}),
            json!({"selection":{"mode":"allEligible"},"retry":true}),
            json!({"selection":{"mode":"allEligible"},"revision":0}),
        ] {
            assert_eq!(routed("POST", &path, body).unwrap_err().status, 400);
        }
    }

    #[test]
    fn malformed_ids_methods_and_cancel_fields_never_form_a_command() {
        assert_eq!(
            routed("GET", "/v1/flows/bad-id", json!({}))
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            routed("POST", "/v1/flows/catalog", json!({}))
                .unwrap_err()
                .status,
            405
        );
        let path = format!("/v1/flow-runs/{ID}/cancel");
        assert_eq!(routed("GET", &path, json!({})).unwrap_err().status, 405);
        assert_eq!(
            routed("POST", &path, json!({"force":true}))
                .unwrap_err()
                .status,
            400
        );
        assert_eq!(
            routed("POST", &path, json!({})).unwrap(),
            TaskCommand::Cancel {
                id: Uuid::parse_str(ID).unwrap()
            }
        );
        assert!(route("POST", "/v1/flows/id/shell", &json!({})).is_none());
        assert_eq!(
            found::<Value>(None, "FlowRunNotFound", "missing")
                .unwrap_err()
                .status,
            404
        );
    }
}
