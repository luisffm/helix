use crate::components::{
  BODY, GLYPH, HEADER_HEIGHT, MICRO, SMALL, TINY, TITLE, TRAFFIC_LIGHTS, attention_badge,
  claude_icon, icon_button, pill, project_glyph, pulsing_dot,
};
use crate::icons::HelixIcon;
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
use helix_github::{BranchReview, ReviewState};
use helix_models::{AgentAttention, AgentStatus};
use helix_models::{GitSnapshot, ProjectInfo, SessionKind};
use helix_worktree::{WorktreeRow, canonical_path};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const COLLAPSE_MS: u64 = 160;

pub enum ProjectPanelEvent {
  OpenProject(PathBuf),
}

/// Where the pull request stands, as the glyph and colour its branch is drawn
/// in. Shape carries the state as well as colour does — GitHub's own merged and
/// closed marks — so the row still reads when the colours do not.
///
/// Conflicts outrank an open state: a branch that cannot merge is the one worth
/// looking at, and the rest of the app already reports that in yellow. A branch
/// nobody has reviewed keeps the plain branch icon.
fn review_visual(review: Option<&BranchReview>, theme: &Theme) -> (HelixIcon, gpui::Hsla, String) {
  let Some(review) = review else {
    return (
      HelixIcon::GitBranch,
      theme.text_muted,
      "No pull request".to_string(),
    );
  };

  let (icon, color, state) = match review.state {
    ReviewState::Merged => (HelixIcon::GitMerge, theme.purple, "merged"),
    ReviewState::Closed => (HelixIcon::GitPullRequestClosed, theme.red, "closed"),
    ReviewState::Draft => (HelixIcon::GitPullRequest, theme.text_dim, "draft"),
    ReviewState::Open if review.conflicting => {
      (HelixIcon::GitPullRequest, theme.yellow, "conflicts")
    }
    ReviewState::Open => (HelixIcon::GitPullRequest, theme.green, "open"),
  };

  (icon, color, format!("PR #{} · {state}", review.number))
}

/// One agent session as a row reports it.
#[derive(Clone)]
struct AgentLine {
  title: String,
  status: AgentStatus,
  attention: Option<AgentAttention>,
}

impl AgentLine {
  fn working(&self) -> bool {
    matches!(
      self.status,
      AgentStatus::Running | AgentStatus::Waiting | AgentStatus::Thinking
    )
  }
}

#[derive(Clone)]
struct ProjectEntry {
  info: ProjectInfo,
  icon: Option<String>,
  emoji: Option<String>,
  accent: Option<String>,
}

pub struct ProjectPanel {
  projects: Vec<ProjectEntry>,
  active_root: PathBuf,
  active_canonical: PathBuf,
  /// Added and removed lines per worktree, keyed by canonical path. Only the
  /// worktree in front produces a snapshot, so an entry appears the first time
  /// its worktree is opened and keeps reporting after the user moves on. Folded
  /// once per git refresh, never per frame.
  lines: HashMap<PathBuf, (usize, usize)>,
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
      icon: p.icon.clone(),
      emoji: p.emoji.clone(),
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
      lines: HashMap::new(),
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

  pub fn set_git(&mut self, root: &Path, git: Option<GitSnapshot>, cx: &mut Context<Self>) {
    let root = canonical_path(root);
    let stats = git
      .as_ref()
      .map(GitSnapshot::line_stats)
      .filter(|(added, removed)| *added > 0 || *removed > 0);

    // A worktree that went clean drops its entry rather than reporting the
    // numbers it had before the commit landed.
    let changed = match stats {
      Some(stats) => self.lines.insert(root, stats) != Some(stats),
      None => self.lines.remove(&root).is_some(),
    };

    if changed {
      cx.notify();
    }
  }

