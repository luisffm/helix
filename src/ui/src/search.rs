use crate::theme::Theme;
use gpui::{
  App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
  SharedString, Task, WeakEntity, Window, div, prelude::*, px,
};
use gpui_component::list::{List, ListDelegate, ListItem, ListState};
use gpui_component::{Icon, IconName, IndexPath};
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

  /// Ranking reorders results by score, so a section keeps its place explicitly
  /// rather than following the best match in it.
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

pub struct SearchDialog {
  list: Entity<ListState<SearchDelegate>>,
  focus_handle: FocusHandle,
}

impl EventEmitter<SearchEvent> for SearchDialog {}

impl Focusable for SearchDialog {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    self.list.read(cx).focus_handle(cx)
  }
}

impl SearchDialog {
  pub fn new(items: Vec<SearchItem>, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let delegate = SearchDelegate::new(items, cx.entity().downgrade());
    let list = cx.new(|cx| ListState::new(delegate, window, cx));

    Self {
      list,
      focus_handle: cx.focus_handle(),
    }
  }
}

impl Render for SearchDialog {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

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
      .child(
        List::new(&self.list)
          .search_placeholder("Search worktrees, tabs, projects, and actions...")
          .flex_1()
          .min_h_0(),
      )
      .child(footer)
  }
}

struct Section {
  name: &'static str,
  matches: Vec<usize>,
}

/// The list widget owns the query input, the keys and the scrolling; the
/// delegate only decides which items match, how they group and what a row
/// looks like.
pub struct SearchDelegate {
  dialog: WeakEntity<SearchDialog>,
  items: Vec<SearchItem>,
  haystacks: Vec<String>,
  matches: Vec<usize>,
  sections: Vec<Section>,
  ranker: Ranker,
  selected: Option<IndexPath>,
}

impl SearchDelegate {
  fn new(items: Vec<SearchItem>, dialog: WeakEntity<SearchDialog>) -> Self {
    let haystacks = items.iter().map(haystack_of).collect();

    let mut delegate = Self {
      dialog,
      items,
      haystacks,
      matches: Vec::new(),
      sections: Vec::new(),
      ranker: Ranker::new(),
      selected: None,
    };

    delegate.rank("");

    delegate
  }

  fn rank(&mut self, query: &str) {
    self.ranker.set_query(query.trim());
    self
      .ranker
      .rank_into(self.haystacks.iter().map(String::as_str), &mut self.matches);

    self
      .matches
      .sort_by_key(|ix| self.items[*ix].target.section_order());

    self.sections.clear();

    for ix in &self.matches {
      let name = self.items[*ix].target.section();

      match self.sections.last_mut() {
        Some(section) if section.name == name => section.matches.push(*ix),
        _ => self.sections.push(Section {
          name,
          matches: vec![*ix],
        }),
      }
    }
  }

  fn item_at(&self, ix: IndexPath) -> Option<&SearchItem> {
    let position = self.sections.get(ix.section)?.matches.get(ix.row)?;

    self.items.get(*position)
  }
}

impl ListDelegate for SearchDelegate {
  type Item = ListItem;

  fn perform_search(
    &mut self,
    query: &str,
    _window: &mut Window,
    _cx: &mut Context<ListState<Self>>,
  ) -> Task<()> {
    self.rank(query);

    Task::ready(())
  }

  fn sections_count(&self, _cx: &App) -> usize {
    self.sections.len()
  }

  fn items_count(&self, section: usize, _cx: &App) -> usize {
    self
      .sections
      .get(section)
      .map(|section| section.matches.len())
      .unwrap_or(0)
  }

  fn render_section_header(
    &mut self,
    section: usize,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<impl IntoElement> {
    let theme = Theme::of(cx).clone();
    let name = self.sections.get(section)?.name;

    Some(
      div()
        .flex()
        .items_center()
        .h(px(26.0))
        .px_3()
        .text_xs()
        .text_color(theme.text_dim)
        .child(name),
    )
  }

  fn render_item(
    &mut self,
    ix: IndexPath,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> Option<Self::Item> {
    let theme = Theme::of(cx).clone();
    let item = self.item_at(ix)?;

    Some(
      ListItem::new(("search-item", ix.section * 1000 + ix.row))
        .selected(self.selected == Some(ix))
        .h(px(ROW_HEIGHT))
        .mx_2()
        .px_2()
        .rounded_md()
        .child(
          div()
            .flex()
            .items_center()
            .gap_2()
            .size_full()
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
        ),
    )
  }

  fn render_empty(
    &mut self,
    _window: &mut Window,
    cx: &mut Context<ListState<Self>>,
  ) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    div()
      .p_4()
      .text_sm()
      .text_color(theme.text_dim)
      .child("No results")
  }

  fn set_selected_index(
    &mut self,
    ix: Option<IndexPath>,
    _window: &mut Window,
    _cx: &mut Context<ListState<Self>>,
  ) {
    self.selected = ix;
  }

  fn confirm(&mut self, _secondary: bool, _window: &mut Window, cx: &mut Context<ListState<Self>>) {
    let Some(target) = self
      .selected
      .and_then(|ix| self.item_at(ix))
      .map(|item| item.target.clone())
    else {
      return;
    };

    self
      .dialog
      .update(cx, |_, cx| cx.emit(SearchEvent::Selected(target)))
      .ok();
  }

  fn cancel(&mut self, _window: &mut Window, cx: &mut Context<ListState<Self>>) {
    self
      .dialog
      .update(cx, |_, cx| cx.emit(SearchEvent::Dismissed))
      .ok();
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
