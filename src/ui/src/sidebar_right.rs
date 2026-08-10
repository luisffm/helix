use crate::components::{
  HEADER_HEIGHT, ago, icon_button, icon_button_path, section_label, spinner,
};
use crate::file_icons;
use crate::theme::Theme;
use gpui::{
  AnyElement, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, SharedString,
  Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName, Sizable};
use helix_github::{BlockedReason, Eligibility, HostedReview, NextAction, ReviewLookupOutcome};
use helix_models::{DiffBase, GitFileKind, GitFileStatus, GitSnapshot};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

const ROW_HEIGHT: f32 = 24.0;
const INDENT: f32 = 16.0;
const BASE_PAD: f32 = 8.0;
const MAX_ROWS: usize = 2000;
const MAX_DEPTH: usize = 16;
const FILTER_MAX_MATCHES: usize = 300;
const FILTER_MAX_DIRS: usize = 4000;
const IGNORED_RANK_PENALTY: u8 = 3;

/// `needle` must already be lowercase. `by_path` widens the match to the
/// workspace-relative path, which only helps once the query has a separator.
pub fn match_rank(name: &str, relative: &str, needle: &str, by_path: bool) -> Option<u8> {
  let name = name.to_lowercase();
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
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GitSection {
  Staged,
  Changes,
  Commits,
}

#[derive(Clone)]
struct FileNode {
  path: PathBuf,
  name: String,
  is_dir: bool,
  ignored: bool,
}

pub struct ContextPanel {
  root: PathBuf,
  active: RightTab,
  expanded: HashSet<PathBuf>,
  dir_cache: BTreeMap<PathBuf, Vec<FileNode>>,
  git: Option<GitSnapshot>,
  file_status: HashMap<PathBuf, GitFileKind>,
  dir_status: HashMap<PathBuf, GitFileKind>,
  selected: Option<PathBuf>,
  show_dotfiles: bool,
  file_filter: Entity<InputState>,
  commit_message: Entity<InputState>,
  generating_message: bool,
  collapsed: HashSet<GitSection>,
  git_error: Option<String>,
  pr: Option<HostedReview>,
  pr_eligibility: Option<Eligibility>,
  pr_busy: bool,
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
    cx.subscribe(&file_filter, |_, _, event: &InputEvent, cx| {
      if matches!(event, InputEvent::Change) {
        cx.notify();
      }
    })
    .detach();
    Self {
      root,
      active: RightTab::Files,
      expanded: HashSet::new(),
      dir_cache: BTreeMap::new(),
      git: None,
      file_status: HashMap::new(),
      dir_status: HashMap::new(),
      selected: None,
      show_dotfiles: false,
      file_filter,
      commit_message,
      generating_message: false,
      collapsed: HashSet::from([GitSection::Commits]),
      git_error: None,
      pr: None,
      pr_eligibility: None,
      pr_busy: false,
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

  fn stage(&mut self, relative: String, cx: &mut Context<Self>) {
    self.git_error = helix_git::index::stage(&self.root, &relative)
      .err()
      .map(|err| err.to_string());
    cx.emit(ContextPanelEvent::GitChanged);
    cx.notify();
  }

  fn unstage(&mut self, relative: String, cx: &mut Context<Self>) {
    self.git_error = helix_git::index::unstage(&self.root, &relative)
      .err()
      .map(|err| err.to_string());
    cx.emit(ContextPanelEvent::GitChanged);
    cx.notify();
  }

  fn stage_all(&mut self, cx: &mut Context<Self>) {
    self.git_error = helix_git::index::stage_all(&self.root)
      .err()
      .map(|err| err.to_string());
    cx.emit(ContextPanelEvent::GitChanged);
    cx.notify();
  }

  fn unstage_all(&mut self, cx: &mut Context<Self>) {
    self.git_error = helix_git::index::unstage_all(&self.root)
      .err()
      .map(|err| err.to_string());
    cx.emit(ContextPanelEvent::GitChanged);
    cx.notify();
  }

  fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let message = self.commit_message.read(cx).value().to_string();
    match helix_git::index::commit(&self.root, &message) {
      Ok(_) => {
        self.git_error = None;
        self
          .commit_message
          .update(cx, |state, cx| state.set_value("", window, cx));
      }
      Err(err) => self.git_error = Some(err.to_string()),
    }
    cx.emit(ContextPanelEvent::GitChanged);
    cx.notify();
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
    self.git = git;
    cx.notify();
  }

  pub fn refresh_files(&mut self, cx: &mut Context<Self>) {
    self.dir_cache.clear();
    cx.notify();
  }

  pub fn set_root(&mut self, root: PathBuf, cx: &mut Context<Self>) {
    self.root = root;
    self.expanded.clear();
    self.dir_cache.clear();
    self.git = None;
    self.file_status.clear();
    self.dir_status.clear();
    self.selected = None;
    cx.notify();
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

  fn children_of(&mut self, dir: &Path) -> Vec<FileNode> {
    if let Some(children) = self.dir_cache.get(dir) {
      return children.clone();
    }
    let show_dotfiles = self.show_dotfiles;
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
        Some(FileNode {
          path: entry.path(),
          name,
          is_dir,
          ignored: false,
        })
      })
      .collect();

    let candidates: Vec<PathBuf> = nodes.iter().map(|node| node.path.clone()).collect();
    let ignored = helix_git::ignored_paths(&self.root, &candidates);
    for node in &mut nodes {
      node.ignored = ignored.contains(&node.path);
    }

    nodes.sort_by(|a, b| {
      b.is_dir
        .cmp(&a.is_dir)
        .then_with(|| natural_cmp(&a.name, &b.name))
    });
    self.dir_cache.insert(dir.to_path_buf(), nodes.clone());
    nodes
  }

  fn filter_matches(&mut self, query: &str) -> Vec<FileNode> {
    let needle = query.to_lowercase();
    let by_path = needle.contains('/');
    let mut ranked: Vec<(u8, FileNode)> = Vec::new();
    let mut queue = std::collections::VecDeque::from([self.root.clone()]);
    let mut dirs = 0usize;

    while let Some(dir) = queue.pop_front() {
      if dirs >= FILTER_MAX_DIRS || ranked.len() >= FILTER_MAX_MATCHES {
        break;
      }
      dirs += 1;
      for node in self.children_of(&dir) {
        if node.is_dir {
          if !node.ignored {
            queue.push_back(node.path.clone());
          }
          continue;
        }
        let relative = node
          .path
          .strip_prefix(&self.root)
          .unwrap_or(&node.path)
          .to_string_lossy()
          .to_string();
        let Some(mut rank) = match_rank(&node.name, &relative, &needle, by_path) else {
          continue;
        };
        if node.ignored {
          rank += IGNORED_RANK_PENALTY;
        }
        ranked.push((rank, node));
      }
    }

    ranked.sort_by(|a, b| {
      a.0
        .cmp(&b.0)
        .then_with(|| natural_cmp(&a.1.name, &b.1.name))
    });
    ranked.into_iter().map(|(_, node)| node).collect()
  }

  fn visible_rows(&mut self, dir: PathBuf, depth: usize, out: &mut Vec<(FileNode, usize)>) {
    if depth > MAX_DEPTH || out.len() > MAX_ROWS {
      return;
    }
    for node in self.children_of(&dir) {
      let expanded = node.is_dir && self.expanded.contains(&node.path);
      out.push((node.clone(), depth));
      if expanded {
        self.visible_rows(node.path.clone(), depth + 1, out);
      }
    }
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
      let mut rows = Vec::new();
      self.visible_rows(self.root.clone(), 0, &mut rows);
      if rows.is_empty() {
        self.empty_files("No files in this workspace", theme)
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
            rows
              .into_iter()
              .enumerate()
              .map(|(ix, (node, depth))| self.render_row(ix, node, depth, theme, cx)),
          )
          .into_any_element()
      }
    } else {
      let matches = self.filter_matches(&query);
      if matches.is_empty() {
        self.empty_files("No file matches", theme)
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
            matches
              .into_iter()
              .enumerate()
              .map(|(ix, node)| self.render_match(ix, node, theme, cx)),
          )
          .into_any_element()
      }
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
    node: FileNode,
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
    node: FileNode,
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
        icon_button_path("files-collapse", "icons/list-collapse.svg", theme).on_click(cx.listener(
          |this, _, _, cx| {
            this.expanded.clear();
            cx.notify();
          },
        )),
      )
      .child(
        icon_button_path("files-refresh", "icons/refresh.svg", theme).on_click(cx.listener(
          |this, _, _, cx| {
            this.dir_cache.clear();
            cx.notify();
          },
        )),
      )
      .child(
        icon_button("files-dotfiles", IconName::Eye, theme).on_click(cx.listener(
          |this, _, _, cx| {
            this.show_dotfiles = !this.show_dotfiles;
            this.dir_cache.clear();
            cx.notify();
          },
        )),
      )
      .into_any_element()
  }

  fn git_file_rows(
    &self,
    files: &[GitFileStatus],
    base: DiffBase,
    prefix: &'static str,
    staged: bool,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> Vec<AnyElement> {
    files
      .iter()
      .enumerate()
      .map(|(ix, file)| {
        let color = file_icons::status_color(file.kind, theme);
        let relative = file.path.clone();
        let toggle_path = file.path.clone();
        let base = base.clone();
        div()
          .id(SharedString::from(format!("git-{prefix}-{ix}")))
          .group(SharedString::from(format!("git-row-{prefix}-{ix}")))
          .flex()
          .items_center()
          .gap_2()
          .h(px(ROW_HEIGHT))
          .px_2()
          .rounded_md()
          .cursor_pointer()
          .text_xs()
          .hover(|s| s.bg(theme.hover))
          .on_click(cx.listener(move |_, _, _, cx| {
            cx.emit(ContextPanelEvent::OpenDiff {
              relative: relative.clone(),
              base: base.clone(),
            });
          }))
          .child(
            div()
              .w(px(10.0))
              .flex_none()
              .text_color(color)
              .child(file_icons::status_letter(file.kind)),
          )
          .child(
            div()
              .flex_1()
              .min_w_0()
              .overflow_hidden()
              .whitespace_nowrap()
              .text_color(theme.text_muted)
              .child(file.path.clone()),
          )
          .child(
            div()
              .id(SharedString::from(format!("git-toggle-{prefix}-{ix}")))
              .flex_none()
              .size(px(16.0))
              .flex()
              .items_center()
              .justify_center()
              .rounded_sm()
              .text_color(theme.text_dim)
              .hover(|s| s.bg(theme.elevated).text_color(theme.text))
              .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
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
              ),
          )
          .into_any_element()
      })
      .collect()
  }

  fn render_commit_box(
    &self,
    git: &GitSnapshot,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let can_commit = !git.staged.is_empty();
    let can_write = can_commit && !self.generating_message;
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
          .id("commit-button")
          .h(px(26.0))
          .flex()
          .items_center()
          .justify_center()
          .rounded_md()
          .text_xs()
          .when(can_commit, |el| {
            el.bg(theme.elevated)
              .text_color(theme.text)
              .cursor_pointer()
              .hover(|s| s.bg(theme.hover))
              .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx)))
          })
          .when(!can_commit, |el| el.text_color(theme.text_dim))
          .child(format!("Commit ({})", git.staged.len())),
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
    let Some(git) = self.git.clone() else {
      return div()
        .p_3()
        .text_xs()
        .text_color(theme.text_dim)
        .child("not a git repository")
        .into_any_element();
    };

    let commit_box = self.render_commit_box(&git, theme, cx);
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
          &git.conflicted,
          DiffBase::Unstaged,
          "conflict",
          false,
          theme,
          cx,
        ));
    }

    let staged = if self.is_open(GitSection::Staged) {
      self.git_file_rows(&git.staged, DiffBase::Staged, "staged", true, theme, cx)
    } else {
      Vec::new()
    };
    let (unstaged, untracked) = if self.is_open(GitSection::Changes) {
      (
        self.git_file_rows(
          &git.unstaged,
          DiffBase::Unstaged,
          "unstaged",
          false,
          theme,
          cx,
        ),
        self.git_file_rows(
          &git.untracked,
          DiffBase::Unstaged,
          "untracked",
          false,
          theme,
          cx,
        ),
      )
    } else {
      (Vec::new(), Vec::new())
    };
    let commits: Vec<helix_models::CommitInfo> = if self.is_open(GitSection::Commits) {
      git.recent_commits.clone()
    } else {
      Vec::new()
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
      .children(commits.into_iter().map(|commit| {
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
        el.child(spinner("pr-busy", theme.text_dim))
      })
      .when(!self.pr_busy, |el| {
        el.child(
          icon_button_path("pr-refresh", "icons/refresh.svg", theme)
            .on_click(cx.listener(|this, _, _, cx| this.refresh_pull_request(cx))),
        )
      })
      .into_any_element()
  }

  fn run_primary_action(&mut self, action: NextAction, cx: &mut Context<Self>) {
    let root = self.root.clone();
    let branch = self
      .git
      .as_ref()
      .map(|git| git.branch.clone())
      .unwrap_or_default();

    let result = match action {
      NextAction::Publish => helix_git::remote::publish(&root, &branch),
      NextAction::Push => helix_git::remote::push(&root),
      NextAction::Sync => helix_git::remote::sync(&root),
      NextAction::Commit => {
        self.active = RightTab::Git;
        Ok(())
      }
      NextAction::Retry => Ok(()),
      NextAction::OpenExistingReview => {
        if let Some(review) = &self.pr {
          let _ = std::process::Command::new("open").arg(&review.url).spawn();
        }
        Ok(())
      }
      NextAction::CreateReview => self.create_pull_request(&root, cx),
      NextAction::Authenticate | NextAction::InstallGh | NextAction::None => Ok(()),
    };

    self.git_error = result.err().map(|err| err.to_string());
    cx.emit(ContextPanelEvent::GitChanged);
    self.refresh_pull_request(cx);
    cx.notify();
  }

  fn create_pull_request(&mut self, root: &PathBuf, _cx: &mut Context<Self>) -> anyhow::Result<()> {
    let base = helix_git::diff::default_base_ref(root)
      .map(|base| base.trim_start_matches("origin/").to_string())
      .unwrap_or_else(|| "main".to_string());
    let title = self
      .git
      .as_ref()
      .and_then(|git| git.recent_commits.first().map(|c| c.summary.clone()))
      .unwrap_or_else(|| "Update".to_string());
    helix_github::review::create(root, &base, &title, "", false)?;
    Ok(())
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

fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
  let lower = a.to_lowercase().cmp(&b.to_lowercase());
  if lower != std::cmp::Ordering::Equal {
    return lower;
  }
  a.cmp(b)
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
            div()
              .id(SharedString::from(format!("right-tab-{}", tab.label())))
              .px_2()
              .py_1()
              .rounded_md()
              .text_xs()
              .cursor_pointer()
              .when(is_active, |el| el.bg(theme.elevated).text_color(theme.text))
              .when(!is_active, |el| {
                el.text_color(theme.text_dim).hover(|s| s.bg(theme.hover))
              })
              .on_click(cx.listener(move |this, _, _, cx| {
                this.active = tab;
                if tab == RightTab::Pr && this.pr_eligibility.is_none() {
                  this.refresh_pull_request(cx);
                }
                cx.notify();
              }))
              .child(tab.label())
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
      .child(tabs)
      .children(toolbar)
      .child(body)
  }
}
