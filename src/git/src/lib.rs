pub mod diff;
pub mod index;
pub mod remote;

use anyhow::Result;
use git2::{Repository, Status, StatusOptions};
use helix_models::{CommitInfo, GitFileKind, GitFileStatus, GitSnapshot};
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub fn snapshot(root: &Path) -> Result<GitSnapshot> {
  let mut repo = Repository::discover(root)?;
  let mut snap = GitSnapshot::default();

  snap.is_linked_worktree = repo.is_worktree();
  if snap.is_linked_worktree {
    snap.main_repo = repo.path().ancestors().nth(3).map(|p| p.to_path_buf());
  }

  match repo.head() {
    Ok(head) => {
      let oid = head.target();
      snap.head_short = oid.map(|o| o.to_string()[..7].to_string());
      if head.is_branch() {
        snap.branch = head.shorthand().unwrap_or("HEAD").to_string();
      } else {
        snap.detached = true;
        snap.branch = format!("detached @ {}", snap.head_short.clone().unwrap_or_default());
      }
    }
    Err(_) => {
      snap.branch = repo
        .find_reference("HEAD")
        .ok()
        .and_then(|head| {
          head.symbolic_target().ok().flatten().map(|target| {
            target
              .strip_prefix("refs/heads/")
              .unwrap_or(target)
              .to_string()
          })
        })
        .unwrap_or_else(|| "no commits yet".to_string());
    }
  }

  collect_statuses(&repo, &mut snap);
  apply_line_counts(&repo, &mut snap);
  collect_commits(&repo, &mut snap);
  collect_ahead_behind(&repo, &mut snap);

  let mut count = 0usize;
  let _ = repo.stash_foreach(|_, _, _| {
    count += 1;
    true
  });
  snap.stash_count = count;

  Ok(snap)
}

/// Discovering the repository is the expensive half of an ignore check, so a
/// caller walking many directories opens one probe and reuses it.
pub struct IgnoreProbe {
  repo: Repository,
  workdir: std::path::PathBuf,
}

impl IgnoreProbe {
  pub fn open(root: &Path) -> Option<Self> {
    let repo = Repository::discover(root).ok()?;
    let workdir = repo.workdir()?.to_path_buf();
    let workdir = workdir.canonicalize().unwrap_or(workdir);

    Some(Self { repo, workdir })
  }

  pub fn is_ignored(&self, path: &Path) -> bool {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let relative = canonical.strip_prefix(&self.workdir).unwrap_or(&canonical);
    let mut probe = relative.to_string_lossy().to_string();

    if probe.is_empty() {
      return false;
    }

    if path.is_dir() && !probe.ends_with('/') {
      probe.push('/');
    }

    self.repo.is_path_ignored(Path::new(&probe)).unwrap_or(false)
  }
}

pub fn ignored_paths(
  root: &Path,
  candidates: &[std::path::PathBuf],
) -> HashSet<std::path::PathBuf> {
  let Some(probe) = IgnoreProbe::open(root) else {
    return HashSet::new();
  };

  candidates
    .iter()
    .filter(|path| probe.is_ignored(path))
    .cloned()
    .collect()
}

fn collect_statuses(repo: &Repository, snap: &mut GitSnapshot) {
  let mut opts = StatusOptions::new();
  opts
    .include_untracked(true)
    .recurse_untracked_dirs(true)
    .renames_head_to_index(true)
    .exclude_submodules(true);

  let Ok(statuses) = repo.statuses(Some(&mut opts)) else {
    return;
  };

  for entry in statuses.iter() {
    let st = entry.status();
    let path = entry.path().unwrap_or_default().to_string();

    if st.contains(Status::CONFLICTED) {
      snap
        .conflicted
        .push(GitFileStatus::new(path, GitFileKind::Conflicted));
      continue;
    }

    if st.contains(Status::WT_NEW) {
      snap
        .untracked
        .push(GitFileStatus::new(path.clone(), GitFileKind::Untracked));
    }

    for (bit, kind) in [
      (Status::INDEX_NEW, GitFileKind::Added),
      (Status::INDEX_MODIFIED, GitFileKind::Modified),
      (Status::INDEX_DELETED, GitFileKind::Deleted),
      (Status::INDEX_RENAMED, GitFileKind::Renamed),
      (Status::INDEX_TYPECHANGE, GitFileKind::Typechange),
    ] {
      if st.contains(bit) {
        snap.staged.push(GitFileStatus::new(path.clone(), kind));
      }
    }

    for (bit, kind) in [
      (Status::WT_MODIFIED, GitFileKind::Modified),
      (Status::WT_DELETED, GitFileKind::Deleted),
      (Status::WT_RENAMED, GitFileKind::Renamed),
      (Status::WT_TYPECHANGE, GitFileKind::Typechange),
    ] {
      if st.contains(bit) {
        snap.unstaged.push(GitFileStatus::new(path.clone(), kind));
      }
    }
  }
}

