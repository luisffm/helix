use anyhow::{Result, anyhow};
use std::path::Path;
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GhErrorKind {
  AuthRequired,
  NotFound,
  AlreadyExists,
  Validation,
  UnknownCompletion,
  Unknown,
}

#[derive(Clone, Debug)]
pub struct GhError {
  pub kind: GhErrorKind,
  pub message: String,
}

impl std::fmt::Display for GhError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.message)
  }
}

impl std::error::Error for GhError {}

pub fn classify(stderr: &str) -> GhErrorKind {
  let lower = stderr.to_lowercase();

  if lower.contains("gh auth login")
    || lower.contains("not logged into")
    || lower.contains("authentication required")
    || lower.contains("bad credentials")
  {
    return GhErrorKind::AuthRequired;
  }

  if lower.contains("already exists") || lower.contains("a pull request already exists") {
    return GhErrorKind::AlreadyExists;
  }

  if lower.contains("could not resolve to") || lower.contains("no pull requests found") {
    return GhErrorKind::NotFound;
  }

  if lower.contains("validation failed") || lower.contains("unprocessable") {
    return GhErrorKind::Validation;
  }

  GhErrorKind::Unknown
}

pub fn run(cwd: &Path, args: &[&str]) -> Result<String> {
  let mut command = helix_process::command("gh");

  command
    .args(args)
    .current_dir(cwd)
    .env("GH_PROMPT_DISABLED", "1")
    .env("GH_NO_UPDATE_NOTIFIER", "1");

  let output = helix_process::output(command, None, TIMEOUT).map_err(|err| {
    anyhow!(GhError {
      kind: GhErrorKind::Unknown,
      message: format!("could not run gh: {err}"),
    })
  })?;

  if output.status.success() {
    return Ok(String::from_utf8_lossy(&output.stdout).to_string());
  }

  let stderr = String::from_utf8_lossy(&output.stderr).to_string();
  let stdout = String::from_utf8_lossy(&output.stdout).to_string();
  let message = if stderr.trim().is_empty() {
    stdout
  } else {
    stderr
  };

  Err(anyhow!(GhError {
    kind: classify(&message),
    message: message.trim().to_string(),
  }))
}

pub fn is_authenticated() -> bool {
  let mut command = helix_process::command("gh");

  command
    .args(["auth", "status"])
    .env("GH_PROMPT_DISABLED", "1");

  helix_process::output(command, None, TIMEOUT)
    .map(|output| output.status.success())
    .unwrap_or(false)
}

pub fn is_installed() -> bool {
  let mut command = helix_process::command("gh");

  command.arg("--version");

  helix_process::output(command, None, TIMEOUT)
    .map(|output| output.status.success())
    .unwrap_or(false)
}

#[cfg(test)]
mod tests {
  use super::{GhErrorKind, classify};

  #[test]
  fn detects_auth_error() {
    assert_eq!(
      classify("gh: To get started with GitHub CLI, please run: gh auth login"),
      GhErrorKind::AuthRequired
    );
  }

  #[test]
  fn detects_duplicate_pr() {
    assert_eq!(
      classify("a pull request already exists for luis:feature"),
      GhErrorKind::AlreadyExists
    );
  }

  #[test]
  fn unknown_stays_unknown() {
    assert_eq!(classify("some transport blip"), GhErrorKind::Unknown);
  }
}
