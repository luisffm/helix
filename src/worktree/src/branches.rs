use crate::{list_worktrees, primary_root};
use git2::{Branch, BranchType, Repository};
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRef {
  pub name: String,
  pub remote: Option<String>,
}

impl BranchRef {
  pub fn label(&self) -> String {
    match &self.remote {
      Some(remote) => format!("{remote}/{}", self.name),
      None => self.name.clone(),
    }
  }
}

/// Branches a new worktree can check out: local branches no worktree already
/// holds, plus remote branches with no local counterpart yet. Ordered by commit
/// recency, so the branch someone just pushed comes first.
pub fn available_branches(root: &Path) -> Vec<BranchRef> {
  let Some(primary) = primary_root(root) else {
    return Vec::new();
  };

  let Ok(repo) = Repository::open(&primary) else {
    return Vec::new();
  };

  let taken: HashSet<String> = list_worktrees(&primary)
    .into_iter()
    .map(|entry| entry.branch)
    .collect();

  let mut locals = HashSet::new();
  let mut dated: Vec<(i64, BranchRef)> = Vec::new();

  if let Ok(branches) = repo.branches(Some(BranchType::Local)) {
    for (branch, _) in branches.flatten() {
      let Some(name) = branch.name().ok().flatten() else {
        continue;
      };

      locals.insert(name.to_string());

      if taken.contains(name) {
        continue;
      }

      dated.push((
        commit_time(&branch),
        BranchRef {
          name: name.to_string(),
          remote: None,
        },
      ));
    }
  }

  if let Ok(branches) = repo.branches(Some(BranchType::Remote)) {
    for (branch, _) in branches.flatten() {
      let Some(full) = branch.name().ok().flatten() else {
        continue;
      };

      let Some((remote, name)) = full.split_once('/') else {
        continue;
      };

      if name == "HEAD" || locals.contains(name) || taken.contains(name) {
        continue;
      }

      dated.push((
        commit_time(&branch),
        BranchRef {
          name: name.to_string(),
          remote: Some(remote.to_string()),
        },
      ));
    }
  }

  dated.sort_by(|a, b| b.0.cmp(&a.0));

  let mut seen = HashSet::new();

  dated
    .into_iter()
    .map(|(_, branch)| branch)
    .filter(|branch| seen.insert(branch.name.clone()))
    .collect()
}

fn commit_time(branch: &Branch) -> i64 {
  branch
    .get()
    .peel_to_commit()
    .map(|commit| commit.time().seconds())
    .unwrap_or(0)
}
