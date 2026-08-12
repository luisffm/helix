use crate::components::{
  BODY, HEADER_HEIGHT, SMALL, TINY, TITLE, claude_icon, icon_button, project_avatar, pulsing_dot,
};
use crate::theme::Theme;
use crate::workspace::Workspace;
use gpui::{
  Animation, AnimationExt, App, Context, Entity, EntityId, EventEmitter, IntoElement,
  ParentElement, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::menu::ContextMenuExt;
use gpui_component::{Icon, IconName};
use helix_commands::{
  CopyPathAction, DeleteWorktreeAction, EditWorktreeAction, OpenInFinderAction, OpenInZedAction,
  OpenProjectSettingsAction, RemoveProjectAction, RemoveWorktreeAction,
};
use helix_github::{BranchReview, CheckStatus};
use helix_models::AgentStatus;
use helix_models::{GitSnapshot, ProjectInfo, SessionKind};
use helix_worktree::{WorktreeRow, canonical_path};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

const COLLAPSE_MS: u64 = 160;

pub enum ProjectPanelEvent {
  OpenProject(PathBuf),
}

/// The branch icon is the only place a worktree's checks are reported, so it
/// separates a running suite from a finished one rather than folding both into
/// the "nothing to say" colour the design gives an unreviewed branch.
fn checks_color(review: Option<&BranchReview>, theme: &Theme) -> gpui::Hsla {
  match review.map(|review| review.checks) {
    Some(CheckStatus::Passing) => theme.green,
    Some(CheckStatus::Failing) => theme.red,
    Some(CheckStatus::Pending) => theme.yellow,
    _ => theme.text_muted,
  }
}

#[derive(Clone)]
struct ProjectEntry {
  info: ProjectInfo,
  accent: Option<String>,
}

pub struct ProjectPanel {
  projects: Vec<ProjectEntry>,
  active_root: PathBuf,
  active_canonical: PathBuf,
  git: Option<GitSnapshot>,
  worktrees: HashMap<PathBuf, Vec<WorktreeRow>>,
  workspaces: HashMap<PathBuf, Entity<Workspace>>,
  observed: HashSet<EntityId>,
  expanded: HashSet<PathBuf>,
  closing: HashSet<PathBuf>,
  reviews: HashMap<PathBuf, HashMap<String, BranchReview>>,
}

impl EventEmitter<ProjectPanelEvent> for ProjectPanel {}

fn load_project_entries() -> Vec<ProjectEntry> {
  helix_state::config::visible_projects()
    .iter()
    .map(|p| ProjectEntry {
      info: ProjectInfo {
        name: p.label(),
        root: p.path.clone(),
      },
      accent: p.accent.clone(),
    })
    .collect()
}

impl ProjectPanel {
  pub fn new(project: ProjectInfo, cx: &mut Context<Self>) -> Self {
    cx.spawn(async move |this, cx| {
      let mut signature: Vec<(AgentStatus, String)> = Vec::new();

      loop {
        let Ok(fast) = this.update(cx, |panel, cx| panel.has_ticking_labels(cx)) else {
          break;
        };

        let interval = if fast {
          Duration::from_secs(1)
        } else {
          Duration::from_secs(10)
        };

        cx.background_executor().timer(interval).await;

        let updated = this.update(cx, |panel, cx| {
          let next = panel.activity_signature(cx);

          if next != signature {
            signature = next;

            cx.notify();
          }
        });

        if updated.is_err() {
          break;
        }
      }
    })
    .detach();

    helix_state::config::ensure_project(&project.root);

    let projects = load_project_entries();
    let mut expanded = HashSet::new();

    expanded.insert(project.root.clone());

    Self {
      active_canonical: canonical_path(&project.root),
      projects,
      active_root: project.root,
      git: None,
      worktrees: HashMap::new(),
      workspaces: HashMap::new(),
      observed: HashSet::new(),
      expanded,
      closing: HashSet::new(),
      reviews: HashMap::new(),
    }
  }

  /// Second-granularity labels only earn a per-second tick while something is
  /// actually running; otherwise they catch up on the slow tick.
  fn has_ticking_labels(&self, cx: &App) -> bool {
    self.workspaces.values().any(|workspace| {
      workspace
        .read(cx)
        .terminals()
        .any(|(_, view)| view.read(cx).status() == AgentStatus::Running)
    })
  }

  fn activity_signature(&self, cx: &App) -> Vec<(AgentStatus, String)> {
    let mut roots: Vec<&PathBuf> = self.workspaces.keys().collect();

    roots.sort();

    roots
      .into_iter()
      .flat_map(|root| {
        self.workspaces[root].read(cx).terminals().map(|(_, view)| {
          let view = view.read(cx);

          (view.status(), view.activity_ago())
        })
      })
      .collect()
  }

  fn toggle_project(&mut self, root: PathBuf, cx: &mut Context<Self>) {
    if self.expanded.remove(&root) {
      self.closing.insert(root.clone());

      cx.spawn(async move |this, cx| {
        cx.background_executor()
          .timer(Duration::from_millis(COLLAPSE_MS))
          .await;

        this
          .update(cx, |panel, cx| {
            panel.closing.remove(&root);
            cx.notify();
          })
          .ok();
      })
      .detach();
    } else {
      self.closing.remove(&root);
      self.expanded.insert(root);
    }

    cx.notify();
  }

  pub fn set_reviews(
    &mut self,
    owner: PathBuf,
    states: HashMap<String, BranchReview>,
    cx: &mut Context<Self>,
  ) {
    if self.reviews.get(&owner) == Some(&states) {
      return;
    }

    self.reviews.insert(owner, states);

    cx.notify();
  }

  pub fn set_git(&mut self, git: Option<GitSnapshot>, cx: &mut Context<Self>) {
    self.git = git;

    cx.notify();
  }

  pub fn set_worktrees(
    &mut self,
    worktrees: HashMap<PathBuf, Vec<WorktreeRow>>,
    every_project: bool,
    cx: &mut Context<Self>,
  ) {
    if every_project {
      self.worktrees = worktrees;
    } else {
      self.worktrees.extend(worktrees);
    }

    cx.notify();
  }

  pub fn set_workspaces(
    &mut self,
    workspaces: HashMap<PathBuf, Entity<Workspace>>,
    cx: &mut Context<Self>,
  ) {
    for workspace in workspaces.values() {
      if self.observed.insert(workspace.entity_id()) {
        cx.observe(workspace, |_, _, cx| cx.notify()).detach();
      }
    }

    self.workspaces = workspaces;

    cx.notify();
  }

  pub fn set_active_project(&mut self, project: ProjectInfo, cx: &mut Context<Self>) {
    self.projects = load_project_entries();
    self.active_root = project.root.clone();
    self.active_canonical = canonical_path(&project.root);

    let owner = self
      .projects
      .iter()
      .find(|entry| {
        entry.info.root == project.root
          || helix_state::config::worktrees_for(&entry.info.root)
            .iter()
            .any(|wt| wt == &project.root)
      })
      .map(|entry| entry.info.root.clone())
      .unwrap_or(project.root);

    self.expanded.insert(owner);

    cx.notify();
  }
}

impl ProjectPanel {
  /// The one agent line a worktree row gets: a session still working wins over
  /// a finished one, because that is the row's live state.
  fn agent_summary(&self, canonical: &PathBuf, cx: &App) -> Option<(String, AgentStatus)> {
    let workspace = self.workspaces.get(canonical)?;

    let sessions: Vec<(String, AgentStatus)> = workspace
      .read(cx)
      .terminals()
      .filter(|(_, view)| view.read(cx).agent_kind() == SessionKind::ClaudeCode)
      .map(|(_, view)| {
        let view = view.read(cx);

        (
          helix_agents::strip_spinner(&view.title).to_string(),
          view.status(),
        )
      })
      .collect();

    sessions
      .iter()
      .find(|(_, status)| {
        matches!(
          status,
          AgentStatus::Running | AgentStatus::Waiting | AgentStatus::Thinking
        )
      })
      .or_else(|| sessions.first())
      .cloned()
  }

  /// The second line of a worktree row: what its agent is doing, or which pull
  /// request it carries. With neither, the row stays one line — the design never
  /// writes "no PR".
  fn worktree_detail(
    &self,
    project_ix: usize,
    ix: usize,
    row: &WorktreeRow,
    review: Option<&BranchReview>,
    theme: &Theme,
    cx: &App,
  ) -> Option<gpui::AnyElement> {
    let line = div()
      .flex()
      .items_center()
      .gap(px(7.0))
      .mt(px(3.0))
      .pl(px(19.0))
      .text_size(px(SMALL));

    if let Some((title, status)) = self.agent_summary(&row.canonical, cx) {
      let working = matches!(
        status,
        AgentStatus::Running | AgentStatus::Waiting | AgentStatus::Thinking
      );

      let trailing: gpui::AnyElement = if working {
        pulsing_dot(
          SharedString::from(format!("agent-dot-{project_ix}-{ix}")),
          theme.claude,
        )
        .into_any_element()
      } else if status == AgentStatus::Error {
        div()
          .flex_none()
          .text_color(theme.red)
          .child(Icon::new(IconName::CircleX).size(px(11.0)))
          .into_any_element()
      } else {
        div()
          .flex_none()
          .text_color(theme.green)
          .child(Icon::new(IconName::Check).size(px(11.0)))
          .into_any_element()
      };

      return Some(
        line
          .child(claude_icon(theme.claude, 11.0))
          .child(
            div()
              .flex_1()
              .min_w_0()
              .text_color(if working {
                theme.text_muted
              } else {
                theme.text_dim
              })
              .overflow_hidden()
              .whitespace_nowrap()
              .text_ellipsis()
              .child(title),
          )
          .child(trailing)
          .into_any_element(),
      );
    }

    let reference = review
      .map(|review| format!("#{}", review.number))
      .or_else(|| row.pr.as_deref().map(helix_github::short_ref))
      .or_else(|| row.issue.as_deref().map(helix_github::short_ref))?;

    Some(
      line
        .child(
          div()
            .flex_none()
            .font_family(theme.font_mono.clone())
            .text_size(px(TINY))
            .text_color(theme.text_dim)
            .child(reference),
        )
        .into_any_element(),
    )
  }

  fn worktree_row(
    &self,
    project_ix: usize,
    ix: usize,
    project_root: &PathBuf,
    row: &WorktreeRow,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> gpui::AnyElement {
    let wt = &row.entry;
    let label = row
      .display_name
      .clone()
      .unwrap_or_else(|| wt.branch.clone());
    let is_active = row.canonical == self.active_canonical;

    let review = self
      .reviews
      .get(project_root)
      .and_then(|found| found.get(&wt.branch));

    let detail = self.worktree_detail(project_ix, ix, row, review, theme, cx);
    let click_root = wt.path.clone();

    let element = div()
      .id(SharedString::from(format!("branch-{project_ix}-{ix}")))
      .flex()
      .flex_col()
      .px(px(8.0))
      .py(px(7.0))
      .rounded(px(8.0))
      .cursor_pointer()
      .when(is_active, |el| el.bg(theme.active))
      .when(!is_active, |el| el.hover(|s| s.bg(theme.hover)))
      .on_click(cx.listener(move |_, _, _, cx| {
        cx.emit(ProjectPanelEvent::OpenProject(click_root.clone()));
      }))
      .child(
        div()
          .flex()
          .items_center()
          .gap(px(7.0))
          .child(
            div()
              .flex_none()
              .text_color(checks_color(review, theme))
              .child(Icon::new(crate::icons::HelixIcon::GitBranch).size(px(12.0))),
          )
          .child(
            div()
              .flex_1()
              .min_w_0()
              .text_size(px(BODY))
              .font_weight(if is_active {
                gpui::FontWeight::SEMIBOLD
              } else {
                gpui::FontWeight::MEDIUM
              })
              .text_color(theme.text)
              .overflow_hidden()
              .whitespace_nowrap()
              .text_ellipsis()
              .child(label.clone()),
          ),
      )
      .children(detail);

    if wt.is_primary {
      return element.into_any_element();
    }

    let owner = project_root.clone();
    let path = wt.path.clone();

    element
      .context_menu(move |menu, window, cx| {
        let owner = owner.clone();
        let path = path.clone();

        menu
          .label(label.clone())
          .menu_with_icon(
            "Edit Worktree",
            Icon::new(IconName::Settings2),
            Box::new(EditWorktreeAction {
              owner: owner.clone(),
              path: path.clone(),
            }),
          )
          .separator()
          .submenu_with_icon(
            Some(Icon::new(IconName::FolderOpen)),
            "Open in",
            window,
            cx,
            {
              let path = path.clone();

              move |menu, _, _| {
                menu
                  .menu_with_icon(
                    "Zed",
                    Icon::new(IconName::ExternalLink),
                    Box::new(OpenInZedAction { path: path.clone() }),
                  )
                  .menu_with_icon(
                    "Finder",
                    Icon::new(IconName::Folder),
                    Box::new(OpenInFinderAction { path: path.clone() }),
                  )
              }
            },
          )
          .menu_with_icon(
            "Copy Path",
            Icon::new(IconName::Copy),
            Box::new(CopyPathAction { path: path.clone() }),
          )
          .separator()
          .menu_with_icon(
            "Remove from Sidebar",
            Icon::new(IconName::CircleX),
            Box::new(RemoveWorktreeAction {
              owner: owner.clone(),
              path: path.clone(),
            }),
          )
          .menu_with_icon(
            "Delete Worktree",
            Icon::new(IconName::Delete),
            Box::new(DeleteWorktreeAction {
              owner: owner.clone(),
              path: path.clone(),
            }),
          )
      })
      .into_any_element()
  }
}

impl Render for ProjectPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    let active_root = self.active_root.clone();
    let active_canonical = self.active_canonical.clone();

    let header = div()
      .id("sidebar-titlebar")
      .window_control_area(gpui::WindowControlArea::Drag)
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .h(px(HEADER_HEIGHT))
      .pl(px(70.0))
      .pr(px(10.0))
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .flex_1()
          .text_size(px(TITLE))
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.text)
          .overflow_hidden()
          .child("Helix"),
      )
      .child(
        icon_button("collapse-left", IconName::PanelLeft, &theme)
          .tooltip_with_action(
            "Hide the project sidebar",
            &helix_commands::ToggleLeftSidebar,
            None,
          )
          .on_click(|_, window, cx| {
            window.dispatch_action(Box::new(helix_commands::ToggleLeftSidebar), cx);
          }),
      );

    let search = div()
      .flex_none()
      .px(px(10.0))
      .pt(px(10.0))
      .pb(px(4.0))
      .child(
        div()
          .id("search-box")
          .flex()
          .items_center()
          .gap_2()
          .h(px(30.0))
          .px(px(9.0))
          .rounded(px(8.0))
          .bg(theme.panel)
          .border_1()
          .border_color(theme.panel_border)
          .text_size(px(BODY))
          .text_color(theme.text_dim)
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover).text_color(theme.text_muted))
          .on_click(|_, window, cx| {
            window.dispatch_action(Box::new(helix_commands::OpenSearch), cx);
          })
          .child(
            div()
              .flex_none()
              .child(Icon::new(IconName::Search).size(px(13.0))),
          )
          .child(div().flex_1().child("Search"))
          .child(crate::components::cap("⌘K", &theme).text_color(theme.text_dim)),
      );

    let mut tree = div()
      .id("project-tree")
      .flex()
      .flex_col()
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .px(px(10.0))
      .pt(px(6.0))
      .pb(px(10.0));

    for (project_ix, entry) in self.projects.iter().enumerate() {
      let project_root = entry.info.root.clone();
      let worktree_list: &[WorktreeRow] = self
        .worktrees
        .get(&project_root)
        .map(Vec::as_slice)
        .unwrap_or_default();
      let is_active_project = project_root == active_root
        || worktree_list
          .iter()
          .any(|row| row.canonical == active_canonical);
      let expanded = self.expanded.contains(&project_root);
      let closing = self.closing.contains(&project_root);

      let toggle_root = project_root.clone();
      // A project whose worktrees have not been described yet has nothing to
      // expand into, so clicking it opens it rather than toggling an empty group.
      let opens_directly = worktree_list.is_empty();

      let project_header = div()
        .id(SharedString::from(format!("project-row-{project_ix}")))
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .px(px(8.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .on_click(cx.listener(move |this, _, _, cx| {
          if opens_directly {
            cx.emit(ProjectPanelEvent::OpenProject(toggle_root.clone()));
          } else {
            this.toggle_project(toggle_root.clone(), cx);
          }
        }))
        .child(
          div().flex_none().text_color(theme.text_dim).child(
            Icon::new(if expanded {
              IconName::ChevronDown
            } else {
              IconName::ChevronRight
            })
            .size(px(10.0)),
          ),
        )
        .child(project_avatar(
          &entry.info.name,
          if is_active_project {
            None
          } else {
            entry.accent.as_deref()
          },
          &theme,
        ))
        .child(
          div()
            .flex_1()
            .min_w_0()
            .text_size(px(BODY))
            .when(is_active_project, |el| {
              el.font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
            })
            .when(!is_active_project, |el| el.text_color(theme.text_muted))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(entry.info.name.clone()),
        )
        .when(expanded && !worktree_list.is_empty(), |el| {
          el.child(
            div()
              .flex_none()
              .text_size(px(TINY))
              .text_color(theme.text_dim)
              .child(worktree_list.len().to_string()),
          )
        });

      let ctx_root = project_root.clone();
      let ctx_name = entry.info.name.clone();
      let project_header = project_header.context_menu(move |menu, window, cx| {
        let root = ctx_root.clone();

        menu
          .label(ctx_name.clone())
          .menu_with_icon(
            "Project Settings",
            Icon::new(IconName::Settings),
            Box::new(OpenProjectSettingsAction { root: root.clone() }),
          )
          .separator()
          .submenu_with_icon(
            Some(Icon::new(IconName::FolderOpen)),
            "Open in",
            window,
            cx,
            {
              let root = root.clone();

              move |menu, _, _| {
                menu
                  .menu_with_icon(
                    "Zed",
                    Icon::new(IconName::ExternalLink),
                    Box::new(OpenInZedAction { path: root.clone() }),
                  )
                  .menu_with_icon(
                    "Finder",
                    Icon::new(IconName::Folder),
                    Box::new(OpenInFinderAction { path: root.clone() }),
                  )
              }
            },
          )
          .menu_with_icon(
            "Copy Path",
            Icon::new(IconName::Copy),
            Box::new(CopyPathAction { path: root.clone() }),
          )
          .separator()
          .menu_with_icon(
            "Remove Project",
            Icon::new(IconName::Delete),
            Box::new(RemoveProjectAction { root: root.clone() }),
          )
      });

      tree = tree.child(project_header);

      if !expanded && !closing {
        continue;
      }

      // The rule down the left is the only thing grouping these rows: no card,
      // no box.
      let group = div()
        .flex()
        .flex_col()
        .gap(px(1.0))
        .mt(px(2.0))
        .mb(px(4.0))
        .ml(px(5.0))
        .pl(px(8.0))
        .border_l_1()
        .border_color(theme.panel_border)
        .children(
          worktree_list
            .iter()
            .enumerate()
            .map(|(ix, row)| self.worktree_row(project_ix, ix, &project_root, row, &theme, cx)),
        );

      tree = tree.child(group.with_animation(
        SharedString::from(format!(
          "worktrees-{project_ix}-{}",
          if expanded { "open" } else { "closed" }
        )),
        Animation::new(Duration::from_millis(COLLAPSE_MS)).with_easing(gpui::ease_in_out),
        move |block, delta| {
          let progress = if expanded { delta } else { 1.0 - delta };

          block
            .opacity(progress)
            .relative()
            .top(px(-6.0 * (1.0 - progress)))
        },
      ));
    }

    div()
      .relative()
      .flex()
      .flex_col()
      .size_full()
      .child(header)
      .child(search)
      .child(tree)
  }
}
