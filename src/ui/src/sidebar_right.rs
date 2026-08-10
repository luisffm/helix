use crate::components::{
  HEADER_HEIGHT, SpinnerClock, Spinning, ago, drive_spinner, icon_button, section_label, spinner,
};
use crate::file_icons;
use crate::icons::HelixIcon;
use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
  Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName, Sizable};
use helix_git::IgnoreProbe;
use helix_github::{BlockedReason, Eligibility, HostedReview, NextAction, ReviewLookupOutcome};
use helix_models::{DiffBase, GitFileKind, GitFileStatus, GitSnapshot};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const ROW_HEIGHT: f32 = 24.0;
const INDENT: f32 = 16.0;
const BASE_PAD: f32 = 8.0;
const MAX_ROWS: usize = 2000;
const MAX_DEPTH: usize = 16;
const FILTER_MAX_MATCHES: usize = 300;
const FILTER_MAX_DIRS: usize = 4000;
const MAX_CACHED_DIRS: usize = 4096;
const FILTER_DEBOUNCE: Duration = Duration::from_millis(100);
const IGNORED_RANK_PENALTY: u8 = 3;

/// `needle` must already be lowercase. `by_path` widens the match to the
/// workspace-relative path, which only helps once the query has a separator.
pub fn match_rank(name: &str, relative: &str, needle: &str, by_path: bool) -> Option<u8> {
  rank_lowered(&name.to_lowercase(), relative, needle, by_path)
}

fn rank_lowered(name: &str, relative: &str, needle: &str, by_path: bool) -> Option<u8> {
  if name.starts_with(needle) {
    return Some(0);
  }

  if name.contains(needle) {
    return Some(1);
  }

  if by_path && relative.to_lowercase().contains(needle) {
    return Some(2);
  }

  None
}

pub enum ContextPanelEvent {
  OpenFile { path: PathBuf, preview: bool },
  OpenDiff { relative: String, base: DiffBase },
  GitChanged,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RightTab {
  Files,
  Git,
  Pr,
}

impl RightTab {
  fn label(&self) -> &'static str {
    match self {
      RightTab::Files => "Files",
      RightTab::Git => "Git",
      RightTab::Pr => "PR",
    }
  }

  fn icon(&self) -> Icon {
    match self {
      RightTab::Files => Icon::new(IconName::File),
      RightTab::Git => Icon::new(HelixIcon::GitBranch),
      RightTab::Pr => Icon::new(HelixIcon::GitCompare),
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GitSection {
  Staged,
  Changes,
  Commits,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GitAction {
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
  fn commits(self) -> bool {
    matches!(self, Self::Commit | Self::CommitPush | Self::CommitSync)
  }

  fn remote_step(self) -> Option<Self> {
    match self {
      Self::Commit => None,
      Self::CommitPush => Some(Self::Push),
      Self::CommitSync => Some(Self::Sync),
      other => Some(other),
    }
  }
}

enum IndexOp {
  Stage(String),
  Unstage(String),
  StageAll,
  UnstageAll,
  Discard(String),
}

impl IndexOp {
  fn run(self, root: &Path) -> anyhow::Result<()> {
    match self {
      Self::Stage(relative) => helix_git::index::stage(root, &relative),
      Self::Unstage(relative) => helix_git::index::unstage(root, &relative),
      Self::StageAll => helix_git::index::stage_all(root),
      Self::UnstageAll => helix_git::index::unstage_all(root),
      Self::Discard(relative) => helix_git::index::discard(root, &relative),
    }
  }
}

fn perform_remote(
  action: GitAction,
  root: &Path,
  branch: &str,
  upstream: &str,
) -> anyhow::Result<()> {
  match action {
    GitAction::Push => helix_git::remote::push(root),
    GitAction::ForcePush => helix_git::remote::force_push(root),
    GitAction::Pull => helix_git::remote::pull(root),
    GitAction::FastForward => helix_git::remote::fast_forward(root),
    GitAction::Sync => helix_git::remote::sync(root),
    GitAction::Rebase => helix_git::remote::rebase(root, upstream),
    GitAction::Fetch => helix_git::remote::fetch(root),
    GitAction::Publish => helix_git::remote::publish(root, branch),
    GitAction::Commit | GitAction::CommitPush | GitAction::CommitSync => Ok(()),
  }
}

#[derive(Clone)]
struct FileNode {
  path: PathBuf,
  name: String,
  lower: String,
  is_dir: bool,
  ignored: bool,
}

/// Everything a git file row draws, built once when the snapshot lands so that
/// render never formats an id or walks a path again.
struct GitRow {
  path: String,
  kind: GitFileKind,
  name: SharedString,
  parent: Option<SharedString>,
  added: Option<SharedString>,
  removed: Option<SharedString>,
  group: SharedString,
  row_id: SharedString,
  toggle_id: SharedString,
  discard_id: SharedString,
}

#[derive(Default)]
struct GitRows {
  conflicted: Vec<GitRow>,
  staged: Vec<GitRow>,
  unstaged: Vec<GitRow>,
  untracked: Vec<GitRow>,
}

fn build_git_rows(files: &[GitFileStatus], prefix: &str) -> Vec<GitRow> {
  files
    .iter()
    .map(|file| {
      let as_path = Path::new(&file.path);

      let name = as_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file.path.clone());

      let parent = as_path
        .parent()
        .map(|dir| dir.to_string_lossy().to_string())
        .filter(|dir| !dir.is_empty());

      GitRow {
        kind: file.kind,
        name: name.into(),
        parent: parent.map(SharedString::from),
        added: (file.added > 0).then(|| SharedString::from(format!("+{}", file.added))),
        removed: (file.removed > 0).then(|| SharedString::from(format!("-{}", file.removed))),
        group: SharedString::from(format!("git-row-{prefix}-{}", file.path)),
        row_id: SharedString::from(format!("git-{prefix}-{}", file.path)),
        toggle_id: SharedString::from(format!("git-toggle-{prefix}-{}", file.path)),
        discard_id: SharedString::from(format!("git-discard-{prefix}-{}", file.path)),
        path: file.path.clone(),
      }
    })
    .collect()
}

fn name_cmp(a: &FileNode, b: &FileNode) -> std::cmp::Ordering {
  a.lower.cmp(&b.lower).then_with(|| a.name.cmp(&b.name))
}

fn scan_dir(dir: &Path, show_dotfiles: bool, probe: Option<&IgnoreProbe>) -> Vec<FileNode> {
  let mut nodes: Vec<FileNode> = std::fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|entry| {
      let name = entry.file_name().to_string_lossy().to_string();

      if name == ".git" || name == "node_modules" {
        return None;
      }

      if !show_dotfiles && name.starts_with('.') {
        return None;
      }

      let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
      let path = entry.path();
      let ignored = probe.is_some_and(|probe| probe.is_ignored(&path));

      Some(FileNode {
        lower: name.to_lowercase(),
        path,
        name,
        is_dir,
        ignored,
      })
    })
    .collect();

  nodes.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| name_cmp(a, b)));

  nodes
}

