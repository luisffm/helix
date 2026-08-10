use helix_state::config::{ensure_project, project_for, set_display_name};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// `set_display_name` is a read-modify-write over one file, and the harness runs
/// these in parallel inside a single process — so they share both the config
/// path and the env var that points at it. Serialize them.
fn guard() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  let lock = LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .unwrap_or_else(|e| e.into_inner());

  let dir = std::env::temp_dir().join(format!("helix-label-cfg-{}", std::process::id()));

  std::fs::create_dir_all(&dir).unwrap();

  unsafe { std::env::set_var("HELIX_CONFIG_DIR", &dir) };

  let _ = std::fs::remove_file(dir.join("config.json"));

  lock
}

fn root(tag: &str) -> PathBuf {
  PathBuf::from(format!("/tmp/helix-label-test-{tag}"))
}

#[test]
fn label_falls_back_to_the_directory_name() {
  let _guard = guard();

  let root = root("fallback");

  ensure_project(&root);

  let project = project_for(&root).expect("project should exist");

  assert_eq!(project.label(), "helix-label-test-fallback");
}

#[test]
fn a_stored_name_wins_over_the_directory() {
  let _guard = guard();

  let root = root("stored");

  set_display_name(&root, "  Helix  ");

  let project = project_for(&root).expect("project should exist");

  assert_eq!(project.display_name.as_deref(), Some("Helix"));
  assert_eq!(project.label(), "Helix");
}

#[test]
fn an_empty_name_is_stored_as_absent() {
  let _guard = guard();

  let root = root("empty");

  set_display_name(&root, "Helix");
  set_display_name(&root, "   ");

  let project = project_for(&root).expect("project should exist");

  assert_eq!(project.display_name, None);
  assert_eq!(project.label(), "helix-label-test-empty");
}

#[test]
fn a_name_equal_to_the_directory_is_stored_as_absent() {
  let _guard = guard();

  let root = root("dir-equal");

  set_display_name(&root, "Helix");
  set_display_name(&root, "helix-label-test-dir-equal");

  let project = project_for(&root).expect("project should exist");

  assert_eq!(project.display_name, None);
}
