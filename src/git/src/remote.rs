use anyhow::{Result, anyhow};
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Result<String> {
  let output = Command::new("git")
    .args(args)
    .current_dir(root)
    .env("GIT_OPTIONAL_LOCKS", "0")
    .env("GIT_TERMINAL_PROMPT", "0")
    .output()?;
  if output.status.success() {
    return Ok(String::from_utf8_lossy(&output.stdout).to_string());
  }
  let stderr = String::from_utf8_lossy(&output.stderr);
  let stdout = String::from_utf8_lossy(&output.stdout);
  let message = if stderr.trim().is_empty() {
    stdout.trim()
  } else {
    stderr.trim()
  };
  Err(anyhow!(message.to_string()))
}

pub fn publish(root: &Path, branch: &str) -> Result<()> {
  git(
    root,
    &[
      "push",
      "--set-upstream",
      "origin",
      branch,
      "--end-of-options",
    ],
  )?;
  Ok(())
}

pub fn push(root: &Path) -> Result<()> {
  git(root, &["push"])?;
  Ok(())
}

pub fn sync(root: &Path) -> Result<()> {
  git(root, &["pull", "--ff-only"])?;
  Ok(())
}

/// Refuses to overwrite a remote that moved since the last fetch, which plain
/// `--force` would happily do.
pub fn force_push(root: &Path) -> Result<()> {
  git(root, &["push", "--force-with-lease"])?;
  Ok(())
}

pub fn pull(root: &Path) -> Result<()> {
  git(root, &["pull"])?;
  Ok(())
}

pub fn fast_forward(root: &Path) -> Result<()> {
  git(root, &["merge", "--ff-only", "@{u}"])?;
  Ok(())
}

pub fn rebase(root: &Path, upstream: &str) -> Result<()> {
  git(root, &["rebase", "--end-of-options", upstream])?;
  Ok(())
}

pub fn fetch(root: &Path) -> Result<()> {
  git(root, &["fetch", "--all", "--prune"])?;
  Ok(())
}

pub fn upstream(root: &Path) -> Option<String> {
  git(
    root,
    &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"],
  )
  .ok()
  .map(|name| name.trim().to_string())
  .filter(|name| !name.is_empty())
}

pub fn commits_ahead_of(root: &Path, base_ref: &str) -> Result<usize> {
  let output = git(root, &["rev-list", "--count", &format!("{base_ref}..HEAD")])?;
  Ok(output.trim().parse().unwrap_or(0))
}