fn scan_matches(root: &Path, needle: &str, by_path: bool, show_dotfiles: bool) -> Vec<FileNode> {
  let probe = IgnoreProbe::open(root);

  let mut ranked: Vec<(u8, FileNode)> = Vec::new();
  let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
  let mut dirs = 0usize;

  while let Some(dir) = queue.pop_front() {
    if dirs >= FILTER_MAX_DIRS || ranked.len() >= FILTER_MAX_MATCHES {
      break;
    }

    dirs += 1;

    for node in scan_dir(&dir, show_dotfiles, probe.as_ref()) {
      if node.is_dir {
        if !node.ignored {
          queue.push_back(node.path.clone());
        }

        continue;
      }

      let relative = node
        .path
        .strip_prefix(root)
        .unwrap_or(&node.path)
        .to_string_lossy()
        .to_string();

      let Some(mut rank) = rank_lowered(&node.lower, &relative, needle, by_path) else {
        continue;
      };

      if node.ignored {
        rank += IGNORED_RANK_PENALTY;
      }

      ranked.push((rank, node));
    }
  }

  ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| name_cmp(&a.1, &b.1)));

  ranked.into_iter().map(|(_, node)| node).collect()
}

pub struct ContextPanel {
  root: PathBuf,
  active: RightTab,
  expanded: HashSet<PathBuf>,
  dir_cache: BTreeMap<PathBuf, Vec<Rc<FileNode>>>,
  scanning: HashSet<PathBuf>,
  scan_token: u64,
  generation: u64,
  rows_generation: u64,
  rows: Vec<(Rc<FileNode>, usize)>,
  matches: Vec<Rc<FileNode>>,
  filter_token: u64,
  filtering: bool,
  git: Option<GitSnapshot>,
  git_rows: GitRows,
  file_status: HashMap<PathBuf, GitFileKind>,
  dir_status: HashMap<PathBuf, GitFileKind>,
  selected: Option<PathBuf>,
  show_dotfiles: bool,
  file_filter: Entity<InputState>,
  commit_message: Entity<InputState>,
  generating_message: bool,
  collapsed: HashSet<GitSection>,
  git_error: Option<String>,
  discard_armed: Option<String>,
  git_menu_open: bool,
  git_busy: bool,
  pr: Option<HostedReview>,
  pr_eligibility: Option<Eligibility>,
  pr_busy: bool,
  spin: SpinnerClock,
}

impl EventEmitter<ContextPanelEvent> for ContextPanel {}

impl Spinning for ContextPanel {
  fn spinner_clock(&mut self) -> &mut SpinnerClock {
    &mut self.spin
  }

  fn spinner_active(&self, _cx: &App) -> bool {
    self.pr_busy
  }
}

