use crate::components::key_hint;
use crate::icons::HelixIcon;
use crate::theme::Theme;
use gpui::{
  App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
  ParentElement, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::Icon;
use gpui_component::input::{Input, InputEvent, InputState};

pub enum WorktreeEditEvent {
  Close,
  Save {
    display_name: String,
    issue: String,
    pr: String,
  },
}

const FIELDS: [(&str, &str); 3] = [
  ("Display Name", "Optional label shown in the sidebar"),
  (
    "GitHub Issue",
    "e.g. https://github.com/org/repo/issues/42 or #42",
  ),
  (
    "GitHub PR",
    "e.g. https://github.com/org/repo/pull/128 or #128",
  ),
];

pub struct WorktreeEditDialog {
  branch: String,
  inputs: [Entity<InputState>; 3],
  focus_handle: FocusHandle,
}

impl EventEmitter<WorktreeEditEvent> for WorktreeEditDialog {}

impl Focusable for WorktreeEditDialog {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl WorktreeEditDialog {
  pub fn new(
    branch: String,
    display_name: String,
    issue: String,
    pr: String,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let mut make = |placeholder: &'static str, value: String| {
      cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(placeholder);

        if !value.is_empty() {
          state = state.default_value(value);
        }

        state
      })
    };

    let inputs = [
      make(FIELDS[0].1, display_name),
      make(FIELDS[1].1, issue),
      make(FIELDS[2].1, pr),
    ];

    for input in &inputs {
      cx.subscribe(input, |this, _, event: &InputEvent, cx| {
        if let InputEvent::PressEnter { .. } = event {
          this.emit_save(cx);
        }
      })
      .detach();
    }

    window.focus(&inputs[0].read(cx).focus_handle(cx));

    Self {
      branch,
      inputs,
      focus_handle: cx.focus_handle(),
    }
  }

  fn emit_save(&mut self, cx: &mut Context<Self>) {
    cx.emit(WorktreeEditEvent::Save {
      display_name: self.inputs[0].read(cx).value().trim().to_string(),
      issue: self.inputs[1].read(cx).value().trim().to_string(),
      pr: self.inputs[2].read(cx).value().trim().to_string(),
    });
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    match event.keystroke.key.as_str() {
      "escape" => cx.emit(WorktreeEditEvent::Close),
      "tab" => {
        let focused = window.focused(cx);

        let current = self.inputs.iter().position(|input| {
          focused
            .as_ref()
            .is_some_and(|handle| *handle == input.read(cx).focus_handle(cx))
        });

        let next = match current {
          Some(ix) if event.keystroke.modifiers.shift => (ix + 2) % 3,
          Some(ix) => (ix + 1) % 3,
          None => 0,
        };

        window.focus(&self.inputs[next].read(cx).focus_handle(cx));
        cx.notify();
      }
      _ => {}
    }
  }
}

impl Render for WorktreeEditDialog {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let mut body = div().flex().flex_col().gap_3().p_3();
    for (ix, (label, _)) in FIELDS.iter().enumerate() {
      body = body.child(
        div()
          .flex()
          .flex_col()
          .gap_1()
          .child(
            div()
              .text_xs()
              .text_color(theme.text_muted)
              .child(SharedString::from(label.to_string())),
          )
          .child(Input::new(&self.inputs[ix]).w_full()),
      );
    }

    div()
      .id("worktree-edit-dialog")
      .occlude()
      .track_focus(&self.focus_handle)
      .on_click(|_, _, cx| cx.stop_propagation())
      .on_key_down(cx.listener(Self::on_key_down))
      .w(px(460.0))
      .rounded_xl()
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.win_tint)
      .shadow_lg()
      .flex()
      .flex_col()
      .overflow_hidden()
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
              .text_color(theme.purple)
              .child(Icon::new(HelixIcon::GitBranch).size_3p5()),
          )
          .child(
            div()
              .text_sm()
              .font_weight(gpui::FontWeight::SEMIBOLD)
              .text_color(theme.text)
              .child(format!("Edit Worktree · {}", self.branch)),
          ),
      )
      .child(body)
      .child(
        div()
          .flex()
          .flex_none()
          .items_center()
          .justify_end()
          .gap_3()
          .h(px(38.0))
          .px_3()
          .border_t_1()
          .border_color(theme.panel_border)
          .child(key_hint("tab", "Next field", &theme))
          .child(key_hint("enter", "Save", &theme))
          .child(key_hint("escape", "Cancel", &theme)),
      )
  }
}
