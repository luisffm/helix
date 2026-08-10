use helix_state::config::{
  DiffBaseSnapshot, TabSnapshot, WorkspaceSession, set_workspace_session, workspace_session,
};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// `set_workspace_session` is a read-modify-write over one file, and the harness
/// runs these in parallel inside a single process — so they share both the config
/// path and the env var that points at it. Serialize them.
fn guard() -> MutexGuard<'static, ()> {
  static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
  let lock = LOCK
    .get_or_init(|| Mutex::new(()))
    .lock()
    .unwrap_or_else(|e| e.into_inner());
  let dir = std::env::temp_dir().join(format!("helix-session-cfg-{}", std::process::id()));
  std::fs::create_dir_all(&dir).unwrap();
  unsafe { std::env::set_var("HELIX_CONFIG_DIR", &dir) };
  let _ = std::fs::remove_file(dir.join("config.json"));
  lock
}

fn root(tag: &str) -> PathBuf {
  PathBuf::from(format!("/tmp/helix-session-test-{tag}"))
}

#[test]
fn round_trips_every_tab_kind() {
  let _guard = guard();
  let root = root("kinds");
  let tabs = vec![
    TabSnapshot::Terminal,
    TabSnapshot::Claude,
    TabSnapshot::Editor {
      path: PathBuf::from("/repo/ui/src/root.rs"),
    },
    TabSnapshot::Diff {
      relative: "git/src/index.rs".to_string(),
      base: DiffBaseSnapshot::Staged,
    },
  ];
  set_workspace_session(&root, tabs.clone(), 2);

  let stored = workspace_session(&root).expect("session should persist");
  assert_eq!(stored.tabs, tabs);
  assert_eq!(stored.active, 2);
}

#[test]
fn overwrites_an_existing_session_for_the_same_root() {
  let _guard = guard();
  let root = root("overwrite");
  set_workspace_session(&root, vec![TabSnapshot::Terminal], 0);
  set_workspace_session(&root, vec![TabSnapshot::Claude, TabSnapshot::Terminal], 1);

  let stored = workspace_session(&root).expect("session should persist");
  assert_eq!(stored.tabs.len(), 2);
  assert_eq!(stored.tabs[0], TabSnapshot::Claude);
  assert_eq!(stored.active, 1);
}

#[test]
fn does_not_create_an_entry_for_an_empty_tab_list() {
  let _guard = guard();
  let root = root("empty");
  set_workspace_session(&root, Vec::new(), 0);
  assert!(workspace_session(&root).is_none());
}

#[test]
fn keeps_sessions_of_other_roots_untouched() {
  let _guard = guard();
  let a = root("keep-a");
  let b = root("keep-b");
  set_workspace_session(&a, vec![TabSnapshot::Terminal], 0);
  set_workspace_session(&b, vec![TabSnapshot::Claude], 0);

  assert_eq!(
    workspace_session(&a).map(|s| s.tabs),
    Some(vec![TabSnapshot::Terminal])
  );
  assert_eq!(
    workspace_session(&b).map(|s| s.tabs),
    Some(vec![TabSnapshot::Claude])
  );
}

#[test]
fn snapshot_json_is_stable_and_self_describing() {
  let _guard = guard();
  let session = WorkspaceSession {
    root: PathBuf::from("/repo"),
    tabs: vec![
      TabSnapshot::Claude,
      TabSnapshot::Diff {
        relative: "a.rs".to_string(),
        base: DiffBaseSnapshot::Unstaged,
      },
    ],
    active: 0,
  };
  let json = serde_json::to_string(&session).unwrap();
  assert!(json.contains(r#""kind":"claude""#), "{json}");
  assert!(json.contains(r#""kind":"diff""#), "{json}");
  assert!(json.contains(r#""base":"unstaged""#), "{json}");

  let back: WorkspaceSession = serde_json::from_str(&json).unwrap();
  assert_eq!(back, session);
}
