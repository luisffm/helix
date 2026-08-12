use crate::gh;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

const LOOKUP_FIELDS: &str = "number,title,state,url,statusCheckRollup,updatedAt,isDraft,mergeable,reviewDecision,mergeStateStatus,baseRefName,headRefName,headRefOid";

/// Comments are only ever read for the branch in front of the user. Asking for
/// them in the repository-wide listing would pull every comment of a hundred
/// pull requests to colour a handful of sidebar rows.
const DETAIL_FIELDS: &str = "number,title,state,url,statusCheckRollup,updatedAt,isDraft,mergeable,reviewDecision,mergeStateStatus,baseRefName,headRefName,headRefOid,comments";

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
pub struct ReviewComment {
  pub author: String,
  pub body: String,
  pub epoch_seconds: i64,
  pub bot: bool,
}

#[derive(Clone, Debug)]
pub struct ReviewCheck {
  pub name: String,
  pub status: CheckStatus,
  pub url: Option<String>,
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
  pub check_runs: Vec<ReviewCheck>,
  pub updated_epoch_seconds: i64,
  pub comments: Vec<ReviewComment>,
}

impl HostedReview {
  pub fn checks_running(&self) -> bool {
    self.checks == CheckStatus::Pending
  }
}

/// The sidebar draws one row per branch, so a listing collapses to the state
/// of the newest review each branch has.
pub fn states_by_branch(reviews: Vec<HostedReview>) -> HashMap<String, ReviewState> {
  let mut states = HashMap::new();

  for review in reviews {
    states.entry(review.head_ref).or_insert(review.state);
  }

  states
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
  #[serde(default)]
  updated_at: Option<String>,
  #[serde(default)]
  comments: Option<Vec<RawComment>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAuthor {
  #[serde(default)]
  login: Option<String>,
  #[serde(default)]
  is_bot: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawComment {
  #[serde(default)]
  author: Option<RawAuthor>,
  #[serde(default)]
  body: Option<String>,
  #[serde(default)]
  created_at: Option<String>,
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
  #[serde(default)]
  name: Option<String>,
  #[serde(default)]
  context: Option<String>,
  #[serde(default)]
  workflow_name: Option<String>,
  #[serde(default)]
  details_url: Option<String>,
  #[serde(default)]
  target_url: Option<String>,
}

pub fn classify_check(check: &RawCheck) -> CheckStatus {
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
    | "ERROR" => CheckStatus::Failing,
    "SUCCESS" | "NEUTRAL" | "SKIPPED" => CheckStatus::Passing,
    _ if status == "COMPLETED" && !verdict.is_empty() => CheckStatus::Passing,
    _ => CheckStatus::Pending,
  }
}

pub fn derive_check_status(checks: &[RawCheck]) -> CheckStatus {
  if checks.is_empty() {
    return CheckStatus::None;
  }

  let mut pending = false;

  for check in checks {
    match classify_check(check) {
      CheckStatus::Failing => return CheckStatus::Failing,
      CheckStatus::Pending => pending = true,
      _ => {}
    }
  }

  if pending {
    CheckStatus::Pending
  } else {
    CheckStatus::Passing
  }
}

pub fn check_runs(checks: &[RawCheck]) -> Vec<ReviewCheck> {
  checks
    .iter()
    .map(|check| ReviewCheck {
      name: check
        .name
        .as_deref()
        .or(check.context.as_deref())
        .or(check.workflow_name.as_deref())
        .unwrap_or("check")
        .to_string(),
      status: classify_check(check),
      url: check
        .details_url
        .clone()
        .or_else(|| check.target_url.clone())
        .filter(|url| !url.is_empty()),
    })
    .collect()
}

/// Days since the civil epoch, from Howard Hinnant's `days_from_civil`. Only
/// the timestamps gh reports are parsed here, which are always UTC and always
/// `YYYY-MM-DDTHH:MM:SSZ`, so a date crate would be a dependency for one format.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
  let year = if month <= 2 { year - 1 } else { year };
  let era = if year >= 0 { year } else { year - 399 } / 400;
  let year_of_era = year - era * 400;
  let month = month as i64;
  let day_of_year =
    (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
  let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

  era * 146_097 + day_of_era - 719_468
}

pub fn epoch_seconds(timestamp: &str) -> i64 {
  let bytes = timestamp.as_bytes();

  if bytes.len() < 19 {
    return 0;
  }

  let number = |range: std::ops::Range<usize>| timestamp[range].parse::<i64>().unwrap_or(0);

  let year = number(0..4);
  let month = number(5..7) as u32;
  let day = number(8..10) as u32;
  let hour = number(11..13);
  let minute = number(14..16);
  let second = number(17..19);

  if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
    return 0;
  }

  days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
}

