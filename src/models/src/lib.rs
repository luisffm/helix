use std::path::PathBuf;
use std::time::SystemTime;

pub type SessionId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentStatus {
  Running,
  Waiting,
  Thinking,
  Idle,
  Error,
  Finished,
}

/// What an agent wants when it rings the bell: an answer to a question, or just
/// to be read. Separate from `AgentStatus`, which reports whether the process is
/// busy rather than whether it is waiting on a person.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentAttention {
  Answer,
  Report,
}

impl AgentStatus {
  pub fn label(&self) -> &'static str {
    match self {
      AgentStatus::Running => "Running",
      AgentStatus::Waiting => "Waiting",
      AgentStatus::Thinking => "Thinking",
      AgentStatus::Idle => "Idle",
      AgentStatus::Error => "Error",
      AgentStatus::Finished => "Finished",
    }
  }
}

/// What an agent claims about itself through its terminal title.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitleStatus {
  Working,
  Idle,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionKind {
  Terminal,
  ClaudeCode,
}

impl SessionKind {
  pub fn default_title(&self, n: usize) -> String {
    match self {
      SessionKind::Terminal => format!("Terminal {n}"),
      SessionKind::ClaudeCode => format!("Claude {n}"),
    }
  }
}

#[derive(Clone, Debug)]
pub struct ProjectInfo {
  pub name: String,
  pub root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
  pub id: SessionId,
  pub title: String,
  pub kind: SessionKind,
  pub status: AgentStatus,
  pub started_at: SystemTime,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GitFileKind {
  Added,
  Modified,
  Deleted,
  Renamed,
  Typechange,
  Untracked,
  Conflicted,
}

impl GitFileKind {
  pub fn glyph(&self) -> &'static str {
    match self {
      GitFileKind::Added => "A",
      GitFileKind::Modified => "M",
      GitFileKind::Deleted => "D",
      GitFileKind::Renamed => "R",
      GitFileKind::Typechange => "T",
      GitFileKind::Untracked => "?",
      GitFileKind::Conflicted => "!",
    }
  }

  pub fn status_letter(&self) -> &'static str {
    match self {
      GitFileKind::Added => "A",
      GitFileKind::Modified => "M",
      GitFileKind::Deleted => "D",
      GitFileKind::Renamed => "R",
      GitFileKind::Typechange => "T",
      GitFileKind::Untracked => "U",
      GitFileKind::Conflicted => "!",
    }
  }

  pub fn dominance(&self) -> u8 {
    match self {
      GitFileKind::Conflicted => 6,
      GitFileKind::Deleted => 5,
      GitFileKind::Modified | GitFileKind::Typechange => 4,
      GitFileKind::Added | GitFileKind::Untracked => 3,
      GitFileKind::Renamed => 2,
    }
  }
}

#[derive(Clone, Debug)]
pub struct GitFileStatus {
  pub path: String,
  pub kind: GitFileKind,
  pub added: usize,
  pub removed: usize,
}

impl GitFileStatus {
  pub fn new(path: String, kind: GitFileKind) -> Self {
    Self {
      path,
      kind,
      added: 0,
      removed: 0,
    }
  }
}

#[derive(Clone, Debug)]
pub struct CommitInfo {
  pub short_id: String,
  pub summary: String,
  pub author: String,
  pub epoch_seconds: i64,
}

#[derive(Clone, Debug, Default)]
pub struct GitSnapshot {
  pub branch: String,
  pub head_short: Option<String>,
  pub detached: bool,
  pub is_linked_worktree: bool,
  pub main_repo: Option<PathBuf>,
  pub ahead: usize,
  pub behind: usize,
  pub upstream: Option<String>,
  pub staged: Vec<GitFileStatus>,
  pub unstaged: Vec<GitFileStatus>,
  pub untracked: Vec<GitFileStatus>,
  pub conflicted: Vec<GitFileStatus>,
  pub recent_commits: Vec<CommitInfo>,
  pub stash_count: usize,
}

impl GitSnapshot {
  pub fn dirty_count(&self) -> usize {
    self.staged.len() + self.unstaged.len() + self.untracked.len() + self.conflicted.len()
  }

  fn tracked(&self) -> impl Iterator<Item = &GitFileStatus> {
    self
      .staged
      .iter()
      .chain(&self.unstaged)
      .chain(&self.untracked)
      .chain(&self.conflicted)
  }

  /// Lines added and removed across everything the worktree has touched since
  /// HEAD. A partly staged file appears in both lists, and the two diffs cover
  /// different hunks of it, so adding them is the total rather than a double
  /// count.
  pub fn line_stats(&self) -> (usize, usize) {
    self.tracked().fold((0, 0), |(added, removed), file| {
      (added + file.added, removed + file.removed)
    })
  }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiffBase {
  Unstaged,
  Staged,
  Head,
  Branch { merge_base: String, head: String },
}

impl DiffBase {
  pub fn label(&self) -> &'static str {
    match self {
      DiffBase::Unstaged => "working tree",
      DiffBase::Staged => "staged",
      DiffBase::Head => "vs HEAD",
      DiffBase::Branch { .. } => "vs merge-base",
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DiffLineKind {
  Context,
  Added,
  Removed,
}

#[derive(Clone, Debug)]
pub struct DiffLine {
  pub kind: DiffLineKind,
  pub old_line: Option<u32>,
  pub new_line: Option<u32>,
  pub range: std::ops::Range<usize>,
}

#[derive(Clone, Debug)]
pub struct DiffHunk {
  pub old_start: u32,
  pub new_start: u32,
  pub lines: Vec<DiffLine>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DiffState {
  Text,
  Identical,
  Binary,
  TooLarge { lines: usize, chars: usize },
}

#[derive(Clone, Debug)]
pub struct FileDiff {
  pub path: String,
  pub base: DiffBase,
  pub language: String,
  pub state: DiffState,
  pub old_text: String,
  pub new_text: String,
  pub hunks: Vec<DiffHunk>,
  pub added: usize,
  pub removed: usize,
}

impl FileDiff {
  pub fn line_text(&self, line: &DiffLine) -> &str {
    let source = match line.kind {
      DiffLineKind::Removed => &self.old_text,
      _ => &self.new_text,
    };

    source.get(line.range.clone()).unwrap_or_default()
  }
}

#[cfg(test)]
mod tests {
  use super::{GitFileKind, GitFileStatus, GitSnapshot};

  fn file(added: usize, removed: usize) -> GitFileStatus {
    let mut status = GitFileStatus::new("a.rs".to_string(), GitFileKind::Modified);

    status.added = added;
    status.removed = removed;

    status
  }

  #[test]
  fn line_stats_cover_every_list() {
    let mut snapshot = GitSnapshot::default();

    snapshot.staged.push(file(10, 2));
    snapshot.unstaged.push(file(4, 1));
    snapshot.untracked.push(file(170, 0));
    snapshot.conflicted.push(file(0, 19));

    assert_eq!(snapshot.line_stats(), (184, 22));
  }

  #[test]
  fn a_clean_worktree_has_no_lines() {
    assert_eq!(GitSnapshot::default().line_stats(), (0, 0));
  }

  #[test]
  fn deleted_outranks_modified() {
    assert!(GitFileKind::Deleted.dominance() > GitFileKind::Modified.dominance());
  }

  #[test]
  fn modified_outranks_untracked() {
    assert!(GitFileKind::Modified.dominance() > GitFileKind::Untracked.dominance());
  }
}
