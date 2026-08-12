use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
  let status = Command::new("git")
    .args(args)
    .current_dir(dir)
    .status()
    .expect("git should run");

  assert!(status.success(), "git {args:?} failed");
}

fn repo(name: &str) -> std::path::PathBuf {
  let dir = std::env::temp_dir().join(format!("helix-line-stats-{name}-{}", std::process::id()));

  let _ = std::fs::remove_dir_all(&dir);
  std::fs::create_dir_all(&dir).unwrap();

  git(&dir, &["init", "-q", "-b", "main"]);
  git(&dir, &["config", "user.email", "test@helix"]);
  git(&dir, &["config", "user.name", "Test"]);
  std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
  git(&dir, &["add", "-A"]);
  git(&dir, &["commit", "-qm", "init"]);

  dir
}

#[test]
fn a_clean_worktree_counts_nothing() {
  let dir = repo("clean");

  assert_eq!(helix_git::line_stats(&dir).unwrap(), (0, 0));

  let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn staged_and_unstaged_work_are_counted_together() {
  let dir = repo("mixed");

  // one staged addition
  std::fs::write(dir.join("b.txt"), "new\n").unwrap();
  git(&dir, &["add", "b.txt"]);

  // and an unstaged edit that drops a line and adds two
  std::fs::write(dir.join("a.txt"), "one\ntwo\nfour\nfive\n").unwrap();

  let (added, removed) = helix_git::line_stats(&dir).unwrap();

  assert_eq!((added, removed), (3, 1));

  let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_untracked_file_counts_as_added() {
  let dir = repo("untracked");

  std::fs::write(dir.join("c.txt"), "x\ny\n").unwrap();

  assert_eq!(helix_git::line_stats(&dir).unwrap(), (2, 0));

  let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_index_stamp_moves_when_the_index_does() {
  let dir = repo("stamp");
  let before = helix_git::index_stamp(&dir).expect("a repository has an index");

  std::thread::sleep(std::time::Duration::from_millis(1100));
  std::fs::write(dir.join("d.txt"), "z\n").unwrap();
  git(&dir, &["add", "d.txt"]);

  let after = helix_git::index_stamp(&dir).expect("still has an index");

  assert!(after > before, "staging should touch the index");

  let _ = std::fs::remove_dir_all(&dir);
}