  pub fn set_worktrees(
    &mut self,
    worktrees: HashMap<PathBuf, Vec<WorktreeRow>>,
    every_project: bool,
    cx: &mut Context<Self>,
  ) {
    if every_project {
      self.worktrees = worktrees;

      // A full listing is the only place that knows which worktrees still
      // exist, so it is where the line cache is pruned.
      let listed: HashSet<PathBuf> = self
        .worktrees
        .values()
        .flatten()
        .map(|row| row.canonical.clone())
        .collect();

      self.lines.retain(|root, _| listed.contains(root));
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
  fn agent_summary(&self, canonical: &PathBuf, cx: &App) -> Option<AgentLine> {
    let workspace = self.workspaces.get(canonical)?;

    let sessions: Vec<AgentLine> = workspace
      .read(cx)
      .terminals()
      .filter(|(_, view)| view.read(cx).agent_kind() == SessionKind::ClaudeCode)
      .map(|(_, view)| {
        let view = view.read(cx);

        AgentLine {
          title: helix_agents::strip_spinner(&view.title).to_string(),
          status: view.status(),
          attention: view.attention(),
        }
      })
      .collect();

    // A session holding for an answer is the one worth surfacing, then one that
    // is working, then whatever there is.
    sessions
      .iter()
      .find(|line| line.attention == Some(AgentAttention::Answer))
      .or_else(|| sessions.iter().find(|line| line.attention.is_some()))
      .or_else(|| sessions.iter().find(|line| line.working()))
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

    if let Some(agent) = self.agent_summary(&row.canonical, cx) {
      let working = agent.working();

      // Sits with the sunburst it belongs to rather than drifting to the far
      // edge, where it read as the row's status instead of the session's. What
      // the session wants outranks what it is doing.
      let glyph: gpui::AnyElement = if let Some(kind) = agent.attention {
        attention_badge(kind, theme).into_any_element()
      } else if working {
        pulsing_dot(
          SharedString::from(format!("agent-dot-{project_ix}-{ix}")),
          theme.claude,
        )
        .into_any_element()
      } else if agent.status == AgentStatus::Error {
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
          .gap(px(5.0))
          .child(claude_icon(theme.claude, 11.0))
          .child(glyph)
          .child(
            div()
              .flex_1()
              .min_w_0()
              .text_color(if working || agent.attention.is_some() {
                theme.text_muted
              } else {
                theme.text_dim
              })
              .overflow_hidden()
              .whitespace_nowrap()
              .text_ellipsis()
              .child(agent.title),
          )
          .into_any_element(),
      );
    }

    // Named rather than bare: a lone `#214` beside a branch says nothing about
    // what it points at, and an issue is not a pull request.
    let reference = review
      .map(|review| format!("PR #{}", review.number))
      .or_else(|| {
        row
          .pr
          .as_deref()
          .map(|pr| format!("PR {}", helix_github::short_ref(pr)))
      })
      .or_else(|| {
        row
          .issue
          .as_deref()
          .map(|issue| format!("Issue {}", helix_github::short_ref(issue)))
      })?;

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

  /// The diffstat rides the branch line so it never competes with the pull
  /// request or the agent for the row's second line.
  fn worktree_diffstat(&self, row: &WorktreeRow, theme: &Theme) -> Option<gpui::Div> {
    let (added, removed) = self.lines.get(&row.canonical).copied()?;

    let mono = |text: String, color: gpui::Hsla| {
      div()
        .flex_none()
        .font_family(theme.font_mono.clone())
        .text_size(px(TINY))
        .text_color(color)
        .child(text)
    };

    // A zero side is dropped rather than printed: at this size the colour reads
    // before the digit, so a red `-0` says "deletions" for a moment before the
    // eye gets to the number.
    Some(
      div()
        .flex_none()
        .flex()
        .gap(px(5.0))
        .children((added > 0).then(|| mono(format!("+{added}"), theme.green)))
        .children((removed > 0).then(|| mono(format!("\u{2212}{removed}"), theme.red))),
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
    let (state_icon, state_color, state_label) = review_visual(review, theme);
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
              .id(SharedString::from(format!(
                "branch-state-{project_ix}-{ix}"
              )))
              .flex_none()
              .text_color(state_color)
              .tooltip(move |window, cx| {
                gpui_component::tooltip::Tooltip::new(state_label.clone()).build(window, cx)
              })
              .child(Icon::new(state_icon).size(px(GLYPH))),
          )
          .child(
            // Shrinks rather than filling, so the tag that follows sits against
            // the name instead of being pushed to the far edge.
            div()
              .flex_shrink()
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
          )
          .children(wt.is_primary.then(|| {
            pill("primary", theme.text_muted, theme.active, MICRO)
              .border_1()
              .border_color(theme.panel_border)
          }))
          .child(div().flex_1())
          .children(self.worktree_diffstat(row, theme)),
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
      .pl(px(TRAFFIC_LIGHTS))
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
              .child(Icon::new(IconName::Search).size(px(GLYPH))),
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
        .h(px(32.0))
        // Proximity is the cheapest hierarchy there is: a project pulls away from
        // the branches of the one above it.
        .when(project_ix > 0, |el| el.mt(px(10.0)))
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
        .child(project_glyph(
          entry.icon.as_deref(),
          entry.emoji.as_deref(),
          entry.accent.as_deref(),
          &theme,
        ))
        .child(
          div()
            .flex_1()
            .min_w_0()
            // Always the larger, heavier of the two levels: size says what kind of
            // row this is, colour says whether it is the one being worked in.
            .text_size(px(TITLE))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .when(is_active_project, |el| el.text_color(theme.text))
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
