use crate::components::{HEADER_HEIGHT, icon_button};
use crate::diff_view::DiffView;
use crate::editor_view::{EditorView, EditorViewEvent};
use crate::terminal_view::{TerminalView, TerminalViewEvent};
use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
  ParentElement, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::{Icon, IconName};
use helix_models::{AgentStatus, DiffBase, SessionKind};
use std::path::PathBuf;

pub enum WorkspaceEvent {
  SessionsChanged,
}

pub enum TabContent {
  Terminal(Entity<TerminalView>),
  Editor(Entity<EditorView>),
  Diff(Entity<DiffView>),
}

pub struct TabItem {
  pub content: TabContent,
  pub preview: bool,
}

impl TabItem {
  pub fn terminal(&self) -> Option<&Entity<TerminalView>> {
    match &self.content {
      TabContent::Terminal(view) => Some(view),
      _ => None,
    }
  }

  pub fn editor(&self) -> Option<&Entity<EditorView>> {
    match &self.content {
      TabContent::Editor(view) => Some(view),
      _ => None,
    }
  }

  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match &self.content {
      TabContent::Terminal(view) => view.read(cx).focus_handle(cx),
      TabContent::Editor(view) => view.read(cx).focus_handle(cx),
      TabContent::Diff(view) => view.read(cx).focus_handle(cx),
    }
  }

  pub fn title(&self, cx: &App) -> String {
    match &self.content {
      TabContent::Terminal(view) => view
        .read(cx)
        .title
        .trim_start_matches(|c: char| "✳✻✶✽*⁕ ".contains(c))
        .to_string(),
      TabContent::Editor(view) => view.read(cx).title.to_string(),
      TabContent::Diff(view) => view.read(cx).title.to_string(),
    }
  }

  pub fn detail(&self, cx: &App) -> String {
    match &self.content {
      TabContent::Terminal(view) => match view.read(cx).kind {
        SessionKind::Terminal => "Terminal tab".to_string(),
        SessionKind::ClaudeCode => "Claude session".to_string(),
      },
      TabContent::Editor(_) => "Editor".to_string(),
      TabContent::Diff(view) => format!("Diff {}", view.read(cx).base.label()),
    }
  }

  fn icon(&self, cx: &App, theme: &Theme) -> (Icon, Hsla) {
    match &self.content {
      TabContent::Terminal(view) => {
        let view = view.read(cx);
        match view.kind {
          SessionKind::Terminal => (
            Icon::new(IconName::SquareTerminal),
            status_color(view.status(), theme),
          ),
          SessionKind::ClaudeCode => (Icon::new(IconName::Asterisk), theme.claude),
        }
      }
      TabContent::Editor(view) => {
        let view = view.read(cx);
        let color = if view.is_dirty() {
          theme.yellow
        } else {
          theme.text_muted
        };
        (crate::file_icons::icon(&view.path), color)
      }
      TabContent::Diff(_) => (Icon::default().path("icons/git-compare.svg"), theme.purple),
    }
  }

  fn element(&self) -> AnyElement {
    match &self.content {
      TabContent::Terminal(view) => view.clone().into_any_element(),
      TabContent::Editor(view) => view.clone().into_any_element(),
      TabContent::Diff(view) => view.clone().into_any_element(),
    }
  }

  fn is_editor_for(&self, path: &PathBuf, cx: &App) -> bool {
    self
      .editor()
      .map(|view| &view.read(cx).path == path)
      .unwrap_or(false)
  }

  fn is_diff_for(&self, relative: &str, base: &DiffBase, cx: &App) -> bool {
    match &self.content {
      TabContent::Diff(view) => {
        let view = view.read(cx);
        view.relative == relative && &view.base == base
      }
      _ => false,
    }
  }
}

