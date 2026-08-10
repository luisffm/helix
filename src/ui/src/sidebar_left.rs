use crate::components::{HEADER_HEIGHT, git_branch_icon, icon_button, project_icon, spinner};
use crate::icons::HelixIcon;
use crate::theme::Theme;
use crate::workspace::Workspace;
use gpui::{
  Context, Entity, EntityId, EventEmitter, IntoElement, ParentElement, Render, SharedString,
  Window, div, prelude::*, px,
};
use gpui_component::menu::ContextMenuExt;
use gpui_component::{Icon, IconName};
use helix_commands::{
  CopyPathAction, DeleteWorktreeAction, EditWorktreeAction, OpenInFinderAction, OpenInZedAction,
  OpenProjectSettingsAction, RemoveProjectAction, RemoveWorktreeAction,
};
use helix_models::AgentStatus;
use helix_models::{GitSnapshot, ProjectInfo, SessionKind};
use helix_worktree::WorktreeEntry;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

pub enum ProjectPanelEvent {
  OpenProject(PathBuf),
  RequestAddProject,
  RequestAddWorktree,
  OpenSettings(Option<PathBuf>),
}

#[derive(Clone)]
pub struct WorktreeRow {
  pub entry: WorktreeEntry,
  pub display_name: Option<String>,
  pub issue: Option<String>,
  pub pr: Option<String>,
}

pub fn short_ref(value: &str) -> String {
  let digits: String = value
    .chars()
    .rev()
    .take_while(|c| c.is_ascii_digit())
    .collect::<String>()
    .chars()
    .rev()
    .collect();
  if digits.is_empty() {
    "link".to_string()
  } else {
    format!("#{digits}")
  }
}

#[derive(Clone)]
enum ProjectGlyph {
  Emoji(String),
  Icon(String),
}

fn agent_status_visual(status: AgentStatus, theme: &Theme) -> (IconName, gpui::Hsla) {
  match status {
    AgentStatus::Running => (IconName::LoaderCircle, theme.yellow),
    AgentStatus::Waiting => (IconName::LoaderCircle, theme.yellow),
    AgentStatus::Thinking => (IconName::LoaderCircle, theme.purple),
    AgentStatus::Idle => (IconName::CircleCheck, theme.green),
    AgentStatus::Finished => (IconName::CircleCheck, theme.green),
    AgentStatus::Error => (IconName::CircleX, theme.red),
  }
}

fn agent_description(status: AgentStatus) -> &'static str {
  match status {
    AgentStatus::Running => "Working…",
    AgentStatus::Waiting => "Waiting…",
    AgentStatus::Thinking => "Thinking…",
    AgentStatus::Idle => "Claude",
    AgentStatus::Finished => "Finished",
    AgentStatus::Error => "Interrupted",
  }
}

#[derive(Clone)]
struct ProjectEntry {
  info: ProjectInfo,
  glyph: ProjectGlyph,
}

pub struct ProjectPanel {
  projects: Vec<ProjectEntry>,
  active_root: PathBuf,
  git: Option<GitSnapshot>,
  worktrees: HashMap<PathBuf, Vec<WorktreeRow>>,
  workspaces: HashMap<PathBuf, Entity<Workspace>>,
  observed: HashSet<EntityId>,
  expanded: HashSet<PathBuf>,
}

impl EventEmitter<ProjectPanelEvent> for ProjectPanel {}

fn load_project_entries() -> Vec<ProjectEntry> {
  helix_state::config::load()
    .projects
    .iter()
    .filter(|p| p.path.is_dir())
    .map(|p| {
      let dir_name = p
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.path.display().to_string());
      let glyph = if let Some(icon) = &p.icon {
        ProjectGlyph::Icon(icon.clone())
      } else if let Some(emoji) = &p.emoji {
        ProjectGlyph::Emoji(emoji.clone())
      } else {
        ProjectGlyph::Icon("folder".to_string())
      };
      ProjectEntry {
        info: ProjectInfo {
          name: p.display_name.clone().unwrap_or(dir_name),
          root: p.path.clone(),
        },
        glyph,
      }
    })
    .collect()
}

impl ProjectPanel {
  pub fn new(project: ProjectInfo, cx: &mut Context<Self>) -> Self {
    cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor().timer(Duration::from_secs(1)).await;
        if this.update(cx, |_, cx| cx.notify()).is_err() {
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
      projects,
      active_root: project.root,
      git: None,
      worktrees: HashMap::new(),
      workspaces: HashMap::new(),
      observed: HashSet::new(),
      expanded,
    }
  }

