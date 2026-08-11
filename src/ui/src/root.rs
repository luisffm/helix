use crate::add_dialog::{AddDialog, AddDialogEvent};
use crate::components::HEADER_HEIGHT;
use crate::search::{SearchDialog, SearchEvent, SearchItem, SearchTarget};
use crate::settings_page::{Section, SettingsEvent, SettingsPage};
use crate::sidebar_left::{ProjectPanel, ProjectPanelEvent};
use crate::sidebar_right::{ContextPanel, ContextPanelEvent};
use crate::theme::Theme;
use crate::workspace::Workspace;
use crate::worktree_dialog::{WorktreeEditDialog, WorktreeEditEvent};
use gpui::{
  Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, Window, div,
  prelude::*, px,
};
use helix_commands::{
  ActivateTab, ActivateWorkspace, CloseActiveTab, CopyPathAction, DeleteWorktreeAction,
  EditWorktreeAction, NewClaudeSession, NewTerminal, NextTab, OpenAppSettings, OpenInFinderAction,
  OpenInZedAction, OpenProjectSettingsAction, OpenSearch, PrevTab, RemoveProjectAction,
  RemoveWorktreeAction, ToggleLeftSidebar, ToggleRightSidebar,
};
use helix_filesystem::FsWatcher;
use helix_models::{ProjectInfo, SessionKind};
use helix_process::usage::{UsageSnapshot, UsageTargets, format_rss};
use helix_worktree::{WorktreeRow, canonical_path, rows_for_projects};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const RETAINED_WORKSPACES: usize = 4;
const REVIEW_MIN_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq)]
enum ResizingSide {
  Left,
  Right,
}

pub struct HelixRoot {
  project: ProjectInfo,
  left_open: bool,
  right_open: bool,
  left_width: f32,
  right_width: f32,
  left_anim: f32,
  right_anim: f32,
  left_animating: bool,
  right_animating: bool,
  resizing: Option<ResizingSide>,
  git: Option<helix_models::GitSnapshot>,
  worktrees: Vec<WorktreeRow>,
  workspace: Entity<Workspace>,
  workspaces: HashMap<PathBuf, Entity<Workspace>>,
  recent_workspaces: Vec<PathBuf>,
  project_panel: Entity<ProjectPanel>,
  context_panel: Entity<ContextPanel>,
  search: Option<Entity<SearchDialog>>,
  add_dialog: Option<Entity<AddDialog>>,
  settings: Option<Entity<SettingsPage>>,
  worktree_edit: Option<Entity<WorktreeEditDialog>>,
  resources: UsageSnapshot,
  resources_history: HashMap<PathBuf, Vec<f32>>,
  resources_open: bool,
  reviews_busy: bool,
  reviews_at: HashMap<PathBuf, Instant>,
  resources_expanded: std::collections::HashSet<PathBuf>,
  focus_handle: FocusHandle,
  _watcher: Option<FsWatcher>,
}

impl HelixRoot {
  pub fn new(project: ProjectInfo, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let workspace = cx.new(|cx| Workspace::new(project.root.clone(), window, cx));
    let project_panel = cx.new(|cx| ProjectPanel::new(project.clone(), cx));
    let context_panel = cx.new(|cx| ContextPanel::new(project.root.clone(), window, cx));

    cx.subscribe_in(&context_panel, window, |this, _, event, window, cx| {
      match event {
        ContextPanelEvent::OpenFile { path, preview } => {
          let path = path.clone();
          let preview = *preview;

          this.workspace.update(cx, |workspace, cx| {
            workspace.open_file(path, preview, window, cx);
          });
        }
        ContextPanelEvent::OpenDiff { relative, base } => {
          let root = this.project.root.clone();
          let relative = relative.clone();
          let base = base.clone();

          this.workspace.update(cx, |workspace, cx| {
            workspace.open_diff(root, relative, base, window, cx);
          });
        }
        ContextPanelEvent::GitChanged => {
          this.refresh_git(cx);
        }
      }

      cx.notify();
    })
    .detach();

    cx.subscribe_in(
      &project_panel,
      window,
      |this, _, event, window, cx| match event {
        ProjectPanelEvent::OpenProject(path) => {
          this.switch_project(path.clone(), window, cx);
        }
        ProjectPanelEvent::RequestAddProject => {
          this.pick_workspace(window, cx);
        }
        ProjectPanelEvent::RequestAddWorktree => {
          this.open_add_dialog(false, window, cx);
        }
        ProjectPanelEvent::OpenSettings(target) => {
          this.open_settings(target.clone(), window, cx);
        }
      },
    )
    .detach();

    let mut workspaces = HashMap::new();
    workspaces.insert(project.root.clone(), workspace.clone());

    let recent_workspaces = vec![project.root.clone()];

    let mut root = Self {
      project,
      left_open: true,
      left_anim: 1.0,
      right_anim: 0.0,
      left_animating: false,
      right_animating: false,
      right_open: false,
      left_width: 280.0,
      right_width: 320.0,
      resizing: None,
      git: None,
      worktrees: Vec::new(),
      workspace,
      workspaces,
      recent_workspaces,
      project_panel,
      context_panel,
      search: None,
      add_dialog: None,
      settings: None,
      worktree_edit: None,
      resources: UsageSnapshot::default(),
      resources_history: HashMap::new(),
      resources_open: false,
      reviews_busy: false,
      reviews_at: HashMap::new(),
      resources_expanded: std::collections::HashSet::new(),
      focus_handle: cx.focus_handle(),
      _watcher: None,
    };

    let workspaces = root.workspaces.clone();

    root.project_panel.update(cx, |panel, cx| {
      panel.set_workspaces(workspaces, cx);
    });

    root.start_watcher(window, cx);
    root.refresh_git(cx);
    root.start_resource_monitor(cx);
    root.detect_terminal_font(cx);

    root
  }

