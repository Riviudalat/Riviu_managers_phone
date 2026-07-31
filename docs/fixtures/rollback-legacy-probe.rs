use riviu_core::db::Database;
use riviu_script_engine::parse_script;

#[test]
fn pre_f0_core_reads_legacy_rows_from_release_migrated_database() {
    let path = std::env::var_os("RIVIU_ROLLBACK_PROOF_DB")
        .expect("RIVIU_ROLLBACK_PROOF_DB");
    let database = Database::open(path).expect("pre-F0 opens migrated database");
    let scripts = database.list_scripts().expect("pre-F0 list_scripts");
    let (_, body) = scripts
        .iter()
        .find(|(name, _)| name == "fixture")
        .expect("fixture script");
    let script = parse_script(body).expect("pre-F0 parses the unchanged v1 script");
    assert_eq!(script.name, "fixture");
    assert_eq!(script.steps.len(), 1);
    let jobs = database.list_jobs(100).expect("pre-F0 list_jobs");
    assert!(jobs.iter().any(|job| {
        job.id.to_string() == "00000000-0000-0000-0000-000000000901"
            && job.script_name == "fixture"
    }));
}