const MAX_COUNTED_DELTAS: usize = 500;

fn line_counts(repo: &Repository, staged: bool) -> HashMap<String, (usize, usize)> {
  let mut out = HashMap::new();

  let mut opts = git2::DiffOptions::new();
  opts
    .include_untracked(true)
    .recurse_untracked_dirs(true)
    .show_untracked_content(true)
    .include_typechange(true);

  let diff = if staged {
    let tree = repo.head().and_then(|head| head.peel_to_tree()).ok();
    repo.diff_tree_to_index(tree.as_ref(), None, Some(&mut opts))
  } else {
    repo.diff_index_to_workdir(None, Some(&mut opts))
  };
  let Ok(diff) = diff else { return out };

  for (ix, delta) in diff.deltas().enumerate().take(MAX_COUNTED_DELTAS) {
    let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
      continue;
    };
    let Ok(Some(patch)) = git2::Patch::from_diff(&diff, ix) else {
      continue;
    };
    let Ok((_, added, removed)) = patch.line_stats() else {
      continue;
    };
    out.insert(path.to_string_lossy().to_string(), (added, removed));
  }

  out
}

fn apply_line_counts(repo: &Repository, snap: &mut GitSnapshot) {
  let staged = line_counts(repo, true);
  let workdir = line_counts(repo, false);

  for file in snap.staged.iter_mut() {
    if let Some((added, removed)) = staged.get(&file.path) {
      file.added = *added;
      file.removed = *removed;
    }
  }

  for file in snap
    .unstaged
    .iter_mut()
    .chain(snap.untracked.iter_mut())
    .chain(snap.conflicted.iter_mut())
  {
    if let Some((added, removed)) = workdir.get(&file.path) {
      file.added = *added;
      file.removed = *removed;
    }
  }
}

fn collect_commits(repo: &Repository, snap: &mut GitSnapshot) {
  let Ok(mut walk) = repo.revwalk() else {
    return;
  };

  if walk.push_head().is_err() {
    return;
  }

  for oid in walk.take(15).flatten() {
    if let Ok(commit) = repo.find_commit(oid) {
      snap.recent_commits.push(CommitInfo {
        short_id: oid.to_string()[..7].to_string(),
        summary: commit
          .summary()
          .ok()
          .flatten()
          .unwrap_or_default()
          .to_string(),
        author: commit.author().name().unwrap_or_default().to_string(),
        epoch_seconds: commit.time().seconds(),
      });
    }
  }
}

fn collect_ahead_behind(repo: &Repository, snap: &mut GitSnapshot) {
  let Ok(head) = repo.head() else { return };

  if !head.is_branch() {
    return;
  }

  let Ok(name) = head.shorthand() else { return };
  let Ok(branch) = repo.find_branch(name, git2::BranchType::Local) else {
    return;
  };
  let Ok(upstream) = branch.upstream() else {
    return;
  };

  snap.upstream = upstream.name().ok().flatten().map(|name| name.to_string());

  let (Some(local), Some(remote)) = (head.target(), upstream.get().target()) else {
    return;
  };

  if let Ok((ahead, behind)) = repo.graph_ahead_behind(local, remote) {
    snap.ahead = ahead;
    snap.behind = behind;
  }
}