  /// Probing for a terminal font reads several config files and spawns
  /// `defaults`, which is not worth delaying the first frame for.
  fn detect_terminal_font(&mut self, cx: &mut Context<Self>) {
    let installed = cx.text_system().all_font_names();

    cx.spawn(async move |this, cx| {
      let detected = cx
        .background_executor()
        .spawn(async move { helix_state::terminal_font::detect(&installed) })
        .await;

      let Some(mono) = detected else { return };

      this
        .update(cx, |_, cx| {
          if cx.global::<Theme>().font_mono.as_ref() == mono.as_str() {
            return;
          }

          cx.global_mut::<Theme>().font_mono = mono.into();

          crate::theme::sync_component_theme(cx);
          cx.refresh_windows();
        })
        .ok();
    })
    .detach();
  }

  fn start_resource_monitor(&mut self, cx: &mut Context<Self>) {
    cx.spawn(async move |this, cx| {
      let mut summary = String::new();
      let mut tick: u32 = 0;

      loop {
        cx.background_executor()
          .timer(std::time::Duration::from_millis(2500))
          .await;

        tick = tick.wrapping_add(1);

        let Ok(targets) = this.update(cx, |root, cx| {
          (root.resources_open || tick % 4 == 0).then(|| root.usage_targets(cx))
        }) else {
          break;
        };

        let Some(targets) = targets else {
          continue;
        };

        let snapshot = cx
          .background_executor()
          .spawn(async move { helix_process::usage::sample(targets) })
          .await;

        if this
          .update(cx, |root, cx| {
            for project in &snapshot.projects {
              let history = root
                .resources_history
                .entry(project.root.clone())
                .or_default();

              history.push(project.rss_mb);

              if history.len() > 40 {
                history.remove(0);
              }
            }

            let next = helix_process::usage::status_summary(&snapshot);
            let changed = if root.resources_open {
              snapshot != root.resources
            } else {
              next != summary
            };

            summary = next;
            root.resources = snapshot;

            if changed {
              cx.notify();
            }
          })
          .is_err()
        {
          break;
        }
      }
    })
    .detach();
  }

  fn usage_targets(&self, cx: &gpui::App) -> UsageTargets {
    self
      .workspaces
      .iter()
      .map(|(root, workspace)| {
        let name = helix_state::config::project_for(root)
          .map(|project| project.label())
          .unwrap_or_else(|| helix_state::config::dir_label(root));

        let sessions = workspace
          .read(cx)
          .terminals()
          .filter_map(|(_, view)| {
            let view = view.read(cx);

            view.shell_pid().map(|pid| {
              (
                helix_agents::strip_spinner(&view.title).to_string(),
                view.agent_kind(),
                pid,
              )
            })
          })
          .collect();

        (name, root.clone(), sessions)
      })
      .collect()
  }

  fn start_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<PathBuf>>();

    self._watcher = helix_filesystem::watch(&self.project.root, tx).ok();

    let this = cx.entity().downgrade();

