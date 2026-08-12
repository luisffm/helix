use crate::components::{BODY, GLYPH, META, cap, key_hint, section_label};
use crate::theme::Theme;
use gpui::{
  App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
  Task, WeakEntity, Window, div, prelude::*, px,
};
use gpui_component::list::{List, ListDelegate, ListItem, ListState};
use gpui_component::{Icon, IconName, IndexPath};
use helix_fuzzy::Ranker;
use std::path::PathBuf;

const ROW_HEIGHT: f32 = 32.0;
const SECTION_HEIGHT: f32 = 26.0;
/// The list's own query input, which lives inside it, plus the footer below it.
const INPUT_HEIGHT: f32 = 34.0;
const FOOTER_HEIGHT: f32 = 36.0;
const MAX_BODY: f32 = 380.0;
const EMPTY_BODY: f32 = 72.0;

#[derive(Clone, Debug)]
pub enum SearchTarget {
  Worktree(PathBuf),
  Project(PathBuf),
  Tab(usize),
  NewTerminal,
  NewClaude,
  AddWorktree,
}

impl SearchTarget {
  fn section(&self) -> &'static str {
    match self {
      SearchTarget::Worktree(_) => "WORKTREES",
      SearchTarget::Tab(_) => "OPEN TABS",
      SearchTarget::Project(_) => "PROJECTS",
      SearchTarget::NewTerminal | SearchTarget::NewClaude | SearchTarget::AddWorktree => "ACTIONS",
    }
  }

  /// Ranking reorders results by score, so a section keeps its place explicitly
  /// rather than following the best match in it.
  fn section_order(&self) -> u8 {
    match self {
      SearchTarget::Worktree(_) => 0,
      SearchTarget::Tab(_) => 1,
      SearchTarget::Project(_) => 2,
      SearchTarget::NewTerminal | SearchTarget::NewClaude | SearchTarget::AddWorktree => 3,
    }
  }

  fn icon(&self) -> Icon {
    match self {
      SearchTarget::Worktree(_) => Icon::new(IconName::GalleryVerticalEnd),
      SearchTarget::Tab(_) => Icon::new(IconName::SquareTerminal),
      SearchTarget::Project(_) => Icon::new(IconName::Folder),
      SearchTarget::NewTerminal => Icon::new(IconName::SquareTerminal),
      SearchTarget::NewClaude => Icon::new(crate::icons::HelixIcon::Claude),
      SearchTarget::AddWorktree => Icon::new(IconName::GalleryVerticalEnd),
    }
  }

  fn icon_color(&self, theme: &Theme) -> gpui::Hsla {
    match self {
      SearchTarget::NewClaude => theme.claude,
      _ => theme.text_dim,
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
    let body = self.list.read(cx).delegate().body_height();

    let footer = div()
      .flex()
      .flex_none()
      .items_center()
      .justify_end()
      .gap(px(14.0))
      .h(px(36.0))
      .px(px(14.0))
      .border_t_1()
      .border_color(theme.panel_border)
      .child(key_hint("enter", "Open", &theme))
      .child(key_hint("escape", "Close", &theme))
      .child(key_hint("up down", "Move", &theme));

    div()
      .id("search-dialog")
      .occlude()
      .track_focus(&self.focus_handle)
      .on_click(|_, _, cx| cx.stop_propagation())
      .w(px(600.0))
      // A virtual list has nothing to lay out against an indefinite height: with
      // only a maximum, the viewport resolved to zero and the palette opened
      // blank. So the height is stated, and derived from the rows there are so a
      // short result set still gets a short dialog.
      .h(px(INPUT_HEIGHT + body + FOOTER_HEIGHT))
      .rounded(px(14.0))
      .border_1()
      .border_color(theme.win_border)
      .bg(theme.win_tint)
      .shadow_2xl()
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

  /// How tall the rows want to be, capped. The list virtualizes against this, so
  /// it is measured from the counts rather than from what happens to be on screen.
  fn body_height(&self) -> f32 {
    if self.matches.is_empty() {
      return EMPTY_BODY;
    }

    let rows = self.matches.len() as f32 * ROW_HEIGHT;
    let headers = self.sections.len() as f32 * SECTION_HEIGHT;

    (rows + headers).min(MAX_BODY)
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
      // The list measures one header under `MinContent` and reuses that height for
      // every section, so the height has to be stated: left to its text, the
      // label wraps at the narrowest word and every section inherits the wrong
      // size, which leaves one row visible and the layout re-measuring forever.
      div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(26.0))
        .px(px(8.0))
        .whitespace_nowrap()
        .child(section_label(name, &theme)),
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
        .mx_1()
        .px(px(9.0))
        .rounded(px(8.0))
        .child(
          div()
            .flex()
            .items_center()
            .gap(px(9.0))
            .size_full()
            .child(
              div()
                .flex_none()
                .text_color(item.target.icon_color(&theme))
                .child(item.target.icon().size(px(GLYPH))),
            )
            .child(
              div()
                .flex_none()
                .text_size(px(BODY))
                .text_color(theme.text)
                .child(item.label.clone()),
            )
            .child(
              div()
                .flex_1()
                .min_w_0()
                .text_size(px(META))
                .text_color(theme.text_dim)
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.detail.clone()),
            )
            .child(cap(item.badge.clone(), &theme)),
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
      .text_size(px(BODY))
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
