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

pub fn commits_ahead_of(root: &Path, base_ref: &str) -> Result<usize> {
  let output = git(root, &["rev-list", "--count", &format!("{base_ref}..HEAD")])?;
  Ok(output.trim().parse().unwrap_or(0))
}
