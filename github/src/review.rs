use crate::gh;
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

const LOOKUP_FIELDS: &str = "number,title,state,url,statusCheckRollup,updatedAt,isDraft,mergeable,reviewDecision,mergeStateStatus,baseRefName,headRefName,headRefOid";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewState {
  Open,
  Draft,
  Merged,
  Closed,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckStatus {
  None,
  Pending,
  Passing,
  Failing,
}

impl CheckStatus {
  pub fn label(&self) -> &'static str {
    match self {
      CheckStatus::None => "no checks",
      CheckStatus::Pending => "checks running",
      CheckStatus::Passing => "checks passing",
      CheckStatus::Failing => "checks failing",
    }
  }
}

#[derive(Clone, Debug)]
pub struct HostedReview {
  pub number: u64,
  pub title: String,
  pub url: String,
  pub state: ReviewState,
  pub checks: CheckStatus,
  pub base_ref: String,
  pub head_ref: String,
  pub review_decision: Option<String>,
  pub conflicting: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawReview {
  number: u64,
  title: String,
  url: String,
  state: String,
  #[serde(default)]
  is_draft: bool,
  #[serde(default)]
  mergeable: Option<String>,
  #[serde(default)]
  merge_state_status: Option<String>,
  #[serde(default)]
  review_decision: Option<String>,
  #[serde(default)]
  base_ref_name: String,
  #[serde(default)]
  head_ref_name: String,
  #[serde(default)]
  status_check_rollup: Option<Vec<RawCheck>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawCheck {
  #[serde(default)]
  status: Option<String>,
  #[serde(default)]
  conclusion: Option<String>,
  #[serde(default)]
  state: Option<String>,
}

pub fn derive_check_status(checks: &[RawCheck]) -> CheckStatus {
  if checks.is_empty() {
    return CheckStatus::None;
  }
  let mut pending = false;
  for check in checks {
    let verdict = check
      .conclusion
      .as_deref()
      .or(check.state.as_deref())
      .unwrap_or_default()
      .to_ascii_uppercase();
    let status = check
      .status
      .as_deref()
      .unwrap_or_default()
      .to_ascii_uppercase();

    match verdict.as_str() {
      "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE"
      | "ERROR" => return CheckStatus::Failing,
      "SUCCESS" | "NEUTRAL" | "SKIPPED" => {}
      _ => {
        if status != "COMPLETED" || verdict.is_empty() {
          pending = true;
        }
      }
    }
  }
  if pending {
    CheckStatus::Pending
  } else {
    CheckStatus::Passing
  }
}

fn map(raw: RawReview) -> HostedReview {
  let state = match raw.state.to_ascii_uppercase().as_str() {
    "MERGED" => ReviewState::Merged,
    "CLOSED" => ReviewState::Closed,
    _ if raw.is_draft => ReviewState::Draft,
    _ => ReviewState::Open,
  };
  let conflicting = raw.mergeable.as_deref() == Some("CONFLICTING")
    || raw.merge_state_status.as_deref() == Some("DIRTY");
  HostedReview {
    number: raw.number,
    title: raw.title,
    url: raw.url,
    state,
    checks: derive_check_status(raw.status_check_rollup.as_deref().unwrap_or_default()),
    base_ref: raw.base_ref_name,
    head_ref: raw.head_ref_name,
    review_decision: raw.review_decision.filter(|value| !value.is_empty()),
    conflicting,
  }
}

pub fn for_branch(cwd: &Path, branch: &str) -> Result<Option<HostedReview>> {
  let output = gh::run(
    cwd,
    &[
      "pr",
      "list",
      "--head",
      branch,
      "--state",
      "all",
      "--limit",
      "1",
      "--json",
      LOOKUP_FIELDS,
    ],
  )?;
  let raws: Vec<RawReview> = serde_json::from_str(output.trim())?;
  Ok(raws.into_iter().next().map(map))
}

pub fn create(
  cwd: &Path,
  base: &str,
  title: &str,
  body: &str,
  draft: bool,
) -> Result<HostedReview> {
  let dir = std::env::temp_dir().join(format!("helix-pr-body-{}", std::process::id()));
  std::fs::create_dir_all(&dir)?;
  let body_path = dir.join("body.md");
  std::fs::write(&body_path, body)?;
  let body_arg = body_path.to_string_lossy().to_string();

  let mut args = vec![
    "pr",
    "create",
    "--base",
    base,
    "--title",
    title,
    "--body-file",
    &body_arg,
  ];
  if draft {
    args.push("--draft");
  }
  let result = gh::run(cwd, &args);
  let _ = std::fs::remove_dir_all(&dir);
  result?;

  let branch = current_branch(cwd)?;
  for_branch(cwd, &branch)?.ok_or_else(|| {
    anyhow::anyhow!(gh::GhError {
      kind: gh::GhErrorKind::UnknownCompletion,
      message: "gh pr create returned success but no pull request was found for this branch"
        .to_string(),
    })
  })
}

fn current_branch(cwd: &Path) -> Result<String> {
  let output = std::process::Command::new("git")
    .args(["branch", "--show-current"])
    .current_dir(cwd)
    .output()?;
  Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn check(status: Option<&str>, conclusion: Option<&str>) -> RawCheck {
    RawCheck {
      status: status.map(str::to_string),
      conclusion: conclusion.map(str::to_string),
      state: None,
    }
  }

  #[test]
  fn empty_rollup_is_none() {
    assert_eq!(derive_check_status(&[]), CheckStatus::None);
  }

  #[test]
  fn any_failure_wins() {
    let checks = [
      check(Some("COMPLETED"), Some("SUCCESS")),
      check(Some("COMPLETED"), Some("FAILURE")),
    ];
    assert_eq!(derive_check_status(&checks), CheckStatus::Failing);
  }

  #[test]
  fn in_progress_is_pending() {
    let checks = [check(Some("IN_PROGRESS"), None)];
    assert_eq!(derive_check_status(&checks), CheckStatus::Pending);
  }

  #[test]
  fn all_success_is_passing() {
    let checks = [
      check(Some("COMPLETED"), Some("SUCCESS")),
      check(Some("COMPLETED"), Some("SKIPPED")),
    ];
    assert_eq!(derive_check_status(&checks), CheckStatus::Passing);
  }
}
