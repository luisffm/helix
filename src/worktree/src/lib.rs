pub mod branches;
pub mod rows;

use anyhow::{Context, Result};
use helix_models::ProjectInfo;
use std::path::{Path, PathBuf};

pub use branches::{BranchRef, available_branches};
pub use rows::{WorktreeRow, canonical_path, rows_for_projects, worktree_rows};

#[derive(Clone, Debug)]
pub struct WorktreeDescriptor {
  pub root: PathBuf,
  pub is_git: bool,
  pub is_linked_worktree: bool,
}

pub fn open_project(path: &Path) -> Result<(ProjectInfo, WorktreeDescriptor)> {
  let root = path
    .canonicalize()
    .with_context(|| format!("project path not found: {}", path.display()))?;
  let name = root
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| root.display().to_string());

  let (is_git, is_linked) = match git2::Repository::discover(&root) {
    Ok(repo) => (true, repo.is_worktree()),
    Err(_) => (false, false),
  };

  Ok((
    ProjectInfo {
      name,
      root: root.clone(),
    },
    WorktreeDescriptor {
      root,
      is_git,
      is_linked_worktree: is_linked,
    },
  ))
}

#[derive(Clone, Debug)]
pub struct WorktreeEntry {
  pub name: String,
  pub path: PathBuf,
  pub branch: String,
  pub is_primary: bool,
}

pub fn list_worktrees(root: &Path) -> Vec<WorktreeEntry> {
  let Ok(repo) = git2::Repository::discover(root) else {
    return Vec::new();
  };

  let main_repo = if repo.is_worktree() {
    match repo
      .path()
      .ancestors()
      .nth(3)
      .and_then(|p| git2::Repository::open(p).ok())
    {
      Some(main) => main,
      None => return Vec::new(),
    }
  } else {
    repo
  };

  let mut entries = Vec::new();

  if let Some(workdir) = main_repo.workdir() {
    entries.push(WorktreeEntry {
      name: workdir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "main".to_string()),
      path: workdir.to_path_buf(),
      branch: branch_of(&main_repo),
      is_primary: true,
    });
  }

  if let Ok(names) = main_repo.worktrees() {
    for name in names.iter().filter_map(|n| n.ok().flatten()) {
      if let Ok(wt) = main_repo.find_worktree(name) {
        let path = wt.path().to_path_buf();
        let branch = git2::Repository::open(&path)
          .map(|r| branch_of(&r))
          .unwrap_or_else(|_| name.to_string());

        entries.push(WorktreeEntry {
          name: name.to_string(),
          path,
          branch,
          is_primary: false,
        });
      }
    }
  }

  entries
}

pub fn describe_worktree(path: &Path) -> Option<WorktreeEntry> {
  let canonical = path.canonicalize().ok()?;
  let repo = git2::Repository::open(&canonical).ok()?;

  Some(WorktreeEntry {
    name: canonical
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_default(),
    path: canonical,
    branch: branch_of(&repo),
    is_primary: !repo.is_worktree(),
  })
}

pub fn primary_root(path: &Path) -> Option<PathBuf> {
  let repo = git2::Repository::discover(path).ok()?;

  if repo.is_worktree() {
    let main = repo.path().ancestors().nth(3)?;

    git2::Repository::open(main)
      .ok()?
      .workdir()
      .map(|w| w.to_path_buf())
  } else {
    repo.workdir().map(|w| w.to_path_buf())
  }
}

