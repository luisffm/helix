use crate::eligibility::{Eligibility, RepoState, evaluate};
use crate::review::HostedReview;
use crate::{ReviewLookupOutcome, gh, review};
use anyhow::Result;
use helix_models::GitSnapshot;
use std::path::Path;

pub fn gather(root: &Path, git: &GitSnapshot) -> (Eligibility, Option<HostedReview>) {
  let gh_installed = gh::is_installed();
  let authenticated = gh_installed && gh::is_authenticated();

  let (lookup, hosted) = if authenticated {
    match review::for_branch(root, &git.branch) {
      Ok(Some(hosted)) => (ReviewLookupOutcome::Found, Some(hosted)),
      Ok(None) => (ReviewLookupOutcome::NotFound, None),
      Err(_) => (ReviewLookupOutcome::Unavailable, None),
    }
  } else {
    (ReviewLookupOutcome::Unavailable, None)
  };

  let base_ref = helix_git::diff::default_base_ref(root);

  let commits_ahead_of_base = base_ref
    .as_deref()
    .and_then(|base| helix_git::remote::commits_ahead_of(root, base).ok())
    .unwrap_or(0);

  let has_upstream = git.ahead > 0
    || git.behind > 0
    || commits_ahead_of_base == 0
    || helix_git::remote::upstream(root).is_some();

  let state = RepoState {
    gh_installed,
    authenticated,
    detached: git.detached,
    branch: git.branch.clone(),
    base_ref,
    dirty_count: git.dirty_count(),
    has_upstream,
    ahead: git.ahead,
    behind: git.behind,
    commits_ahead_of_base,
  };

  let eligibility = evaluate(&state, lookup, hosted.clone());

  (eligibility, hosted)
}

pub fn create_pull_request(root: &Path, title: &str) -> Result<()> {
  let base = helix_git::diff::default_base_ref(root)
    .map(|base| base.trim_start_matches("origin/").to_string())
    .unwrap_or_else(|| "main".to_string());

  review::create(root, &base, title, "", false)?;

  Ok(())
}
