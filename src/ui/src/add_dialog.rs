use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
  KeyDownEvent, ParentElement, Render, ScrollStrategy, SharedString, UniformListScrollHandle,
  Window, div, prelude::*, px, uniform_list,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::radio::Radio;
use gpui_component::select::{Select, SelectEvent, SelectState};
use gpui_component::{Icon, IconName, IndexPath, Sizable};
use helix_fuzzy::Ranker;
use helix_worktree::{BranchRef, BranchSource};
use std::path::PathBuf;

const VISIBLE_BRANCHES: usize = 8;
const BRANCH_ROW_HEIGHT: f32 = 26.0;

pub enum AddDialogEvent {
  Dismissed,
  ChooseWorkspace,
  CreateWorktree {
    owner: PathBuf,
    name: String,
    source: BranchSource,
  },
  AddExistingWorktree(PathBuf),
}

#[derive(PartialEq)]
enum Step {
  Choose,
  Name,
  Existing,
}

#[derive(Clone, Copy, PartialEq)]
enum BranchMode {
  New,
  Existing,
}

#[derive(Clone, Copy, PartialEq)]
enum AddOption {
  Workspace,
  NewWorktree,
  ExistingWorktree,
}

pub struct AddDialog {
  step: Step,
  selected: usize,
  existing_selected: usize,
  existing_filter: Entity<InputState>,
  existing_haystacks: Vec<String>,
  existing_matches: Vec<usize>,
  branch_haystacks: Vec<String>,
  branch_matches: Vec<usize>,
  ranker: Ranker,
  worktree_name: Entity<InputState>,
  branch_name: Entity<InputState>,
  ai_context: Entity<InputState>,
  targets: Vec<(String, PathBuf)>,
  target_select: Entity<SelectState<Vec<String>>>,
  branch_mode: BranchMode,
  branches: Vec<BranchRef>,
  branch_filter: Entity<InputState>,
  branch_scroll: UniformListScrollHandle,
  branch_selected: usize,
  loading_branches: bool,
  describing: bool,
  generating_branch: bool,
  error: Option<String>,
  project_root: PathBuf,
  project_name: String,
  can_worktree: bool,
  include_workspace: bool,
  existing: Vec<(String, PathBuf)>,
  focus_handle: FocusHandle,
}

impl EventEmitter<AddDialogEvent> for AddDialog {}

impl Focusable for AddDialog {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl AddDialog {
  pub fn new(
    project_root: PathBuf,
    project_name: String,
    can_worktree: bool,
    include_workspace: bool,
    existing: Vec<(String, PathBuf)>,
    targets: Vec<(String, PathBuf)>,
    active_target: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let titles: Vec<String> = targets.iter().map(|(name, _)| name.clone()).collect();

    let target_select =
      cx.new(|cx| SelectState::new(titles, Some(IndexPath::new(active_target)), window, cx));
    let worktree_name = cx.new(|cx| InputState::new(window, cx).placeholder("payments-refactor"));
    let branch_name = cx.new(|cx| InputState::new(window, cx).placeholder("feat/my-feature"));
    let ai_context =
      cx.new(|cx| InputState::new(window, cx).placeholder("what should this branch do?"));
    let branch_filter = cx.new(|cx| InputState::new(window, cx).placeholder("filter branches..."));
    let existing_filter =
      cx.new(|cx| InputState::new(window, cx).placeholder("filter worktrees..."));

    cx.subscribe(
      &existing_filter,
      |this, _, event: &InputEvent, cx| match event {
        InputEvent::Change => {
          this.refresh_existing_matches(cx);

          cx.notify();
        }
        InputEvent::PressEnter { .. } => {
          if let Some((_, path)) = this.existing_at(this.existing_selected) {
            cx.emit(AddDialogEvent::AddExistingWorktree(path.clone()));
          }
        }
        _ => {}
      },
    )
    .detach();

    for input in [&worktree_name, &branch_name, &branch_filter] {
      cx.subscribe(input, |this, _, event: &InputEvent, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
          this.confirm_worktree(cx);
        }
      })
      .detach();
    }

