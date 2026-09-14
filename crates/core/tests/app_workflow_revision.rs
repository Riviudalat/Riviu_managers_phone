use riviu_core::{app_workflow::*, db::Database, AutomationKind};
use uuid::Uuid;

#[test]
fn app_revisions_survive_reopen_and_invalid_updates_preserve_previous_bytes() {
    let path = std::env::temp_dir().join(format!("riviu-app-revision-{}.db", Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    let original = new_app_workflow(AutomationKind::Nurture);
    let first = db.save_app_workflow(original.clone(), None).unwrap();
    let mut second = first.clone();
    let like = second
        .nodes
        .iter_mut()
        .find(|node| node.action == "like")
        .unwrap();
    like.position.x += 52.0;
    like.config
        .insert("probability".into(), serde_json::json!(61));
    let second = db.save_app_workflow(second, Some(1)).unwrap();
    assert_eq!(second.revision, 2);
    assert!(db.save_app_workflow(first.clone(), Some(1)).is_err());
    let mut invalid = second.clone();
    invalid.edges.clear();
    assert!(db.save_app_workflow(invalid, Some(2)).is_err());
    drop(db);
    let db = Database::open(&path).unwrap();
    assert_eq!(
        db.get_app_workflow(first.id, Some(1)).unwrap().unwrap(),
        first
    );
    assert_eq!(
        db.get_app_workflow(first.id, None).unwrap().unwrap(),
        second
    );
    let compiled = compile_app_profile(&second).unwrap();
    assert_eq!(compiled["settings"]["likeProb"], 61);
    db.archive_app_workflow(first.id, 2).unwrap();
    assert!(db.list_app_workflows().unwrap().is_empty());
    assert!(db.get_app_workflow(first.id, Some(1)).unwrap().is_some());
}
