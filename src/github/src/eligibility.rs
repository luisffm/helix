use crate::review::HostedReview;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReviewLookupOutcome {
  Found,
  NotFound,
  Unavailable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockedReason {
  GhMissing,
  AuthRequired,
  DetachedHead,
  DefaultBranch,
  ExistingReview,
  Dirty,
  NoUpstream,
  NeedsPush,
  NeedsSync,
  Diverged,
  NoCommits,
  LookupUnavailable,
}

impl BlockedReason {
  pub fn message(&self) -> &'static str {
    match self {
      BlockedReason::GhMissing => "GitHub CLI not found — install gh",
      BlockedReason::AuthRequired => "Not authenticated — run gh auth login",
      BlockedReason::DetachedHead => "Detached HEAD — check out a branch",
      BlockedReason::DefaultBranch => "On the base branch — create a feature branch",
      BlockedReason::ExistingReview => "This branch already has a pull request",
      BlockedReason::Dirty => "Uncommitted changes — commit first",
      BlockedReason::NoUpstream => "Branch has no upstream — publish it",
      BlockedReason::NeedsPush => "Local commits not pushed",
      BlockedReason::NeedsSync => "Branch is behind its upstream — pull first",
      BlockedReason::Diverged => "Branch diverged from upstream — resolve manually",
      BlockedReason::NoCommits => "No commits ahead of the base branch",
      BlockedReason::LookupUnavailable => {
        "Could not confirm whether this branch already has a pull request"
      }
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NextAction {
  InstallGh,
  Authenticate,
  Commit,
  Publish,
  Push,
  Sync,
  OpenExistingReview,
  CreateReview,
  Retry,
  None,
}

#[derive(Clone, Debug)]
pub struct Eligibility {
  pub can_create: bool,
  pub blocked_reason: Option<BlockedReason>,
  pub next_action: NextAction,
  pub lookup_outcome: ReviewLookupOutcome,
  pub review: Option<HostedReview>,
}

#[derive(Clone, Debug)]
pub struct RepoState {
  pub gh_installed: bool,
  pub authenticated: bool,
  pub detached: bool,
  pub branch: String,
  pub base_ref: Option<String>,
  pub dirty_count: usize,
  pub has_upstream: bool,
  pub ahead: usize,
  pub behind: usize,
  pub commits_ahead_of_base: usize,
}

pub fn evaluate(
  state: &RepoState,
  lookup: ReviewLookupOutcome,
  review: Option<HostedReview>,
) -> Eligibility {
  let blocked = |reason: BlockedReason, next_action: NextAction| Eligibility {
    can_create: false,
    blocked_reason: Some(reason),
    next_action,
    lookup_outcome: lookup,
    review: review.clone(),
  };

  if !state.gh_installed {
    return blocked(BlockedReason::GhMissing, NextAction::InstallGh);
  }
  if !state.authenticated {
    return blocked(BlockedReason::AuthRequired, NextAction::Authenticate);
  }
  if state.detached {
    return blocked(BlockedReason::DetachedHead, NextAction::None);
  }
  if review.is_some() {
    return blocked(
      BlockedReason::ExistingReview,
      NextAction::OpenExistingReview,
    );
  }
  if lookup == ReviewLookupOutcome::Unavailable {
    return blocked(BlockedReason::LookupUnavailable, NextAction::Retry);
  }
  if state
    .base_ref
    .as_deref()
    .map(|base| base.trim_start_matches("origin/") == state.branch)
    .unwrap_or(false)
  {
    return blocked(BlockedReason::DefaultBranch, NextAction::None);
  }
  if state.dirty_count > 0 {
    return blocked(BlockedReason::Dirty, NextAction::Commit);
  }
  if !state.has_upstream {
    return blocked(BlockedReason::NoUpstream, NextAction::Publish);
  }
  if state.ahead > 0 && state.behind > 0 {
    return blocked(BlockedReason::Diverged, NextAction::None);
  }
  if state.behind > 0 {
    return blocked(BlockedReason::NeedsSync, NextAction::Sync);
  }
  if state.ahead > 0 {
    return blocked(BlockedReason::NeedsPush, NextAction::Push);
  }
  if state.commits_ahead_of_base == 0 {
    return blocked(BlockedReason::NoCommits, NextAction::None);
  }

  Eligibility {
    can_create: true,
    blocked_reason: None,
    next_action: NextAction::CreateReview,
    lookup_outcome: lookup,
    review: None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ready() -> RepoState {
    RepoState {
      gh_installed: true,
      authenticated: true,
      detached: false,
      branch: "feature".to_string(),
      base_ref: Some("origin/main".to_string()),
      dirty_count: 0,
      has_upstream: true,
      ahead: 0,
      behind: 0,
      commits_ahead_of_base: 3,
    }
  }

  #[test]
  fn clean_branch_can_create() {
    let result = evaluate(&ready(), ReviewLookupOutcome::NotFound, None);

    assert!(result.can_create);
    assert_eq!(result.next_action, NextAction::CreateReview);
  }

  #[test]
  fn unavailable_lookup_never_creates() {
    let result = evaluate(&ready(), ReviewLookupOutcome::Unavailable, None);

    assert!(!result.can_create);
    assert_eq!(
      result.blocked_reason,
      Some(BlockedReason::LookupUnavailable)
    );
    assert_eq!(result.next_action, NextAction::Retry);
  }

  #[test]
  fn dirty_tree_asks_for_commit() {
    let mut state = ready();
    state.dirty_count = 2;

    let result = evaluate(&state, ReviewLookupOutcome::NotFound, None);

    assert_eq!(result.blocked_reason, Some(BlockedReason::Dirty));
    assert_eq!(result.next_action, NextAction::Commit);
  }

  #[test]
  fn unpushed_commits_ask_for_push() {
    let mut state = ready();
    state.ahead = 2;

    let result = evaluate(&state, ReviewLookupOutcome::NotFound, None);

    assert_eq!(result.next_action, NextAction::Push);
  }

  #[test]
  fn missing_upstream_asks_for_publish() {
    let mut state = ready();
    state.has_upstream = false;

    let result = evaluate(&state, ReviewLookupOutcome::NotFound, None);

    assert_eq!(result.next_action, NextAction::Publish);
  }

  #[test]
  fn diverged_is_not_auto_handled() {
    let mut state = ready();
    state.ahead = 1;
    state.behind = 1;

    let result = evaluate(&state, ReviewLookupOutcome::NotFound, None);

    assert_eq!(result.blocked_reason, Some(BlockedReason::Diverged));
    assert_eq!(result.next_action, NextAction::None);
  }

  #[test]
  fn logged_out_asks_for_auth_before_anything_else() {
    let mut state = ready();
    state.authenticated = false;
    state.dirty_count = 5;

    let result = evaluate(&state, ReviewLookupOutcome::Unavailable, None);

    assert_eq!(result.next_action, NextAction::Authenticate);
  }

  #[test]
  fn base_branch_is_blocked() {
    let mut state = ready();
    state.branch = "main".to_string();

    let result = evaluate(&state, ReviewLookupOutcome::NotFound, None);

    assert_eq!(result.blocked_reason, Some(BlockedReason::DefaultBranch));
  }
}
