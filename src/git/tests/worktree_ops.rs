use helix_models::{DiffBase, DiffLineKind, DiffState};
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempRepo {
  path: PathBuf,
}

impl TempRepo {
  fn new(name: &str) -> Self {
    let path = std::env::temp_dir().join(format!("helix-git-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    git(&path, &["init", "--initial-branch=main"]);
    git(&path, &["config", "user.name", "Helix Test"]);
    git(&path, &["config", "user.email", "test@helix.local"]);
    git(&path, &["config", "commit.gpgsign", "false"]);
    Self { path }
  }

  fn write(&self, relative: &str, contents: &str) {
    std::fs::write(self.path.join(relative), contents).unwrap();
  }

  fn commit_all(&self, message: &str) {
    git(&self.path, &["add", "-A"]);
    git(&self.path, &["commit", "-m", message]);
  }
}

impl Drop for TempRepo {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.path);
  }
}

fn git(cwd: &Path, args: &[&str]) {
  let output = Command::new("git")
    .args(args)
    .current_dir(cwd)
    .output()
    .unwrap();
  assert!(
    output.status.success(),
    "git {args:?} failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

#[test]
fn unstaged_diff_matches_working_tree_edit() {
  let repo = TempRepo::new("unstaged");
  repo.write("main.rs", "fn main() {\n    one();\n    two();\n}\n");
  repo.commit_all("initial");
  repo.write("main.rs", "fn main() {\n    uno();\n    two();\n}\n");

  let diff = helix_git::diff::file_diff(&repo.path, "main.rs", &DiffBase::Unstaged).unwrap();
  assert_eq!(diff.state, DiffState::Text);
  assert_eq!((diff.added, diff.removed), (1, 1));
  assert_eq!(diff.language, "rust");

  let lines = &diff.hunks[0].lines;
  let removed = lines
    .iter()
    .find(|line| line.kind == DiffLineKind::Removed)
    .unwrap();
  let added = lines
    .iter()
    .find(|line| line.kind == DiffLineKind::Added)
    .unwrap();
  assert_eq!(diff.line_text(removed), "    one();");
  assert_eq!(diff.line_text(added), "    uno();");
  assert_eq!(removed.old_line, Some(2));
  assert_eq!(added.new_line, Some(2));
}

#[test]
fn staged_diff_only_shows_index_changes() {
  let repo = TempRepo::new("staged");
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");

  repo.write("a.txt", "two\n");
  helix_git::index::stage(&repo.path, "a.txt").unwrap();
  repo.write("a.txt", "three\n");

  let staged = helix_git::diff::file_diff(&repo.path, "a.txt", &DiffBase::Staged).unwrap();
  assert_eq!((staged.added, staged.removed), (1, 1));
  let added = staged
    .hunks
    .iter()
    .flat_map(|hunk| hunk.lines.iter())
    .find(|line| line.kind == DiffLineKind::Added)
    .unwrap();
  assert_eq!(staged.line_text(added), "two");

  let unstaged = helix_git::diff::file_diff(&repo.path, "a.txt", &DiffBase::Unstaged).unwrap();
  let added = unstaged
    .hunks
    .iter()
    .flat_map(|hunk| hunk.lines.iter())
    .find(|line| line.kind == DiffLineKind::Added)
    .unwrap();
  assert_eq!(unstaged.line_text(added), "three");
}

#[test]
fn head_diff_sees_new_file_as_all_added() {
  let repo = TempRepo::new("newfile");
  repo.write("seed.txt", "seed\n");
  repo.commit_all("initial");
  repo.write("fresh.txt", "alpha\nbeta\n");

  let diff = helix_git::diff::file_diff(&repo.path, "fresh.txt", &DiffBase::Head).unwrap();
  assert_eq!((diff.added, diff.removed), (2, 0));
}

#[test]
fn identical_content_reports_identical() {
  let repo = TempRepo::new("identical");
  repo.write("same.txt", "unchanged\n");
  repo.commit_all("initial");

  let diff = helix_git::diff::file_diff(&repo.path, "same.txt", &DiffBase::Unstaged).unwrap();
  assert_eq!(diff.state, DiffState::Identical);
  assert!(diff.hunks.is_empty());
}

#[test]
fn binary_file_is_reported_as_binary() {
  let repo = TempRepo::new("binary");
  std::fs::write(repo.path.join("blob.bin"), [0u8, 1, 2, 3]).unwrap();
  repo.commit_all("initial");
  std::fs::write(repo.path.join("blob.bin"), [0u8, 9, 9, 9]).unwrap();

  let diff = helix_git::diff::file_diff(&repo.path, "blob.bin", &DiffBase::Unstaged).unwrap();
  assert_eq!(diff.state, DiffState::Binary);
}

#[test]
fn stage_then_commit_advances_head() {
  let repo = TempRepo::new("commit");
  repo.write("first.txt", "one\n");
  repo.commit_all("initial");

  let before = helix_git::snapshot(&repo.path).unwrap();
  repo.write("second.txt", "two\n");
  helix_git::index::stage(&repo.path, "second.txt").unwrap();

  let staged = helix_git::snapshot(&repo.path).unwrap();
  assert_eq!(staged.staged.len(), 1);
  assert_eq!(staged.staged[0].path, "second.txt");

  let oid = helix_git::index::commit(&repo.path, "add second").unwrap();
  let after = helix_git::snapshot(&repo.path).unwrap();
  assert_ne!(before.head_short, after.head_short);
  assert!(oid.starts_with(after.head_short.as_deref().unwrap()));
  assert_eq!(after.dirty_count(), 0);
  assert_eq!(after.recent_commits[0].summary, "add second");
}

#[test]
fn unstage_returns_file_to_working_tree() {
  let repo = TempRepo::new("unstage");
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");

  repo.write("a.txt", "two\n");
  helix_git::index::stage(&repo.path, "a.txt").unwrap();
  assert_eq!(helix_git::snapshot(&repo.path).unwrap().staged.len(), 1);

  helix_git::index::unstage(&repo.path, "a.txt").unwrap();
  let snapshot = helix_git::snapshot(&repo.path).unwrap();
  assert!(snapshot.staged.is_empty());
  assert_eq!(snapshot.unstaged.len(), 1);
}

#[test]
fn unstage_all_clears_the_index_without_touching_the_working_tree() {
  let repo = TempRepo::new("unstage-all");
  repo.write("tracked.txt", "one\n");
  repo.commit_all("initial");

  repo.write("tracked.txt", "two\n");
  repo.write("fresh.txt", "new\n");
  helix_git::index::stage_all(&repo.path).unwrap();
  assert_eq!(helix_git::snapshot(&repo.path).unwrap().staged.len(), 2);

  helix_git::index::unstage_all(&repo.path).unwrap();
  let snapshot = helix_git::snapshot(&repo.path).unwrap();
  assert!(snapshot.staged.is_empty());
  assert_eq!(snapshot.unstaged.len(), 1);
  assert_eq!(snapshot.untracked.len(), 1);

  assert_eq!(
    std::fs::read_to_string(repo.path.join("tracked.txt")).unwrap(),
    "two\n"
  );
  assert!(repo.path.join("fresh.txt").exists());
}

#[test]
fn unstage_all_on_a_repo_without_commits_clears_everything() {
  let path = std::env::temp_dir().join(format!("helix-unstage-empty-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&path);
  std::fs::create_dir_all(&path).unwrap();
  git(&path, &["init", "--initial-branch=main"]);
  std::fs::write(path.join("a.txt"), "one\n").unwrap();
  helix_git::index::stage_all(&path).unwrap();
  assert_eq!(helix_git::snapshot(&path).unwrap().staged.len(), 1);

  helix_git::index::unstage_all(&path).unwrap();
  let snapshot = helix_git::snapshot(&path).unwrap();
  assert!(snapshot.staged.is_empty());
  assert_eq!(snapshot.untracked.len(), 1);
  let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn commit_with_empty_message_is_rejected() {
  let repo = TempRepo::new("emptymsg");
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");
  repo.write("a.txt", "two\n");
  helix_git::index::stage(&repo.path, "a.txt").unwrap();

  let err = helix_git::index::commit(&repo.path, "   ").unwrap_err();
  assert!(err.to_string().contains("empty commit message"));
}

#[test]
fn commit_with_nothing_staged_is_rejected() {
  let repo = TempRepo::new("nothing");
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");

  let err = helix_git::index::commit(&repo.path, "no-op").unwrap_err();
  assert!(err.to_string().contains("nothing to commit"));
}

#[test]
fn untracked_files_are_listed_individually_not_as_directories() {
  let repo = TempRepo::new("untracked-recurse");
  repo.write("seed.txt", "seed\n");
  repo.commit_all("initial");

  std::fs::create_dir_all(repo.path.join("nested/deep")).unwrap();
  repo.write("nested/one.rs", "fn one() {}\n");
  repo.write("nested/deep/two.rs", "fn two() {}\n");

  let snapshot = helix_git::snapshot(&repo.path).unwrap();
  let paths: Vec<&str> = snapshot
    .untracked
    .iter()
    .map(|file| file.path.as_str())
    .collect();

  assert!(
    paths.contains(&"nested/one.rs"),
    "expected nested/one.rs, got {paths:?}"
  );
  assert!(
    paths.contains(&"nested/deep/two.rs"),
    "expected nested/deep/two.rs, got {paths:?}"
  );
  assert!(
    !paths.iter().any(|path| path.ends_with('/')),
    "no entry should be a collapsed directory, got {paths:?}"
  );
}

#[test]
fn nested_untracked_file_can_be_staged_and_diffed() {
  let repo = TempRepo::new("nested-ops");
  repo.write("seed.txt", "seed\n");
  repo.commit_all("initial");
  std::fs::create_dir_all(repo.path.join("ui/src")).unwrap();
  repo.write("ui/src/view.rs", "fn view() {}\n");

  let diff = helix_git::diff::file_diff(&repo.path, "ui/src/view.rs", &DiffBase::Head).unwrap();
  assert_eq!(diff.state, DiffState::Text);
  assert_eq!((diff.added, diff.removed), (1, 0));

  helix_git::index::stage(&repo.path, "ui/src/view.rs").unwrap();
  let snapshot = helix_git::snapshot(&repo.path).unwrap();
  assert_eq!(snapshot.staged.len(), 1);
  assert_eq!(snapshot.staged[0].path, "ui/src/view.rs");
  assert!(snapshot.untracked.is_empty());
}

#[test]
fn gitignored_paths_are_reported_as_ignored() {
  let repo = TempRepo::new("ignored");
  repo.write(".gitignore", "/target\n*.log\n");
  repo.commit_all("initial");

  std::fs::create_dir_all(repo.path.join("target/debug")).unwrap();
  repo.write("target/debug/binary", "junk\n");
  repo.write("noise.log", "junk\n");
  repo.write("keep.rs", "fn keep() {}\n");

  let candidates = vec![
    repo.path.join("target"),
    repo.path.join("target/debug/binary"),
    repo.path.join("noise.log"),
    repo.path.join("keep.rs"),
  ];
  let ignored = helix_git::ignored_paths(&repo.path, &candidates);

  assert!(ignored.contains(&repo.path.join("target")));
  assert!(ignored.contains(&repo.path.join("target/debug/binary")));
  assert!(ignored.contains(&repo.path.join("noise.log")));
  assert!(!ignored.contains(&repo.path.join("keep.rs")));
}

#[test]
fn gitignored_files_stay_out_of_the_status_snapshot() {
  let repo = TempRepo::new("ignored-status");
  repo.write(".gitignore", "/target\n");
  repo.commit_all("initial");
  std::fs::create_dir_all(repo.path.join("target")).unwrap();
  repo.write("target/artifact", "junk\n");
  repo.write("tracked.rs", "fn main() {}\n");

  let snapshot = helix_git::snapshot(&repo.path).unwrap();
  let paths: Vec<&str> = snapshot
    .untracked
    .iter()
    .map(|file| file.path.as_str())
    .collect();
  assert_eq!(paths, vec!["tracked.rs"]);
}

#[test]
fn branch_diff_uses_merge_base() {
  let repo = TempRepo::new("branch");
  repo.write("a.txt", "base\n");
  repo.commit_all("initial");
  git(&repo.path, &["checkout", "-b", "feature"]);
  repo.write("a.txt", "feature\n");
  repo.commit_all("feature change");

  let (merge_base, head) = helix_git::diff::merge_base(&repo.path, "main").unwrap();
  let diff =
    helix_git::diff::file_diff(&repo.path, "a.txt", &DiffBase::Branch { merge_base, head })
      .unwrap();
  assert_eq!((diff.added, diff.removed), (1, 1));

  let ahead = helix_git::remote::commits_ahead_of(&repo.path, "main").unwrap();
  assert_eq!(ahead, 1);
}

#[test]
fn snapshot_counts_added_and_removed_lines() {
  let repo = TempRepo::new("linecounts");
  repo.write("kept.txt", "one\ntwo\nthree\n");
  repo.commit_all("initial");
  repo.write("kept.txt", "one\nTWO\nthree\nfour\n");
  repo.write("fresh.txt", "a\nb\n");

  let snap = helix_git::snapshot(&repo.path).unwrap();
  let modified = snap
    .unstaged
    .iter()
    .find(|file| file.path == "kept.txt")
    .expect("kept.txt should be unstaged");
  assert_eq!((modified.added, modified.removed), (2, 1));

  let untracked = snap
    .untracked
    .iter()
    .find(|file| file.path == "fresh.txt")
    .expect("fresh.txt should be untracked");
  assert_eq!((untracked.added, untracked.removed), (2, 0));
}

#[test]
fn snapshot_counts_staged_lines_separately_from_the_worktree() {
  let repo = TempRepo::new("stagedcounts");
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");
  repo.write("a.txt", "one\ntwo\n");
  helix_git::index::stage(&repo.path, "a.txt").unwrap();
  repo.write("a.txt", "one\ntwo\nthree\n");

  let snap = helix_git::snapshot(&repo.path).unwrap();
  let staged = snap
    .staged
    .iter()
    .find(|file| file.path == "a.txt")
    .expect("a.txt should be staged");
  assert_eq!((staged.added, staged.removed), (1, 0), "index vs HEAD");
  let unstaged = snap
    .unstaged
    .iter()
    .find(|file| file.path == "a.txt")
    .expect("a.txt should also be unstaged");
  assert_eq!(
    (unstaged.added, unstaged.removed),
    (1, 0),
    "worktree vs index"
  );
}

#[test]
fn discard_restores_a_tracked_file_from_the_index() {
  let repo = TempRepo::new("discard-tracked");
  repo.write("a.txt", "original\n");
  repo.commit_all("initial");
  repo.write("a.txt", "ruined\n");

  helix_git::index::discard(&repo.path, "a.txt").unwrap();

  let contents = std::fs::read_to_string(repo.path.join("a.txt")).unwrap();
  assert_eq!(contents, "original\n");
  let snap = helix_git::snapshot(&repo.path).unwrap();
  assert!(snap.unstaged.is_empty(), "{:?}", snap.unstaged);
}

#[test]
fn discard_keeps_staged_work_and_only_drops_the_worktree_edit() {
  let repo = TempRepo::new("discard-staged");
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");
  repo.write("a.txt", "one\ntwo\n");
  helix_git::index::stage(&repo.path, "a.txt").unwrap();
  repo.write("a.txt", "one\ntwo\nthree\n");

  helix_git::index::discard(&repo.path, "a.txt").unwrap();

  let contents = std::fs::read_to_string(repo.path.join("a.txt")).unwrap();
  assert_eq!(
    contents, "one\ntwo\n",
    "should fall back to the index, not HEAD"
  );
  let snap = helix_git::snapshot(&repo.path).unwrap();
  assert_eq!(snap.staged.len(), 1, "staged work survives");
  assert!(snap.unstaged.is_empty());
}

#[test]
fn discard_deletes_an_untracked_file() {
  let repo = TempRepo::new("discard-untracked");
  repo.write("keep.txt", "keep\n");
  repo.commit_all("initial");
  repo.write("junk.txt", "junk\n");

  helix_git::index::discard(&repo.path, "junk.txt").unwrap();

  assert!(!repo.path.join("junk.txt").exists());
  assert!(repo.path.join("keep.txt").exists());
}

fn with_local_remote(name: &str) -> (TempRepo, PathBuf) {
  let remote = std::env::temp_dir().join(format!("helix-remote-{name}-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&remote);
  std::fs::create_dir_all(&remote).unwrap();
  git(&remote, &["init", "--bare", "--initial-branch=main"]);

  let repo = TempRepo::new(name);
  repo.write("a.txt", "one\n");
  repo.commit_all("initial");
  git(
    &repo.path,
    &["remote", "add", "origin", remote.to_str().unwrap()],
  );
  helix_git::remote::publish(&repo.path, "main").unwrap();
  (repo, remote)
}

#[test]
fn upstream_is_reported_after_publishing() {
  let (repo, remote) = with_local_remote("upstream");
  assert_eq!(
    helix_git::remote::upstream(&repo.path).as_deref(),
    Some("origin/main")
  );
  let _ = std::fs::remove_dir_all(remote);
}

#[test]
fn fetch_and_fast_forward_move_the_branch() {
  let (repo, remote) = with_local_remote("fastforward");

  let other = std::env::temp_dir().join(format!("helix-clone-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&other);
  git(
    &std::env::temp_dir(),
    &["clone", remote.to_str().unwrap(), other.to_str().unwrap()],
  );
  git(&other, &["config", "user.name", "Other"]);
  git(&other, &["config", "user.email", "other@helix.local"]);
  std::fs::write(other.join("b.txt"), "two\n").unwrap();
  git(&other, &["add", "-A"]);
  git(&other, &["commit", "-m", "from elsewhere"]);
  git(&other, &["push"]);

  helix_git::remote::fetch(&repo.path).unwrap();
  assert_eq!(
    helix_git::snapshot(&repo.path).unwrap().behind,
    1,
    "fetch should reveal the remote commit"
  );

  helix_git::remote::fast_forward(&repo.path).unwrap();
  let snap = helix_git::snapshot(&repo.path).unwrap();
  assert_eq!((snap.ahead, snap.behind), (0, 0));
  assert!(repo.path.join("b.txt").exists());

  let _ = std::fs::remove_dir_all(remote);
  let _ = std::fs::remove_dir_all(other);
}

#[test]
fn fast_forward_refuses_to_merge_divergent_history() {
  let (repo, remote) = with_local_remote("divergent");

  let other = std::env::temp_dir().join(format!("helix-clone-div-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&other);
  git(
    &std::env::temp_dir(),
    &["clone", remote.to_str().unwrap(), other.to_str().unwrap()],
  );
  git(&other, &["config", "user.name", "Other"]);
  git(&other, &["config", "user.email", "other@helix.local"]);
  std::fs::write(other.join("b.txt"), "theirs\n").unwrap();
  git(&other, &["add", "-A"]);
  git(&other, &["commit", "-m", "theirs"]);
  git(&other, &["push"]);

  repo.write("c.txt", "mine\n");
  repo.commit_all("mine");
  helix_git::remote::fetch(&repo.path).unwrap();

  assert!(
    helix_git::remote::fast_forward(&repo.path).is_err(),
    "diverged branches cannot fast-forward"
  );

  let _ = std::fs::remove_dir_all(remote);
  let _ = std::fs::remove_dir_all(other);
}
