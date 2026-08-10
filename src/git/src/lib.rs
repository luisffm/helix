pub mod diff;
pub mod index;
pub mod remote;

use anyhow::Result;
use git2::{Repository, Status, StatusOptions};
use helix_models::{CommitInfo, GitFileKind, GitFileStatus, GitSnapshot};
use std::collections::HashSet;
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

pub fn ignored_paths(
  root: &Path,
  candidates: &[std::path::PathBuf],
) -> HashSet<std::path::PathBuf> {
  let Ok(repo) = Repository::discover(root) else {
    return HashSet::new();
  };
  let Some(workdir) = repo.workdir().map(|dir| dir.to_path_buf()) else {
    return HashSet::new();
  };
  let workdir = workdir.canonicalize().unwrap_or(workdir);

  candidates
    .iter()
    .filter(|path| {
      let canonical = path.canonicalize().unwrap_or_else(|_| (*path).clone());
      let relative = canonical.strip_prefix(&workdir).unwrap_or(&canonical);
      let mut probe = relative.to_string_lossy().to_string();
      if probe.is_empty() {
        return false;
      }
      if path.is_dir() && !probe.ends_with('/') {
        probe.push('/');
      }
      repo.is_path_ignored(Path::new(&probe)).unwrap_or(false)
    })
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
      snap.conflicted.push(GitFileStatus {
        path,
        kind: GitFileKind::Conflicted,
      });
      continue;
    }
    if st.contains(Status::WT_NEW) {
      snap.untracked.push(GitFileStatus {
        path: path.clone(),
        kind: GitFileKind::Untracked,
      });
    }
    for (bit, kind) in [
      (Status::INDEX_NEW, GitFileKind::Added),
      (Status::INDEX_MODIFIED, GitFileKind::Modified),
      (Status::INDEX_DELETED, GitFileKind::Deleted),
      (Status::INDEX_RENAMED, GitFileKind::Renamed),
      (Status::INDEX_TYPECHANGE, GitFileKind::Typechange),
    ] {
      if st.contains(bit) {
        snap.staged.push(GitFileStatus {
          path: path.clone(),
          kind,
        });
      }
    }
    for (bit, kind) in [
      (Status::WT_MODIFIED, GitFileKind::Modified),
      (Status::WT_DELETED, GitFileKind::Deleted),
      (Status::WT_RENAMED, GitFileKind::Renamed),
      (Status::WT_TYPECHANGE, GitFileKind::Typechange),
    ] {
      if st.contains(bit) {
        snap.unstaged.push(GitFileStatus {
          path: path.clone(),
          kind,
        });
      }
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
  let (Some(local), Some(remote)) = (head.target(), upstream.get().target()) else {
    return;
  };
  if let Ok((ahead, behind)) = repo.graph_ahead_behind(local, remote) {
    snap.ahead = ahead;
    snap.behind = behind;
  }
}
