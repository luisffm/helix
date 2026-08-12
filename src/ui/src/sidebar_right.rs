use crate::components::{
  BODY, GLYPH, HEADER_HEIGHT, META, MICRO, SMALL, TINY, TITLE, UI, ago, icon_button, pill,
  section_label, spinner,
};
use crate::file_icons;
use crate::icons::HelixIcon;
use crate::theme::Theme;
use gpui::{
  AnyElement, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, ScrollStrategy,
  SharedString, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName, Sizable};
use helix_filesystem::scan::{FileNode, scan_contents, scan_dir, scan_matches};
use helix_git::IgnoreProbe;
use helix_git::ops::{GitAction, IndexOp, perform_remote};
use helix_github::{
  BlockedReason, CheckStatus, Eligibility, HostedReview, NextAction, ReviewCheck, merge_readiness,
};
use helix_models::{DiffBase, GitFileKind, GitFileStatus, GitSnapshot};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

const ROW_HEIGHT: f32 = 26.0;
const GIT_ROW_HEIGHT: f32 = 28.0;
const CHECK_POLL_INTERVAL: Duration = Duration::from_secs(30);
const INDENT: f32 = 16.0;
const BASE_PAD: f32 = 6.0;
const MAX_ROWS: usize = 2000;
const MAX_DEPTH: usize = 16;
const MAX_CACHED_DIRS: usize = 4096;
const FILTER_DEBOUNCE: Duration = Duration::from_millis(300);

pub enum ContextPanelEvent {
  OpenFile { path: PathBuf, preview: bool },
  OpenDiff { relative: String, base: DiffBase },
  GitChanged,
}

/// Which side of the workspace the file filter searches.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FileSearch {
  Names,
  Contents,
}

