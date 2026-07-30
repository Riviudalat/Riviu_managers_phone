use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rtmmo_re::model::{
    AuthRequirement, HttpMethod, JsonValueKind, RequestBodyContract, RouteContract,
    RouteContractEntry, RouteEvidenceStatus, SessionRequirement,
};
use rtmmo_re::routes;

fn entry(path: &str) -> RouteContractEntry {
    RouteContractEntry {
        method: HttpMethod::Post,
        path: path.into(),
        auth: AuthRequirement::Protected,
        session: SessionRequirement::None,
        request_body: Some(RequestBodyContract {
            required: vec!["value".into()],
            properties: BTreeMap::from([("value".into(), JsonValueKind::Array)]),
        }),
        evidence: "fixture.rs".into(),
    }
}

#[test]
fn checked_in_route_contract_is_valid() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("contracts/oracle-routes.json");
    let contract = routes::read_contract(&path).unwrap();

    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.routes.len(), 8);
    assert!(contract.routes.iter().all(|route| {
        if route.method == HttpMethod::Post {
            route.request_body.is_some()
        } else {
            route.request_body.is_none()
        }
    }));
}

#[test]
fn route_contract_rejects_invalid_auth_session_and_duplicates() {
    let mut exempt = entry("/wda/keys");
    exempt.auth = AuthRequirement::Exempt;
    let error = routes::validate(&RouteContract {
        schema_version: 1,
        routes: vec![exempt],
    })
    .unwrap_err();
    assert!(error.to_string().contains("GET /status"));

    let mut required = entry("/wda/keys");
    required.session = SessionRequirement::Required;
    let error = routes::validate(&RouteContract {
        schema_version: 1,
        routes: vec![required],
    })
    .unwrap_err();
    assert!(error.to_string().contains("{sessionId}"));

    let duplicate = entry("/wda/tap");
    let error = routes::validate(&RouteContract {
        schema_version: 1,
        routes: vec![duplicate.clone(), duplicate],
    })
    .unwrap_err();
    assert!(error.to_string().contains("duplicate"));

    let mut get_with_body = entry("/status");
    get_with_body.method = HttpMethod::Get;
    let error = routes::validate(&RouteContract {
        schema_version: 1,
        routes: vec![get_with_body],
    })
    .unwrap_err();
    assert!(error.to_string().contains("must not define a request body"));

    let mut post_without_body = entry("/wda/tap");
    post_without_body.request_body = None;
    let error = routes::validate(&RouteContract {
        schema_version: 1,
        routes: vec![post_without_body],
    })
    .unwrap_err();
    assert!(error.to_string().contains("must define a request body"));
}

#[test]
fn route_contract_rejects_unknown_http_methods_during_parsing() {
    let error = serde_json::from_str::<RouteContract>(
        r#"{"schemaVersion":1,"routes":[{"method":"TRACE","path":"/status","auth":"exempt","session":"none","evidence":"fixture"}]}"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("unknown variant"));
}

#[test]
fn route_cross_check_marks_path_confirmed_documented_and_oracle_only_paths() {
    let contract = RouteContract {
        schema_version: 1,
        routes: vec![entry("/status"), entry("/documented")],
    };
    let checks = routes::cross_check(
        &contract,
        ["/status".into(), "/session/live/wda/custom".into()],
        BTreeSet::<String>::new(),
    )
    .unwrap();

    assert!(checks.iter().any(|check| {
        check.path == "/status" && check.status == RouteEvidenceStatus::PathConfirmed
    }));
    assert!(checks.iter().any(|check| {
        check.path == "/documented" && check.status == RouteEvidenceStatus::DocumentedOnly
    }));
    assert!(checks.iter().any(|check| {
        check.path == "/session/{sessionId}/wda/custom"
            && check.status == RouteEvidenceStatus::OracleOnly
    }));
}

#[test]
fn route_cross_check_uses_baseline_only_routes_as_baseline_evidence() {
    let contract = RouteContract {
        schema_version: 1,
        routes: vec![entry("/session/{sessionId}/wda/keys")],
    };
    let checks = routes::cross_check(
        &contract,
        BTreeSet::<String>::new(),
        ["/session/{sessionId}/wda/keys".to_owned()],
    )
    .unwrap();

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].status, RouteEvidenceStatus::PathConfirmed);
    assert_eq!(checks[0].evidence, "WDA baseline");
    assert_eq!(checks[0].contract_evidence.as_deref(), Some("fixture.rs"));
}

#[test]
fn route_path_evidence_does_not_claim_method_or_schema_confirmation() {
    let mut documented = entry("/wda/tap");
    documented.method = HttpMethod::Delete;
    documented.request_body = None;
    let contract = RouteContract {
        schema_version: 1,
        routes: vec![documented],
    };

    let checks = routes::cross_check(
        &contract,
        ["/wda/tap".to_owned()],
        BTreeSet::<String>::new(),
    )
    .unwrap();

    assert_eq!(checks[0].status, RouteEvidenceStatus::PathConfirmed);
    assert_eq!(checks[0].evidence, "oracle Mach-O");
}

#[test]
fn route_cross_check_preserves_undocumented_shared_and_baseline_only_paths() {
    let contract = RouteContract {
        schema_version: 1,
        routes: vec![entry("/documented")],
    };

    let checks = routes::cross_check(
        &contract,
        ["/shared".to_owned(), "/oracle-only".to_owned()],
        ["/shared".to_owned(), "/baseline-only".to_owned()],
    )
    .unwrap();

    assert!(checks.iter().any(|check| {
        check.path == "/shared"
            && check.method.is_none()
            && check.status == RouteEvidenceStatus::PathConfirmed
            && check.evidence == "oracle Mach-O + WDA baseline"
    }));
    assert!(checks.iter().any(|check| {
        check.path == "/oracle-only" && check.status == RouteEvidenceStatus::OracleOnly
    }));
    assert!(checks.iter().any(|check| {
        check.path == "/baseline-only" && check.status == RouteEvidenceStatus::BaselineOnly
    }));
    assert!(checks
        .iter()
        .filter(|check| check.method.is_none())
        .all(|check| check.contract_evidence.is_none()));
}
