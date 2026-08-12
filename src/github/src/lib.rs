pub mod eligibility;
pub mod gh;
pub mod probe;
pub mod review;

pub use eligibility::{
  BlockedReason, Eligibility, MergeReadiness, NextAction, ReviewLookupOutcome, merge_readiness,
};
pub use review::{CheckStatus, HostedReview, ReviewCheck, ReviewComment, ReviewState};

#[derive(Clone, Debug)]
pub struct PullRequestInfo {
  pub number: u64,
  pub title: String,
  pub body: String,
  pub author: String,
  pub reviewers: Vec<String>,
  pub checks: Vec<CheckRun>,
  pub comments: usize,
  pub commits: usize,
  pub changed_files: usize,
}

#[derive(Clone, Debug)]
pub struct CheckRun {
  pub name: String,
  pub conclusion: CheckConclusion,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CheckConclusion {
  Pending,
  Success,
  Failure,
  Skipped,
}

pub fn current_pull_request() -> Option<PullRequestInfo> {
  None
}

pub fn short_ref(value: &str) -> String {
  let digits: String = value
    .chars()
    .rev()
    .take_while(|c| c.is_ascii_digit())
    .collect::<String>()
    .chars()
    .rev()
    .collect();

  if digits.is_empty() {
    "link".to_string()
  } else {
    format!("#{digits}")
  }
}