impl ContextPanel {
  pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let commit_message = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(2, 6)
        .placeholder("Commit message")
    });

    let file_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Find files"));

    cx.subscribe(&file_filter, |panel, _, event: &InputEvent, cx| {
      if matches!(event, InputEvent::Change) {
        panel.schedule_filter(cx);
      }
    })
    .detach();

    Self {
      root,
      active: RightTab::Files,
      expanded: HashSet::new(),
      dir_cache: BTreeMap::new(),
      scanning: HashSet::new(),
      scan_token: 0,
      generation: 0,
      rows_generation: u64::MAX,
      rows: Vec::new(),
      matches: Vec::new(),
      filter_token: 0,
      filtering: false,
      git: None,
      git_rows: GitRows::default(),
      file_status: HashMap::new(),
      dir_status: HashMap::new(),
      selected: None,
      show_dotfiles: false,
      file_filter,
      commit_message,
      generating_message: false,
      collapsed: HashSet::from([GitSection::Commits]),
      git_error: None,
      discard_armed: None,
      git_menu_open: false,
      git_busy: false,
      pr: None,
      pr_eligibility: None,
      pr_busy: false,
      spin: SpinnerClock::default(),
    }
  }

  fn is_open(&self, section: GitSection) -> bool {
    !self.collapsed.contains(&section)
  }

  fn toggle_section(&mut self, section: GitSection, cx: &mut Context<Self>) {
    if !self.collapsed.remove(&section) {
      self.collapsed.insert(section);
    }

    cx.notify();
  }

  fn section_toggle(
    &self,
    id: &'static str,
    label: &'static str,
    count: usize,
    section: GitSection,
    action: Option<AnyElement>,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let open = self.is_open(section);

    div()
      .id(id)
      .group(SharedString::from(id))
      .flex()
      .items_center()
      .gap_1()
      .px_1()
      .pt_3()
      .pb_1()
      .rounded_md()
      .cursor_pointer()
      .text_xs()
      .text_color(theme.text_dim)
      .hover(|s| s.text_color(theme.text_muted))
      .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(section, cx)))
      .child(
        Icon::new(if open {
          IconName::ChevronDown
        } else {
          IconName::ChevronRight
        })
        .size_3(),
      )
      .child(format!("{label} ({count})"))
      .child(div().flex_1())
      .children(action)
      .into_any_element()
  }

  /// Bulk action shown on a section header. Kept mounted and dim rather than
  /// hidden on hover: an invisible element still takes clicks, and an
  /// accidental "unstage everything" is not worth the tidier header.
  fn header_action(
    &self,
    id: &'static str,
    group: &'static str,
    icon: IconName,
    tooltip: &'static str,
    enabled: bool,
    theme: &Theme,
  ) -> gpui::Stateful<gpui::Div> {
    div()
      .id(id)
      .size(px(18.0))
      .flex()
      .flex_none()
      .items_center()
      .justify_center()
      .rounded_sm()
      .text_color(theme.text_dim)
      .when(enabled, |el| {
        el.cursor_pointer()
          .group_hover(group, |s| s.text_color(theme.text_muted))
          .hover(|s| s.bg(theme.hover).text_color(theme.text))
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(tooltip).build(window, cx)
          })
      })
      .child(Icon::new(icon).size_3())
  }

  /// Index writes are serialized behind `git_busy`: git2 rewrites the whole
  /// index file, so two of them racing on one repository would lose work.
  fn run_index_op(&mut self, op: IndexOp, cx: &mut Context<Self>) {
    if self.git_busy {
      return;
    }

    self.git_busy = true;
    self.git_error = None;

    cx.notify();

    let root = self.root.clone();
    let task = cx.background_executor().spawn(async move { op.run(&root) });

    cx.spawn(async move |this, cx| {
      let result = task.await;

      this
        .update(cx, |panel, cx| {
          panel.git_busy = false;
          panel.git_error = result.err().map(|err| err.to_string());

          cx.emit(ContextPanelEvent::GitChanged);
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn stage(&mut self, relative: String, cx: &mut Context<Self>) {
    self.run_index_op(IndexOp::Stage(relative), cx);
  }

  fn run_git_action(&mut self, action: GitAction, window: &mut Window, cx: &mut Context<Self>) {
    self.git_menu_open = false;

    if self.git_busy {
      return;
    }

    let message = action
      .commits()
      .then(|| self.commit_message.read(cx).value().to_string());
    let step = action.remote_step();

    self.git_busy = true;
    self.git_error = None;
    cx.notify();

    let root = self.root.clone();
    let branch = self
      .git
      .as_ref()
      .map(|git| git.branch.clone())
      .unwrap_or_default();

    let upstream = self
      .git
      .as_ref()
      .and_then(|git| git.upstream.clone())
      .unwrap_or_default();

    let task = cx.background_executor().spawn(async move {
      if let Some(message) = &message {
        if let Err(err) = helix_git::index::commit(&root, message) {
          return (false, Err(err));
        }
      }

      let result = match step {
        Some(step) => perform_remote(step, &root, &branch, &upstream),
        None => Ok(()),
      };

      (message.is_some(), result)
    });

    let this = cx.entity().downgrade();

    window
      .spawn(cx, async move |cx| {
        let (committed, result) = task.await;

        this
          .update_in(cx, |panel, window, cx| {
            panel.git_busy = false;
            panel.git_error = result.err().map(|err| err.to_string());

            if committed && panel.git_error.is_none() {
              panel
                .commit_message
                .update(cx, |state, cx| state.set_value("", window, cx));
            }

            cx.emit(ContextPanelEvent::GitChanged);
            cx.notify();
          })
          .ok();
      })
      .detach();
  }

  fn discard(&mut self, relative: String, cx: &mut Context<Self>) {
    self.discard_armed = None;

    self.run_index_op(IndexOp::Discard(relative), cx);
  }

  fn unstage(&mut self, relative: String, cx: &mut Context<Self>) {
    self.run_index_op(IndexOp::Unstage(relative), cx);
  }

  fn stage_all(&mut self, cx: &mut Context<Self>) {
    self.run_index_op(IndexOp::StageAll, cx);
  }

  fn unstage_all(&mut self, cx: &mut Context<Self>) {
    self.run_index_op(IndexOp::UnstageAll, cx);
  }

  fn generate_commit_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.generating_message {
      return;
    }

    let staged = self
      .git
      .as_ref()
      .map(|git| git.staged.len())
      .unwrap_or_default();

    if staged == 0 {
      self.git_error = Some("stage something before writing a message".to_string());
      cx.notify();

      return;
    }

    self.generating_message = true;
    self.git_error = None;

    cx.notify();

    let root = self.root.clone();
    let branch = self
      .git
      .as_ref()
      .map(|git| git.branch.clone())
      .unwrap_or_default();

    let task = cx.background_executor().spawn(async move {
      let context = helix_agents::commit_message::Context {
        branch,
        name_status: helix_git::index::staged_name_status(&root).unwrap_or_default(),
        patch: helix_git::index::staged_patch(
          &root,
          helix_agents::commit_message::PATCH_BYTE_BUDGET,
        )
        .unwrap_or_default(),
      };

      if context.patch.trim().is_empty() && context.name_status.trim().is_empty() {
        return Err(anyhow::anyhow!("nothing staged to describe"));
      }

      let prompt = helix_agents::commit_message::build_prompt(&context, None);
      let spec = helix_agents::commit_message::plan(prompt, None);

      helix_agents::commit_message::generate(&root, &spec)
    });

    let this = cx.entity().downgrade();

    window
      .spawn(cx, async move |cx| {
        let result = task.await;

        this
          .update_in(cx, |panel, window, cx| {
            panel.generating_message = false;

            match result {
              Ok(message) => panel
                .commit_message
                .update(cx, |state, cx| state.set_value(message, window, cx)),
              Err(err) => panel.git_error = Some(err.to_string()),
            }

            cx.notify();
          })
          .ok();
      })
      .detach();
  }

  pub fn refresh_pull_request(&mut self, cx: &mut Context<Self>) {
    if self.pr_busy {
      return;
    }

    let Some(git) = self.git.clone() else { return };

    if git.detached {
      return;
    }

    self.pr_busy = true;

    let root = self.root.clone();
    let branch = git.branch.clone();
    let dirty_count = git.dirty_count();
    let ahead = git.ahead;
    let behind = git.behind;

    let task = cx.background_executor().spawn(async move {
      let gh_installed = helix_github::gh::is_installed();
      let authenticated = gh_installed && helix_github::gh::is_authenticated();

      let (lookup, review) = if authenticated {
        match helix_github::review::for_branch(&root, &branch) {
          Ok(Some(review)) => (ReviewLookupOutcome::Found, Some(review)),
          Ok(None) => (ReviewLookupOutcome::NotFound, None),
          Err(_) => (ReviewLookupOutcome::Unavailable, None),
        }
      } else {
        (ReviewLookupOutcome::Unavailable, None)
      };

      let base_ref = helix_git::diff::default_base_ref(&root);

      let commits_ahead_of_base = base_ref
        .as_deref()
        .and_then(|base| helix_git::remote::commits_ahead_of(&root, base).ok())
        .unwrap_or(0);

      let has_upstream = ahead > 0 || behind > 0 || commits_ahead_of_base == 0 || {
        std::process::Command::new("git")
          .args(["rev-parse", "--abbrev-ref", "@{upstream}"])
          .current_dir(&root)
          .output()
          .map(|output| output.status.success())
          .unwrap_or(false)
      };

      let state = helix_github::eligibility::RepoState {
        gh_installed,
        authenticated,
        detached: false,
        branch,
        base_ref,
        dirty_count,
        has_upstream,
        ahead,
        behind,
        commits_ahead_of_base,
      };

      let eligibility = helix_github::eligibility::evaluate(&state, lookup, review.clone());

      (eligibility, review)
    });

    cx.spawn(async move |this, cx| {
      let (eligibility, review) = task.await;

      this
        .update(cx, |panel, cx| {
          panel.pr_busy = false;
          panel.pr_eligibility = Some(eligibility);
          panel.pr = review;

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  pub fn set_git(&mut self, git: Option<GitSnapshot>, cx: &mut Context<Self>) {
    self.rebuild_status(git.as_ref());

    self.git_rows = match git.as_ref() {
      Some(git) => GitRows {
        conflicted: build_git_rows(&git.conflicted, "conflict"),
        staged: build_git_rows(&git.staged, "staged"),
        unstaged: build_git_rows(&git.unstaged, "unstaged"),
        untracked: build_git_rows(&git.untracked, "untracked"),
      },
      None => GitRows::default(),
    };

    self.git = git;

    cx.notify();
  }

  pub fn refresh_files(&mut self, changed: Option<&[PathBuf]>, cx: &mut Context<Self>) {
    match changed {
      Some(paths) => {
        let mut invalidated = false;

        for path in paths {
          let dirs = [Some(path.as_path()), path.parent()];

          for dir in dirs.into_iter().flatten() {
            invalidated |= self.dir_cache.remove(dir).is_some();
          }
        }

        if !invalidated {
          return;
        }
      }
      None => {
        if self.dir_cache.is_empty() {
          return;
        }

        self.dir_cache.clear();
      }
    }

    self.invalidate_rows();

    cx.notify();
  }

  pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
    self.root = root;
    self.expanded.clear();
    self.git = None;
    self.file_status.clear();
    self.dir_status.clear();
    self.selected = None;

    self.reset_scans();

    cx.notify();
  }

  fn reset_scans(&mut self) {
    self.dir_cache.clear();
    self.scanning.clear();
    self.matches.clear();
    self.scan_token = self.scan_token.wrapping_add(1);

    self.invalidate_rows();
  }

  fn invalidate_rows(&mut self) {
    self.generation = self.generation.wrapping_add(1);
  }

  /// Only the root and the expanded directories can appear as rows; anything
  /// else in the cache is a directory the tree walked past once.
  fn trim_dir_cache(&mut self) {
    if self.dir_cache.len() <= MAX_CACHED_DIRS {
      return;
    }

    let root = self.root.clone();

    self
      .dir_cache
      .retain(|dir, _| *dir == root || self.expanded.contains(dir));
  }

  pub fn set_selected(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
    self.selected = path;

    cx.notify();
  }

  fn rebuild_status(&mut self, git: Option<&GitSnapshot>) {
    self.file_status.clear();
    self.dir_status.clear();
    let Some(git) = git else { return };

    let groups = [&git.conflicted, &git.staged, &git.unstaged, &git.untracked];

    for group in groups {
      for file in group.iter() {
        let path = self.root.join(&file.path);

        let replace = self
          .file_status
          .get(&path)
          .map(|existing| file_icons::dominance(file.kind) > file_icons::dominance(*existing))
          .unwrap_or(true);

        if replace {
          self.file_status.insert(path, file.kind);
        }
      }
    }

    let entries: Vec<(PathBuf, GitFileKind)> = self
      .file_status
      .iter()
      .map(|(path, kind)| (path.clone(), *kind))
      .collect();

    for (path, kind) in entries {
      if kind == GitFileKind::Deleted {
        continue;
      }

      for ancestor in path.ancestors().skip(1) {
        if !ancestor.starts_with(&self.root) || ancestor == self.root {
          break;
        }

        let replace = self
          .dir_status
          .get(ancestor)
          .map(|existing| file_icons::dominance(kind) > file_icons::dominance(*existing))
          .unwrap_or(true);

        if replace {
          self.dir_status.insert(ancestor.to_path_buf(), kind);
        }
      }
    }
  }

  fn request_scan(&mut self, dir: PathBuf, cx: &mut Context<Self>) {
    if self.dir_cache.contains_key(&dir) || !self.scanning.insert(dir.clone()) {
      return;
    }

    let show_dotfiles = self.show_dotfiles;
    let root = self.root.clone();
    let token = self.scan_token;
    let target = dir.clone();

    let task = cx.background_executor().spawn(async move {
      let probe = IgnoreProbe::open(&root);

      scan_dir(&target, show_dotfiles, probe.as_ref())
    });

    cx.spawn(async move |this, cx| {
      let nodes = task.await;

      this
        .update(cx, |panel, cx| {
          if panel.scan_token != token {
            return;
          }

          panel.scanning.remove(&dir);
          panel
            .dir_cache
            .insert(dir.clone(), nodes.into_iter().map(Rc::new).collect());

          panel.trim_dir_cache();
          panel.invalidate_rows();

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn collect_rows(
    &self,
    dir: &Path,
    depth: usize,
    out: &mut Vec<(Rc<FileNode>, usize)>,
    pending: &mut Vec<PathBuf>,
  ) {
    if depth > MAX_DEPTH || out.len() > MAX_ROWS {
      return;
    }

    let Some(children) = self.dir_cache.get(dir) else {
      pending.push(dir.to_path_buf());

      return;
    };

    for node in children {
      let expanded = node.is_dir && self.expanded.contains(&node.path);

      out.push((node.clone(), depth));

      if expanded {
        self.collect_rows(&node.path, depth + 1, out, pending);
      }
    }
  }

  fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
    let mut rows = Vec::new();
    let mut pending = Vec::new();

    self.collect_rows(&self.root.clone(), 0, &mut rows, &mut pending);

    self.rows = rows;
    self.rows_generation = self.generation;

    for dir in pending {
      self.request_scan(dir, cx);
    }
  }

  fn schedule_filter(&mut self, cx: &mut Context<Self>) {
    self.filter_token = self.filter_token.wrapping_add(1);

    let token = self.filter_token;
    let query = self.file_filter.read(cx).value().trim().to_string();

    if query.is_empty() {
      self.matches.clear();
      self.filtering = false;

      cx.notify();

      return;
    }

    self.filtering = true;

    let root = self.root.clone();
    let show_dotfiles = self.show_dotfiles;

    cx.spawn(async move |this, cx| {
      cx.background_executor().timer(FILTER_DEBOUNCE).await;

      if !matches!(this.update(cx, |panel, _| panel.filter_token), Ok(current) if current == token)
      {
        return;
      }

      let found = cx
        .background_executor()
        .spawn(async move {
          let needle = query.to_lowercase();
          let by_path = needle.contains('/');

          scan_matches(&root, &needle, by_path, show_dotfiles)
        })
        .await;

      this
        .update(cx, |panel, cx| {
          if panel.filter_token != token {
            return;
          }

          panel.matches = found.into_iter().map(Rc::new).collect();
          panel.filtering = false;

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn render_filter(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let dirty = !self.file_filter.read(cx).value().trim().is_empty();
    div()
      .flex()
      .flex_none()
      .items_center()
      .gap_1()
      .h(px(28.0))
      .mx_2()
      .mt_2()
      .px_2()
      .rounded_md()
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.elevated)
      .child(
        div()
          .flex_none()
          .text_color(theme.text_dim)
          .child(Icon::new(IconName::Search).size_3()),
      )
      .child(
        div()
          .flex_1()
          .min_w_0()
          .child(Input::new(&self.file_filter).appearance(false).xsmall()),
      )
      .when(dirty, |el| {
        el.child(
          div()
            .id("files-filter-clear")
            .flex_none()
            .size(px(16.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .text_color(theme.text_dim)
            .hover(|s| s.bg(theme.hover).text_color(theme.text))
            .on_click(cx.listener(|this, _, window, cx| {
              this
                .file_filter
                .update(cx, |state, cx| state.set_value("", window, cx));
              cx.notify();
            }))
            .child(Icon::new(IconName::Close).size_3()),
        )
      })
      .into_any_element()
  }

  fn render_files(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let query = self.file_filter.read(cx).value().trim().to_string();
    let filter = self.render_filter(theme, cx);

    let body = if query.is_empty() {
      if self.rows_generation != self.generation {
        self.rebuild_rows(cx);
      }

      if self.rows.is_empty() {
        self.scanning_or_empty("No files in this workspace", theme)
      } else {
        div()
          .id("files-scroll")
          .flex_1()
          .min_h_0()
          .overflow_y_scroll()
          .py_2()
          .px_1()
          .flex()
          .flex_col()
          .children(
            self
              .rows
              .iter()
              .enumerate()
              .map(|(ix, (node, depth))| self.render_row(ix, node, *depth, theme, cx)),
          )
          .into_any_element()
      }
    } else if self.matches.is_empty() {
      self.scanning_or_empty("No file matches", theme)
    } else {
      div()
        .id("files-matches")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .py_2()
        .px_1()
        .flex()
        .flex_col()
        .children(
          self
            .matches
            .iter()
            .enumerate()
            .map(|(ix, node)| self.render_match(ix, node, theme, cx)),
        )
        .into_any_element()
    };

    div()
      .flex_1()
      .min_h_0()
      .flex()
      .flex_col()
      .child(filter)
      .child(body)
      .into_any_element()
  }

  fn scanning_or_empty(&self, message: &'static str, theme: &Theme) -> AnyElement {
    if self.scanning.is_empty() && !self.filtering {
      self.empty_files(message, theme)
    } else {
      div().flex_1().into_any_element()
    }
  }

  fn empty_files(&self, message: &'static str, theme: &Theme) -> AnyElement {
    div()
      .flex_1()
      .flex()
      .items_center()
      .justify_center()
      .text_xs()
      .text_color(theme.text_dim)
      .child(message)
      .into_any_element()
  }

  fn render_match(
    &self,
    ix: usize,
    node: &FileNode,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let status = self.file_status.get(&node.path).copied();
    let name_color = match status {
      Some(kind) => file_icons::status_color(kind, theme),
      None if node.ignored => file_icons::ignored_color(),
      None => theme.text,
    };
    let parent = node
      .path
      .parent()
      .and_then(|dir| dir.strip_prefix(&self.root).ok())
      .map(|dir| dir.to_string_lossy().to_string())
      .filter(|dir| !dir.is_empty());
    let is_selected = self.selected.as_ref() == Some(&node.path);
    let path = node.path.clone();

    let mut row = div()
      .id(SharedString::from(format!("file-match-{ix}")))
      .flex()
      .items_center()
      .gap_1()
      .h(px(ROW_HEIGHT))
      .pl(px(BASE_PAD))
      .pr_2()
      .rounded_md()
      .cursor_pointer()
      .text_xs()
      .when(is_selected, |el| {
        el.bg(theme.elevated)
          .border_1()
          .border_color(theme.panel_border)
      })
      .when(!is_selected, |el| el.hover(|s| s.bg(theme.hover)))
      .on_click(
        cx.listener(move |this, event: &gpui::ClickEvent, _window, cx| {
          this.selected = Some(path.clone());
          cx.emit(ContextPanelEvent::OpenFile {
            path: path.clone(),
            preview: event.click_count() < 2,
          });
          cx.notify();
        }),
      )
      .child(
        div()
          .flex_none()
          .text_color(if node.ignored {
            file_icons::ignored_color()
          } else {
            theme.text_dim
          })
          .child(file_icons::icon(&node.path).size_3()),
      )
      .child(
        div()
          .flex_none()
          .text_color(name_color)
          .child(node.name.clone()),
      );

    if let Some(parent) = parent {
      row = row.child(
        div()
          .flex_1()
          .min_w_0()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_color(theme.text_dim)
          .child(parent),
      );
    } else {
      row = row.child(div().flex_1());
    }

    if let Some(kind) = status {
      row = row.child(
        div()
          .flex_none()
          .text_color(file_icons::status_color(kind, theme))
          .child(file_icons::status_letter(kind)),
      );
    }

    row.into_any_element()
  }

  fn render_row(
    &self,
    ix: usize,
    node: &FileNode,
    depth: usize,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let expanded = self.expanded.contains(&node.path);
    let is_selected = self.selected.as_ref() == Some(&node.path);
    let status = if node.is_dir {
      self.dir_status.get(&node.path).copied()
    } else {
      self.file_status.get(&node.path).copied()
    };
    let name_color = match status {
      Some(kind) => file_icons::status_color(kind, theme),
      None if node.ignored => file_icons::ignored_color(),
      None => theme.text,
    };
    let icon_color = if node.ignored {
      file_icons::ignored_color()
    } else {
      theme.text_dim
    };

    let leading: AnyElement = if node.is_dir {
      div()
        .flex_none()
        .size(px(12.0))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.text_dim)
        .child(
          Icon::new(if expanded {
            IconName::ChevronDown
          } else {
            IconName::ChevronRight
          })
          .size_3(),
        )
        .into_any_element()
    } else {
      div().flex_none().size(px(12.0)).into_any_element()
    };

    let type_icon = if node.is_dir {
      file_icons::folder_icon(expanded)
    } else {
      file_icons::icon(&node.path)
    };

    let path = node.path.clone();
    let is_dir = node.is_dir;

    let mut row = div()
      .id(SharedString::from(format!("file-row-{ix}")))
      .flex()
      .items_center()
      .gap_1()
      .h(px(ROW_HEIGHT))
      .pl(px(BASE_PAD + depth as f32 * INDENT))
      .pr_2()
      .rounded_md()
      .cursor_pointer()
      .text_xs()
      .when(is_selected, |el| {
        el.bg(theme.elevated)
          .border_1()
          .border_color(theme.panel_border)
      })
      .when(!is_selected, |el| el.hover(|s| s.bg(theme.hover)))
      .on_click(
        cx.listener(move |this, event: &gpui::ClickEvent, _window, cx| {
          if is_dir {
            if !this.expanded.insert(path.clone()) {
              this.expanded.remove(&path);
            }

            this.invalidate_rows();
            cx.notify();
          } else {
            this.selected = Some(path.clone());
            cx.emit(ContextPanelEvent::OpenFile {
              path: path.clone(),
              preview: event.click_count() < 2,
            });
            cx.notify();
          }
        }),
      )
      .child(leading)
      .child(
        div()
          .flex_none()
          .text_color(icon_color)
          .child(type_icon.size_3()),
      )
      .child(
        div()
          .flex_1()
          .min_w_0()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_color(name_color)
          .child(node.name.clone()),
      );

    if let Some(kind) = status {
      row = row.child(
        div()
          .flex_none()
          .text_color(file_icons::status_color(kind, theme))
          .child(file_icons::status_letter(kind)),
      );
    }

    row.into_any_element()
  }

  fn render_files_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let name = self
      .root
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_else(|| self.root.display().to_string());

    div()
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .h(px(32.0))
      .px_2()
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .flex_1()
          .min_w_0()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_xs()
          .text_color(theme.text)
          .child(name),
      )
      .child(
        icon_button("files-collapse", HelixIcon::ListCollapse, theme).on_click(cx.listener(
          |this, _, _, cx| {
            this.expanded.clear();
            this.invalidate_rows();
            cx.notify();
          },
        )),
      )
      .child(
        icon_button("files-refresh", HelixIcon::Refresh, theme).on_click(cx.listener(
          |this, _, _, cx| {
            this.reset_scans();
            this.schedule_filter(cx);
            cx.notify();
          },
        )),
      )
      .child(
        icon_button("files-dotfiles", IconName::Eye, theme).on_click(cx.listener(
          |this, _, _, cx| {
            this.show_dotfiles = !this.show_dotfiles;

            this.reset_scans();
            this.schedule_filter(cx);
            cx.notify();
          },
        )),
      )
      .into_any_element()
  }

  fn git_file_rows(
    &self,
    rows: &[GitRow],
    base: DiffBase,
    staged: bool,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> Vec<AnyElement> {
    rows
      .iter()
      .map(|file| {
        let color = file_icons::status_color(file.kind, theme);
        let group = file.group.clone();
        let relative = file.path.clone();
        let toggle_path = file.path.clone();
        let discard_path = file.path.clone();
        let base = base.clone();
        let armed = self.discard_armed.as_deref() == Some(file.path.as_str());
        let as_path = Path::new(&file.path);

        let stat = div()
          .flex_none()
          .flex()
          .items_center()
          .gap_1()
          .group_hover(group.clone(), |s| s.invisible())
          .children(
            file
              .added
              .clone()
              .map(|added| div().text_color(theme.green).child(added)),
          )
          .children(
            file
              .removed
              .clone()
              .map(|removed| div().text_color(theme.red).child(removed)),
          )
          .child(
            div()
              .text_color(color)
              .child(file_icons::status_letter(file.kind)),
          );

        let toggle = div()
          .id(file.toggle_id.clone())
          .flex_none()
          .size(px(18.0))
          .flex()
          .items_center()
          .justify_center()
          .rounded_sm()
          .cursor_pointer()
          .text_color(theme.text_dim)
          .hover(|s| s.bg(theme.elevated).text_color(theme.text))
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(if staged {
              "Unstage this file"
            } else {
              "Stage this file"
            })
            .build(window, cx)
          })
          .on_click(cx.listener(move |this, _, _, cx| {
            cx.stop_propagation();
            this.discard_armed = None;
            if staged {
              this.unstage(toggle_path.clone(), cx);
            } else {
              this.stage(toggle_path.clone(), cx);
            }
          }))
          .child(
            Icon::new(if staged {
              IconName::Minus
            } else {
              IconName::Plus
            })
            .size_3(),
          );

        let discard = (!staged).then(|| {
          div()
            .id(file.discard_id.clone())
            .flex_none()
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .text_color(if armed { theme.red } else { theme.text_dim })
            .hover(|s| s.bg(theme.elevated))
            .tooltip(move |window, cx| {
              gpui_component::tooltip::Tooltip::new(if armed {
                "Click again to throw the changes away"
              } else {
                "Discard changes"
              })
              .build(window, cx)
            })
            .on_click(cx.listener(move |this, _, _, cx| {
              cx.stop_propagation();
              if this.discard_armed.as_deref() == Some(discard_path.as_str()) {
                this.discard(discard_path.clone(), cx);
              } else {
                this.discard_armed = Some(discard_path.clone());
                cx.notify();
              }
            }))
            .child(
              Icon::new(if armed {
                IconName::Check
              } else {
                IconName::Undo2
              })
              .size_3(),
            )
        });

        div()
          .id(file.row_id.clone())
          .group(group.clone())
          .relative()
          .flex()
          .items_center()
          .gap_2()
          .h(px(ROW_HEIGHT))
          .px_2()
          .rounded_md()
          .cursor_pointer()
          .text_xs()
          .hover(|s| s.bg(theme.hover))
          .on_click(cx.listener(move |this, _, _, cx| {
            this.discard_armed = None;
            cx.emit(ContextPanelEvent::OpenDiff {
              relative: relative.clone(),
              base: base.clone(),
            });
          }))
          .child(
            div()
              .flex_none()
              .text_color(theme.text_dim)
              .child(file_icons::icon(as_path).size_3()),
          )
          .child(
            div()
              .flex_none()
              .text_color(theme.text)
              .child(file.name.clone()),
          )
          .children(file.parent.clone().map(|parent| {
            div()
              .flex_1()
              .min_w_0()
              .overflow_hidden()
              .whitespace_nowrap()
              .text_color(theme.text_dim)
              .child(parent)
          }))
          .child(div().flex_1())
          .child(stat)
          .child(
            div()
              .absolute()
              .right_1()
              .flex()
              .items_center()
              .gap_0p5()
              .invisible()
              .group_hover(group, |s| s.visible())
              .children(discard)
              .child(toggle),
          )
          .into_any_element()
      })
      .collect()
  }

  fn render_git_menu(
    &self,
    git: &GitSnapshot,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let staged = !git.staged.is_empty();
    let published = git.upstream.is_some();
    let upstream = git
      .upstream
      .clone()
      .unwrap_or_else(|| "upstream".to_string());
    let feature_branch = !matches!(git.branch.as_str(), "main" | "master" | "trunk");

    let entries: Vec<(&'static str, Option<String>, bool, Option<GitAction>)> = vec![
      ("Commit", None, staged, Some(GitAction::Commit)),
      (
        "Commit & Push",
        None,
        staged && published,
        Some(GitAction::CommitPush),
      ),
      (
        "Commit & Sync",
        None,
        staged && published,
        Some(GitAction::CommitSync),
      ),
      ("", None, false, None),
      ("Push", None, published, Some(GitAction::Push)),
      ("Force Push", None, published, Some(GitAction::ForcePush)),
      (
        "Create PR",
        (!feature_branch).then(|| "Switch to a feature branch".to_string()),
        feature_branch,
        None,
      ),
      ("", None, false, None),
      ("Pull", None, published, Some(GitAction::Pull)),
      (
        "Fast-forward",
        None,
        published && git.behind > 0,
        Some(GitAction::FastForward),
      ),
      ("Sync", None, published, Some(GitAction::Sync)),
      (
        "Rebase",
        Some(format!("from {upstream}")),
        published,
        Some(GitAction::Rebase),
      ),
      ("Fetch", None, true, Some(GitAction::Fetch)),
      (
        "Publish Branch",
        published.then(|| format!("already tracking {upstream}")),
        !published,
        Some(GitAction::Publish),
      ),
    ];

    let mut menu = div()
      .id("git-menu")
      .occlude()
      .absolute()
      .top(px(30.0))
      .right_0()
      .w(px(240.0))
      .max_h(px(420.0))
      .overflow_y_scroll()
      .rounded_lg()
      .border_1()
      .border_color(theme.panel_border)
      .bg(crate::theme::ca(0x1b1b1bfa))
      .shadow_lg()
      .py_1()
      .flex()
      .flex_col();

    for (ix, (label, hint, enabled, action)) in entries.into_iter().enumerate() {
      if label.is_empty() {
        menu = menu.child(div().my_1().h(px(1.0)).bg(theme.panel_border).flex_none());
        continue;
      }
      let enabled = enabled && !self.git_busy;
      menu = menu.child(
        div()
          .id(SharedString::from(format!("git-menu-{ix}")))
          .flex()
          .flex_col()
          .px_3()
          .py_1()
          .text_xs()
          .when(enabled, |el| {
            el.cursor_pointer()
              .text_color(theme.text)
              .hover(|s| s.bg(theme.hover))
          })
          .when(!enabled, |el| el.text_color(theme.text_dim))
          .when_some(action.filter(|_| enabled), |el, action| {
            el.on_click(cx.listener(move |this, _, window, cx| {
              cx.stop_propagation();
              this.run_git_action(action, window, cx);
            }))
          })
          .when(enabled && action.is_none(), |el| {
            el.on_click(cx.listener(|this, _, _, cx| {
              cx.stop_propagation();
              this.git_menu_open = false;
              this.active = RightTab::Pr;
              cx.notify();
            }))
          })
          .child(label)
          .children(hint.map(|hint| div().text_color(theme.text_dim).child(hint))),
      );
    }

    menu.into_any_element()
  }

  fn render_commit_box(
    &self,
    git: &GitSnapshot,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let can_commit = !git.staged.is_empty();
    let can_write = can_commit && !self.generating_message;
    let pending = git.unstaged.len() + git.untracked.len() + git.conflicted.len();
    let primary_enabled = (can_commit || pending > 0) && !self.git_busy;
    // Always interactive: gating the handler on `can_write` left a dim glyph with
    // no click, no tooltip and no way to learn why. It now explains itself.
    let write = div()
      .id("generate-message-button")
      .size(px(20.0))
      .flex()
      .flex_none()
      .items_center()
      .justify_center()
      .rounded_sm()
      .cursor_pointer()
      .text_color(if can_write {
        theme.claude
      } else {
        theme.text_dim
      })
      .hover(|s| s.bg(theme.hover))
      .tooltip(move |window, cx| {
        gpui_component::tooltip::Tooltip::new(if can_write {
          "Write the message from the staged diff"
        } else {
          "Stage something first"
        })
        .build(window, cx)
      })
      .on_click(cx.listener(|this, _, window, cx| this.generate_commit_message(window, cx)))
      .child(if self.generating_message {
        "…"
      } else {
        "✦"
      });

    div()
      .flex()
      .flex_none()
      .flex_col()
      .gap_1()
      .px_2()
      .py_2()
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        // The sparkle is an overlay rather than Input::suffix: the suffix slot
        // is vertically centred (wrong for a multi-line field) and the input
        // captures the mouse on press for drag-selection, so a click there
        // never completes. `occlude` keeps the press off the editor below.
        div()
          .relative()
          .rounded_md()
          .border_1()
          .border_color(theme.panel_border)
          .bg(theme.elevated)
          .pl_1()
          .pr(px(24.0))
          .child(Input::new(&self.commit_message).appearance(false))
          .child(
            div()
              .absolute()
              .top(px(3.0))
              .right(px(3.0))
              .occlude()
              .child(write),
          ),
      )
      .child(
        div()
          .relative()
          .flex()
          .items_center()
          .gap_0p5()
          .child(
            div()
              .id("commit-button")
              .flex_1()
              .h(px(26.0))
              .flex()
              .items_center()
              .justify_center()
              .rounded_md()
              .gap_1()
              .text_xs()
              .bg(theme.elevated)
              .when(primary_enabled, |el| {
                el.text_color(theme.text)
                  .cursor_pointer()
                  .hover(|s| s.bg(theme.hover))
                  .on_click(cx.listener(move |this, _, window, cx| {
                    if can_commit {
                      this.run_git_action(GitAction::Commit, window, cx);
                    } else {
                      this.stage_all(cx);
                    }
                  }))
              })
              .when(!primary_enabled, |el| el.text_color(theme.text_dim))
              .children(
                (!can_commit && !self.git_busy)
                  .then(|| div().flex_none().child(Icon::new(IconName::Plus).size_3())),
              )
              .child(if self.git_busy {
                "Working…".to_string()
              } else if can_commit {
                format!("Commit ({})", git.staged.len())
              } else {
                "Stage All".to_string()
              }),
          )
          .child(
            div()
              .id("commit-menu-button")
              .flex_none()
              .w(px(26.0))
              .h(px(26.0))
              .flex()
              .items_center()
              .justify_center()
              .rounded_md()
              .bg(theme.elevated)
              .text_color(theme.text_muted)
              .cursor_pointer()
              .hover(|s| s.bg(theme.hover).text_color(theme.text))
              .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new("More commit and remote actions")
                  .build(window, cx)
              })
              .on_click(cx.listener(|this, _, _, cx| {
                this.git_menu_open = !this.git_menu_open;
                cx.notify();
              }))
              .child(Icon::new(IconName::ChevronDown).size_3()),
          )
          .children(
            self
              .git_menu_open
              .then(|| gpui::deferred(self.render_git_menu(git, theme, cx))),
          ),
      )
      .children(
        self
          .git_error
          .clone()
          .map(|err| div().text_xs().text_color(theme.red).child(err)),
      )
      .into_any_element()
  }

  fn render_git(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let Some(git) = self.git.as_ref() else {
      return div()
        .p_3()
        .text_xs()
        .text_color(theme.text_dim)
        .child("not a git repository")
        .into_any_element();
    };

    let commit_box = self.render_commit_box(git, theme, cx);
    let mut content = div()
      .id("git-scroll")
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .px_1()
      .flex()
      .flex_col()
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .px_2()
          .pt_2()
          .text_xs()
          .text_color(theme.text)
          .child(div().text_color(theme.purple).child("⎇"))
          .child(git.branch.clone()),
      );

    if !git.conflicted.is_empty() {
      content = content
        .child(section_label(
          format!("CONFLICTS ({})", git.conflicted.len()),
          theme,
        ))
        .children(self.git_file_rows(
          &self.git_rows.conflicted,
          DiffBase::Unstaged,
          false,
          theme,
          cx,
        ));
    }

    let staged = if self.is_open(GitSection::Staged) {
      self.git_file_rows(&self.git_rows.staged, DiffBase::Staged, true, theme, cx)
    } else {
      Vec::new()
    };
    let (unstaged, untracked) = if self.is_open(GitSection::Changes) {
      (
        self.git_file_rows(&self.git_rows.unstaged, DiffBase::Unstaged, false, theme, cx),
        self.git_file_rows(
          &self.git_rows.untracked,
          DiffBase::Unstaged,
          false,
          theme,
          cx,
        ),
      )
    } else {
      (Vec::new(), Vec::new())
    };
    let commits: &[helix_models::CommitInfo] = if self.is_open(GitSection::Commits) {
      &git.recent_commits
    } else {
      &[]
    };

    content = content
      .child(
        self.section_toggle(
          "staged-toggle",
          "STAGED",
          git.staged.len(),
          GitSection::Staged,
          Some(
            self
              .header_action(
                "unstage-all-button",
                "staged-toggle",
                IconName::Minus,
                "Unstage everything",
                !git.staged.is_empty(),
                theme,
              )
              .when(!git.staged.is_empty(), |el| {
                el.on_click(cx.listener(|this, _, _, cx| {
                  cx.stop_propagation();
                  this.unstage_all(cx);
                }))
              })
              .into_any_element(),
          ),
          theme,
          cx,
        ),
      )
      .children(staged)
      .child(
        self.section_toggle(
          "changes-toggle",
          "CHANGES",
          git.unstaged.len() + git.untracked.len(),
          GitSection::Changes,
          Some(
            self
              .header_action(
                "stage-all-button",
                "changes-toggle",
                IconName::Plus,
                "Stage all changes",
                git.unstaged.len() + git.untracked.len() > 0,
                theme,
              )
              .when(git.unstaged.len() + git.untracked.len() > 0, |el| {
                el.on_click(cx.listener(|this, _, _, cx| {
                  cx.stop_propagation();
                  this.stage_all(cx);
                }))
              })
              .into_any_element(),
          ),
          theme,
          cx,
        ),
      )
      .children(unstaged)
      .children(untracked)
      .child(self.section_toggle(
        "commits-toggle",
        "COMMITS",
        git.recent_commits.len(),
        GitSection::Commits,
        None,
        theme,
        cx,
      ))
      .children(commits.iter().map(|commit| {
        div()
          .flex()
          .flex_col()
          .px_2()
          .py_1()
          .child(
            div()
              .flex()
              .items_center()
              .gap_2()
              .text_xs()
              .text_color(theme.text_muted)
              .child(
                div()
                  .font_family(theme.font_mono.clone())
                  .text_color(theme.accent)
                  .child(commit.short_id.clone()),
              )
              .child(
                div()
                  .flex_1()
                  .overflow_hidden()
                  .whitespace_nowrap()
                  .child(commit.summary.clone()),
              ),
          )
          .child(div().text_xs().text_color(theme.text_dim).child(format!(
            "{} · {} ago",
            commit.author,
            ago(commit.epoch_seconds)
          )))
      }));

    if git.stash_count > 0 {
      content = content.child(section_label("STASH", theme)).child(
        div()
          .px_2()
          .text_xs()
          .text_color(theme.text_muted)
          .child(format!("{} entries", git.stash_count)),
      );
    }

    div()
      .flex_1()
      .min_h_0()
      .flex()
      .flex_col()
      .child(commit_box)
      .child(content)
      .into_any_element()
  }

  fn render_pr(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let mut panel = div()
      .flex_1()
      .min_h_0()
      .p_3()
      .flex()
      .flex_col()
      .gap_2()
      .text_xs();

    if let Some(review) = self.pr.clone() {
      let (state_label, state_color) = match review.state {
        helix_github::ReviewState::Open => ("open", theme.green),
        helix_github::ReviewState::Draft => ("draft", theme.text_dim),
        helix_github::ReviewState::Merged => ("merged", theme.purple),
        helix_github::ReviewState::Closed => ("closed", theme.red),
      };
      let check_color = match review.checks {
        helix_github::CheckStatus::Passing => theme.green,
        helix_github::CheckStatus::Failing => theme.red,
        helix_github::CheckStatus::Pending => theme.yellow,
        helix_github::CheckStatus::None => theme.text_dim,
      };
      panel = panel
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .flex_none()
                .text_color(theme.accent)
                .child(format!("#{}", review.number)),
            )
            .child(
              div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_color(theme.text)
                .child(review.title.clone()),
            )
            .child(div().flex_none().text_color(state_color).child(state_label)),
        )
        .child(div().text_color(check_color).child(review.checks.label()))
        .child(
          div()
            .text_color(theme.text_dim)
            .child(format!("{} → {}", review.head_ref, review.base_ref)),
        );
      if let Some(decision) = review.review_decision.clone() {
        panel = panel.child(div().text_color(theme.text_muted).child(decision));
      }
      if review.conflicting {
        panel = panel.child(div().text_color(theme.red).child("merge conflicts"));
      }
      panel = panel.child(
        div()
          .text_color(theme.text_dim)
          .overflow_hidden()
          .child(review.url.clone()),
      );
    }

    match &self.pr_eligibility {
      None => {
        panel = panel.child(div().text_color(theme.text_dim).child(if self.pr_busy {
          "Checking GitHub…"
        } else {
          "No pull request data yet."
        }));
      }
      Some(eligibility) => {
        if let Some(reason) = eligibility.blocked_reason {
          if reason != BlockedReason::ExistingReview {
            panel = panel.child(
              div()
                .text_color(if reason == BlockedReason::LookupUnavailable {
                  theme.yellow
                } else {
                  theme.text_muted
                })
                .child(reason.message()),
            );
          }
        }
        if let Some(label) = primary_action_label(eligibility.next_action) {
          let action = eligibility.next_action;
          panel = panel.child(
            div()
              .id("pr-primary-action")
              .h(px(26.0))
              .px_3()
              .flex()
              .items_center()
              .justify_center()
              .rounded_md()
              .bg(theme.elevated)
              .text_color(theme.text)
              .cursor_pointer()
              .hover(|s| s.bg(theme.hover))
              .on_click(cx.listener(move |this, _, _, cx| {
                this.run_primary_action(action, cx);
              }))
              .child(label),
          );
        }
      }
    }

    panel.into_any_element()
  }

  fn render_pr_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let label = self
      .pr
      .as_ref()
      .map(|review| format!("#{}", review.number))
      .unwrap_or_else(|| "Pull request".to_string());

    div()
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .h(px(32.0))
      .px_2()
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .flex_1()
          .min_w_0()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_xs()
          .text_color(theme.text)
          .child(label),
      )
      .when(self.pr_busy, |el| {
        el.child(spinner(self.spin.step(), theme.text_dim))
      })
      .when(!self.pr_busy, |el| {
        el.child(
          icon_button("pr-refresh", HelixIcon::Refresh, theme)
            .on_click(cx.listener(|this, _, _, cx| this.refresh_pull_request(cx))),
        )
      })
      .into_any_element()
  }

  fn run_primary_action(&mut self, action: NextAction, cx: &mut Context<Self>) {
    match action {
      NextAction::Commit => {
        self.active = RightTab::Git;

        cx.notify();

        return;
      }
      NextAction::OpenExistingReview => {
        if let Some(review) = &self.pr {
          let _ = std::process::Command::new("open").arg(&review.url).spawn();
        }

        return;
      }
      NextAction::Retry => {
        self.refresh_pull_request(cx);

        return;
      }
      NextAction::Authenticate | NextAction::InstallGh | NextAction::None => return,
      _ => {}
    }

    if self.pr_busy {
      return;
    }

    self.pr_busy = true;
    self.git_error = None;

    cx.notify();

    let root = self.root.clone();
    let branch = self
      .git
      .as_ref()
      .map(|git| git.branch.clone())
      .unwrap_or_default();
    let title = self
      .git
      .as_ref()
      .and_then(|git| git.recent_commits.first().map(|c| c.summary.clone()))
      .unwrap_or_else(|| "Update".to_string());

    let task = cx.background_executor().spawn(async move {
      match action {
        NextAction::Publish => helix_git::remote::publish(&root, &branch),
        NextAction::Push => helix_git::remote::push(&root),
        NextAction::Sync => helix_git::remote::sync(&root),
        NextAction::CreateReview => create_pull_request(&root, &title),
        _ => Ok(()),
      }
    });

    cx.spawn(async move |this, cx| {
      let result = task.await;

      this
        .update(cx, |panel, cx| {
          panel.pr_busy = false;
          panel.git_error = result.err().map(|err| err.to_string());

          cx.emit(ContextPanelEvent::GitChanged);

          panel.refresh_pull_request(cx);
          cx.notify();
        })
        .ok();
    })
    .detach();
  }
}

fn create_pull_request(root: &Path, title: &str) -> anyhow::Result<()> {
  let base = helix_git::diff::default_base_ref(root)
    .map(|base| base.trim_start_matches("origin/").to_string())
    .unwrap_or_else(|| "main".to_string());

  helix_github::review::create(root, &base, title, "", false)?;

  Ok(())
}

fn primary_action_label(action: NextAction) -> Option<&'static str> {
  match action {
    NextAction::InstallGh => None,
    NextAction::Authenticate => Some("Run gh auth login"),
    NextAction::Commit => Some("Commit changes"),
    NextAction::Publish => Some("Publish branch"),
    NextAction::Push => Some("Push commits"),
    NextAction::Sync => Some("Pull (fast-forward)"),
    NextAction::OpenExistingReview => Some("Open pull request"),
    NextAction::CreateReview => Some("Create pull request"),
    NextAction::Retry => None,
    NextAction::None => None,
  }
}

impl Render for ContextPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    drive_spinner(self, cx);

    let theme = Theme::of(cx).clone();
    let active = self.active;

    let tabs = div()
      .id("right-tabs")
      .window_control_area(gpui::WindowControlArea::Drag)
      .flex()
      .flex_none()
      .items_center()
      .gap_1()
      .h(px(HEADER_HEIGHT))
      .px_2()
      .border_b_1()
      .border_color(theme.panel_border)
      .children(
        [RightTab::Files, RightTab::Git, RightTab::Pr]
          .into_iter()
          .map(|tab| {
            let is_active = tab == active;
            let label = tab.label();
            div()
              .id(SharedString::from(format!("right-tab-{label}")))
              .size(px(28.0))
              .flex()
              .flex_none()
              .items_center()
              .justify_center()
              .rounded_md()
              .cursor_pointer()
              .when(is_active, |el| el.bg(theme.elevated).text_color(theme.text))
              .when(!is_active, |el| {
                el.text_color(theme.text_dim).hover(|s| s.bg(theme.hover))
              })
              .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(label).build(window, cx)
              })
              .on_click(cx.listener(move |this, _, _, cx| {
                this.active = tab;
                if tab == RightTab::Pr && this.pr_eligibility.is_none() {
                  this.refresh_pull_request(cx);
                }
                cx.notify();
              }))
              .child(tab.icon().size_4())
          }),
      )
      .child(div().flex_1())
      .child(
        icon_button("close-right-sidebar", IconName::PanelRightClose, &theme).on_click(
          |_, window, cx| {
            window.dispatch_action(Box::new(helix_commands::ToggleRightSidebar), cx);
          },
        ),
      );

    let toolbar = match active {
      RightTab::Files => Some(self.render_files_toolbar(&theme, cx)),
      RightTab::Pr => Some(self.render_pr_toolbar(&theme, cx)),
      RightTab::Git => None,
    };

    let body = match self.active {
      RightTab::Files => self.render_files(&theme, cx),
      RightTab::Git => self.render_git(&theme, cx),
      RightTab::Pr => self.render_pr(&theme, cx),
    };

    div()
      .flex()
      .flex_col()
      .size_full()
      .bg(theme.panel)
      .border_l_1()
      .border_color(theme.panel_border)
      .when(self.git_menu_open, |el| {
        el.on_mouse_down(
          gpui::MouseButton::Left,
          cx.listener(|this, _, _, cx| {
            this.git_menu_open = false;
            cx.notify();
          }),
        )
      })
      .child(tabs)
      .children(toolbar)
      .child(body)
  }
}
