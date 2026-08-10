use crate::theme::Theme;
use gpui::{
  App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement,
  Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::{Icon, IconName};
use std::path::PathBuf;

pub enum AddDialogEvent {
  Dismissed,
  ChooseWorkspace,
  CreateWorktree(String),
  AddExistingWorktree(PathBuf),
}

#[derive(PartialEq)]
enum Step {
  Choose,
  Name,
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
  existing_query: String,
  name: String,
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
    project_name: String,
    can_worktree: bool,
    include_workspace: bool,
    existing: Vec<(String, PathBuf)>,
    cx: &mut Context<Self>,
  ) -> Self {
    Self {
      step: Step::Choose,
      selected: 0,
      existing_selected: 0,
      existing_query: String::new(),
      name: String::new(),
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

  fn filtered_existing(&self) -> Vec<(String, PathBuf)> {
    if self.existing_query.is_empty() {
      return self.existing.clone();
    }
    let query = self.existing_query.to_lowercase();
    self
      .existing
      .iter()
      .filter(|(branch, path)| {
        branch.to_lowercase().contains(&query)
          || path.display().to_string().to_lowercase().contains(&query)
      })
      .cloned()
      .collect()
  }

  fn confirm_choice(&mut self, cx: &mut Context<Self>) {
    let options = self.options();
    let Some(option) = options.get(self.selected) else {
      return;
    };
    match option {
      AddOption::Workspace => cx.emit(AddDialogEvent::ChooseWorkspace),
      AddOption::NewWorktree if self.can_worktree => {
        self.step = Step::Name;
        cx.notify();
      }
      AddOption::ExistingWorktree if !self.existing.is_empty() => {
        self.step = Step::Existing;
        cx.notify();
      }
      _ => {}
    }
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    match self.step {
      Step::Choose => match event.keystroke.key.as_str() {
        "escape" => cx.emit(AddDialogEvent::Dismissed),
        "enter" => self.confirm_choice(cx),
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
          self.existing_query.clear();
          self.existing_selected = 0;
          cx.notify();
        }
        "enter" => {
          if let Some((_, path)) = self.filtered_existing().get(self.existing_selected) {
            cx.emit(AddDialogEvent::AddExistingWorktree(path.clone()));
          }
        }
        "up" => {
          let count = self.filtered_existing().len().max(1);
          self.existing_selected = (self.existing_selected + count - 1) % count;
          cx.notify();
        }
        "down" => {
          let count = self.filtered_existing().len().max(1);
          self.existing_selected = (self.existing_selected + 1) % count;
          cx.notify();
        }
        "backspace" => {
          self.existing_query.pop();
          self.existing_selected = 0;
          cx.notify();
        }
        _ => {
          let mods = event.keystroke.modifiers;
          if mods.platform || mods.control || mods.function {
            return;
          }
          if let Some(text) = &event.keystroke.key_char {
            self.existing_query.push_str(text);
            self.existing_selected = 0;
            cx.notify();
          }
        }
      },
      Step::Name => match event.keystroke.key.as_str() {
        "escape" => {
          self.step = Step::Choose;
          cx.notify();
        }
        "enter" => {
          let name = self.name.trim().to_string();
          if !name.is_empty() {
            cx.emit(AddDialogEvent::CreateWorktree(name));
          }
        }
        "backspace" => {
          self.name.pop();
          cx.notify();
        }
        _ => {
          let mods = event.keystroke.modifiers;
          if mods.platform || mods.control || mods.function {
            return;
          }
          if let Some(text) = &event.keystroke.key_char {
            self.name.push_str(text);
            cx.notify();
          }
        }
      },
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
        .on_click(cx.listener(move |this, _, _, cx| {
          this.selected = ix;
          this.confirm_choice(cx);
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
                "Active project is not a git repository".to_string()
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
        let filtered = self.filtered_existing();
        let selected_ix = self.existing_selected.min(filtered.len().saturating_sub(1));
        let home = std::env::var("HOME").unwrap_or_default();
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
            .child(if self.existing_query.is_empty() {
              div()
                .text_sm()
                .text_color(theme.text_dim)
                .child("Filter worktrees...")
            } else {
              div()
                .text_sm()
                .text_color(theme.text)
                .child(self.existing_query.clone())
            })
            .child(div().w(px(2.0)).h(px(14.0)).bg(theme.accent).rounded_sm())
            .child(div().flex_1())
            .child(
              div()
                .text_xs()
                .text_color(theme.text_dim)
                .child(format!("{}", filtered.len())),
            ),
        );
        if filtered.is_empty() {
          list = list.child(
            div()
              .p_3()
              .text_sm()
              .text_color(theme.text_dim)
              .child("No worktrees match"),
          );
        }
        list = list.children(
          filtered
            .into_iter()
            .enumerate()
            .map(|(ix, (branch, path))| {
              let is_selected = ix == selected_ix;
              let target = path.clone();
              let display_path = {
                let p = path.display().to_string();
                if !home.is_empty() && p.starts_with(&home) {
                  format!("~{}", &p[home.len()..])
                } else {
                  p
                }
              };
              div()
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
                )
            }),
        );
        list.into_any_element()
      }
      Step::Name => div()
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .child(
          div()
            .text_xs()
            .text_color(theme.text_dim)
            .child(format!("Branch / worktree name for {}", self.project_name)),
        )
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .h(px(32.0))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(theme.active)
            .bg(theme.elevated)
            .child(crate::components::git_branch_icon(theme.purple))
            .child(if self.name.is_empty() {
              div()
                .text_sm()
                .text_color(theme.text_dim)
                .child("feat/my-feature")
            } else {
              div()
                .text_sm()
                .text_color(theme.text)
                .child(self.name.clone())
            })
            .child(div().w(px(2.0)).h(px(14.0)).bg(theme.accent).rounded_sm()),
        )
        .into_any_element(),
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
      .on_mouse_down(
        gpui::MouseButton::Left,
        cx.listener(|this, _, window, _| {
          window.focus(&this.focus_handle);
        }),
      )
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
                Step::Name => {
                  format!("Creates a branch + worktree in {}", self.project_name)
                }
                Step::Existing => "Type to filter worktrees found on disk".to_string(),
              }),
          ),
      )
      .child(body)
      .child(footer)
  }
}