fn comments(raw: Vec<RawComment>) -> Vec<ReviewComment> {
  raw
    .into_iter()
    .map(|comment| {
      let author = comment.author.unwrap_or(RawAuthor {
        login: None,
        is_bot: None,
      });
      let login = author.login.unwrap_or_else(|| "unknown".to_string());
      let bot = author.is_bot.unwrap_or(false) || login.ends_with("[bot]");

      ReviewComment {
        author: login.trim_end_matches("[bot]").to_string(),
        body: comment.body.unwrap_or_default(),
        epoch_seconds: comment.created_at.map(|at| epoch_seconds(&at)).unwrap_or(0),
        bot,
      }
    })
    .collect()
}

fn map(mut raw: RawReview) -> HostedReview {
  let rollup = raw.status_check_rollup.take().unwrap_or_default();

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
    checks: derive_check_status(&rollup),
    check_runs: check_runs(&rollup),
    base_ref: raw.base_ref_name,
    head_ref: raw.head_ref_name,
    review_decision: raw.review_decision.filter(|value| !value.is_empty()),
    conflicting,
    updated_epoch_seconds: raw
      .updated_at
      .map(|at| epoch_seconds(&at))
      .unwrap_or_default(),
    comments: comments(raw.comments.unwrap_or_default()),
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
      DETAIL_FIELDS,
    ],
  )?;

  let raws: Vec<RawReview> = serde_json::from_str(output.trim())?;

  Ok(raws.into_iter().next().map(map))
}

/// One listing per repository instead of one lookup per worktree: the sidebar
/// needs a state for every branch it draws, and each gh call is a subprocess
/// plus a round trip.
pub fn list_for_repo(cwd: &Path, limit: usize) -> Result<Vec<HostedReview>> {
  let limit = limit.to_string();
  let output = gh::run(
    cwd,
    &[
      "pr",
      "list",
      "--state",
      "all",
      "--limit",
      &limit,
      "--json",
      LOOKUP_FIELDS,
    ],
  )?;

  let raws: Vec<RawReview> = serde_json::from_str(output.trim())?;

  Ok(raws.into_iter().map(map).collect())
}

pub fn merge(cwd: &Path, number: u64) -> Result<()> {
  gh::run(cwd, &["pr", "merge", &number.to_string(), "--merge"])?;

  Ok(())
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
      name: None,
      context: None,
      workflow_name: None,
      details_url: None,
      target_url: None,
    }
  }

  fn review(head_ref: &str, state: ReviewState) -> HostedReview {
    HostedReview {
      number: 1,
      title: String::new(),
      url: String::new(),
      state,
      checks: CheckStatus::None,
      base_ref: "main".to_string(),
      head_ref: head_ref.to_string(),
      review_decision: None,
      conflicting: false,
      check_runs: Vec::new(),
      updated_epoch_seconds: 0,
      comments: Vec::new(),
    }
  }

  #[test]
  fn a_branch_keeps_the_first_review_listed() {
    let states = states_by_branch(vec![
      review("feature", ReviewState::Open),
      review("feature", ReviewState::Closed),
      review("other", ReviewState::Merged),
    ]);

    assert_eq!(states.get("feature"), Some(&ReviewState::Open));
    assert_eq!(states.get("other"), Some(&ReviewState::Merged));
  }

  #[test]
  fn a_running_suite_is_the_only_thing_worth_polling() {
    let mut pending = review("feature", ReviewState::Open);
    pending.checks = CheckStatus::Pending;

    assert!(pending.checks_running());

    pending.checks = CheckStatus::Passing;

    assert!(!pending.checks_running());
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
  fn a_run_falls_back_to_its_context_for_a_name() {
    let mut raw = check(Some("COMPLETED"), Some("SUCCESS"));
    raw.context = Some("ci/circleci".to_string());

    let runs = check_runs(&[raw]);

    assert_eq!(runs[0].name, "ci/circleci");
    assert_eq!(runs[0].status, CheckStatus::Passing);
  }

  #[test]
  fn a_run_without_any_label_still_lists() {
    let runs = check_runs(&[check(Some("IN_PROGRESS"), None)]);

    assert_eq!(runs[0].name, "check");
    assert_eq!(runs[0].status, CheckStatus::Pending);
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

#[cfg(test)]
mod timestamps {
  use super::epoch_seconds;

  #[test]
  fn parses_a_gh_timestamp() {
    assert_eq!(epoch_seconds("1970-01-01T00:00:00Z"), 0);
    assert_eq!(epoch_seconds("2026-08-12T09:12:00Z"), 1_786_525_920);
  }

  #[test]
  fn a_leap_day_lands_where_it_should() {
    assert_eq!(epoch_seconds("2000-02-29T12:00:00Z"), 951_825_600);
  }

  #[test]
  fn junk_is_zero_rather_than_a_panic() {
    assert_eq!(epoch_seconds(""), 0);
    assert_eq!(epoch_seconds("not-a-date"), 0);
    assert_eq!(epoch_seconds("2026-13-99T00:00:00Z"), 0);
  }
}