pub fn delete_worktree(repo_root: &Path, worktree: &Path) -> Result<()> {
  let output = std::process::Command::new("git")
    .arg("-C")
    .arg(repo_root)
    .args(["worktree", "remove", "--force"])
    .arg(worktree)
    .output()
    .context("failed to run git")?;

  if !output.status.success() {
    anyhow::bail!(
      "git worktree remove failed: {}",
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }

  Ok(())
}

/// Which branch a new worktree checks out: a branch git still has to create, or
/// one that already exists locally or on a remote.
#[derive(Clone, Debug)]
pub enum BranchSource {
  New(Option<String>),
  Existing(BranchRef),
}

/// `name` becomes the worktree directory suffix. For a new branch left
/// unnamed, the worktree name doubles as the branch name.
pub fn create_worktree(repo_root: &Path, name: &str, source: &BranchSource) -> Result<PathBuf> {
  let slug = slugify(name);

  if slug.is_empty() {
    anyhow::bail!("worktree name is empty");
  }

  let dest = destination(repo_root, &slug)?;

  match source {
    BranchSource::New(branch) => {
      let branch = match branch.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        Some(branch) => slugify(branch),
        None => slug,
      };

      if branch.is_empty() {
        anyhow::bail!("branch name is empty");
      }

      add_new_branch(repo_root, &dest, &branch)?;
    }
    BranchSource::Existing(branch) => add_existing_branch(repo_root, &dest, branch)?,
  }

  Ok(dest)
}

fn destination(repo_root: &Path, slug: &str) -> Result<PathBuf> {
  let root_name = repo_root
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| "worktree".to_string());
  let dir_name = format!("{root_name}-{}", slug.replace('/', "-"));

  let parent = repo_root
    .parent()
    .context("project root has no parent directory")?;
  let dest = parent.join(dir_name);

  if dest.exists() {
    anyhow::bail!("destination already exists: {}", dest.display());
  }

  Ok(dest)
}

fn add_new_branch(repo_root: &Path, dest: &Path, branch: &str) -> Result<()> {
  let created = std::process::Command::new("git")
    .arg("-C")
    .arg(repo_root)
    .args(["worktree", "add"])
    .arg(dest)
    .args(["-b", branch])
    .output()
    .context("failed to run git")?;

  if !created.status.success() {
    let retry = std::process::Command::new("git")
      .arg("-C")
      .arg(repo_root)
      .args(["worktree", "add"])
      .arg(dest)
      .arg(branch)
      .output()
      .context("failed to run git")?;

    if !retry.status.success() {
      anyhow::bail!(
        "git worktree add failed: {}",
        String::from_utf8_lossy(&created.stderr).trim()
      );
    }
  }

  Ok(())
}

fn add_existing_branch(repo_root: &Path, dest: &Path, branch: &BranchRef) -> Result<()> {
  let mut command = std::process::Command::new("git");

  command.arg("-C").arg(repo_root).args(["worktree", "add"]);

  match &branch.remote {
    Some(remote) => {
      command
        .args(["--track", "-b", &branch.name])
        .arg(dest)
        .arg(format!("{remote}/{}", branch.name));
    }
    None => {
      command.arg(dest).arg(&branch.name);
    }
  }

  let output = command.output().context("failed to run git")?;

  if !output.status.success() {
    anyhow::bail!(
      "git worktree add failed: {}",
      String::from_utf8_lossy(&output.stderr).trim()
    );
  }

  Ok(())
}

fn slugify(value: &str) -> String {
  value
    .trim()
    .chars()
    .map(|c| if c.is_whitespace() { '-' } else { c })
    .collect()
}

fn branch_of(repo: &git2::Repository) -> String {
  match repo.head() {
    Ok(head) if head.is_branch() => head
      .shorthand()
      .map(str::to_string)
      .unwrap_or_else(|_| "HEAD".to_string()),
    Ok(head) => format!(
      "detached @ {}",
      head
        .target()
        .map(|oid| oid.to_string()[..7].to_string())
        .unwrap_or_default()
    ),
    Err(_) => unborn_branch_name(repo).unwrap_or_else(|| "no branch".to_string()),
  }
}

fn unborn_branch_name(repo: &git2::Repository) -> Option<String> {
  let head = repo.find_reference("HEAD").ok()?;
  let target = head.symbolic_target().ok().flatten()?;

  Some(
    target
      .strip_prefix("refs/heads/")
      .unwrap_or(target)
      .to_string(),
  )
}

pub fn list_linked_worktrees(root: &Path) -> Vec<String> {
  let Ok(repo) = git2::Repository::discover(root) else {
    return Vec::new();
  };

  repo
    .worktrees()
    .map(|names| {
      names
        .iter()
        .filter_map(|name| name.ok().flatten().map(str::to_string))
        .collect()
    })
    .unwrap_or_default()
}