pub struct Workspace {
  project_root: PathBuf,
  pub tabs: Vec<TabItem>,
  pub active: usize,
  pub left_sidebar_open: bool,
  new_menu_open: bool,
  next_session: u64,
  terminal_count: usize,
  claude_count: usize,
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

impl Workspace {
  pub fn new(project_root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let mut workspace = Self {
      project_root,
      tabs: Vec::new(),
      active: 0,
      left_sidebar_open: true,
      new_menu_open: false,
      next_session: 0,
      terminal_count: 0,
      claude_count: 0,
    };
    workspace.open_tab(SessionKind::Terminal, window, cx);
    workspace
  }

  pub fn terminals<'a>(&'a self) -> impl Iterator<Item = (usize, &'a Entity<TerminalView>)> + 'a {
    self
      .tabs
      .iter()
      .enumerate()
      .filter_map(|(ix, tab)| tab.terminal().map(|view| (ix, view)))
  }

  pub fn open_tab(&mut self, kind: SessionKind, window: &mut Window, cx: &mut Context<Self>) {
    self.next_session += 1;
    let ordinal = match kind {
      SessionKind::Terminal => {
        self.terminal_count += 1;
        self.terminal_count
      }
      SessionKind::ClaudeCode => {
        self.claude_count += 1;
        self.claude_count
      }
    };
    let title = kind.default_title(ordinal);
    let id = self.next_session;
    let root = self.project_root.clone();
    let view = cx.new(|cx| TerminalView::new(id, kind, title, root, cx));
    cx.subscribe(&view, Self::handle_terminal_event).detach();
    self.push_tab(
      TabItem {
        content: TabContent::Terminal(view),
        preview: false,
      },
      window,
      cx,
    );
    cx.emit(WorkspaceEvent::SessionsChanged);
  }

  pub fn open_file(
    &mut self,
    path: PathBuf,
    preview: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if let Some(ix) = self
      .tabs
      .iter()
      .position(|tab| tab.is_editor_for(&path, cx))
    {
      if !preview {
        self.tabs[ix].preview = false;
      }
      self.activate(ix, window, cx);
      return;
    }

    let view = cx.new(|cx| EditorView::new(path, window, cx));
    cx.subscribe(&view, |_, _, _: &EditorViewEvent, cx| cx.notify())
      .detach();
    let tab = TabItem {
      content: TabContent::Editor(view),
      preview,
    };

    match preview.then(|| self.preview_slot(cx)).flatten() {
      Some(ix) => {
        self.tabs[ix] = tab;
        self.activate(ix, window, cx);
      }
      None => self.push_tab(tab, window, cx),
    }
  }

  pub fn open_diff(
    &mut self,
    root: PathBuf,
    relative: String,
    base: DiffBase,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if let Some(ix) = self
      .tabs
      .iter()
      .position(|tab| tab.is_diff_for(&relative, &base, cx))
    {
      if let TabContent::Diff(view) = &self.tabs[ix].content {
        view.update(cx, |view, cx| view.reload(cx));
      }
      self.activate(ix, window, cx);
      return;
    }
    let view = cx.new(|cx| DiffView::new(root, relative, base, cx));
    self.push_tab(
      TabItem {
        content: TabContent::Diff(view),
        preview: false,
      },
      window,
      cx,
    );
  }

  fn preview_slot(&self, _cx: &App) -> Option<usize> {
    self.tabs.iter().position(|tab| tab.preview)
  }

  fn push_tab(&mut self, tab: TabItem, window: &mut Window, cx: &mut Context<Self>) {
    self.tabs.push(tab);
    self.active = self.tabs.len() - 1;
    self.focus_active(window, cx);
    cx.notify();
  }

  pub fn refresh_open_files(
    &mut self,
    changed: &[PathBuf],
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let git_moved = changed.iter().any(|path| {
      path
        .components()
        .any(|component| component.as_os_str() == ".git")
    });

    for tab in &self.tabs {
      match &tab.content {
        TabContent::Editor(view) => {
          let path = view.read(cx).path.clone();
          if changed.iter().any(|changed| *changed == path) {
            view.update(cx, |view, cx| view.note_external_change(window, cx));
          }
        }
        TabContent::Diff(view) => {
          let absolute = {
            let view = view.read(cx);
            view.root.join(&view.relative)
          };
          if git_moved || changed.iter().any(|changed| *changed == absolute) {
            view.update(cx, |view, cx| view.reload(cx));
          }
        }
        TabContent::Terminal(_) => {}
      }
    }
  }

  pub fn save_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(view) = self.tabs.get(self.active).and_then(|tab| tab.editor()) {
      view.update(cx, |view, cx| view.save(window, cx));
    }
  }

