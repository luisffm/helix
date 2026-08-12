use crate::components::{
  BODY, HEADER_HEIGHT, META, MICRO, TINY, TITLE, claude_icon, git_branch_icon, icon_button, pill,
  project_avatar, pulsing_dot, section_label, spinner,
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
use helix_github::{ReviewState, short_ref};
use helix_models::AgentStatus;
use helix_models::{GitSnapshot, ProjectInfo, SessionKind};
use helix_worktree::{WorktreeRow, canonical_path};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

const COLLAPSE_MS: u64 = 160;

pub enum ProjectPanelEvent {
  OpenProject(PathBuf),
  /// The sidebar's single `+`: the dialog it opens covers both adding a
  /// worktree and picking a new workspace.
  RequestAdd,
}

fn review_color(state: Option<ReviewState>, theme: &Theme) -> gpui::Hsla {
  match state {
    Some(ReviewState::Open) => theme.green,
    Some(ReviewState::Merged) => theme.purple,
    Some(ReviewState::Closed) => theme.red,
    Some(ReviewState::Draft) | None => theme.text_dim,
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
  reviews: HashMap<PathBuf, HashMap<String, ReviewState>>,
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
    states: HashMap<String, ReviewState>,
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
  fn agent_rows(
    &self,
    project_ix: usize,
    worktree: &PathBuf,
    canonical: &PathBuf,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> Option<gpui::AnyElement> {
    let workspace = self.workspaces.get(canonical)?.clone();
    let is_active_card = *canonical == self.active_canonical;
    let active_tab = workspace.read(cx).active;

    let rows: Vec<_> = workspace
      .read(cx)
      .terminals()
      .filter(|(_, view)| view.read(cx).agent_kind() == SessionKind::ClaudeCode)
      .map(|(tab_ix, view)| {
        let view = view.read(cx);
        let status = view.status();
        let title = helix_agents::strip_spinner(&view.title).to_string();
        let ago = view.activity_ago();
        let is_tab_active = tab_ix == active_tab && is_active_card;

        let status_element: gpui::AnyElement = match status {
          AgentStatus::Running | AgentStatus::Waiting | AgentStatus::Thinking => pulsing_dot(
            SharedString::from(format!("agent-dot-{project_ix}-{tab_ix}")),
            theme.claude,
          )
          .into_any_element(),
          AgentStatus::Error => div()
            .flex_none()
            .text_color(theme.red)
            .child(Icon::new(IconName::CircleX).size(px(12.0)))
            .into_any_element(),
          AgentStatus::Idle | AgentStatus::Finished => div()
            .flex_none()
            .text_color(theme.green)
            .child(Icon::new(IconName::Check).size(px(12.0)))
            .into_any_element(),
        };

        let ws = workspace.clone();
        let wt_root = worktree.clone();
        let switch_first = !is_active_card;

        div()
          .id(SharedString::from(format!("agent-{project_ix}-{tab_ix}")))
          .flex()
          .items_center()
          .gap(px(7.0))
          .h(px(27.0))
          .px(px(6.0))
          .rounded(px(7.0))
          .cursor_pointer()
          .when(is_tab_active, |el| el.bg(theme.hover))
          .when(!is_tab_active, |el| el.hover(|s| s.bg(theme.hover)))
          .on_click(cx.listener(move |_, _, window, cx| {
            if switch_first {
              cx.emit(ProjectPanelEvent::OpenProject(wt_root.clone()));
            }

            ws.update(cx, |workspace, cx| {
              workspace.activate(tab_ix, window, cx);
            });
          }))
          .child(claude_icon(theme.claude, 13.0))
          .child(
            div()
              .flex_1()
              .min_w_0()
              .text_size(px(BODY))
              .text_color(theme.text)
              .overflow_hidden()
              .whitespace_nowrap()
              .text_ellipsis()
              .child(title),
          )
          .child(
            div()
              .flex_none()
              .text_size(px(META))
              .text_color(theme.text_dim)
              .child(ago),
          )
          .child(status_element)
      })
      .collect();

    if rows.is_empty() {
      return None;
    }

    Some(
      div()
        .flex()
        .flex_col()
        .gap(px(1.0))
        .py(px(2.0))
        .pl(px(14.0))
        .children(rows)
        .into_any_element(),
    )
  }

  fn worktree_block(
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
    let is_active_card = row.canonical == self.active_canonical;

    let review = review_color(
      self
        .reviews
        .get(project_root)
        .and_then(|states| states.get(&wt.branch))
        .copied(),
      theme,
    );

    let any_running = self
      .workspaces
      .get(&row.canonical)
      .is_some_and(|workspace| {
        workspace.read(cx).terminals().any(|(_, view)| {
          let view = view.read(cx);

          view.agent_kind() == SessionKind::ClaudeCode && view.status() == AgentStatus::Running
        })
      });

    let branch_icon: gpui::AnyElement = if any_running {
      spinner(
        SharedString::from(format!("branch-spin-{project_ix}-{ix}")),
        theme.claude,
      )
      .into_any_element()
    } else {
      git_branch_icon(if is_active_card {
        theme.text_muted
      } else {
        review
      })
      .into_any_element()
    };

    let click_root = wt.path.clone();
    let mut branch_row = div()
      .id(SharedString::from(format!("branch-{project_ix}-{ix}")))
      .flex()
      .items_center()
      .gap(px(7.0))
      .h(px(28.0))
      .px(px(6.0))
      .mt(px(1.0))
      .rounded(px(7.0))
      .cursor_pointer()
      .when(is_active_card, |el| el.bg(theme.active))
      .when(!is_active_card, |el| el.hover(|s| s.bg(theme.hover)))
      .on_click(cx.listener(move |_, _, _, cx| {
        cx.emit(ProjectPanelEvent::OpenProject(click_root.clone()));
      }))
      .child(branch_icon)
      .child(
        div()
          .flex_1()
          .min_w_0()
          .text_size(px(BODY))
          .font_weight(gpui::FontWeight::MEDIUM)
          .text_color(if is_active_card {
            theme.text
          } else {
            theme.text_muted
          })
          .overflow_hidden()
          .whitespace_nowrap()
          .text_ellipsis()
          .child(label.clone()),
      );

    if wt.is_primary {
      branch_row = branch_row.child(pill("primary", theme.text_muted, theme.active, MICRO));
    }

    for reference in [row.issue.as_ref(), row.pr.as_ref()].into_iter().flatten() {
      branch_row = branch_row.child(
        div()
          .flex_none()
          .text_size(px(TINY))
          .text_color(theme.text_dim)
          .child(short_ref(reference)),
      );
    }

    let branch_element: gpui::AnyElement = if wt.is_primary {
      branch_row.into_any_element()
    } else {
      let owner = project_root.clone();
      let path = wt.path.clone();
      let menu_label = label;

      branch_row
        .context_menu(move |menu, window, cx| {
          let owner = owner.clone();
          let path = path.clone();

          menu
            .label(menu_label.clone())
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
    };

    let agents = self.agent_rows(project_ix, &wt.path, &row.canonical, theme, cx);

    div()
      .flex()
      .flex_col()
      .child(branch_element)
      .children(agents)
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
      .pb(px(10.0))
      .child(
        div()
          .flex()
          .items_center()
          .px(px(4.0))
          .pt(px(8.0))
          .pb(px(6.0))
          .child(div().flex_1().child(section_label("PROJECTS", &theme)))
          .child(
            icon_button("project-add", IconName::Plus, &theme)
              .tooltip("Add a project or worktree")
              .on_click(cx.listener(|_, _, _, cx| {
                cx.emit(ProjectPanelEvent::RequestAdd);
              })),
          ),
      );

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

      let avatar = project_avatar(
        &entry.info.name,
        if is_active_project {
          None
        } else {
          entry.accent.as_deref()
        },
        &theme,
      );

      let toggle_root = project_root.clone();
      // A project whose worktrees have not been described yet has nothing to
      // expand into, so clicking it opens it rather than toggling an empty card.
      let opens_directly = worktree_list.is_empty();
      let mut project_row = div()
        .id(SharedString::from(format!("project-row-{project_ix}")))
        .flex()
        .items_center()
        .gap_2()
        .h(px(30.0))
        .rounded(px(if expanded { 7.0 } else { 8.0 }))
        .px(px(if expanded { 6.0 } else { 10.0 }))
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .on_click(cx.listener(move |this, _, _, cx| {
          if opens_directly {
            cx.emit(ProjectPanelEvent::OpenProject(toggle_root.clone()));
          } else {
            this.toggle_project(toggle_root.clone(), cx);
          }
        }))
        .child(avatar)
        .child(
          div()
            .flex_1()
            .min_w_0()
            .text_size(px(TITLE))
            .when(expanded, |el| {
              el.font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
            })
            .when(!expanded, |el| el.text_color(theme.text_muted))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .child(entry.info.name.clone()),
        );

      project_row = project_row.child(
        div().flex_none().text_color(theme.text_dim).child(
          Icon::new(if expanded {
            IconName::ChevronDown
          } else {
            IconName::ChevronRight
          })
          .size(px(11.0)),
        ),
      );

      let ctx_root = project_root.clone();
      let ctx_name = entry.info.name.clone();
      let project_row = project_row.context_menu(move |menu, window, cx| {
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

      if !expanded && !closing {
        tree = tree.child(div().mb(px(1.0)).child(project_row));

        continue;
      }

      let (primary, others): (Vec<_>, Vec<_>) = worktree_list
        .iter()
        .enumerate()
        .partition(|(_, row)| row.entry.is_primary);

      let mut body = div().flex().flex_col();

      for (ix, row) in primary {
        body = body.child(self.worktree_block(project_ix, ix, &project_root, row, &theme, cx));
      }

      if !others.is_empty() {
        let mut rest = div()
          .flex()
          .flex_col()
          .gap(px(1.0))
          .pt(px(2.0))
          .mt(px(2.0))
          .border_t_1()
          .border_color(theme.panel_border);

        for (ix, row) in others {
          rest = rest.child(self.worktree_block(project_ix, ix, &project_root, row, &theme, cx));
        }

        body = body.child(rest);
      }

      let body = body.with_animation(
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
      );

      tree = tree.child(
        div()
          .mb(px(6.0))
          .p(px(4.0))
          .rounded(px(10.0))
          .bg(theme.panel)
          .border_1()
          .border_color(theme.panel_border)
          .child(project_row)
          .child(body),
      );
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