    window
      .spawn(cx, async move |cx| {
        while let Some(batch) = rx.recv().await {
          let updated = this.update_in(cx, |root, window, cx| {
            root.refresh_git_for(Some(batch.clone()), cx);

            root.workspace.update(cx, |workspace, cx| {
              workspace.refresh_open_files(&batch, window, cx);
            });
          });

          if updated.is_err() {
            break;
          }
        }
      })
      .detach();
  }

  fn switch_project(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
    let Ok((project, _worktree)) = helix_worktree::open_project(&path) else {
      return;
    };

    if project.root == self.project.root {
      return;
    }

    self.project = project.clone();
    self.git = None;
    self.worktrees = Vec::new();

    self.start_watcher(window, cx);

    let left_open = self.left_open;
    let right_open = self.right_open;
    let workspace = match self.workspaces.get(&project.root) {
      Some(existing) => existing.clone(),
      None => {
        let created = cx.new(|cx| Workspace::new(project.root.clone(), window, cx));

        self
          .workspaces
          .insert(project.root.clone(), created.clone());

        created
      }
    };

    workspace.update(cx, |workspace, cx| {
      workspace.left_sidebar_open = left_open;
      workspace.right_sidebar_open = right_open;

      cx.notify();
    });

    self.workspace = workspace.clone();

    self.context_panel.update(cx, |panel, cx| {
      panel.set_root(project.root.clone(), cx);
    });

    self.touch_workspace(project.root.clone());
    self.evict_idle_workspaces(cx);

    let workspaces = self.workspaces.clone();

    self.project_panel.update(cx, |panel, cx| {
      panel.set_workspaces(workspaces, cx);
      panel.set_active_project(project.clone(), cx);
    });

    window.set_window_title(&format!("Helix — {}", project.name));

    self.refresh_git(cx);

    cx.notify();
  }

  fn touch_workspace(&mut self, root: PathBuf) {
    self.recent_workspaces.retain(|path| *path != root);
    self.recent_workspaces.insert(0, root);
  }

  /// Every project ever visited kept its terminals, editors and diffs alive.
  /// The recently used ones stay; the rest are dropped, but only once nothing
  /// in them is still working.
  fn evict_idle_workspaces(&mut self, cx: &mut Context<Self>) {
    if self.workspaces.len() <= RETAINED_WORKSPACES {
      return;
    }

    let keep: std::collections::HashSet<&PathBuf> = self
      .recent_workspaces
      .iter()
      .take(RETAINED_WORKSPACES)
      .collect();

    let evictable: Vec<PathBuf> = self
      .workspaces
      .iter()
      .filter(|(root, workspace)| !keep.contains(root) && workspace.read(cx).is_idle(cx))
      .map(|(root, _)| root.clone())
      .collect();

    for root in evictable {
      self.workspaces.remove(&root);
      self.recent_workspaces.retain(|path| *path != root);
    }
  }

  /// One listing covers every branch the sidebar draws, so it is tied to
  /// project-level refreshes rather than to watcher batches.
  fn refresh_reviews(&mut self, owner: PathBuf, cx: &mut Context<Self>) {
    let fresh = self
      .reviews_at
      .get(&owner)
      .is_some_and(|at| at.elapsed() < REVIEW_MIN_INTERVAL);

    if self.reviews_busy || fresh {
      return;
    }

    self.reviews_busy = true;
    self.reviews_at.insert(owner.clone(), Instant::now());

    let task = cx
      .background_executor()
      .spawn(async move { (helix_github::review::list_for_repo(&owner, 100), owner) });

    cx.spawn(async move |this, cx| {
      let (listed, owner) = task.await;

      this
        .update(cx, |root_view, cx| {
          root_view.reviews_busy = false;

          let Ok(reviews) = listed else { return };

          let states = helix_github::review::states_by_branch(reviews);

          root_view.project_panel.update(cx, |panel, cx| {
            panel.set_reviews(owner, states, cx);
          });
        })
        .ok();
    })
    .detach();
  }

  fn refresh_git(&mut self, cx: &mut Context<Self>) {
    self.refresh_git_for(None, cx);
  }

  /// A watcher batch only ever touches the active project, so it rebuilds that
  /// project's worktree rows. Describing every configured project means a
  /// canonicalize plus a Repository::open per worktree, which is reserved for
  /// project-level changes.
  fn refresh_git_for(&mut self, changed: Option<Vec<PathBuf>>, cx: &mut Context<Self>) {
    let root = self.project.root.clone();
    let every_project = changed.is_none();

    let task = cx.background_executor().spawn(async move {
      let snapshot = helix_git::snapshot(&root).ok();
      let owner = helix_worktree::primary_root(&root).unwrap_or(root);

      let only = (!every_project).then_some(owner.as_path());
      let worktrees = rows_for_projects(&helix_state::config::load().projects, only);

      (snapshot, worktrees, owner)
    });

    cx.spawn(async move |this, cx| {
      let (snapshot, worktrees, owner) = task.await;

      this
        .update(cx, |root_view, cx| {
          root_view.git = snapshot.clone();
          root_view.worktrees = worktrees.get(&owner).cloned().unwrap_or_default();

          root_view.project_panel.update(cx, |panel, cx| {
            panel.set_git(snapshot.clone(), cx);
            panel.set_worktrees(worktrees, every_project, cx);
          });

          root_view.context_panel.update(cx, |panel, cx| {
            panel.set_git(snapshot, cx);
            panel.refresh_files(changed.as_deref(), cx);
          });

          if every_project {
            root_view.refresh_reviews(owner, cx);
          }

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn open_worktree_edit(
    &mut self,
    owner: PathBuf,
    path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let meta = helix_state::config::worktree_config_for(&owner, &path);

    let branch = helix_worktree::describe_worktree(&path)
      .map(|entry| entry.branch)
      .unwrap_or_else(|| {
        path
          .file_name()
          .map(|n| n.to_string_lossy().to_string())
          .unwrap_or_default()
      });

    let (display_name, issue, pr) = meta
      .map(|m| {
        (
          m.display_name.unwrap_or_default(),
          m.issue.unwrap_or_default(),
          m.pr.unwrap_or_default(),
        )
      })
      .unwrap_or_default();

    let dialog = cx.new(|cx| WorktreeEditDialog::new(branch, display_name, issue, pr, window, cx));

    cx.subscribe_in(
      &dialog,
      window,
      move |this, _, event, _window, cx| match event {
        WorktreeEditEvent::Close => {
          this.worktree_edit = None;

          cx.notify();
        }
        WorktreeEditEvent::Save {
          display_name,
          issue,
          pr,
        } => {
          helix_state::config::set_worktree_meta(
            &owner,
            &path,
            Some(display_name.clone()),
            Some(issue.clone()),
            Some(pr.clone()),
          );

          this.worktree_edit = None;

          this.refresh_git(cx);
          cx.notify();
        }
      },
    )
    .detach();

    self.worktree_edit = Some(dialog);

    cx.notify();
  }

  fn delete_worktree(
    &mut self,
    owner: PathBuf,
    path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if path == self.project.root {
      self.switch_project(owner.clone(), window, cx);
    }

    let task = cx.background_executor().spawn({
      let owner = owner.clone();
      let path = path.clone();

      async move {
        let result = helix_worktree::delete_worktree(&owner, &path);

        helix_state::config::remove_worktree(&owner, &path);

        result
      }
    });

    cx.spawn(async move |this, cx| {
      if let Err(err) = task.await {
        eprintln!("helix: delete worktree failed: {err}");
      }

      this
        .update(cx, |root, cx| {
          root.refresh_git(cx);
        })
        .ok();
    })
    .detach();
  }

  fn close_session(
    &mut self,
    root: PathBuf,
    pid: u32,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(workspace) = self.workspaces.get(&root).cloned() else {
      return;
    };

    let terminals: Vec<_> = workspace
      .read(cx)
      .terminals()
      .map(|(ix, view)| (ix, view.clone()))
      .collect();

    let Some(ix) = terminals
      .into_iter()
      .find_map(|(ix, view)| (view.read(cx).shell_pid() == Some(pid)).then_some(ix))
    else {
      return;
    };

    workspace.update(cx, |workspace, cx| {
      workspace.close_tab(ix, window, cx);
    });

    for project in &mut self.resources.projects {
      if project.root == root {
        project.sessions.retain(|session| session.pid != pid);
      }
    }

    cx.notify();
  }

  fn render_resource_panel(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
    let theme = Theme::of(cx).clone();
    let snapshot = &self.resources;

    let row_text = |value: String, color: gpui::Hsla, width: f32| {
      div()
        .w(px(width))
        .flex_none()
        .text_xs()
        .text_color(color)
        .flex()
        .justify_end()
        .child(value)
    };

    let mut list = div()
      .id("resource-list")
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .flex()
      .flex_col()
      .py_1();

    for project in &snapshot.projects {
      let expanded = self.resources_expanded.contains(&project.root);
      let toggle_root = project.root.clone();
      let history: &[f32] = self
        .resources_history
        .get(&project.root)
        .map(Vec::as_slice)
        .unwrap_or_default();
      list = list.child(
        div()
          .id(gpui::SharedString::from(format!(
            "res-{}",
            project.root.display()
          )))
          .flex()
          .items_center()
          .gap_2()
          .mx_2()
          .px_2()
          .h(px(28.0))
          .rounded_md()
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover))
          .on_click(cx.listener(move |this, _, _, cx| {
            if !this.resources_expanded.insert(toggle_root.clone()) {
              this.resources_expanded.remove(&toggle_root);
            }
            cx.notify();
          }))
          .child(
            div().flex_none().text_color(theme.text_dim).child(
              gpui_component::Icon::new(if expanded {
                gpui_component::IconName::ChevronDown
              } else {
                gpui_component::IconName::ChevronRight
              })
              .size_3(),
            ),
          )
          .child(
            div()
              .flex_1()
              .text_sm()
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .text_color(theme.text)
              .overflow_hidden()
              .whitespace_nowrap()
              .child(project.name.clone()),
          )
          .child(crate::components::sparkline(history, theme.text_dim))
          .child(row_text(
            format!("{:.1}%", project.cpu),
            theme.text_muted,
            48.0,
          ))
          .child(row_text(format_rss(project.rss_mb), theme.text, 64.0)),
      );
      if expanded {
        for (ix, session) in project.sessions.iter().enumerate() {
          let color = match session.kind {
            helix_models::SessionKind::ClaudeCode => theme.claude,
            helix_models::SessionKind::Terminal => theme.green,
          };
          let close_root = project.root.clone();
          let close_pid = session.pid;
          let close_title = session.title.clone();
          list = list.child(
            div()
              .id(gpui::SharedString::from(format!(
                "res-s-{}-{ix}",
                project.root.display()
              )))
              .flex()
              .items_center()
              .gap_2()
              .ml(px(28.0))
              .mr_2()
              .px_2()
              .h(px(24.0))
              .rounded_md()
              .hover(|s| s.bg(theme.hover))
              .child(crate::components::status_dot(color))
              .child(
                div()
                  .flex_1()
                  .text_xs()
                  .text_color(theme.text_muted)
                  .overflow_hidden()
                  .whitespace_nowrap()
                  .child(session.title.clone()),
              )
              .child(row_text(
                format!("{:.1}%", session.cpu),
                theme.text_dim,
                48.0,
              ))
              .child(row_text(format_rss(session.rss_mb), theme.text_muted, 64.0))
              .child(
                div()
                  .id(gpui::SharedString::from(format!(
                    "res-close-{}-{ix}",
                    project.root.display()
                  )))
                  .size(px(18.0))
                  .flex()
                  .flex_none()
                  .items_center()
                  .justify_center()
                  .rounded_sm()
                  .cursor_pointer()
                  .text_color(theme.text_dim)
                  .hover(|s| s.bg(theme.elevated).text_color(theme.text))
                  .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(format!("Close {close_title}"))
                      .build(window, cx)
                  })
                  .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_session(close_root.clone(), close_pid, window, cx);
                  }))
                  .child(gpui_component::Icon::new(gpui_component::IconName::Close).size_3()),
              ),
          );
        }
      }
    }

    list = list.child(
      div()
        .flex()
        .items_center()
        .gap_2()
        .mx_2()
        .px_2()
        .h(px(28.0))
        .child(div().w(px(12.0)).flex_none())
        .child(
          div()
            .flex_1()
            .text_sm()
            .text_color(theme.text_muted)
            .child("Helix"),
        )
        .child(row_text(
          format!("{:.1}%", snapshot.app_cpu),
          theme.text_muted,
          48.0,
        ))
        .child(row_text(format_rss(snapshot.app_rss_mb), theme.text, 64.0)),
    );

    div()
      .id("resource-panel")
      .occlude()
      .absolute()
      .right(px(12.0))
      .bottom(px(32.0))
      .w(px(380.0))
      .max_h(px(420.0))
      .rounded_xl()
      .border_1()
      .border_color(theme.panel_border)
      .bg(crate::theme::ca(0x161616f8))
      .shadow_lg()
      .flex()
      .flex_col()
      .overflow_hidden()
      .on_mouse_down_out(cx.listener(|this, _, _, cx| {
        this.resources_open = false;
        cx.notify();
      }))
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .px_3()
          .pt_3()
          .pb_2()
          .border_b_1()
          .border_color(theme.panel_border)
          .child(
            div()
              .flex_none()
              .text_color(theme.text_muted)
              .child(gpui_component::Icon::new(gpui_component::IconName::ChartPie).size_3p5()),
          )
          .child(
            div()
              .flex_1()
              .text_sm()
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .text_color(theme.text)
              .child("Resource Manager"),
          )
          .child(div().text_xs().text_color(theme.text_dim).child(format!(
            "{:.1}% · {} Σ RSS",
            snapshot.total_cpu,
            format_rss(snapshot.total_rss_mb)
          ))),
      )
      .child(
        div()
          .flex()
          .items_center()
          .gap_2()
          .mx_2()
          .mt_1()
          .px_2()
          .h(px(20.0))
          .child(
            div()
              .flex_1()
              .text_xs()
              .text_color(theme.text_dim)
              .child("Name"),
          )
          .child(row_text("CPU".to_string(), theme.text_dim, 48.0))
          .child(row_text("RSS".to_string(), theme.text_dim, 64.0)),
      )
      .child(list)
      .into_any_element()
  }

  fn kick_animation(&mut self, side: ResizingSide, cx: &mut Context<Self>) {
    let already = match side {
      ResizingSide::Left => std::mem::replace(&mut self.left_animating, true),
      ResizingSide::Right => std::mem::replace(&mut self.right_animating, true),
    };
    if already {
      return;
    }

    cx.spawn(async move |this, cx| {
      loop {
        cx.background_executor()
          .timer(std::time::Duration::from_millis(14))
          .await;

        let done = this.update(cx, |root, cx| {
          let (anim, target) = match side {
            ResizingSide::Left => (
              &mut root.left_anim,
              if root.left_open { 1.0f32 } else { 0.0 },
            ),
            ResizingSide::Right => (
              &mut root.right_anim,
              if root.right_open { 1.0f32 } else { 0.0 },
            ),
          };

          let step = 0.09;

          if (*anim - target).abs() <= step {
            *anim = target;
          } else if *anim < target {
            *anim += step;
          } else {
            *anim -= step;
          }

          cx.notify();

          *anim == target
        });

        match done {
          Ok(true) => {
            this
              .update(cx, |root, _| match side {
                ResizingSide::Left => root.left_animating = false,
                ResizingSide::Right => root.right_animating = false,
              })
              .ok();

            break;
          }
          Ok(false) => {}
          Err(_) => break,
        }
      }
    })
    .detach();
  }

  fn open_settings(
    &mut self,
    target: Option<PathBuf>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let (section, root) = match target {
      Some(root) => (Section::Project, root),
      None => (Section::General, self.project.root.clone()),
    };

    let page = cx.new(|cx| SettingsPage::new(section, root, window, cx));

    cx.subscribe_in(&page, window, |this, _, event, window, cx| match event {
      SettingsEvent::Close => {
        this.settings = None;

        cx.notify();
      }
      SettingsEvent::Changed => {
        this.apply_settings_change(window, cx);
      }
    })
    .detach();

    window.focus(&page.read(cx).focus_handle(cx));

    self.settings = Some(page);

    cx.notify();
  }

  fn apply_settings_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let config = helix_state::config::load();

    let level = config
      .blur_level
      .clone()
      .unwrap_or_else(|| "medium".to_string());

    let mut theme = Theme::dark();
    crate::theme::apply_blur_level(&mut theme, &level);

    let fonts = cx.text_system().all_font_names();

    if let Some(mono) = helix_state::terminal_font::detect(&fonts) {
      theme.font_mono = mono.into();
    }

    cx.set_global(theme);
    crate::theme::sync_component_theme(cx);

    window.set_background_appearance(crate::theme::appearance_for_level(&level));
    crate::macos_blur::apply_blur_material();

    let project = self.project.clone();

    self.project_panel.update(cx, |panel, cx| {
      panel.set_active_project(project, cx);
    });

    cx.refresh_windows();
  }

  fn open_add_dialog(
    &mut self,
    include_workspace: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let active_owner = helix_worktree::primary_root(&self.project.root);

    let owner = active_owner
      .clone()
      .unwrap_or_else(|| self.project.root.clone());

    let listed: Vec<PathBuf> = self
      .worktrees
      .iter()
      .map(|wt| wt.entry.path.clone())
      .collect();

    let existing: Vec<(String, PathBuf)> = helix_worktree::list_worktrees(&owner)
      .into_iter()
      .filter(|wt| {
        let canonical = canonical_path(&wt.path);

        !listed.contains(&canonical) && !wt.is_primary
      })
      .map(|wt| (wt.branch, wt.path))
      .collect();

    let mut targets: Vec<(String, PathBuf)> = Vec::new();

    for project in helix_state::config::load().projects {
      let Some(primary) = helix_worktree::primary_root(&project.path) else {
        continue;
      };

      if targets.iter().any(|(_, path)| *path == primary) {
        continue;
      }

      targets.push((project.label(), primary));
    }

    if let Some(active_owner) = &active_owner {
      if targets.iter().all(|(_, path)| path != active_owner) {
        let name = helix_state::config::dir_label(active_owner);

        targets.insert(0, (name, active_owner.clone()));
      }
    }

    let active_target = active_owner
      .as_ref()
      .and_then(|owner| targets.iter().position(|(_, path)| path == owner))
      .unwrap_or(0);

    let can_worktree = !targets.is_empty();

    let dialog = cx.new(|cx| {
      AddDialog::new(
        self.project.root.clone(),
        self.project.name.clone(),
        can_worktree,
        include_workspace,
        existing,
        targets,
        active_target,
        window,
        cx,
      )
    });

    cx.subscribe_in(&dialog, window, |this, _, event, window, cx| match event {
      AddDialogEvent::Dismissed => {
        this.add_dialog = None;

        cx.notify();
      }
      AddDialogEvent::ChooseWorkspace => {
        this.add_dialog = None;

        this.pick_workspace(window, cx);
        cx.notify();
      }
      AddDialogEvent::CreateWorktree {
        owner,
        name,
        source,
      } => {
        this.add_dialog = None;

        this.create_worktree(owner.clone(), name.clone(), source.clone(), window, cx);
        cx.notify();
      }
      AddDialogEvent::AddExistingWorktree(path) => {
        this.add_dialog = None;

        let owner = helix_worktree::primary_root(path).unwrap_or_else(|| path.clone());

        helix_state::config::add_worktree(&owner, path);

        this.switch_project(path.clone(), window, cx);
        this.refresh_git(cx);

        cx.notify();
      }
    })
    .detach();

    window.focus(&dialog.read(cx).focus_handle(cx));

    self.add_dialog = Some(dialog);

    cx.notify();
  }

  fn pick_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
      files: false,
      directories: true,
      multiple: false,
      prompt: Some("Add Workspace".into()),
    });

    cx.spawn_in(window, async move |this, cx| {
      if let Ok(Ok(Some(paths))) = receiver.await {
        if let Some(path) = paths.into_iter().next() {
          helix_state::config::ensure_project(&path);

          this
            .update_in(cx, |root, window, cx| {
              root.switch_project(path, window, cx);
            })
            .ok();
        }
      }
    })
    .detach();
  }

  fn create_worktree(
    &mut self,
    owner: PathBuf,
    name: String,
    source: helix_worktree::BranchSource,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let task = cx.background_executor().spawn(async move {
      helix_worktree::create_worktree(&owner, &name, &source).map(|dest| {
        helix_state::config::add_worktree(&owner, &dest);

        dest
      })
    });

    cx.spawn_in(window, async move |this, cx| match task.await {
      Ok(dest) => {
        this
          .update_in(cx, |root, window, cx| {
            root.switch_project(dest, window, cx);
            root.refresh_git(cx);
          })
          .ok();
      }
      Err(err) => {
        eprintln!("helix: create worktree failed: {err}");

        this
          .update_in(cx, |root, _, cx| {
            root.refresh_git(cx);
          })
          .ok();
      }
    })
    .detach();
  }

  fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let mut items = Vec::new();

    for row in &self.worktrees {
      items.push(SearchItem {
        label: row
          .display_name
          .clone()
          .unwrap_or_else(|| row.entry.branch.clone()),
        detail: row.entry.path.display().to_string(),
        badge: self.project.name.clone(),
        target: SearchTarget::Worktree(row.entry.path.clone()),
      });
    }

    for (ix, tab) in self.workspace.read(cx).tabs.iter().enumerate() {
      items.push(SearchItem {
        label: tab.title(cx),
        detail: tab.detail(cx),
        badge: self.project.name.clone(),
        target: SearchTarget::Tab(ix),
      });
    }

    for project in helix_state::config::visible_projects() {
      items.push(SearchItem {
        label: project.label(),
        detail: project.path.display().to_string(),
        badge: "project".to_string(),
        target: SearchTarget::Project(project.path.clone()),
      });
    }

    items.push(SearchItem {
      label: "New Terminal".to_string(),
      detail: "⌘T".to_string(),
      badge: "action".to_string(),
      target: SearchTarget::NewTerminal,
    });

    items.push(SearchItem {
      label: "New Claude Session".to_string(),
      detail: "⌘⇧T".to_string(),
      badge: "action".to_string(),
      target: SearchTarget::NewClaude,
    });

    let dialog = cx.new(|cx| SearchDialog::new(items, window, cx));

    cx.subscribe_in(&dialog, window, |this, _, event, window, cx| match event {
      SearchEvent::Dismissed => {
        this.search = None;

        cx.notify();
      }
      SearchEvent::Selected(target) => {
        this.search = None;

        this.activate_target(target.clone(), window, cx);
        cx.notify();
      }
    })
    .detach();

    window.focus(&dialog.read(cx).focus_handle(cx));

    self.search = Some(dialog);

    cx.notify();
  }

  fn activate_target(&mut self, target: SearchTarget, window: &mut Window, cx: &mut Context<Self>) {
    match target {
      SearchTarget::Tab(ix) => {
        self.workspace.update(cx, |workspace, cx| {
          workspace.activate(ix, window, cx);
        });
      }
      SearchTarget::Worktree(path) | SearchTarget::Project(path) => {
        self.switch_project(path, window, cx);
      }
      SearchTarget::NewTerminal => {
        self.workspace.update(cx, |workspace, cx| {
          workspace.open_tab(SessionKind::Terminal, window, cx);
        });
      }
      SearchTarget::NewClaude => {
        self.workspace.update(cx, |workspace, cx| {
          workspace.open_tab(SessionKind::ClaudeCode, window, cx);
        });
      }
    }
  }

  fn reclaim_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
    if window.focused(cx).is_some() {
      return;
    }

    self.workspace.update(cx, |workspace, cx| {
      workspace.focus_active(window, cx);
    });

    if window.focused(cx).is_none() {
      window.focus(&self.focus_handle);
    }
  }
}