  fn handle_terminal_event(
    &mut self,
    _view: Entity<TerminalView>,
    event: &TerminalViewEvent,
    cx: &mut Context<Self>,
  ) {
    match event {
      TerminalViewEvent::Activity | TerminalViewEvent::Retitled => cx.notify(),
      TerminalViewEvent::Exited(_) => {
        cx.emit(WorkspaceEvent::SessionsChanged);
        cx.notify();
      }
    }
  }

  pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    if ix >= self.tabs.len() {
      return;
    }
    self.tabs.remove(ix);
    if self.active >= self.tabs.len() && !self.tabs.is_empty() {
      self.active = self.tabs.len() - 1;
    }
    self.focus_active(window, cx);
    cx.emit(WorkspaceEvent::SessionsChanged);
    cx.notify();
  }

  pub fn close_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.close_tab(self.active, window, cx);
  }

  pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    if ix < self.tabs.len() {
      self.active = ix;
      self.focus_active(window, cx);
      cx.notify();
    }
  }

  pub fn activate_next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.tabs.is_empty() {
      self.activate((self.active + 1) % self.tabs.len(), window, cx);
    }
  }

  pub fn activate_prev(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.tabs.is_empty() {
      self.activate(
        (self.active + self.tabs.len() - 1) % self.tabs.len(),
        window,
        cx,
      );
    }
  }

  fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
    if let Some(tab) = self.tabs.get(self.active) {
      window.focus(&tab.focus_handle(cx));
    }
  }

  pub fn count_of(&self, kind: SessionKind, cx: &App) -> usize {
    self
      .terminals()
      .filter(|(_, view)| view.read(cx).kind == kind)
      .count()
  }
}

pub fn status_color(status: AgentStatus, theme: &Theme) -> gpui::Hsla {
  match status {
    AgentStatus::Running => theme.green,
    AgentStatus::Waiting => theme.yellow,
    AgentStatus::Thinking => theme.purple,
    AgentStatus::Idle => theme.text_dim,
    AgentStatus::Error => theme.red,
    AgentStatus::Finished => theme.blue,
  }
}

impl Render for Workspace {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let mut tab_bar = div()
      .id("tab-bar")
      .window_control_area(gpui::WindowControlArea::Drag)
      .flex()
      .flex_none()
      .items_center()
      .h(px(HEADER_HEIGHT))
      .px_2()
      .gap_1()
      .border_b_1()
      .border_color(theme.panel_border);

    if !self.left_sidebar_open {
      tab_bar = tab_bar.child(div().w(px(68.0)).flex_none()).child(
        icon_button("reopen-left", IconName::PanelLeftOpen, &theme).on_click(|_, window, cx| {
          window.dispatch_action(Box::new(helix_commands::ToggleLeftSidebar), cx);
        }),
      );
    }