    cx.subscribe(&branch_filter, |this, _, event: &InputEvent, cx| {
      if matches!(event, InputEvent::Change) {
        this.refresh_branch_matches(cx);

        cx.notify();
      }
    })
    .detach();

    cx.subscribe_in(
      &ai_context,
      window,
      |this, _, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. }) {
          this.generate_branch_name(window, cx);
        }
      },
    )
    .detach();

    cx.subscribe_in(
      &target_select,
      window,
      |this, _, _: &SelectEvent<Vec<String>>, window, cx| {
        this.branches.clear();
        this.branch_haystacks.clear();
        this.branch_matches.clear();
        this.branch_selected = 0;

        this
          .branch_filter
          .update(cx, |state, cx| state.set_value("", window, cx));

        if this.branch_mode == BranchMode::Existing {
          this.load_branches(cx);
        }

        cx.notify();
      },
    )
    .detach();

    Self {
      step: Step::Choose,
      selected: 0,
      existing_selected: 0,
      existing_filter,
      existing_haystacks: existing
        .iter()
        .map(|(branch, path)| format!("{branch} {}", path.display()))
        .collect(),
      existing_matches: (0..existing.len()).collect(),
      branch_haystacks: Vec::new(),
      branch_matches: Vec::new(),
      ranker: Ranker::new(),
      worktree_name,
      branch_name,
      ai_context,
      targets,
      target_select,
      branch_mode: BranchMode::New,
      branches: Vec::new(),
      branch_filter,
      branch_scroll: UniformListScrollHandle::new(),
      branch_selected: 0,
      loading_branches: false,
      describing: false,
      generating_branch: false,
      error: None,
      project_root,
      project_name,
      can_worktree,
      include_workspace,
      existing,
      focus_handle: cx.focus_handle(),
    }
  }

  fn options(&self) -> Vec<AddOption> {
    if self.include_workspace {
      vec![
        AddOption::Workspace,
        AddOption::NewWorktree,
        AddOption::ExistingWorktree,
      ]
    } else {
      vec![AddOption::NewWorktree, AddOption::ExistingWorktree]
    }
  }

  /// Both pickers keep their matches as indices into the list they came from, so
  /// filtering never clones a row and `render` never filters.
  fn refresh_existing_matches(&mut self, cx: &App) {
    let query = self.existing_filter.read(cx).value();

    self.existing_selected = 0;
    self.ranker.set_query(query.trim());

    self.ranker.rank_into(
      self.existing_haystacks.iter().map(String::as_str),
      &mut self.existing_matches,
    );
  }

  fn refresh_branch_matches(&mut self, cx: &App) {
    let query = self.branch_filter.read(cx).value();

    self.branch_selected = 0;
    self.ranker.set_query(query.trim());

    self.ranker.rank_into(
      self.branch_haystacks.iter().map(String::as_str),
      &mut self.branch_matches,
    );
  }

  fn existing_at(&self, position: usize) -> Option<&(String, PathBuf)> {
    self
      .existing_matches
      .get(position)
      .and_then(|ix| self.existing.get(*ix))
  }

  fn confirm_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let options = self.options();

    let Some(option) = options.get(self.selected) else {
      return;
    };

    match option {
      AddOption::Workspace => cx.emit(AddDialogEvent::ChooseWorkspace),
      AddOption::NewWorktree if self.can_worktree => {
        self.step = Step::Name;

        window.focus(&self.worktree_name.read(cx).focus_handle(cx));
        cx.notify();
      }
      AddOption::ExistingWorktree if !self.existing.is_empty() => {
        self.step = Step::Existing;

        window.focus(&self.existing_filter.read(cx).focus_handle(cx));
        cx.notify();
      }
      _ => {}
    }
  }

  fn selected_target(&self, cx: &App) -> Option<PathBuf> {
    let row = self
      .target_select
      .read(cx)
      .selected_index(cx)
      .map(|index| index.row)
      .unwrap_or(0);

    self.targets.get(row).map(|(_, path)| path.clone())
  }

  fn selected_branch(&self, _cx: &App) -> Option<BranchRef> {
    let position = self
      .branch_selected
      .min(self.branch_matches.len().saturating_sub(1));

    self
      .branch_matches
      .get(position)
      .and_then(|ix| self.branches.get(*ix))
      .cloned()
  }

  fn load_branches(&mut self, cx: &mut Context<Self>) {
    let Some(owner) = self.selected_target(cx) else {
      return;
    };

    self.loading_branches = true;

    let task = cx
      .background_executor()
      .spawn(async move { helix_worktree::available_branches(&owner) });

    cx.spawn(async move |this, cx| {
      let branches = task.await;

      this
        .update(cx, |dialog, cx| {
          dialog.branch_haystacks = branches.iter().map(BranchRef::label).collect();
          dialog.branches = branches;
          dialog.loading_branches = false;

          dialog.refresh_branch_matches(cx);

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn render_branch_row(
    &self,
    ix: usize,
    branch: &BranchRef,
    selected: bool,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    div()
      .id(SharedString::from(format!("branch-{ix}")))
      .flex()
      .items_center()
      .gap_2()
      .px_2()
      .h(px(BRANCH_ROW_HEIGHT))
      .rounded_md()
      .border_1()
      .cursor_pointer()
      .when(selected, |el| {
        el.border_color(theme.active).bg(theme.elevated)
      })
      .when(!selected, |el| {
        el.border_color(gpui::transparent_black())
          .hover(|s| s.bg(theme.hover))
      })
      .on_click(cx.listener(move |this, _, _, cx| {
        this.branch_selected = ix;
        this.error = None;

        cx.notify();
      }))
      .child(crate::components::git_branch_icon(theme.purple))
      .child(
        div()
          .flex_1()
          .text_xs()
          .text_color(theme.text)
          .overflow_hidden()
          .whitespace_nowrap()
          .child(branch.name.clone()),
      )
      .children(branch.remote.clone().map(|remote| {
        div()
          .flex_none()
          .text_xs()
          .text_color(theme.text_dim)
          .child(remote)
      }))
      .into_any_element()
  }

  fn set_branch_mode(&mut self, mode: BranchMode, window: &mut Window, cx: &mut Context<Self>) {
    self.branch_mode = mode;
    self.error = None;

    match mode {
      BranchMode::New => window.focus(&self.branch_name.read(cx).focus_handle(cx)),
      BranchMode::Existing => {
        window.focus(&self.branch_filter.read(cx).focus_handle(cx));

        if self.branches.is_empty() && !self.loading_branches {
          self.load_branches(cx);
        }
      }
    }

    cx.notify();
  }

  fn confirm_worktree(&mut self, cx: &mut Context<Self>) {
    let Some(owner) = self.selected_target(cx) else {
      self.error = Some("no git project to create a worktree in".to_string());
      cx.notify();

      return;
    };

    let mut name = self.worktree_name.read(cx).value().trim().to_string();

    let source = match self.branch_mode {
      BranchMode::New => {
        if name.is_empty() {
          self.error = Some("worktree name is required".to_string());
          cx.notify();

          return;
        }

        let branch = self.branch_name.read(cx).value().trim().to_string();
        let effective = if branch.is_empty() { &name } else { &branch };

        if !helix_agents::branch_name::is_valid(effective) {
          self.error = Some(format!("git will not accept the branch name `{effective}`"));
          cx.notify();

          return;
        }

        BranchSource::New((!branch.is_empty()).then_some(branch))
      }
      BranchMode::Existing => {
        let Some(branch) = self.selected_branch(cx) else {
          self.error = Some("pick a branch to check out".to_string());
          cx.notify();

          return;
        };

        if name.is_empty() {
          name = branch.name.clone();
        }

        BranchSource::Existing(branch)
      }
    };

    self.error = None;

    cx.emit(AddDialogEvent::CreateWorktree {
      owner,
      name,
      source,
    });
  }

  fn ask_for_a_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.describing {
      self.generate_branch_name(window, cx);

      return;
    }

    self.describing = true;
    self.error = None;

    window.focus(&self.ai_context.read(cx).focus_handle(cx));
    cx.notify();
  }

  fn generate_branch_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if self.generating_branch {
      return;
    }

    let context = self.ai_context.read(cx).value().trim().to_string();

    if context.is_empty() {
      self.error = Some("describe the work so the model has something to name".to_string());
      cx.notify();

      return;
    }

    self.generating_branch = true;
    self.error = None;

    cx.notify();

    let root = self.project_root.clone();

    let task = cx
      .background_executor()
      .spawn(async move { helix_agents::branch_name::generate(&root, &context, None) });

    let this = cx.entity().downgrade();

    window
      .spawn(cx, async move |cx| {
        let result = task.await;

        this
          .update_in(cx, |dialog, window, cx| {
            dialog.generating_branch = false;

            match result {
              Ok(name) => {
                dialog.describing = false;

                dialog
                  .branch_name
                  .update(cx, |state, cx| state.set_value(name, window, cx));

                window.focus(&dialog.branch_name.read(cx).focus_handle(cx));
              }
              Err(err) => dialog.error = Some(err.to_string()),
            }

            cx.notify();
          })
          .ok();
      })
      .detach();
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    match self.step {
      Step::Choose => match event.keystroke.key.as_str() {
        "escape" => cx.emit(AddDialogEvent::Dismissed),
        "enter" => self.confirm_choice(window, cx),
        "up" => {
          let count = self.options().len();

          self.selected = (self.selected + count - 1) % count;

          cx.notify();
        }
        "down" => {
          let count = self.options().len();

          self.selected = (self.selected + 1) % count;

          cx.notify();
        }
        _ => {}
      },
      Step::Existing => match event.keystroke.key.as_str() {
        "escape" => {
          self.step = Step::Choose;

          self
            .existing_filter
            .update(cx, |state, cx| state.set_value("", window, cx));

          window.focus(&self.focus_handle);
          cx.notify();
        }
        "up" => {
          let count = self.existing_matches.len().max(1);

          self.existing_selected = (self.existing_selected + count - 1) % count;

          cx.notify();
        }
        "down" => {
          let count = self.existing_matches.len().max(1);

          self.existing_selected = (self.existing_selected + 1) % count;

          cx.notify();
        }
        _ => {}
      },
      Step::Name => {
        if event.keystroke.key.as_str() == "escape" {
          self.step = Step::Choose;
          self.error = None;

          window.focus(&self.focus_handle);
          cx.notify();

          return;
        }

        if self.branch_mode != BranchMode::Existing {
          return;
        }

        match event.keystroke.key.as_str() {
          "up" => {
            let count = self.branch_matches.len().max(1);

            self.branch_selected = (self.branch_selected + count - 1) % count;

            self
              .branch_scroll
              .scroll_to_item(self.branch_selected, ScrollStrategy::Top);
            cx.notify();
          }
          "down" => {
            let count = self.branch_matches.len().max(1);

            self.branch_selected = (self.branch_selected + 1) % count;

            self
              .branch_scroll
              .scroll_to_item(self.branch_selected, ScrollStrategy::Bottom);
            cx.notify();
          }
          _ => {}
        }
      }
    }
  }
}

impl Render for AddDialog {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let option = |ix: usize,
                  icon: IconName,
                  title: &'static str,
                  description: String,
                  enabled: bool,
                  selected: bool,
                  cx: &mut Context<Self>| {
      div()
        .id(("add-option", ix))
        .flex()
        .items_center()
        .gap_3()
        .mx_2()
        .px_3()
        .py_2()
        .rounded_lg()
        .border_1()
        .cursor_pointer()
        .when(selected, |el| {
          el.border_color(theme.active).bg(theme.elevated)
        })
        .when(!selected, |el| {
          el.border_color(gpui::transparent_black())
            .hover(|s| s.bg(theme.hover))
        })
        .when(!enabled, |el| el.opacity(0.4))
        .on_click(cx.listener(move |this, _, window, cx| {
          this.selected = ix;
          this.confirm_choice(window, cx);
        }))
        .child(
          div()
            .flex_none()
            .text_color(theme.text_muted)
            .child(Icon::new(icon).size_5()),
        )
        .child(
          div()
            .flex()
            .flex_col()
            .child(
              div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.text_dim)
                .child(description),
            ),
        )
    };

    let body: gpui::AnyElement = match self.step {
      Step::Choose => {
        let mut list = div().flex().flex_col().gap_1().py_2();
        for (ix, opt) in self.options().into_iter().enumerate() {
          let element = match opt {
            AddOption::Workspace => option(
              ix,
              IconName::FolderOpen,
              "New Workspace",
              "Open a folder on this computer as a project".to_string(),
              true,
              self.selected == ix,
              cx,
            ),
            AddOption::NewWorktree => option(
              ix,
              IconName::GalleryVerticalEnd,
              "New Worktree",
              if self.can_worktree {
                format!("Create a git worktree inside {}", self.project_name)
              } else {
                "No git project to create a worktree in".to_string()
              },
              self.can_worktree,
              self.selected == ix,
              cx,
            ),
            AddOption::ExistingWorktree => option(
              ix,
              IconName::FolderClosed,
              "Add Existing Worktree",
              if self.existing.is_empty() {
                "No unlisted worktrees found on disk".to_string()
              } else {
                format!("{} worktree(s) found on disk", self.existing.len())
              },
              !self.existing.is_empty(),
              self.selected == ix,
              cx,
            ),
          };
          list = list.child(element);
        }
        list.into_any_element()
      }
      Step::Existing => {
        let selected_ix = self
          .existing_selected
          .min(self.existing_matches.len().saturating_sub(1));
        let mut list = div().flex().flex_col().gap_0p5().pb_2().child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .mx_2()
            .mb_1()
            .h(px(32.0))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(theme.active)
            .bg(theme.elevated)
            .child(
              div()
                .flex_none()
                .text_color(theme.text_dim)
                .child(Icon::new(IconName::Search).size_4()),
            )
            .child(
              div()
                .flex_1()
                .child(Input::new(&self.existing_filter).appearance(false).small()),
            )
            .child(
              div()
                .text_xs()
                .text_color(theme.text_dim)
                .child(format!("{}", self.existing_matches.len())),
            ),
        );
        if self.existing_matches.is_empty() {
          list = list.child(
            div()
              .p_3()
              .text_sm()
              .text_color(theme.text_dim)
              .child("No worktrees match"),
          );
        }
        list = list.children(self.existing_matches.iter().enumerate().filter_map(
          |(ix, item_ix)| {
            let (branch, path) = self.existing.get(*item_ix)?;
            let is_selected = ix == selected_ix;
            let target = path.clone();
            let display_path = helix_filesystem::paths::abbreviate_home(path);
            let row = div()
              .id(SharedString::from(format!("existing-{ix}")))
              .flex()
              .items_center()
              .gap_2()
              .mx_2()
              .px_3()
              .h(px(30.0))
              .rounded_md()
              .border_1()
              .cursor_pointer()
              .when(is_selected, |el| {
                el.border_color(theme.active).bg(theme.elevated)
              })
              .when(!is_selected, |el| {
                el.border_color(gpui::transparent_black())
                  .hover(|s| s.bg(theme.hover))
              })
              .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(AddDialogEvent::AddExistingWorktree(target.clone()));
              }))
              .child(crate::components::git_branch_icon(theme.purple))
              .child(
                div()
                  .flex_none()
                  .text_sm()
                  .font_weight(gpui::FontWeight::SEMIBOLD)
                  .text_color(theme.text)
                  .child(branch.clone()),
              )
              .child(
                div()
                  .flex_1()
                  .text_xs()
                  .text_color(theme.text_dim)
                  .overflow_hidden()
                  .whitespace_nowrap()
                  .child(display_path),
              );

            Some(row)
          },
        ));
        list.into_any_element()
      }
      Step::Name => {
        let field = |label: &'static str, hint: &'static str, input: &Entity<InputState>| {
          div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
              div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(theme.text_muted).child(label))
                .child(div().text_xs().text_color(theme.text_dim).child(hint)),
            )
            .child(
              div()
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(theme.panel_border)
                .bg(theme.elevated)
                .child(Input::new(input).appearance(false).xsmall()),
            )
        };

        let mode_radio =
          |id: &'static str, label: &'static str, mode: BranchMode, cx: &mut Context<Self>| {
            Radio::new(id)
              .label(label)
              .xsmall()
              .checked(self.branch_mode == mode)
              .on_click(cx.listener(move |this, _: &bool, window, cx| {
                this.set_branch_mode(mode, window, cx);
              }))
          };

        let mut form = div()
          .flex()
          .flex_col()
          .gap_3()
          .p_3()
          .when(self.targets.len() > 1, |el| {
            el.child(
              div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                  div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child("Project"),
                )
                .child(Select::new(&self.target_select).xsmall().w_full()),
            )
          })
          .child(field(
            "Worktree",
            match self.branch_mode {
              BranchMode::New => "directory name",
              BranchMode::Existing => "directory name, defaults to the branch",
            },
            &self.worktree_name,
          ))
          .child(
            div()
              .flex()
              .items_center()
              .gap_4()
              .child(mode_radio(
                "branch-mode-new",
                "New branch",
                BranchMode::New,
                cx,
              ))
              .child(mode_radio(
                "branch-mode-existing",
                "Existing branch",
                BranchMode::Existing,
                cx,
              )),
          );

        if self.branch_mode == BranchMode::New {
          let describing = self.describing;
          let sparkle = div()
            .id("generate-branch")
            .size(px(18.0))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .text_xs()
            .text_color(theme.claude)
            .hover(|s| s.bg(theme.hover))
            .tooltip(move |window, cx| {
              gpui_component::tooltip::Tooltip::new(if describing {
                "Name it from the description"
              } else {
                "Let Claude name this branch"
              })
              .build(window, cx)
            })
            .on_click(cx.listener(|this, _, window, cx| this.ask_for_a_name(window, cx)))
            .child(if self.generating_branch { "…" } else { "✦" });

          form = form.child(
            div()
              .flex()
              .flex_col()
              .gap_1()
              .child(
                div()
                  .flex()
                  .items_center()
                  .gap_2()
                  .child(div().text_xs().text_color(theme.text_muted).child("Branch"))
                  .child(
                    div()
                      .text_xs()
                      .text_color(theme.text_dim)
                      .child("git will create it, defaults to the worktree name"),
                  ),
              )
              .child(
                div()
                  .relative()
                  .px_2()
                  .py_1()
                  .pr(px(24.0))
                  .rounded_md()
                  .border_1()
                  .border_color(theme.panel_border)
                  .bg(theme.elevated)
                  .child(Input::new(&self.branch_name).appearance(false).xsmall())
                  .child(
                    div()
                      .absolute()
                      .top(px(3.0))
                      .right(px(3.0))
                      .occlude()
                      .child(sparkle),
                  ),
              )
              .when(self.describing, |el| {
                el.child(
                  div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pt_1()
                    .child(
                      div()
                        .text_xs()
                        .text_color(theme.text_dim)
                        .child("Describe the work — any language, Enter to name it"),
                    )
                    .child(
                      div()
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.active)
                        .bg(theme.elevated)
                        .child(Input::new(&self.ai_context).appearance(false).xsmall()),
                    ),
                )
              }),
          );
        } else {
          let mut picker = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
              div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(theme.text_muted).child("Branch"))
                .child(div().text_xs().text_color(theme.text_dim).child(
                  if self.loading_branches {
                    "reading the repository…".to_string()
                  } else {
                    format!("{} ready to check out", self.branches.len())
                  },
                )),
            )
            .child(
              div()
                .px_2()
                .py_1()
                .rounded_md()
                .border_1()
                .border_color(theme.panel_border)
                .bg(theme.elevated)
                .child(Input::new(&self.branch_filter).appearance(false).xsmall()),
            );

          if self.branch_matches.is_empty() {
            if !self.loading_branches {
              picker = picker.child(div().text_xs().text_color(theme.text_dim).child(
                if self.branches.is_empty() {
                  "Every branch is already checked out somewhere"
                } else {
                  "No branch matches"
                },
              ));
            }
          } else {
            let height =
              px(BRANCH_ROW_HEIGHT * self.branch_matches.len().min(VISIBLE_BRANCHES) as f32);
            let entity = cx.entity();
            let rows = self.branch_matches.clone();

            picker = picker.child(
              uniform_list("branch-list", rows.len(), move |range, _, cx| {
                entity.update(cx, |dialog, cx| {
                  let theme = Theme::of(cx).clone();
                  let selected_ix = dialog.branch_selected.min(rows.len() - 1);

                  range
                    .filter_map(|ix| {
                      let branch = dialog.branches.get(*rows.get(ix)?)?;

                      Some(dialog.render_branch_row(ix, branch, ix == selected_ix, &theme, cx))
                    })
                    .collect()
                })
              })
              .track_scroll(self.branch_scroll.clone())
              .h(height),
            );
          }

          form = form.child(picker);
        }

        form
          .children(
            self
              .error
              .clone()
              .map(|err| div().text_xs().text_color(theme.red).child(err)),
          )
          .child(
            div()
              .id("create-worktree")
              .h(px(28.0))
              .flex()
              .items_center()
              .justify_center()
              .rounded_md()
              .bg(theme.elevated)
              .text_xs()
              .text_color(theme.text)
              .cursor_pointer()
              .hover(|s| s.bg(theme.hover))
              .on_click(cx.listener(|this, _, _, cx| this.confirm_worktree(cx)))
              .child("Create worktree"),
          )
          .into_any_element()
      }
    };

    let hint = |key: &'static str, action: &'static str| {
      div()
        .flex()
        .items_center()
        .gap_1()
        .child(
          div()
            .px_1p5()
            .py_0p5()
            .rounded_sm()
            .border_1()
            .border_color(theme.panel_border)
            .text_xs()
            .text_color(theme.text_muted)
            .child(key),
        )
        .child(div().text_xs().text_color(theme.text_dim).child(action))
    };

    let footer = div()
      .flex()
      .flex_none()
      .items_center()
      .justify_end()
      .gap_3()
      .h(px(38.0))
      .px_3()
      .border_t_1()
      .border_color(theme.panel_border)
      .child(hint(
        "Enter",
        if self.step == Step::Choose {
          "Select"
        } else {
          "Create"
        },
      ))
      .child(hint("Esc", "Back"));

    div()
      .id("add-dialog")
      .occlude()
      .track_focus(&self.focus_handle)
      .on_click(|_, _, cx| cx.stop_propagation())
      .when(self.step != Step::Name, |el| {
        el.on_mouse_down(
          gpui::MouseButton::Left,
          cx.listener(|this, _, window, _| {
            window.focus(&this.focus_handle);
          }),
        )
      })
      .on_key_down(cx.listener(Self::on_key_down))
      .w(px(440.0))
      .rounded_xl()
      .border_1()
      .border_color(theme.panel_border)
      .bg(crate::theme::ca(0x161616f5))
      .shadow_lg()
      .flex()
      .flex_col()
      .overflow_hidden()
      .child(
        div()
          .flex()
          .flex_col()
          .px_3()
          .pt_3()
          .pb_2()
          .border_b_1()
          .border_color(theme.panel_border)
          .child(
            div()
              .text_sm()
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .text_color(theme.text)
              .child(match self.step {
                Step::Choose => "Add",
                Step::Name => "New Worktree",
                Step::Existing => "Add Existing Worktree",
              }),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme.text_dim)
              .child(match self.step {
                Step::Choose => "Choose what to add to Helix".to_string(),
                Step::Name => match self.branch_mode {
                  BranchMode::New => {
                    format!("Creates a branch + worktree in {}", self.project_name)
                  }
                  BranchMode::Existing => {
                    format!("Checks an existing branch out in {}", self.project_name)
                  }
                },
                Step::Existing => "Type to filter worktrees found on disk".to_string(),
              }),
          ),
      )
      .child(body)
      .child(footer)
  }
}