  pub fn set_state(
    &mut self,
    git: Option<GitSnapshot>,
    worktrees: HashMap<PathBuf, Vec<WorktreeRow>>,
    cx: &mut Context<Self>,
  ) {
    self.git = git;
    self.worktrees = worktrees;
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

impl Render for ProjectPanel {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    let active_root = self.active_root.clone();
    let active_canonical = active_root
      .canonicalize()
      .unwrap_or_else(|_| active_root.clone());

    let titlebar_strip = div()
      .id("sidebar-titlebar")
      .window_control_area(gpui::WindowControlArea::Drag)
      .flex()
      .flex_none()
      .items_center()
      .h(px(HEADER_HEIGHT))
      .pl(px(76.0))
      .pr_2()
      .gap_2()
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .flex_1()
          .text_sm()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.text)
          .overflow_hidden()
          .child("Helix"),
      )
      .child(
        icon_button("collapse-left", IconName::PanelLeftClose, &theme).on_click(|_, window, cx| {
          window.dispatch_action(Box::new(helix_commands::ToggleLeftSidebar), cx);
        }),
      );

    let search = div()
      .id("search-box")
      .mx_2()
      .mt_2()
      .mb_2()
      .h(px(28.0))
      .flex()
      .items_center()
      .gap_2()
      .px_2()
      .rounded_md()
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.elevated)
      .text_sm()
      .text_color(theme.text_dim)
      .cursor_pointer()
      .hover(|s| s.bg(theme.hover))
      .on_click(|_, window, cx| {
        window.dispatch_action(Box::new(helix_commands::OpenSearch), cx);
      })
      .child(Icon::new(IconName::Search).size_4())
      .child("Search")
      .child(div().flex_1())
      .child(div().text_xs().text_color(theme.text_dim).child("⌘K"));

    let projects_header = div()
      .flex()
      .items_center()
      .px_3()
      .pb_1()
      .child(
        div()
          .flex_1()
          .text_xs()
          .text_color(theme.text_dim)
          .child("Projects"),
      )
      .child(
        icon_button("project-add", HelixIcon::FolderPlus, &theme).on_click(cx.listener(
          |_, _, _, cx| {
            cx.emit(ProjectPanelEvent::RequestAddProject);
          },
        )),
      )
      .child(
        icon_button("worktree-add", IconName::Plus, &theme).on_click(cx.listener(|_, _, _, cx| {
          cx.emit(ProjectPanelEvent::RequestAddWorktree);
        })),
      );

    let mut tree = div()
      .id("project-tree")
      .flex()
      .flex_col()
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .gap_0p5();

    let projects = self.projects.clone();
    for (project_ix, entry) in projects.iter().enumerate() {
      let project_root = entry.info.root.clone();
      let worktree_list = self
        .worktrees
        .get(&project_root)
        .cloned()
        .unwrap_or_default();
      let is_active_project = project_root == active_root
        || worktree_list.iter().any(|row| {
          row
            .entry
            .path
            .canonicalize()
            .unwrap_or_else(|_| row.entry.path.clone())
            == active_canonical
        });
      let expanded = self.expanded.contains(&project_root);
      let has_worktrees = !worktree_list.is_empty();

      let chevron_root = project_root.clone();
      let row_root = project_root.clone();
      let menu_root = project_root.clone();

      let glyph_element: gpui::AnyElement = match &entry.glyph {
        ProjectGlyph::Icon(name) => div()
          .flex_none()
          .text_color(theme.text_muted)
          .child(Icon::new(project_icon(name).unwrap_or(IconName::Folder)).size_3p5())
          .into_any_element(),
        ProjectGlyph::Emoji(emoji) => div()
          .flex_none()
          .text_xs()
          .child(emoji.clone())
          .into_any_element(),
      };

      let project_row = div()
        .id(SharedString::from(format!("project-row-{project_ix}")))
        .flex()
        .items_center()
        .gap_1p5()
        .mx_2()
        .px_2()
        .h(px(30.0))
        .rounded_md()
        .cursor_pointer()
        .hover(|s| s.bg(theme.hover))
        .on_click(cx.listener(move |this, _, _, cx| {
          if has_worktrees {
            if !this.expanded.insert(row_root.clone()) {
              this.expanded.remove(&row_root);
            }
            cx.notify();
          } else {
            cx.emit(ProjectPanelEvent::OpenProject(row_root.clone()));
          }
        }))
        .child(
          div()
            .id(SharedString::from(format!("project-chevron-{project_ix}")))
            .flex_none()
            .size(px(18.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .text_color(theme.text_dim)
            .hover(|s| s.bg(theme.hover))
            .on_click(cx.listener(move |this, _, _, cx| {
              cx.stop_propagation();
              if !this.expanded.insert(chevron_root.clone()) {
                this.expanded.remove(&chevron_root);
              }
              cx.notify();
            }))
            .child(
              Icon::new(if expanded {
                IconName::ChevronDown
              } else {
                IconName::ChevronRight
              })
              .size_4(),
            ),
        )
        .child(glyph_element)
        .child(
          div()
            .flex_1()
            .text_sm()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(if is_active_project {
              theme.text
            } else {
              theme.text_muted
            })
            .overflow_hidden()
            .child(entry.info.name.clone()),
        );

      let ctx_root = menu_root.clone();
      let ctx_name = entry.info.name.clone();
      tree = tree.child(project_row.context_menu(move |menu, window, cx| {
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
      }));

      if !expanded {
        tree = tree.child(div().h(px(4.0)));
        continue;
      }

      for (ix, row) in worktree_list.clone().into_iter().enumerate() {
        let wt = row.entry.clone();
        let branch_label = row
          .display_name
          .clone()
          .unwrap_or_else(|| wt.branch.clone());
        let wt_canonical = wt.path.canonicalize().unwrap_or_else(|_| wt.path.clone());
        let is_active_card = wt_canonical == active_canonical;
        let workspace = self.workspaces.get(&wt_canonical).cloned();
        let icon_color = if is_active_card {
          theme.green
        } else {
          theme.purple
        };

        let any_running = workspace.as_ref().is_some_and(|ws| {
          ws.read(cx).terminals().any(|(_, view)| {
            let view = view.read(cx);
            view.agent_kind() == SessionKind::ClaudeCode && view.status() == AgentStatus::Running
          })
        });
        let branch_icon: gpui::AnyElement = if any_running {
          spinner(
            SharedString::from(format!("branch-spin-{project_ix}-{ix}")),
            theme.yellow,
          )
          .into_any_element()
        } else {
          git_branch_icon(icon_color).into_any_element()
        };
        let click_root = wt.path.clone();
        let wt_menu_owner = project_root.clone();
        let wt_menu_path = wt.path.clone();
        let is_primary_wt = wt.is_primary;
        let mut branch_row = div()
          .id(SharedString::from(format!("branch-{project_ix}-{ix}")))
          .flex()
          .items_center()
          .gap_2()
          .px_2()
          .h(px(24.0))
          .rounded_md()
          .cursor_pointer()
          .when(!is_active_card, |el| el.hover(|s| s.bg(theme.hover)))
          .on_click(cx.listener(move |_, _, _, cx| {
            cx.emit(ProjectPanelEvent::OpenProject(click_root.clone()));
          }))
          .child(branch_icon)
          .child(
            div()
              .flex_1()
              .text_size(px(13.0))
              .text_color(if is_active_card {
                theme.text
              } else {
                theme.text_muted
              })
              .overflow_hidden()
              .child(branch_label.clone()),
          );
        if wt.is_primary {
          branch_row = branch_row.child(
            div()
              .px_1()
              .rounded_sm()
              .border_1()
              .border_color(theme.panel_border)
              .text_xs()
              .text_color(theme.text_dim)
              .child("primary"),
          );
        }
        if let Some(issue) = &row.issue {
          branch_row = branch_row.child(
            div()
              .flex()
              .items_center()
              .gap_0p5()
              .px_1()
              .rounded_sm()
              .border_1()
              .border_color(theme.panel_border)
              .text_xs()
              .text_color(theme.text_dim)
              .child(
                div()
                  .flex_none()
                  .child(Icon::new(IconName::CircleCheck).size_3()),
              )
              .child(short_ref(issue)),
          );
        }
        if let Some(pr) = &row.pr {
          branch_row = branch_row.child(
            div()
              .flex()
              .items_center()
              .gap_0p5()
              .px_1()
              .rounded_sm()
              .border_1()
              .border_color(theme.panel_border)
              .text_xs()
              .text_color(theme.text_dim)
              .child(
                div()
                  .flex_none()
                  .child(Icon::new(IconName::GitHub).size_3()),
              )
              .child(short_ref(pr)),
          );
        }

        let branch_element: gpui::AnyElement = if is_primary_wt {
          branch_row.into_any_element()
        } else {
          let menu_owner = wt_menu_owner.clone();
          let menu_path = wt_menu_path.clone();
          let menu_label = branch_label.clone();
          branch_row
            .context_menu(move |menu, window, cx| {
              let owner = menu_owner.clone();
              let path = menu_path.clone();
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

        let mut card = div()
          .id(SharedString::from(format!("worktree-{project_ix}-{ix}")))
          .flex()
          .flex_col()
          .ml(px(22.0))
          .mr_2()
          .py_0p5()
          .rounded_lg()
          .border_1()
          .when(is_active_card, |el| {
            el.border_color(theme.active).bg(theme.elevated)
          })
          .when(!is_active_card, |el| {
            el.border_color(gpui::transparent_black())
          });

        let has_agents = workspace.is_some();
        if has_agents {
          let workspace_entity = workspace.clone().unwrap();
          let workspace_read = workspace_entity.read(cx);
          let agent_rows: Vec<_> = workspace_read
            .terminals()
            .filter(|(_, view)| view.read(cx).agent_kind() == SessionKind::ClaudeCode)
            .map(|(tab_ix, view)| {
              let view = view.read(cx);
              let status = view.status();
              let (status_icon, status_color) = agent_status_visual(status, &theme);
              let loading = matches!(
                status,
                AgentStatus::Running | AgentStatus::Waiting | AgentStatus::Thinking
              );
              let status_element: gpui::AnyElement = if loading {
                spinner(
                  SharedString::from(format!("agent-spin-{project_ix}-{tab_ix}")),
                  status_color,
                )
                .into_any_element()
              } else {
                div()
                  .flex_none()
                  .text_color(status_color)
                  .child(Icon::new(status_icon).size_3())
                  .into_any_element()
              };
              let title = view
                .title
                .trim_start_matches(|c: char| "✳✻✶✽*⁕ ".contains(c))
                .to_string();
              let ago = view.activity_ago();
              let is_tab_active = tab_ix == workspace_read.active && is_active_card;
              let ws = workspace_entity.clone();
              let wt_root = wt.path.clone();
              let switch_first = !is_active_card;
              div()
                .id(SharedString::from(format!("agent-{project_ix}-{tab_ix}")))
                .flex()
                .items_center()
                .gap_1p5()
                .ml(px(22.0))
                .mr_1()
                .px_2()
                .h(px(24.0))
                .rounded_md()
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
                .child(status_element)
                .child(
                  div()
                    .flex_none()
                    .text_color(theme.claude)
                    .child(Icon::new(IconName::Asterisk).size_3()),
                )
                .child(
                  div()
                    .flex_none()
                    .text_size(px(13.0))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(title),
                )
                .child(
                  div()
                    .flex_1()
                    .text_size(px(13.0))
                    .text_color(theme.text_dim)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(format!("- {}", agent_description(status))),
                )
                .child(
                  div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.text_dim)
                    .child(ago),
                )
            })
            .collect();

          card = card.child(branch_element).children(agent_rows).pb_0p5();
        } else {
          card = card.child(branch_element);
        }
        tree = tree.child(card);
      }

      tree = tree.child(div().h(px(8.0)));
    }

    let (terminals, claudes, running) = self
      .workspaces
      .get(&active_root)
      .map(|workspace| {
        let workspace = workspace.read(cx);
        let terminals = workspace.count_of(SessionKind::Terminal, cx);
        let claudes = workspace.count_of(SessionKind::ClaudeCode, cx);
        let running = workspace
          .terminals()
          .filter(|(_, view)| view.read(cx).status() == helix_models::AgentStatus::Running)
          .count();
        (terminals, claudes, running)
      })
      .unwrap_or((0, 0, 0));

    let bottom_bar = div()
      .flex()
      .flex_none()
      .items_center()
      .h(px(36.0))
      .px_2()
      .gap_1()
      .child(
        icon_button("sidebar-settings", IconName::Settings, &theme).on_click(cx.listener(
          |_, _, _, cx| {
            cx.emit(ProjectPanelEvent::OpenSettings(None));
          },
        )),
      )
      .child(icon_button("sidebar-help", IconName::Info, &theme))
      .child(div().flex_1())
      .child(
        div()
          .text_xs()
          .text_color(theme.text_dim)
          .child(format!("{terminals}T · {claudes}C · {running}R")),
      );

    div()
      .relative()
      .flex()
      .flex_col()
      .size_full()
      .bg(theme.panel)
      .border_r_1()
      .border_color(theme.panel_border)
      .child(titlebar_strip)
      .child(search)
      .child(projects_header)
      .child(tree)
      .child(bottom_bar)
  }
}