impl FileSearch {
  fn label(self) -> &'static str {
    match self {
      FileSearch::Names => "Names",
      FileSearch::Contents => "Contents",
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommentFilter {
  All,
  Humans,
  Bots,
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
      RightTab::Pr => Icon::new(HelixIcon::GitPullRequest),
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GitSection {
  Staged,
  Changes,
  Commits,
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
  rows_scroll: UniformListScrollHandle,
  matches_scroll: UniformListScrollHandle,
  filter_token: u64,
  filtering: bool,
  git: Option<GitSnapshot>,
  git_rows: GitRows,
  file_status: HashMap<PathBuf, GitFileKind>,
  dir_status: HashMap<PathBuf, GitFileKind>,
  selected: Option<PathBuf>,
  show_dotfiles: bool,
  comment_filter: CommentFilter,
  file_search: FileSearch,
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
  pr_error: Option<String>,
  merge_armed: bool,
  merge_busy: bool,
  checks_polling: bool,
}

impl EventEmitter<ContextPanelEvent> for ContextPanel {}

impl ContextPanel {
  pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let commit_message = cx.new(|cx| {
      InputState::new(window, cx)
        .auto_grow(2, 6)
        .placeholder("Commit message")
    });

    let file_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Find files"));

    cx.subscribe(
      &file_filter,
      |panel, _, event: &InputEvent, cx| match event {
        InputEvent::Change => panel.schedule_filter(cx),
        InputEvent::PressEnter { .. } => panel.activate_selection(cx),
        _ => {}
      },
    )
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
      rows_scroll: UniformListScrollHandle::new(),
      matches_scroll: UniformListScrollHandle::new(),
      filter_token: 0,
      filtering: false,
      git: None,
      git_rows: GitRows::default(),
      file_status: HashMap::new(),
      dir_status: HashMap::new(),
      selected: None,
      show_dotfiles: false,
      comment_filter: CommentFilter::All,
      file_search: FileSearch::Names,
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
      pr_error: None,
      merge_armed: false,
      merge_busy: false,
      checks_polling: false,
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
      .gap(px(6.0))
      .px(px(4.0))
      .pt(px(12.0))
      .pb(px(4.0))
      .cursor_pointer()
      .text_color(theme.text_dim)
      .hover(|s| s.text_color(theme.text_muted))
      .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(section, cx)))
      .child(
        div().flex_none().child(
          Icon::new(if open {
            IconName::ChevronDown
          } else {
            IconName::ChevronRight
          })
          .size(px(11.0)),
        ),
      )
      .child(
        div()
          .flex_1()
          .child(section_label(format!("{label} \u{b7} {count}"), theme)),
      )
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

  fn checks_pending(&self) -> bool {
    self.active == RightTab::Pr && self.pr.as_ref().is_some_and(HostedReview::checks_running)
  }

  /// A running suite is the only thing worth re-polling for, so the timer only
  /// exists while one is running and stops itself once the rollup settles.
  fn schedule_check_poll(&mut self, cx: &mut Context<Self>) {
    if self.checks_polling || !self.checks_pending() {
      return;
    }

    self.checks_polling = true;

    cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(CHECK_POLL_INTERVAL).await;

        let polled = this.update(cx, |panel, cx| {
          if !panel.checks_pending() {
            panel.checks_polling = false;

            return false;
          }

          panel.refresh_pull_request(cx);

          true
        });

        if !matches!(polled, Ok(true)) {
          break;
        }
      }
    })
    .detach();
  }

  fn merge_pull_request(&mut self, cx: &mut Context<Self>) {
    if self.merge_busy {
      return;
    }

    let Some(number) = self.pr.as_ref().map(|review| review.number) else {
      return;
    };

    self.merge_busy = true;
    self.merge_armed = false;
    self.pr_error = None;

    let root = self.root.clone();

    let task = cx
      .background_executor()
      .spawn(async move { helix_github::review::merge(&root, number) });

    cx.spawn(async move |this, cx| {
      let result = task.await;

      this
        .update(cx, |panel, cx| {
          panel.merge_busy = false;
          panel.pr_error = result.err().map(|err| err.to_string());

          panel.refresh_pull_request(cx);
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

    let task = cx
      .background_executor()
      .spawn(async move { helix_github::probe::gather(&root, &git) });

    cx.spawn(async move |this, cx| {
      let (eligibility, review) = task.await;

      this
        .update(cx, |panel, cx| {
          panel.pr_busy = false;
          panel.pr_eligibility = Some(eligibility);

          if panel.pr.as_ref().map(|current| current.number)
            != review.as_ref().map(|next| next.number)
          {
            panel.merge_armed = false;
          }

          panel.pr = review;

          panel.schedule_check_poll(cx);

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

    // The lookup needs a branch, so a tab opened before the snapshot arrived —
    // or left open across a project switch — asks again once it does.
    if self.active == RightTab::Pr && self.pr_eligibility.is_none() && self.git.is_some() {
      self.refresh_pull_request(cx);
    }

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

    // Every one of these describes the branch that was open a moment ago. Left
    // behind, a settled eligibility also stops the tab from ever looking the new
    // branch up, because it only looks when it has nothing.
    self.pr = None;
    self.pr_eligibility = None;
    self.pr_error = None;
    self.merge_armed = false;

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

  fn select_prev(
    &mut self,
    _: &helix_commands::SelectPrev,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.step_file_selection(-1, cx);
  }

  fn select_next(
    &mut self,
    _: &helix_commands::SelectNext,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    self.step_file_selection(1, cx);
  }

  /// The filter input usually holds the focus, so walking the file list arrives
  /// as an action rather than a key event. Whichever list is on screen — the
  /// tree or the filter matches — is the one that moves.
  fn step_file_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
    if self.active != RightTab::Files {
      cx.propagate();

      return;
    }

    let filtering = !self.file_filter.read(cx).value().trim().is_empty();

    let paths: Vec<&PathBuf> = if filtering {
      self.matches.iter().map(|node| &node.path).collect()
    } else {
      self.rows.iter().map(|(node, _)| &node.path).collect()
    };

    if paths.is_empty() {
      cx.propagate();

      return;
    }

    let current = self
      .selected
      .as_ref()
      .and_then(|selected| paths.iter().position(|path| *path == selected));

    let next = match current {
      Some(ix) => (ix + (paths.len() as isize + delta) as usize) % paths.len(),
      None if delta > 0 => 0,
      None => paths.len() - 1,
    };

    self.selected = Some(paths[next].clone());

    let scroll = if filtering {
      &self.matches_scroll
    } else {
      &self.rows_scroll
    };

    scroll.scroll_to_item(next, ScrollStrategy::Center);
    cx.notify();
  }

  /// Enter opens the selected file, or folds the selected directory the way a
  /// click on it would.
  fn activate_selection(&mut self, cx: &mut Context<Self>) {
    let Some(path) = self.selected.clone() else {
      return;
    };

    let is_dir = self
      .rows
      .iter()
      .find(|(node, _)| node.path == path)
      .map(|(node, _)| node.is_dir)
      .unwrap_or(false);

    if is_dir {
      if !self.expanded.insert(path.clone()) {
        self.expanded.remove(&path);
      }

      self.invalidate_rows();
    } else {
      cx.emit(ContextPanelEvent::OpenFile {
        path,
        preview: true,
      });
    }

    cx.notify();
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
          .map(|existing| file.kind.dominance() > existing.dominance())
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
          .map(|existing| kind.dominance() > existing.dominance())
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

      scan_dir(&target, show_dotfiles, &|path, is_dir| {
        probe
          .as_ref()
          .is_some_and(|probe| probe.is_ignored(path, is_dir))
      })
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
    let search = self.file_search;

    cx.spawn(async move |this, cx| {
      cx.background_executor().timer(FILTER_DEBOUNCE).await;

      if !matches!(this.update(cx, |panel, _| panel.filter_token), Ok(current) if current == token)
      {
        return;
      }

      let found = cx
        .background_executor()
        .spawn(async move {
          let probe = IgnoreProbe::open(&root);
          let ignored = |path: &Path, is_dir: bool| {
            probe
              .as_ref()
              .is_some_and(|probe| probe.is_ignored(path, is_dir))
          };

          match search {
            FileSearch::Names => scan_matches(&root, &query, show_dotfiles, &ignored),
            FileSearch::Contents => scan_contents(&root, &query, show_dotfiles, &ignored),
          }
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
      .gap_2()
      .h(px(28.0))
      .px(px(9.0))
      .rounded(px(8.0))
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.panel)
      .text_size(px(UI))
      .child(
        div()
          .flex_none()
          .text_color(theme.text_dim)
          .child(Icon::new(HelixIcon::ListFilter).size(px(12.0))),
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
        let entity = cx.entity();

        uniform_list("files-scroll", self.rows.len(), move |range, _, cx| {
          entity.update(cx, |panel, cx| {
            let theme = Theme::of(cx).clone();

            range
              .filter_map(|ix| {
                let (node, depth) = panel.rows.get(ix).map(|(n, d)| (n.clone(), *d))?;

                Some(panel.render_row(ix, &node, depth, &theme, cx))
              })
              .collect()
          })
        })
        .track_scroll(self.rows_scroll.clone())
        .flex_1()
        .min_h_0()
        .into_any_element()
      }
    } else if self.matches.is_empty() {
      self.scanning_or_empty("No file matches", theme)
    } else {
      let entity = cx.entity();

      uniform_list("files-matches", self.matches.len(), move |range, _, cx| {
        entity.update(cx, |panel, cx| {
          let theme = Theme::of(cx).clone();

          range
            .filter_map(|ix| {
              let node = panel.matches.get(ix).cloned()?;

              Some(panel.render_match(ix, &node, &theme, cx))
            })
            .collect()
        })
      })
      .track_scroll(self.matches_scroll.clone())
      .flex_1()
      .min_h_0()
      .into_any_element()
    };

    div()
      .key_context("FileTree")
      .on_action(cx.listener(Self::select_prev))
      .on_action(cx.listener(Self::select_next))
      .flex_1()
      .min_h_0()
      .flex()
      .flex_col()
      .gap_2()
      .p(px(10.0))
      .child(filter)
      .child(self.render_search_mode(theme, cx))
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
      None if node.ignored => file_icons::ignored_color(theme),
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
      .w_full()
      .items_center()
      .gap_1()
      .h(px(ROW_HEIGHT))
      .pl(px(BASE_PAD))
      .pr_2()
      .rounded(px(6.0))
      .cursor_pointer()
      .text_size(px(BODY))
      .when(is_selected, |el| el.bg(theme.hover))
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
            file_icons::ignored_color(theme)
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
      row = row.child(status_letter(kind, theme));
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
      None if node.ignored => file_icons::ignored_color(theme),
      None => theme.text,
    };
    let icon_color = if node.ignored {
      file_icons::ignored_color(theme)
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
      .w_full()
      .items_center()
      .gap_1()
      .h(px(ROW_HEIGHT))
      .pl(px(BASE_PAD + depth as f32 * INDENT))
      .pr_2()
      .rounded(px(6.0))
      .cursor_pointer()
      .text_size(px(BODY))
      .when(is_selected, |el| el.bg(theme.hover))
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
      row = row.child(status_letter(kind, theme));
    }

    row.into_any_element()
  }

  /// `Names | Contents`, plus the tree actions the design leaves out but the
  /// panel still needs: collapsing every folder, rescanning, and dotfiles.
  fn render_search_mode(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    div()
      .flex()
      .flex_none()
      .items_center()
      .gap_1()
      .child(
        div()
          .flex_1()
          .flex()
          .gap(px(2.0))
          .p(px(2.0))
          .rounded(px(8.0))
          .bg(theme.panel)
          .border_1()
          .border_color(theme.panel_border)
          .children([FileSearch::Names, FileSearch::Contents].map(|mode| {
            let is_active = mode == self.file_search;

            div()
              .id(SharedString::from(format!("file-search-{}", mode.label())))
              .flex_1()
              .flex()
              .items_center()
              .justify_center()
              .py(px(3.0))
              .rounded(px(6.0))
              .cursor_pointer()
              .text_size(px(SMALL))
              .when(is_active, |el| {
                el.bg(theme.active)
                  .font_weight(gpui::FontWeight::MEDIUM)
                  .text_color(theme.text)
              })
              .when(!is_active, |el| {
                el.text_color(theme.text_dim).hover(|s| s.bg(theme.hover))
              })
              .on_click(cx.listener(move |this, _, _, cx| {
                if this.file_search == mode {
                  return;
                }

                this.file_search = mode;

                this.schedule_filter(cx);
                cx.notify();
              }))
              .child(mode.label())
          })),
      )
      .child(self.render_files_toolbar(theme, cx))
      .into_any_element()
  }

  fn render_files_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    div()
      .flex()
      .flex_none()
      .items_center()
      .child(
        icon_button("files-collapse", HelixIcon::ListCollapse, theme)
          .tooltip("Collapse every folder")
          .on_click(cx.listener(|this, _, _, cx| {
            this.expanded.clear();
            this.invalidate_rows();
            cx.notify();
          })),
      )
      .child(
        icon_button("files-refresh", HelixIcon::Refresh, theme)
          .tooltip("Rescan the workspace")
          .on_click(cx.listener(|this, _, _, cx| {
            this.reset_scans();
            this.schedule_filter(cx);
            cx.notify();
          })),
      )
      .child(
        icon_button("files-dotfiles", IconName::Eye, theme)
          .tooltip("Show hidden files")
          .on_click(cx.listener(|this, _, _, cx| {
            this.show_dotfiles = !this.show_dotfiles;

            this.reset_scans();
            this.schedule_filter(cx);
            cx.notify();
          })),
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

        let stat = div()
          .flex_none()
          .flex()
          .items_center()
          .gap_1()
          .group_hover(group.clone(), |s| s.invisible())
          .child(
            div()
              .flex()
              .gap_1()
              .font_family(theme.font_mono.clone())
              .text_size(px(META))
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
              ),
          )
          .child(
            div()
              .text_size(px(META))
              .text_color(color)
              .child(file.kind.status_letter()),
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
          .gap(px(7.0))
          .h(px(GIT_ROW_HEIGHT))
          .px(px(8.0))
          .rounded(px(7.0))
          .cursor_pointer()
          .text_size(px(BODY))
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
              .text_color(theme.text)
              .child(file.name.clone()),
          )
          .children(file.parent.clone().map(|parent| {
            div()
              .flex_1()
              .min_w_0()
              .overflow_hidden()
              .whitespace_nowrap()
              .text_size(px(META))
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
    let feature_branch = helix_github::eligibility::is_feature_branch(&git.branch);

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
      .bg(theme.win_tint)
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
      .size(px(26.0))
      .flex()
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(px(7.0))
      .border_1()
      .border_color(theme.panel_border)
      .cursor_pointer()
      .text_size(px(UI))
      .text_color(if can_write {
        theme.claude
      } else {
        theme.text_dim
      })
      .hover(|s| s.bg(theme.claude_soft))
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
        "\u{2026}"
      } else {
        "\u{2726}"
      });

    div()
      .flex()
      .flex_none()
      .flex_col()
      .gap(px(9.0))
      .p(px(10.0))
      .rounded(px(10.0))
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.panel)
      .child(
        div()
          .min_h(px(34.0))
          .text_size(px(BODY))
          .child(Input::new(&self.commit_message).appearance(false)),
      )
      .child(
        div()
          .relative()
          .flex()
          .items_center()
          .gap(px(6.0))
          .child(write)
          .child(div().flex_1())
          .child(
            div()
              .id("commit-button")
              .h(px(26.0))
              .px(px(12.0))
              .flex()
              .items_center()
              .justify_center()
              .gap_1()
              .rounded(px(7.0))
              .text_size(px(UI))
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .when(primary_enabled, |el| {
                el.bg(theme.accent)
                  .text_color(theme.accent_text)
                  .cursor_pointer()
                  .on_click(cx.listener(move |this, _, window, cx| {
                    if can_commit {
                      this.run_git_action(GitAction::Commit, window, cx);
                    } else {
                      this.stage_all(cx);
                    }
                  }))
              })
              .when(!primary_enabled, |el| {
                el.bg(theme.panel2)
                  .border_1()
                  .border_color(theme.panel_border)
                  .text_color(theme.text_dim)
              })
              .children(
                (!can_commit && !self.git_busy)
                  .then(|| div().flex_none().child(Icon::new(IconName::Plus).size_3())),
              )
              .child(if self.git_busy {
                "Working\u{2026}".to_string()
              } else if can_commit {
                format!("Commit ({})", git.staged.len())
              } else {
                "Stage All".to_string()
              }),
          )
          .child(
            div()
              .id("commit-menu-button")
              .size(px(26.0))
              .flex()
              .flex_none()
              .items_center()
              .justify_center()
              .rounded(px(7.0))
              .border_1()
              .border_color(theme.panel_border)
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
              .child(Icon::new(IconName::ChevronDown).size(px(11.0))),
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
          .map(|err| div().text_size(px(META)).text_color(theme.red).child(err)),
      )
      .into_any_element()
  }

  fn render_git(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let Some(git) = self.git.as_ref() else {
      return div()
        .p(px(10.0))
        .text_size(px(BODY))
        .text_color(theme.text_dim)
        .child("Not a git repository")
        .into_any_element();
    };

    let mut content = div()
      .id("git-scroll")
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .p(px(10.0))
      .flex()
      .flex_col()
      .child(self.render_commit_box(git, theme, cx))
      .child(
        div()
          .flex()
          .items_center()
          .gap(px(7.0))
          .px(px(4.0))
          .pt(px(12.0))
          .pb(px(4.0))
          .text_size(px(BODY))
          .font_weight(gpui::FontWeight::MEDIUM)
          .text_color(theme.text)
          .child(crate::components::git_branch_icon(theme.text_muted, 12.0))
          .child(git.branch.clone()),
      );

    if !git.conflicted.is_empty() {
      content = content
        .child(
          div()
            .px(px(4.0))
            .pt(px(12.0))
            .pb(px(4.0))
            .child(section_label(
              format!("Conflicts \u{b7} {}", git.conflicted.len()),
              theme,
            )),
        )
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
        self.git_file_rows(
          &self.git_rows.unstaged,
          DiffBase::Unstaged,
          false,
          theme,
          cx,
        ),
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
          .px(px(8.0))
          .py_1()
          .child(
            div()
              .flex()
              .items_center()
              .gap_2()
              .text_size(px(BODY))
              .text_color(theme.text_muted)
              .child(
                div()
                  .font_family(theme.font_mono.clone())
                  .text_size(px(META))
                  .text_color(theme.text_muted)
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
          .child(
            div()
              .text_size(px(META))
              .text_color(theme.text_dim)
              .child(format!(
                "{} · {} ago",
                commit.author,
                ago(commit.epoch_seconds)
              )),
          )
      }));

    if git.stash_count > 0 {
      content = content
        .child(
          div()
            .px(px(4.0))
            .pt(px(12.0))
            .pb(px(4.0))
            .child(section_label("STASH", theme)),
        )
        .child(
          div()
            .px(px(8.0))
            .text_size(px(BODY))
            .text_color(theme.text_muted)
            .child(format!("{} entries", git.stash_count)),
        );
    }

    content.into_any_element()
  }

  fn render_pr(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let mut panel = div()
      .id("pr-scroll")
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .p(px(12.0))
      .flex()
      .flex_col()
      .gap(px(12.0))
      .text_size(px(UI));

    if let Some(review) = self.pr.clone() {
      let (state_label, state_color, state_bg) = match review.state {
        helix_github::ReviewState::Open => ("OPEN", theme.green, theme.green_soft),
        helix_github::ReviewState::Draft => ("DRAFT", theme.text_muted, theme.panel),
        helix_github::ReviewState::Merged => ("MERGED", theme.purple, theme.purple_soft),
        helix_github::ReviewState::Closed => ("CLOSED", theme.red, theme.claude_soft),
      };
      let url = review.url.clone();

      panel = panel
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .child(
              div()
                .flex_none()
                .text_color(theme.text_muted)
                .child(Icon::new(HelixIcon::GitPullRequest).size(px(GLYPH))),
            )
            .child(
              div()
                .flex_none()
                .font_family(theme.font_mono.clone())
                .text_size(px(TITLE))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(format!("#{}", review.number)),
            )
            .child(
              pill(state_label, state_color, state_bg, MICRO)
                .font_weight(gpui::FontWeight::SEMIBOLD),
            )
            .child(div().flex_1())
            .when(self.pr_busy, |el| {
              el.child(spinner("pr-spin", theme.text_dim, 12.0))
            })
            .when(!self.pr_busy, |el| {
              el.child(
                icon_button("pr-refresh", HelixIcon::Refresh, theme)
                  .tooltip("Refresh the pull request")
                  .on_click(cx.listener(|this, _, _, cx| this.refresh_pull_request(cx))),
              )
            })
            .child(
              icon_button("pr-open", IconName::ExternalLink, theme)
                .tooltip("Open on GitHub")
                .on_click(cx.listener(move |_, _, _, cx| {
                  let url = url.clone();

                  cx.background_executor()
                    .spawn(async move { helix_process::open_url(&url) })
                    .detach();
                })),
            ),
        )
        .child(
          div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .child(
              div()
                .text_size(px(TITLE))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .line_height(px(19.0))
                .child(review.title.clone()),
            )
            .child(div().text_size(px(META)).text_color(theme.text_dim).child(
              if review.updated_epoch_seconds > 0 {
                format!("Updated {} ago", ago(review.updated_epoch_seconds))
              } else {
                format!("{} \u{2192} {}", review.head_ref, review.base_ref)
              },
            )),
        );

      if let Some(button) = self.render_merge_button(&review, theme, cx) {
        panel = panel.child(button);
      }

      if review.conflicting {
        panel = panel.child(self.render_conflict_banner(theme, cx));
      }

      panel = panel.child(
        div()
          .font_family(theme.font_mono.clone())
          .text_size(px(META))
          .text_color(theme.text_dim)
          .child(format!("{} \u{2192} {}", review.head_ref, review.base_ref)),
      );

      if !review.check_runs.is_empty() {
        let passing = review
          .check_runs
          .iter()
          .filter(|run| run.status == CheckStatus::Passing)
          .count();
        let rollup = check_color(review.checks, theme);

        panel =
          panel.child(
            div()
              .flex()
              .flex_col()
              .gap(px(2.0))
              .pt(px(10.0))
              .border_t_1()
              .border_color(theme.panel_border)
              .child(
                div()
                  .flex()
                  .items_center()
                  .gap(px(7.0))
                  .pb(px(6.0))
                  .child(
                    div()
                      .flex_none()
                      .text_color(rollup)
                      .child(Icon::new(check_icon(review.checks)).size(px(12.0))),
                  )
                  .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(
                    match review.checks {
                      CheckStatus::Passing => format!("{passing} passing"),
                      _ => review.checks.label().to_string(),
                    },
                  )),
              )
              .children(
                review
                  .check_runs
                  .iter()
                  .enumerate()
                  .map(|(ix, run)| self.render_check_run(ix, run, theme, cx)),
              ),
          );
      }

      if let Some(decision) = review.review_decision.clone() {
        panel = panel.child(
          div()
            .text_size(px(META))
            .text_color(theme.text_muted)
            .child(decision),
        );
      }

      if !review.comments.is_empty() {
        panel = panel.child(self.render_comments(&review, theme, cx));
      }
    }

    if let Some(error) = &self.pr_error {
      panel = panel.child(
        div()
          .text_size(px(META))
          .text_color(theme.red)
          .child(error.clone()),
      );
    }

    match &self.pr_eligibility {
      None => {
        panel = panel.child(div().text_size(px(BODY)).text_color(theme.text_dim).child(
          if self.pr_busy {
            "Checking GitHub\u{2026}"
          } else {
            "No pull request data yet."
          },
        ));
      }
      Some(eligibility) => {
        if let Some(reason) = eligibility.blocked_reason {
          if reason != BlockedReason::ExistingReview {
            panel = panel.child(
              div()
                .text_size(px(META))
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
              .h(px(30.0))
              .px_3()
              .flex()
              .items_center()
              .justify_center()
              .rounded(px(8.0))
              .bg(theme.accent)
              .text_color(theme.accent_text)
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .cursor_pointer()
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

  /// Resolving conflicts is agent work, so the banner's action opens a Claude
  /// session on this worktree rather than pretending the app can merge them.
  fn render_conflict_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    div()
      .flex()
      .items_center()
      .gap(px(9.0))
      .p(px(10.0))
      .rounded(px(10.0))
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.panel)
      .child(crate::components::status_dot(theme.yellow))
      .child(
        div()
          .flex_1()
          .min_w_0()
          .flex()
          .flex_col()
          .child(
            div()
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .child("Conflicts block this PR"),
          )
          .child(
            div()
              .text_size(px(META))
              .text_color(theme.text_dim)
              .overflow_hidden()
              .whitespace_nowrap()
              .text_ellipsis()
              .child("Resolve conflicts before checks can run"),
          ),
      )
      .child(
        div()
          .id("pr-resolve")
          .flex_none()
          .flex()
          .items_center()
          .gap(px(5.0))
          .h(px(24.0))
          .px(px(10.0))
          .rounded(px(7.0))
          .bg(theme.claude)
          .text_size(px(SMALL))
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.claude_text)
          .cursor_pointer()
          .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new("Open a Claude session to resolve them")
              .build(window, cx)
          })
          .on_click(cx.listener(|_, _, window, cx| {
            window.dispatch_action(Box::new(helix_commands::NewClaudeSession), cx);
          }))
          .child("\u{2726} Resolve"),
      )
      .into_any_element()
  }

  fn render_comments(
    &self,
    review: &HostedReview,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let bots = review.comments.iter().filter(|c| c.bot).count();
    let humans = review.comments.len() - bots;

    let shown: Vec<&helix_github::ReviewComment> = review
      .comments
      .iter()
      .filter(|comment| match self.comment_filter {
        CommentFilter::All => true,
        CommentFilter::Humans => !comment.bot,
        CommentFilter::Bots => comment.bot,
      })
      .collect();

    div()
      .flex()
      .flex_col()
      .gap_2()
      .pt(px(10.0))
      .border_t_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .flex()
          .items_center()
          .gap(px(7.0))
          .child(section_label("COMMENTS", theme))
          .child(
            div()
              .flex_none()
              .min_w(px(16.0))
              .h(px(16.0))
              .px_1()
              .flex()
              .items_center()
              .justify_center()
              .rounded_full()
              .bg(theme.panel)
              .border_1()
              .border_color(theme.panel_border)
              .text_size(px(MICRO))
              .text_color(theme.text_muted)
              .child(review.comments.len().to_string()),
          ),
      )
      .child(
        div()
          .flex()
          .gap(px(2.0))
          .p(px(2.0))
          .rounded(px(8.0))
          .bg(theme.panel)
          .border_1()
          .border_color(theme.panel_border)
          .children(
            [
              (CommentFilter::All, "All", review.comments.len()),
              (CommentFilter::Humans, "Humans", humans),
              (CommentFilter::Bots, "Bots", bots),
            ]
            .map(|(filter, label, count)| {
              let is_active = filter == self.comment_filter;

              div()
                .id(SharedString::from(format!("comment-filter-{label}")))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .py(px(3.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(SMALL))
                .when(is_active, |el| {
                  el.bg(theme.active)
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(theme.text)
                })
                .when(!is_active, |el| {
                  el.text_color(theme.text_dim).hover(|s| s.bg(theme.hover))
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                  this.comment_filter = filter;
                  cx.notify();
                }))
                .child(format!("{label} {count}"))
            }),
          ),
      )
      .children(shown.into_iter().enumerate().map(|(ix, comment)| {
        div()
          .flex()
          .flex_col()
          .rounded(px(10.0))
          .border_1()
          .border_color(theme.panel_border)
          .bg(theme.panel)
          .overflow_hidden()
          .child(
            div()
              .flex()
              .items_center()
              .gap_2()
              .px(px(10.0))
              .py(px(9.0))
              .border_b_1()
              .border_color(theme.panel_border)
              .child(
                div()
                  .size(px(20.0))
                  .flex_none()
                  .flex()
                  .items_center()
                  .justify_center()
                  .rounded_full()
                  .bg(theme.claude_soft)
                  .text_color(theme.claude)
                  .text_size(px(META))
                  .child(if comment.bot {
                    "\u{2726}".to_string()
                  } else {
                    comment
                      .author
                      .chars()
                      .next()
                      .map(|c| c.to_uppercase().to_string())
                      .unwrap_or_default()
                  }),
              )
              .child(
                div()
                  .flex()
                  .flex_col()
                  .child(
                    div()
                      .font_weight(gpui::FontWeight::SEMIBOLD)
                      .child(comment.author.clone()),
                  )
                  .child(
                    div()
                      .text_size(px(TINY))
                      .text_color(theme.text_dim)
                      .child(format!("{} ago", ago(comment.epoch_seconds))),
                  ),
              )
              .child(div().flex_1())
              .when(comment.bot, |el| {
                el.child(crate::components::cap("BOT", theme))
              }),
          )
          .child(
            div()
              .id(SharedString::from(format!("pr-comment-{ix}")))
              .px(px(10.0))
              .py(px(10.0))
              .line_height(px(18.0))
              .text_color(theme.text_muted)
              .child(comment.body.clone()),
          )
      }))
      .into_any_element()
  }

  fn render_check_run(
    &self,
    ix: usize,
    run: &ReviewCheck,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let color = check_color(run.status, theme);
    let url = run.url.clone();

    let mut row = div()
      .id(SharedString::from(format!("pr-check-{ix}")))
      .flex()
      .items_center()
      .gap_2()
      .h(px(28.0))
      .px(px(6.0))
      .rounded(px(7.0));

    row = match run.status {
      CheckStatus::Pending => row.child(spinner(
        SharedString::from(format!("pr-check-spin-{ix}")),
        color,
        11.0,
      )),
      _ => row.child(
        div()
          .flex_none()
          .text_color(color)
          .child(Icon::new(check_icon(run.status)).size(px(11.0))),
      ),
    };

    row
      .child(
        div()
          .flex_1()
          .min_w_0()
          .overflow_hidden()
          .whitespace_nowrap()
          .text_ellipsis()
          .child(run.name.clone()),
      )
      .child(
        div()
          .flex_none()
          .text_size(px(META))
          .text_color(theme.text_dim)
          .child(check_run_label(run.status)),
      )
      .when_some(url, |el, url| {
        el.cursor_pointer()
          .hover(|s| s.bg(theme.hover))
          .child(
            div()
              .flex_none()
              .text_color(theme.text_dim)
              .child(Icon::new(IconName::ExternalLink).size(px(10.0))),
          )
          .on_click(cx.listener(move |_, _, _, cx| {
            let url = url.clone();

            cx.background_executor()
              .spawn(async move { helix_process::open_url(&url) })
              .detach();
          }))
      })
      .into_any_element()
  }

  fn render_merge_button(
    &self,
    review: &HostedReview,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> Option<AnyElement> {
    let readiness = merge_readiness(review)?;
    let ready = readiness.is_ready();
    let label = if self.merge_busy {
      "Merging\u{2026}"
    } else if self.merge_armed {
      "Confirm merge"
    } else {
      readiness.label()
    };

    let (fg, bg) = if self.merge_armed {
      (theme.yellow, theme.yellow_soft)
    } else if ready {
      (theme.green, theme.green_soft)
    } else {
      (theme.yellow, theme.yellow_soft)
    };

    let mut button = div()
      .id("pr-merge")
      .h(px(30.0))
      .px_3()
      .flex()
      .items_center()
      .justify_center()
      .gap(px(7.0))
      .rounded(px(8.0))
      .border_1()
      .border_color(theme.panel_border)
      .font_weight(gpui::FontWeight::SEMIBOLD)
      .text_size(px(BODY));

    button = if ready && !self.merge_busy {
      button
        .bg(bg)
        .text_color(fg)
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
          if this.merge_armed {
            this.merge_pull_request(cx);
          } else {
            this.merge_armed = true;
          }

          cx.notify();
        }))
    } else {
      button.bg(bg).text_color(fg)
    };

    if self.merge_busy {
      button = button.child(spinner("pr-merge-spin", fg, 12.0));
    } else {
      button = button.child(
        div()
          .flex_none()
          .child(Icon::new(HelixIcon::GitBranch).size(px(12.0))),
      );
    }

    Some(button.child(label).into_any_element())
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
          let url = review.url.clone();

          cx.background_executor()
            .spawn(async move { helix_process::open_url(&url) })
            .detach();
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
        NextAction::CreateReview => helix_github::probe::create_pull_request(&root, &title),
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

/// The design draws a status letter as plain coloured text at 11px: no badge,
/// no weight, nothing behind it.
fn status_letter(kind: GitFileKind, theme: &Theme) -> gpui::Div {
  div()
    .flex_none()
    .text_size(px(META))
    .text_color(file_icons::status_color(kind, theme))
    .child(kind.status_letter())
}

fn check_color(status: CheckStatus, theme: &Theme) -> gpui::Hsla {
  match status {
    CheckStatus::Passing => theme.green,
    CheckStatus::Failing => theme.red,
    CheckStatus::Pending => theme.yellow,
    CheckStatus::None => theme.text_dim,
  }
}

fn check_run_label(status: CheckStatus) -> &'static str {
  match status {
    CheckStatus::Passing => "Successful",
    CheckStatus::Failing => "Failed",
    CheckStatus::Pending => "Running",
    CheckStatus::None => "Skipped",
  }
}

fn check_icon(status: CheckStatus) -> IconName {
  match status {
    CheckStatus::Failing => IconName::CircleX,
    _ => IconName::CircleCheck,
  }
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
    let theme = Theme::of(cx).clone();
    let active = self.active;

    let tabs = div()
      .id("right-tabs")
      .window_control_area(gpui::WindowControlArea::Drag)
      .flex()
      .flex_none()
      .items_center()
      .gap(px(3.0))
      .h(px(HEADER_HEIGHT))
      .px(px(10.0))
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
              .w(px(28.0))
              .h(px(26.0))
              .flex()
              .flex_none()
              .items_center()
              .justify_center()
              .rounded(px(7.0))
              .cursor_pointer()
              .when(is_active, |el| el.bg(theme.active).text_color(theme.text))
              .when(!is_active, |el| {
                el.text_color(theme.text_dim)
                  .hover(|s| s.bg(theme.hover).text_color(theme.text))
              })
              .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(label).build(window, cx)
              })
              .on_click(cx.listener(move |this, _, _, cx| {
                this.active = tab;
                this.merge_armed = false;
                if tab == RightTab::Pr {
                  if this.pr_eligibility.is_none() {
                    this.refresh_pull_request(cx);
                  }

                  this.schedule_check_poll(cx);
                }
                cx.notify();
              }))
              .child(tab.icon().size(px(GLYPH)))
          }),
      )
      .child(div().flex_1())
      .child(
        icon_button(
          "toggle-theme",
          if theme.is_dark() {
            IconName::Sun
          } else {
            IconName::Moon
          },
          &theme,
        )
        .tooltip(if theme.is_dark() {
          "Switch to the light theme"
        } else {
          "Switch to the dark theme"
        })
        .on_click(|_, window, cx| {
          window.dispatch_action(Box::new(helix_commands::ToggleTheme), cx);
        }),
      )
      .child(
        icon_button("open-settings", HelixIcon::Sliders, &theme)
          .tooltip_with_action("Settings", &helix_commands::OpenAppSettings, None)
          .on_click(|_, window, cx| {
            window.dispatch_action(Box::new(helix_commands::OpenAppSettings), cx);
          }),
      )
      .child(
        icon_button("close-right-sidebar", IconName::PanelRight, &theme)
          .tooltip_with_action(
            "Hide the context sidebar",
            &helix_commands::ToggleRightSidebar,
            None,
          )
          .on_click(|_, window, cx| {
            window.dispatch_action(Box::new(helix_commands::ToggleRightSidebar), cx);
          }),
      );

    let body = match self.active {
      RightTab::Files => self.render_files(&theme, cx),
      RightTab::Git => self.render_git(&theme, cx),
      RightTab::Pr => self.render_pr(&theme, cx),
    };

    div()
      .flex()
      .flex_col()
      .size_full()
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
      .child(body)
  }
}
