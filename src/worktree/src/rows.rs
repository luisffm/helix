use crate::{WorktreeEntry, describe_worktree};
use helix_state::config::ProjectConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct WorktreeRow {
  pub entry: WorktreeEntry,
  pub canonical: PathBuf,
  pub display_name: Option<String>,
  pub issue: Option<String>,
  pub pr: Option<String>,
}

impl WorktreeRow {
  pub fn new(
    entry: WorktreeEntry,
    display_name: Option<String>,
    issue: Option<String>,
    pr: Option<String>,
  ) -> Self {
    let canonical = canonical_path(&entry.path);

    Self {
      entry,
      canonical,
      display_name,
      issue,
      pr,
    }
  }
}

pub fn canonical_path(path: &Path) -> PathBuf {
  path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub fn worktree_rows(project: &ProjectConfig) -> Vec<WorktreeRow> {
  let mut entries: Vec<WorktreeRow> = Vec::new();

  if let Some(entry) = describe_worktree(&project.path) {
    entries.push(WorktreeRow::new(entry, None, None, None));
  }

  for wt in &project.worktrees {
    let Some(entry) = describe_worktree(&wt.path) else {
      continue;
    };

    if entries.iter().any(|e| e.entry.path == entry.path) {
      continue;
    }

    entries.push(WorktreeRow::new(
      entry,
      wt.display_name.clone(),
      wt.issue.clone(),
      wt.pr.clone(),
    ));
  }

  entries
}

/// `only` restricts the map to a single owning project, which is what a
/// filesystem batch needs: describing every configured project costs a
/// canonicalize plus a `Repository::open` per worktree.
pub fn rows_for_projects(
  projects: &[ProjectConfig],
  only: Option<&Path>,
) -> HashMap<PathBuf, Vec<WorktreeRow>> {
  projects
    .iter()
    .filter(|project| project.path.is_dir())
    .filter(|project| only.is_none_or(|owner| project.path == owner))
    .map(|project| (project.path.clone(), worktree_rows(project)))
    .collect()
}
