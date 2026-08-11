use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyDownEvent,
  ParentElement, Render, ScrollStrategy, SharedString, UniformListScrollHandle, Window, div,
  prelude::*, px, uniform_list,
};
use gpui_component::{Icon, IconName};
use helix_fuzzy::Ranker;
use std::path::PathBuf;

const ROW_HEIGHT: f32 = 34.0;

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

  /// Ranking reorders items by score, so the section a row belongs to has to
  /// keep its place explicitly or the headers would repeat down the list.
  fn section_order(&self) -> u8 {
    match self {
      SearchTarget::Worktree(_) => 0,
      SearchTarget::Tab(_) => 1,
      SearchTarget::Project(_) => 2,
      SearchTarget::NewTerminal | SearchTarget::NewClaude => 3,
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

/// The list mixes section headers with results, and `uniform_list` needs one
/// height for every row, so both are laid out as rows of `ROW_HEIGHT`.
#[derive(Clone, Copy)]
enum Row {
  Header(&'static str),
  Result(usize),
}

pub struct SearchDialog {
  items: Vec<SearchItem>,
  haystacks: Vec<String>,
  matches: Vec<usize>,
  rows: Vec<Row>,
  match_rows: Vec<usize>,
  scroll: UniformListScrollHandle,
  ranker: Ranker,
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
    let haystacks = items.iter().map(haystack_of).collect();

    let mut dialog = Self {
      items,
      haystacks,
      matches: Vec::new(),
      rows: Vec::new(),
      match_rows: Vec::new(),
      scroll: UniformListScrollHandle::new(),
      ranker: Ranker::new(),
      query: String::new(),
      selected: 0,
      focus_handle: cx.focus_handle(),
    };

    dialog.refresh_matches();

    dialog
  }

  /// Matching runs over the prebuilt haystacks and yields indices, so a
  /// keystroke never clones the item list and `render` never filters.
  fn refresh_matches(&mut self) {
    self.selected = 0;

    self.ranker.set_query(&self.query);
    self
      .ranker
      .rank_into(self.haystacks.iter().map(String::as_str), &mut self.matches);

    self
      .matches
      .sort_by_key(|ix| self.items[*ix].target.section_order());

    self.rebuild_rows();
  }

  fn rebuild_rows(&mut self) {
    self.rows.clear();
    self.match_rows.clear();

    let mut last_section = "";

    for (position, ix) in self.matches.iter().enumerate() {
      let section = self.items[*ix].target.section();

      if section != last_section {
        last_section = section;

        self.rows.push(Row::Header(section));
      }

      self.match_rows.push(self.rows.len());
      self.rows.push(Row::Result(position));
    }
  }

  fn render_row(
    &self,
    row: Row,
    selected: usize,
    theme: &Theme,
    cx: &mut Context<Self>,
  ) -> AnyElement {
    let position = match row {
      Row::Header(section) => {
        return div()
          .flex()
          .items_center()
          .h(px(ROW_HEIGHT))
          .px_3()
          .text_xs()
          .text_color(theme.text_dim)
          .child(section)
          .into_any_element();
      }
      Row::Result(position) => position,
    };

    let Some(item) = self
      .matches
      .get(position)
      .and_then(|ix| self.items.get(*ix))
    else {
      return div().h(px(ROW_HEIGHT)).into_any_element();
    };

    let is_selected = position == selected;
    let target = item.target.clone();

    div()
      .id(SharedString::from(format!("search-item-{position}")))
      .flex()
      .items_center()
      .gap_2()
      .mx_2()
      .px_2()
      .h(px(ROW_HEIGHT))
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
      )
      .into_any_element()
  }

  fn reveal_selection(&mut self) {
    if let Some(row) = self.match_rows.get(self.selected) {
      self.scroll.scroll_to_item(*row, ScrollStrategy::Center);
    }
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let count = self.matches.len();

    match event.keystroke.key.as_str() {
      "escape" => {
        cx.emit(SearchEvent::Dismissed);
        return;
      }
      "enter" => {
        if let Some(item) = self
          .matches
          .get(self.selected)
          .and_then(|ix| self.items.get(*ix))
        {
          cx.emit(SearchEvent::Selected(item.target.clone()));
        }

        return;
      }
      "up" => {
        if count > 0 {
          self.selected = (self.selected + count - 1) % count;
          self.reveal_selection();
        }

        cx.notify();

        return;
      }
      "down" => {
        if count > 0 {
          self.selected = (self.selected + 1) % count;
          self.reveal_selection();
        }

        cx.notify();

        return;
      }
      "backspace" => {
        self.query.pop();
        self.refresh_matches();

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
      self.refresh_matches();

      cx.notify();
    }
  }
}

fn haystack_of(item: &SearchItem) -> String {
  [
    item.label.as_str(),
    item.detail.as_str(),
    item.badge.as_str(),
  ]
  .join(" ")
}

impl Render for SearchDialog {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    let selected = self.selected.min(self.matches.len().saturating_sub(1));

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

    let list: AnyElement = if self.rows.is_empty() {
      div()
        .flex_1()
        .p_4()
        .text_sm()
        .text_color(theme.text_dim)
        .child("No results")
        .into_any_element()
    } else {
      let rows = self.rows.clone();
      let entity = cx.entity();

      uniform_list("search-results", rows.len(), move |range, _, cx| {
        entity.update(cx, |dialog, cx| {
          let theme = Theme::of(cx).clone();

          range
            .filter_map(|ix| Some(dialog.render_row(*rows.get(ix)?, selected, &theme, cx)))
            .collect()
        })
      })
      .track_scroll(self.scroll.clone())
      .flex_1()
      .min_h_0()
      .py_1()
      .into_any_element()
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