    let tab_bar = tab_bar
      .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
        let is_active = ix == self.active;
        let title = tab.title(cx);
        let (kind_icon, icon_color) = tab.icon(cx, &theme);
        let preview = tab.preview;
        div()
          .id(SharedString::from(format!("tab-{ix}")))
          .flex()
          .items_center()
          .gap_1p5()
          .px_2()
          .h(px(24.0))
          .rounded_md()
          .border_1()
          .cursor_pointer()
          .when(is_active, |el| {
            el.border_color(theme.active)
              .bg(theme.elevated)
              .text_color(theme.text)
          })
          .when(!is_active, |el| {
            el.border_color(gpui::transparent_black())
              .text_color(theme.text_dim)
              .hover(|s| s.bg(theme.hover))
          })
          .when(preview, |el| el.italic())
          .text_xs()
          .on_click(
            cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
              if event.click_count() >= 2 {
                if let Some(tab) = this.tabs.get_mut(ix) {
                  tab.preview = false;
                }
              }
              this.activate(ix, window, cx);
            }),
          )
          .child(
            div()
              .flex_none()
              .text_color(icon_color)
              .child(kind_icon.size_3p5()),
          )
          .child(title)
          .child(
            div()
              .id(SharedString::from(format!("tab-close-{ix}")))
              .ml_0p5()
              .size(px(14.0))
              .flex()
              .items_center()
              .justify_center()
              .rounded_sm()
              .text_xs()
              .text_color(theme.text_dim)
              .hover(|s| s.bg(theme.hover).text_color(theme.text))
              .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_tab(ix, window, cx);
              }))
              .child(Icon::new(IconName::Close).size_3()),
          )
      }))
      .child(
        div()
          .relative()
          .child(
            icon_button("new-tab", IconName::Plus, &theme).on_click(cx.listener(
              |this, _, _, cx| {
                this.new_menu_open = !this.new_menu_open;
                cx.notify();
              },
            )),
          )
          .when(self.new_menu_open, |el| {
            el.child(gpui::deferred(
              div()
                .id("new-tab-menu")
                .occlude()
                .absolute()
                .top(px(26.0))
                .left_0()
                .w(px(190.0))
                .rounded_lg()
                .border_1()
                .border_color(theme.panel_border)
                .bg(crate::theme::ca(0x1a1a1af5))
                .shadow_lg()
                .py_1()
                .flex()
                .flex_col()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                  this.new_menu_open = false;
                  cx.notify();
                }))
                .child(
                  div()
                    .id("new-menu-terminal")
                    .flex()
                    .items_center()
                    .gap_2()
                    .mx_1()
                    .px_2()
                    .h(px(26.0))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                      this.new_menu_open = false;
                      this.open_tab(SessionKind::Terminal, window, cx);
                    }))
                    .child(
                      div()
                        .flex_none()
                        .text_color(theme.text_muted)
                        .child(Icon::new(IconName::SquareTerminal).size_3p5()),
                    )
                    .child(
                      div()
                        .flex_1()
                        .text_sm()
                        .text_color(theme.text)
                        .child("Terminal"),
                    )
                    .child(div().text_xs().text_color(theme.text_dim).child("⌘T")),
                )
                .child(
                  div()
                    .id("new-menu-claude")
                    .flex()
                    .items_center()
                    .gap_2()
                    .mx_1()
                    .px_2()
                    .h(px(26.0))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                      this.new_menu_open = false;
                      this.open_tab(SessionKind::ClaudeCode, window, cx);
                    }))
                    .child(
                      div()
                        .flex_none()
                        .text_color(theme.claude)
                        .child(Icon::new(IconName::Asterisk).size_3p5()),
                    )
                    .child(
                      div()
                        .flex_1()
                        .text_sm()
                        .text_color(theme.text)
                        .child("Claude Code"),
                    )
                    .child(div().text_xs().text_color(theme.text_dim).child("⌘⇧T")),
                ),
            ))
          }),
      )
      .child(div().flex_1())
      .child(
        icon_button("toggle-right", IconName::PanelRight, &theme).on_click(|_, window, cx| {
          window.dispatch_action(Box::new(helix_commands::ToggleRightSidebar), cx);
        }),
      );

    let content: AnyElement = if let Some(tab) = self.tabs.get(self.active) {
      div()
        .flex_1()
        .min_h_0()
        .child(tab.element())
        .into_any_element()
    } else {
      div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(theme.text_dim)
        .child(div().text_lg().child("No sessions"))
        .child(
          div()
            .text_sm()
            .child("⌘T new terminal · ⌘⇧T new Claude session"),
        )
        .into_any_element()
    };

    div()
      .flex()
      .flex_col()
      .size_full()
      .min_w_0()
      .child(tab_bar)
      .child(content)
  }
}