impl Render for HelixRoot {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    self.reclaim_focus(window, cx);

    let branch = self
      .git
      .as_ref()
      .map(|git| git.branch.clone())
      .unwrap_or_else(|| "no git".to_string());
    let dirty = self.git.as_ref().map(|git| git.dirty_count()).unwrap_or(0);
    let sessions = self.workspace.read(cx).tabs.len();
    let status_bar = div()
      .flex()
      .flex_none()
      .items_center()
      .h(px(26.0))
      .px_3()
      .gap_3()
      .border_t_1()
      .border_color(theme.panel_border)
      .text_xs()
      .text_color(theme.text_dim)
      .child(
        div()
          .flex()
          .items_center()
          .gap_1()
          .child(crate::components::git_branch_icon(theme.text_dim))
          .child(branch),
      )
      .child(if dirty == 0 {
        "clean".to_string()
      } else {
        format!("● {dirty} changes")
      })
      .child(div().flex_1())
      .child(format!(
        "{sessions} session{}",
        if sessions == 1 { "" } else { "s" }
      ))
      .child(
        div()
          .id("resource-manager-toggle")
          .flex()
          .items_center()
          .gap_1()
          .px_1p5()
          .rounded_sm()
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover))
          .when(self.resources_open, |el| el.bg(theme.hover))
          .on_click(cx.listener(|this, _, _, cx| {
            this.resources_open = !this.resources_open;
            cx.notify();
          }))
          .child(
            div()
              .flex_none()
              .child(gpui_component::Icon::new(gpui_component::IconName::ChartPie).size_3()),
          )
          .child(helix_process::usage::status_summary(&self.resources)),
      )
      .child("Helix 0.1");

    let resize_handle = |id: &'static str, side: ResizingSide, cx: &mut Context<Self>| {
      div()
        .id(id)
        .w(px(5.0))
        .h_full()
        .flex_none()
        .flex()
        .flex_col()
        .cursor_col_resize()
        .hover(|s| s.bg(theme.active))
        .when(self.resizing == Some(side), |el| el.bg(theme.active))
        .on_mouse_down(
          gpui::MouseButton::Left,
          cx.listener(move |this, _, _, cx| {
            this.resizing = Some(side);
            cx.notify();
          }),
        )
        .child(
          div()
            .h(px(HEADER_HEIGHT))
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(theme.panel_border),
        )
    };

    let ease = |t: f32| t * t * (3.0 - 2.0 * t);

    let mut body = div().flex().flex_1().min_h_0();
    if self.left_open || self.left_anim > 0.001 {
      body = body.child(
        div()
          .w(px(self.left_width * ease(self.left_anim)))
          .flex_none()
          .h_full()
          .overflow_hidden()
          .child(
            div()
              .w(px(self.left_width))
              .flex_none()
              .h_full()
              .child(self.project_panel.clone()),
          ),
      );
      if self.left_open && !self.left_animating {
        body = body.child(resize_handle("resize-left", ResizingSide::Left, cx));
      }
    }
    let center: gpui::AnyElement = if let Some(settings) = &self.settings {
      settings.clone().into_any_element()
    } else {
      self.workspace.clone().into_any_element()
    };
    body = body.child(
      div()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme.content)
        .child(center),
    );
    if self.right_open || self.right_anim > 0.001 {
      if self.right_open && !self.right_animating {
        body = body.child(resize_handle("resize-right", ResizingSide::Right, cx));
      }
      body = body.child(
        div()
          .w(px(self.right_width * ease(self.right_anim)))
          .flex_none()
          .h_full()
          .overflow_hidden()
          .child(
            div()
              .w(px(self.right_width))
              .flex_none()
              .h_full()
              .child(self.context_panel.clone()),
          ),
      );
    }

    let mut root = div()
      .key_context("Helix")
      .track_focus(&self.focus_handle)
      .relative()
      .size_full()
      .flex()
      .flex_col()
      .font_family(theme.font_ui.clone())
      .text_color(theme.text)
      .on_mouse_move(
        cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
          let Some(side) = this.resizing else { return };
          let x = f32::from(event.position.x);
          match side {
            ResizingSide::Left => {
              this.left_width = x.clamp(200.0, 480.0);
            }
            ResizingSide::Right => {
              let total = f32::from(window.viewport_size().width);
              this.right_width = (total - x).clamp(220.0, 560.0);
            }
          }
          cx.notify();
        }),
      )
      .on_mouse_up(
        gpui::MouseButton::Left,
        cx.listener(|this, _, _, cx| {
          if this.resizing.take().is_some() {
            cx.notify();
          }
        }),
      )
      .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
        this.workspace.update(cx, |workspace, cx| {
          workspace.open_tab(SessionKind::Terminal, window, cx);
        });
      }))
      .on_action(cx.listener(|this, _: &NewClaudeSession, window, cx| {
        this.workspace.update(cx, |workspace, cx| {
          workspace.open_tab(SessionKind::ClaudeCode, window, cx);
        });
      }))
      .on_action(cx.listener(|this, _: &CloseActiveTab, window, cx| {
        this.workspace.update(cx, |workspace, cx| {
          workspace.close_active(window, cx);
        });
      }))
      .on_action(cx.listener(|this, action: &ActivateTab, window, cx| {
        this.workspace.update(cx, |workspace, cx| {
          workspace.activate(action.index, window, cx);
        });
      }))
      .on_action(cx.listener(|this, action: &ActivateWorkspace, window, cx| {
        let target = helix_state::config::visible_projects()
          .into_iter()
          .nth(action.index)
          .map(|project| project.path);
        if let Some(target) = target {
          this.switch_project(target, window, cx);
        }
      }))
      .on_action(cx.listener(|this, _: &NextTab, window, cx| {
        this.workspace.update(cx, |workspace, cx| {
          workspace.activate_next(window, cx);
        });
      }))
      .on_action(cx.listener(|this, _: &PrevTab, window, cx| {
        this.workspace.update(cx, |workspace, cx| {
          workspace.activate_prev(window, cx);
        });
      }))
      .on_action(cx.listener(|this, _: &ToggleLeftSidebar, _, cx| {
        this.left_open = !this.left_open;
        let open = this.left_open;
        this.workspace.update(cx, |workspace, cx| {
          workspace.left_sidebar_open = open;
          cx.notify();
        });
        this.kick_animation(ResizingSide::Left, cx);
        cx.notify();
      }))
      .on_action(cx.listener(|this, _: &ToggleRightSidebar, _, cx| {
        this.right_open = !this.right_open;
        let open = this.right_open;
        this.workspace.update(cx, |workspace, cx| {
          workspace.right_sidebar_open = open;
          cx.notify();
        });
        this.kick_animation(ResizingSide::Right, cx);
        cx.notify();
      }))
      .on_action(cx.listener(|this, _: &OpenSearch, window, cx| {
        if this.search.is_none() {
          this.open_search(window, cx);
        }
      }))
      .on_action(cx.listener(|this, _: &OpenAppSettings, window, cx| {
        if this.settings.is_none() {
          this.open_settings(None, window, cx);
        }
      }))
      .on_action(
        cx.listener(|this, action: &OpenProjectSettingsAction, window, cx| {
          this.open_settings(Some(action.root.clone()), window, cx);
        }),
      )
      .on_action(
        cx.listener(|this, action: &RemoveProjectAction, window, cx| {
          helix_state::config::remove_project(&action.root);
          if action.root == this.project.root {
            if let Some(next) = helix_state::config::load()
              .projects
              .first()
              .map(|p| p.path.clone())
            {
              this.switch_project(next, window, cx);
            }
          }
          let project = this.project.clone();
          this.project_panel.update(cx, |panel, cx| {
            panel.set_active_project(project, cx);
          });
          this.refresh_git(cx);
        }),
      )
      .on_action(
        cx.listener(|this, action: &EditWorktreeAction, window, cx| {
          this.open_worktree_edit(action.owner.clone(), action.path.clone(), window, cx);
        }),
      )
      .on_action(
        cx.listener(|this, action: &RemoveWorktreeAction, window, cx| {
          helix_state::config::remove_worktree(&action.owner, &action.path);
          if action.path == this.project.root {
            this.switch_project(action.owner.clone(), window, cx);
          }
          this.refresh_git(cx);
        }),
      )
      .on_action(
        cx.listener(|this, action: &DeleteWorktreeAction, window, cx| {
          this.delete_worktree(action.owner.clone(), action.path.clone(), window, cx);
        }),
      )
      .on_action(cx.listener(|_, action: &OpenInZedAction, _, cx| {
        let path = action.path.clone();

        cx.background_executor()
          .spawn(async move { helix_process::open_with("Zed", &path) })
          .detach();
      }))
      .on_action(cx.listener(|_, action: &OpenInFinderAction, _, cx| {
        let path = action.path.clone();

        cx.background_executor()
          .spawn(async move { helix_process::open_path(&path) })
          .detach();
      }))
      .on_action(cx.listener(|_, action: &CopyPathAction, _, cx| {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
          action.path.display().to_string(),
        ));
      }))
      .child(body)
      .child(status_bar);

    if let Some(dialog) = &self.search {
      root = root.child(
        div()
          .id("search-overlay")
          .occlude()
          .absolute()
          .inset_0()
          .flex()
          .justify_center()
          .items_start()
          .pt(px(110.0))
          .bg(crate::theme::ca(0x00000066))
          .on_click(cx.listener(|this, _, _, cx| {
            this.search = None;
            cx.notify();
          }))
          .child(dialog.clone()),
      );
    }

    if self.resources_open {
      root = root.child(gpui::deferred(self.render_resource_panel(cx)));
    }

    if let Some(dialog) = &self.worktree_edit {
      root = root.child(
        div()
          .id("worktree-edit-overlay")
          .occlude()
          .absolute()
          .inset_0()
          .flex()
          .justify_center()
          .items_start()
          .pt(px(140.0))
          .bg(crate::theme::ca(0x00000066))
          .on_click(cx.listener(|this, _, _, cx| {
            this.worktree_edit = None;
            cx.notify();
          }))
          .child(dialog.clone()),
      );
    }

    if let Some(dialog) = &self.add_dialog {
      root = root.child(
        div()
          .id("add-overlay")
          .occlude()
          .absolute()
          .inset_0()
          .flex()
          .justify_center()
          .items_start()
          .pt(px(150.0))
          .bg(crate::theme::ca(0x00000066))
          .on_click(cx.listener(|this, _, _, cx| {
            this.add_dialog = None;
            cx.notify();
          }))
          .child(dialog.clone()),
      );
    }

    root
  }
}
