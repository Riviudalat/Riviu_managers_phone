use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::model::{
    AuthRequirement, HttpMethod, RouteContract, RouteEvidence, RouteEvidenceStatus,
    SessionRequirement,
};

pub fn read_contract(path: &Path) -> Result<RouteContract> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read route contract: {}", path.display()))?;
    let contract = serde_json::from_slice(&bytes).context("parse route contract JSON")?;
    validate(&contract)?;
    Ok(contract)
}

pub fn validate(contract: &RouteContract) -> Result<()> {
    if contract.schema_version != 1 {
        bail!(
            "unsupported route contract schema version: {}",
            contract.schema_version
        );
    }
    if contract.routes.is_empty() {
        bail!("route contract must contain at least one route");
    }

    let mut unique_routes = BTreeSet::new();
    for route in &contract.routes {
        if !route.path.starts_with('/') || route.path.chars().any(char::is_whitespace) {
            bail!("route path must be absolute and contain no whitespace");
        }
        if route.session == SessionRequirement::Required && !route.path.contains("{sessionId}") {
            bail!("session-required route must contain {{sessionId}}");
        }
        if route.auth == AuthRequirement::Exempt
            && !(route.method == HttpMethod::Get && route.path == "/status")
        {
            bail!("auth exemption is limited to GET /status");
        }
        match (route.method, &route.request_body) {
            (HttpMethod::Post, Some(body)) => {
                if body.required.is_empty() || body.properties.is_empty() {
                    bail!("POST route request body must define required properties");
                }
                let mut required = BTreeSet::new();
                for field in &body.required {
                    if field.trim().is_empty() || !required.insert(field) {
                        bail!("request body required fields must be non-empty and unique");
                    }
                    if !body.properties.contains_key(field) {
                        bail!("request body required field has no property type: {field}");
                    }
                }
            }
            (HttpMethod::Post, None) => bail!("POST route must define a request body"),
            (_, Some(_)) => bail!("GET/DELETE route must not define a request body"),
            (_, None) => {}
        }
        if route.evidence.trim().is_empty() {
            bail!("route evidence path must not be empty");
        }
        if !unique_routes.insert((route.method, route.path.as_str())) {
            bail!("duplicate method/path route in contract");
        }
    }
    Ok(())
}

pub fn cross_check(
    contract: &RouteContract,
    oracle_routes: impl IntoIterator<Item = String>,
    baseline_routes: impl IntoIterator<Item = String>,
) -> Result<Vec<RouteEvidence>> {
    validate(contract)?;
    let oracle = normalized_set(oracle_routes);
    let baseline = normalized_set(baseline_routes);
    let documented = contract
        .routes
        .iter()
        .map(|route| normalize_path(&route.path))
        .collect::<BTreeSet<_>>();
    let mut evidence = contract
        .routes
        .iter()
        .map(|route| {
            let path = normalize_path(&route.path);
            let mut sources = Vec::new();
            if oracle.contains(&path) {
                sources.push("oracle Mach-O");
            }
            if baseline.contains(&path) {
                sources.push("WDA baseline");
            }
            RouteEvidence {
                method: Some(route.method),
                path,
                status: if sources.is_empty() {
                    RouteEvidenceStatus::DocumentedOnly
                } else {
                    RouteEvidenceStatus::PathConfirmed
                },
                evidence: if sources.is_empty() {
                    "none".into()
                } else {
                    sources.join(" + ")
                },
                contract_evidence: Some(route.evidence.clone()),
            }
        })
        .collect::<Vec<_>>();

    let observed = oracle.union(&baseline).cloned().collect::<BTreeSet<_>>();
    for path in observed.difference(&documented) {
        let in_oracle = oracle.contains(path);
        let in_baseline = baseline.contains(path);
        evidence.push(RouteEvidence {
            method: None,
            path: path.clone(),
            status: match (in_oracle, in_baseline) {
                (true, true) => RouteEvidenceStatus::PathConfirmed,
                (true, false) => RouteEvidenceStatus::OracleOnly,
                (false, true) => RouteEvidenceStatus::BaselineOnly,
                (false, false) => unreachable!("path came from observed route union"),
            },
            evidence: match (in_oracle, in_baseline) {
                (true, true) => "oracle Mach-O + WDA baseline".into(),
                (true, false) => "oracle Mach-O route candidate".into(),
                (false, true) => "WDA baseline route candidate".into(),
                (false, false) => unreachable!("path came from observed route union"),
            },
            contract_evidence: None,
        });
    }
    evidence.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then_with(|| left.method.cmp(&right.method))
    });
    Ok(evidence)
}

pub fn normalize_path(path: &str) -> String {
    let path = path.replace(":sessionId", "{sessionId}");
    let Some(rest) = path.strip_prefix("/session/") else {
        return path;
    };
    let Some((session, suffix)) = rest.split_once('/') else {
        return if rest == "{sessionId}" {
            path
        } else {
            "/session/{sessionId}".into()
        };
    };
    if session == "{sessionId}" {
        path
    } else {
        format!("/session/{{sessionId}}/{suffix}")
    }
}

fn normalized_set(values: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    values
        .into_iter()
        .map(|value| normalize_path(&value))
        .collect()
}
