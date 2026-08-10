use crate::theme::Theme;
use gpui::{
  App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement,
  Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::{Icon, IconName};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum SearchTarget {
  Worktree(PathBuf),
  Project(PathBuf),
  Tab(usize),
  NewTerminal,
  NewClaude,
}

impl SearchTarget {
  fn section(&self) -> &'static str {
    match self {
      SearchTarget::Worktree(_) => "WORKTREES",
      SearchTarget::Tab(_) => "OPEN TABS",
      SearchTarget::Project(_) => "PROJECTS",
      SearchTarget::NewTerminal | SearchTarget::NewClaude => "ACTIONS",
    }
  }

  fn icon(&self) -> IconName {
    match self {
      SearchTarget::Worktree(_) => IconName::GalleryVerticalEnd,
      SearchTarget::Tab(_) => IconName::SquareTerminal,
      SearchTarget::Project(_) => IconName::Folder,
      SearchTarget::NewTerminal => IconName::Plus,
      SearchTarget::NewClaude => IconName::Asterisk,
    }
  }
}

#[derive(Clone)]
pub struct SearchItem {
  pub label: String,
  pub detail: String,
  pub badge: String,
  pub target: SearchTarget,
}

pub enum SearchEvent {
  Dismissed,
  Selected(SearchTarget),
}

pub struct SearchDialog {
  items: Vec<SearchItem>,
  query: String,
  selected: usize,
  focus_handle: FocusHandle,
}

impl EventEmitter<SearchEvent> for SearchDialog {}

impl Focusable for SearchDialog {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl SearchDialog {
  pub fn new(items: Vec<SearchItem>, cx: &mut Context<Self>) -> Self {
    Self {
      items,
      query: String::new(),
      selected: 0,
      focus_handle: cx.focus_handle(),
    }
  }

  fn filtered(&self) -> Vec<SearchItem> {
    if self.query.is_empty() {
      return self.items.clone();
    }
    let query = self.query.to_lowercase();
    self
      .items
      .iter()
      .filter(|item| {
        item.label.to_lowercase().contains(&query)
          || item.detail.to_lowercase().contains(&query)
          || item.badge.to_lowercase().contains(&query)
      })
      .cloned()
      .collect()
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let count = self.filtered().len();
    match event.keystroke.key.as_str() {
      "escape" => {
        cx.emit(SearchEvent::Dismissed);
        return;
      }
      "enter" => {
        if let Some(item) = self.filtered().get(self.selected) {
          cx.emit(SearchEvent::Selected(item.target.clone()));
        }
        return;
      }
      "up" => {
        if count > 0 {
          self.selected = (self.selected + count - 1) % count;
        }
        cx.notify();
        return;
      }
      "down" => {
        if count > 0 {
          self.selected = (self.selected + 1) % count;
        }
        cx.notify();
        return;
      }
      "backspace" => {
        self.query.pop();
        self.selected = 0;
        cx.notify();
        return;
      }
      _ => {}
    }
    let mods = event.keystroke.modifiers;
    if mods.platform || mods.control || mods.function {
      return;
    }
    if let Some(text) = &event.keystroke.key_char {
      self.query.push_str(text);
      self.selected = 0;
      cx.notify();
    }
  }
}

impl Render for SearchDialog {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    let filtered = self.filtered();
    let selected = self.selected.min(filtered.len().saturating_sub(1));

    let input_row = div()
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .h(px(46.0))
      .px_3()
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .flex_none()
          .text_color(theme.text_dim)
          .child(Icon::new(IconName::Search).size_4()),
      )
      .child(if self.query.is_empty() {
        div()
          .text_sm()
          .text_color(theme.text_dim)
          .child("Search worktrees, tabs, projects, and actions...")
      } else {
        div()
          .text_sm()
          .text_color(theme.text)
          .child(self.query.clone())
      })
      .child(div().w(px(2.0)).h(px(16.0)).bg(theme.accent).rounded_sm());

    let mut list = div()
      .id("search-results")
      .flex_1()
      .min_h_0()
      .overflow_y_scroll()
      .flex()
      .flex_col()
      .py_1();

    let mut last_section = "";
    for (ix, item) in filtered.iter().enumerate() {
      let section = item.target.section();
      if section != last_section {
        last_section = section;
        list = list.child(
          div()
            .px_3()
            .pt_2()
            .pb_1()
            .text_xs()
            .text_color(theme.text_dim)
            .child(SharedString::from(section.to_string())),
        );
      }
      let is_selected = ix == selected;
      let target = item.target.clone();
      list = list.child(
        div()
          .id(SharedString::from(format!("search-item-{ix}")))
          .flex()
          .items_center()
          .gap_2()
          .mx_2()
          .px_2()
          .h(px(34.0))
          .rounded_md()
          .cursor_pointer()
          .when(is_selected, |el| {
            el.bg(theme.elevated).border_1().border_color(theme.active)
          })
          .when(!is_selected, |el| el.hover(|s| s.bg(theme.hover)))
          .on_click(cx.listener(move |_, _, _, cx| {
            cx.emit(SearchEvent::Selected(target.clone()));
          }))
          .child(
            div()
              .flex_none()
              .text_color(theme.text_dim)
              .child(Icon::new(item.target.icon()).size_4()),
          )
          .child(
            div()
              .text_sm()
              .text_color(theme.text)
              .child(item.label.clone()),
          )
          .child(
            div()
              .flex_1()
              .text_xs()
              .text_color(theme.text_dim)
              .overflow_hidden()
              .child(item.detail.clone()),
          )
          .child(
            div()
              .flex_none()
              .px_1p5()
              .py_0p5()
              .rounded_sm()
              .border_1()
              .border_color(theme.panel_border)
              .text_xs()
              .text_color(theme.text_muted)
              .child(item.badge.clone()),
          ),
      );
    }

    if filtered.is_empty() {
      list = list.child(
        div()
          .p_4()
          .text_sm()
          .text_color(theme.text_dim)
          .child("No results"),
      );
    }

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
      .child(hint("Enter", "Open"))
      .child(hint("Esc", "Close"))
      .child(hint("↑↓", "Move"));

    div()
      .id("search-dialog")
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
      .w(px(640.0))
      .max_h(px(520.0))
      .rounded_xl()
      .border_1()
      .border_color(theme.panel_border)
      .bg(crate::theme::ca(0x161616f5))
      .shadow_lg()
      .flex()
      .flex_col()
      .overflow_hidden()
      .child(input_row)
      .child(list)
      .child(footer)
  }
}
