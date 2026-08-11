use helix_worktree::{BranchRef, BranchSource, available_branches, create_worktree};
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempRepo {
  sandbox: PathBuf,
  path: PathBuf,
}

impl TempRepo {
  fn new(name: &str) -> Self {
    let sandbox =
      std::env::temp_dir().join(format!("helix-worktree-test-{name}-{}", std::process::id()));
    let path = sandbox.join("project");
    let _ = std::fs::remove_dir_all(&sandbox);

    std::fs::create_dir_all(&path).unwrap();

    git(&path, &["init", "--initial-branch=main"]);
    git(&path, &["config", "user.name", "Helix Test"]);
    git(&path, &["config", "user.email", "test@helix.local"]);
    git(&path, &["config", "commit.gpgsign", "false"]);

    std::fs::write(path.join("readme.md"), "helix").unwrap();

    git(&path, &["add", "-A"]);
    git(&path, &["commit", "-m", "initial"]);

    Self { sandbox, path }
  }

  fn with_remote(self) -> Self {
    let remote = self.sandbox.join("origin.git");

    git(&self.sandbox, &["init", "--bare", "origin.git"]);
    git(
      &self.path,
      &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&self.path, &["branch", "feat/remote-only"]);
    git(&self.path, &["push", "origin", "main", "feat/remote-only"]);
    git(&self.path, &["branch", "-D", "feat/remote-only"]);
    git(&self.path, &["fetch", "origin"]);

    self
  }
}

impl Drop for TempRepo {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.sandbox);
  }
}

fn git(cwd: &Path, args: &[&str]) {
  let output = Command::new("git")
    .current_dir(cwd)
    .args(args)
    .output()
    .unwrap();

  assert!(
    output.status.success(),
    "git {args:?} failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

fn labels(root: &Path) -> Vec<String> {
  available_branches(root)
    .iter()
    .map(BranchRef::label)
    .collect()
}

fn branch_of(root: &Path) -> String {
  let output = Command::new("git")
    .current_dir(root)
    .args(["rev-parse", "--abbrev-ref", "HEAD"])
    .output()
    .unwrap();

  String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn checked_out_branches_are_not_available() {
  let repo = TempRepo::new("checked-out");

  git(&repo.path, &["branch", "feat/local"]);

  assert_eq!(labels(&repo.path), vec!["feat/local".to_string()]);
}

#[test]
fn remote_branches_without_a_local_counterpart_are_available() {
  let repo = TempRepo::new("remote-only").with_remote();

  assert_eq!(
    labels(&repo.path),
    vec!["origin/feat/remote-only".to_string()]
  );
}

#[test]
fn an_existing_local_branch_becomes_the_worktree_head() {
  let repo = TempRepo::new("existing-local");

  git(&repo.path, &["branch", "feat/local"]);

  let source = BranchSource::Existing(BranchRef {
    name: "feat/local".to_string(),
    remote: None,
  });
  let dest = create_worktree(&repo.path, "feat/local", &source).unwrap();

  assert!(dest.is_dir());
  assert_eq!(branch_of(&dest), "feat/local");
  assert!(labels(&repo.path).is_empty());
}

#[test]
fn a_remote_branch_is_tracked_by_a_new_local_branch() {
  let repo = TempRepo::new("existing-remote").with_remote();

  let source = BranchSource::Existing(BranchRef {
    name: "feat/remote-only".to_string(),
    remote: Some("origin".to_string()),
  });
  let dest = create_worktree(&repo.path, "remote-only", &source).unwrap();

  assert_eq!(branch_of(&dest), "feat/remote-only");

  let upstream = Command::new("git")
    .current_dir(&dest)
    .args(["rev-parse", "--abbrev-ref", "feat/remote-only@{upstream}"])
    .output()
    .unwrap();

  assert_eq!(
    String::from_utf8_lossy(&upstream.stdout).trim(),
    "origin/feat/remote-only"
  );
}

#[test]
fn a_new_branch_is_created_from_the_worktree_name() {
  let repo = TempRepo::new("new-branch");

  let dest = create_worktree(&repo.path, "payments", &BranchSource::New(None)).unwrap();

  assert_eq!(branch_of(&dest), "payments");
}
