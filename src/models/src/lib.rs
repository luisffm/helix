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
