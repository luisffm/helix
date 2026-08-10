use anyhow::Result;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum GitAction {
  Commit,
  CommitPush,
  CommitSync,
  Push,
  ForcePush,
  Pull,
  FastForward,
  Sync,
  Rebase,
  Fetch,
  Publish,
}

impl GitAction {
  pub fn commits(self) -> bool {
    matches!(self, Self::Commit | Self::CommitPush | Self::CommitSync)
  }

  pub fn remote_step(self) -> Option<Self> {
    match self {
      Self::Commit => None,
      Self::CommitPush => Some(Self::Push),
      Self::CommitSync => Some(Self::Sync),
      other => Some(other),
    }
  }
}

pub enum IndexOp {
  Stage(String),
  Unstage(String),
  StageAll,
  UnstageAll,
  Discard(String),
}

impl IndexOp {
  pub fn run(self, root: &Path) -> Result<()> {
    match self {
      Self::Stage(relative) => crate::index::stage(root, &relative),
      Self::Unstage(relative) => crate::index::unstage(root, &relative),
      Self::StageAll => crate::index::stage_all(root),
      Self::UnstageAll => crate::index::unstage_all(root),
      Self::Discard(relative) => crate::index::discard(root, &relative),
    }
  }
}

pub fn perform_remote(action: GitAction, root: &Path, branch: &str, upstream: &str) -> Result<()> {
  match action {
    GitAction::Push => crate::remote::push(root),
    GitAction::ForcePush => crate::remote::force_push(root),
    GitAction::Pull => crate::remote::pull(root),
    GitAction::FastForward => crate::remote::fast_forward(root),
    GitAction::Sync => crate::remote::sync(root),
    GitAction::Rebase => crate::remote::rebase(root, upstream),
    GitAction::Fetch => crate::remote::fetch(root),
    GitAction::Publish => crate::remote::publish(root, branch),
    GitAction::Commit | GitAction::CommitPush | GitAction::CommitSync => Ok(()),
  }
}
