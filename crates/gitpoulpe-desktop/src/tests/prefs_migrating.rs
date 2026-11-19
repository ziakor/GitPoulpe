use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::prefs_migration::{
    CURRENT_FORMAT, FORMAT_KEY, Outcome, STEPS, Step, migrate_file, open_at,
};

fn legacy() -> Value {
    json!({
        "connections": {"items": [{"id": "gh", "provider": "github"}]},
        "favorites": {"items": [{"Local": {"path": "/r", "name": "r"}}]},
        "workspaces": {"items": [{"id": 1, "name": "w", "repos": []}], "next_id": 2, "active": 1},
        "mode": "split"
    })
}

fn put(dir: &Path, value: &Value) -> PathBuf {
    let path = dir.join("prefs.json");
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn read(path: &Path) -> Map<String, Value> {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn rename_key(map: &mut Map<String, Value>) -> Result<(), String> {
    let value = map.remove("mode").ok_or("missing mode")?;
    map.insert("diff_mode".to_owned(), value);
    Ok(())
}

fn identity(_: &mut Map<String, Value>) -> Result<(), String> {
    Ok(())
}

fn failing(_: &mut Map<String, Value>) -> Result<(), String> {
    Err("boom".to_owned())
}

#[test]
fn a_legacy_file_migrates_without_losing_anything_and_keeps_a_copy() {
    let dir = tempfile::tempdir().unwrap();
    let original = legacy();
    let path = put(dir.path(), &original);
    let before = std::fs::read(&path).unwrap();
    let Outcome::Migrated { from, backup } = migrate_file(&path, CURRENT_FORMAT, &STEPS) else {
        panic!("expected a migration");
    };
    assert_eq!(from, 0);
    assert_eq!(std::fs::read(&backup).unwrap(), before);
    let migrated = read(&path);
    assert_eq!(migrated[FORMAT_KEY], json!(CURRENT_FORMAT));
    for (key, value) in original.as_object().unwrap() {
        assert_eq!(&migrated[key], value, "{key}");
    }
    assert_eq!(
        migrate_file(&path, CURRENT_FORMAT, &STEPS),
        Outcome::Current
    );
}

#[test]
fn several_steps_run_in_order_from_the_stored_format() {
    let dir = tempfile::tempdir().unwrap();
    let steps: [Step; 2] = [identity, rename_key];
    let path = put(dir.path(), &legacy());
    assert!(matches!(
        migrate_file(&path, 2, &steps),
        Outcome::Migrated { from: 0, .. }
    ));
    let map = read(&path);
    assert_eq!(map["diff_mode"], json!("split"));
    assert!(!map.contains_key("mode"));
    assert_eq!(map[FORMAT_KEY], json!(2));
    let mut v1 = legacy();
    v1[FORMAT_KEY] = json!(1);
    let other = tempfile::tempdir().unwrap();
    let path = put(other.path(), &v1);
    assert!(matches!(
        migrate_file(&path, 2, &steps),
        Outcome::Migrated { from: 1, .. }
    ));
    assert_eq!(read(&path)["diff_mode"], json!("split"));
}

#[test]
fn an_invalid_file_keeps_a_recoverable_copy_and_never_overwrites_an_older_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    std::fs::write(&path, b"{ not json").unwrap();
    let Outcome::Unreadable { backup, reason } = migrate_file(&path, CURRENT_FORMAT, &STEPS) else {
        panic!("expected unreadable");
    };
    assert!(!reason.is_empty());
    assert_eq!(std::fs::read(&backup).unwrap(), b"{ not json");
    std::fs::write(&path, b"[1, 2]").unwrap();
    let Outcome::Unreadable { backup: second, .. } = migrate_file(&path, CURRENT_FORMAT, &STEPS)
    else {
        panic!("expected unreadable");
    };
    assert_ne!(backup, second);
    assert_eq!(std::fs::read(&backup).unwrap(), b"{ not json");
    assert_eq!(std::fs::read(&second).unwrap(), b"[1, 2]");
    let (_, warning) = open_at(&path);
    assert!(warning.unwrap().contains("illisibles"));
}

#[test]
fn a_failing_step_leaves_the_file_untouched_with_a_copy_and_blocks_saving() {
    let dir = tempfile::tempdir().unwrap();
    let steps: [Step; 1] = [failing];
    let path = put(dir.path(), &legacy());
    let before = std::fs::read(&path).unwrap();
    let Outcome::Failed { backup, reason } = migrate_file(&path, 1, &steps) else {
        panic!("expected a failure");
    };
    assert!(reason.contains("boom") && reason.contains("0 → 1"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(backup.unwrap()).unwrap(), before);
    assert!(
        !Outcome::Failed {
            backup: None,
            reason: String::new()
        }
        .writable()
    );
}

#[test]
fn an_interrupted_migration_is_simply_run_again_without_piling_up_copies() {
    let dir = tempfile::tempdir().unwrap();
    let path = put(dir.path(), &legacy());
    let before = std::fs::read(&path).unwrap();
    std::fs::write(dir.path().join("prefs.json.v0.bak"), &before).unwrap();
    std::fs::write(dir.path().join("prefs.json.migrating.99999"), b"{ half").unwrap();
    let Outcome::Migrated { backup, .. } = migrate_file(&path, CURRENT_FORMAT, &STEPS) else {
        panic!("expected a migration");
    };
    assert_eq!(backup, dir.path().join("prefs.json.v0.bak"));
    assert!(!dir.path().join("prefs.json.v0.bak.1").exists());
    assert_eq!(read(&path)[FORMAT_KEY], json!(CURRENT_FORMAT));
}

#[test]
fn a_newer_format_is_neither_overwritten_nor_silently_reset() {
    let dir = tempfile::tempdir().unwrap();
    let mut future = legacy();
    future[FORMAT_KEY] = json!(CURRENT_FORMAT + 5);
    future["brand_new_key"] = json!({"shape": "unknown"});
    let path = put(dir.path(), &future);
    let before = std::fs::read(&path).unwrap();
    assert_eq!(
        migrate_file(&path, CURRENT_FORMAT, &STEPS),
        Outcome::Newer {
            found: CURRENT_FORMAT + 5
        }
    );
    let (mut prefs, warning) = open_at(&path);
    assert!(warning.unwrap().contains("plus récente"));
    assert_eq!(prefs.get::<String>("mode").as_deref(), Some("split"));
    let _ = prefs.set("mode", "unified");
    let _ = prefs.set("anything", &1u8);
    assert!(!prefs.is_persistent());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(!dir.path().join("prefs.json.v0.bak").exists());
}

#[test]
fn an_invalid_format_number_is_refused_without_changes() {
    for bad in [json!("2"), json!(-1), json!(1.5), json!(null)] {
        let dir = tempfile::tempdir().unwrap();
        let mut value = legacy();
        value[FORMAT_KEY] = bad.clone();
        let path = put(dir.path(), &value);
        let before = std::fs::read(&path).unwrap();
        assert!(
            matches!(
                migrate_file(&path, CURRENT_FORMAT, &STEPS),
                Outcome::Failed { backup: None, .. }
            ),
            "{bad}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "{bad}");
    }
}

#[test]
fn a_new_install_is_stamped_and_a_current_file_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("prefs.json");
    assert_eq!(migrate_file(&path, CURRENT_FORMAT, &STEPS), Outcome::Fresh);
    let (prefs, warning) = open_at(&path);
    assert!(warning.is_none() && prefs.is_persistent());
    assert_eq!(read(&path)[FORMAT_KEY], json!(CURRENT_FORMAT));
    let stamp = std::fs::read(&path).unwrap();
    let (_prefs, warning) = open_at(&path);
    assert!(warning.is_none());
    assert_eq!(std::fs::read(&path).unwrap(), stamp);
}

#[cfg(unix)]
#[test]
fn the_resolved_location_is_the_one_the_toolkit_would_use() {
    let ours = crate::prefs_migration::prefs_path("gitpoulpe");
    let theirs = rune::prefs::Prefs::open("gitpoulpe")
        .path()
        .map(Path::to_path_buf);
    assert_eq!(ours, theirs);
}

#[test]
fn migrated_data_is_what_the_application_reads_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = put(dir.path(), &legacy());
    let (prefs, warning) = open_at(&path);
    assert!(warning.is_none());
    let book: gitpoulpe_core::WorkspaceBook = prefs.get("workspaces").unwrap();
    assert_eq!(book.items.len(), 1);
    assert_eq!(book.active, Some(1));
    assert_eq!(prefs.get::<String>("mode").as_deref(), Some("split"));
}

#[test]
fn a_preferences_problem_is_shown_to_the_user() {
    let mut state = crate::state::State::new(None, rune::prefs::Prefs::memory());
    state.prefs_warning = Some("Les préférences (x) sont illisibles".to_owned());
    let rt = super::runtime(state);
    assert!(
        rt.elements()
            .into_iter()
            .any(|e| e.text.as_deref().is_some_and(|t| t.contains("illisibles")))
    );
}
