use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
  ParentElement, Render, SharedString, Window, div, img, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName};
use helix_buffer::FileContent;
use std::path::{Path, PathBuf};

const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 19.0;

pub enum EditorViewEvent {
  DirtyChanged,
  Saved,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ExternalMutation {
  Changed,
  Deleted,
}

enum Body {
  Text(Entity<InputState>),
  Image { path: PathBuf },
  Binary,
  TooLarge { bytes: u64 },
  Error(String),
}

pub struct EditorView {
  pub path: PathBuf,
  pub title: SharedString,
  body: Body,
  disk_signature: Option<u64>,
  dirty: bool,
  external: Option<ExternalMutation>,
  save_error: Option<String>,
  focus_handle: FocusHandle,
}

impl EventEmitter<EditorViewEvent> for EditorView {}

impl Focusable for EditorView {
  fn focus_handle(&self, cx: &App) -> FocusHandle {
    match &self.body {
      Body::Text(state) => state.read(cx).focus_handle(cx),
      _ => self.focus_handle.clone(),
    }
  }
}

impl EditorView {
  pub fn new(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
    let title = path
      .file_name()
      .map(|name| name.to_string_lossy().to_string())
      .unwrap_or_else(|| path.display().to_string());

    let mut view = Self {
      path,
      title: title.into(),
      body: Body::Binary,
      disk_signature: None,
      dirty: false,
      external: None,
      save_error: None,
      focus_handle: cx.focus_handle(),
    };
    view.load(window, cx);
    view
  }

  pub fn is_dirty(&self) -> bool {
    self.dirty
  }

  pub fn external_mutation(&self) -> Option<ExternalMutation> {
    self.external
  }

  fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    let language = helix_buffer::language::of(&self.path);
    match helix_buffer::read(&self.path) {
      Ok(FileContent::Text { text, signature }) => {
        self.disk_signature = Some(signature);
        match &self.body {
          Body::Text(state) => {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
          }
          _ => {
            let state = cx.new(|cx| {
              let mut state = InputState::new(window, cx)
                .code_editor(language)
                .soft_wrap(false);
              state.set_value(text, window, cx);
              state
            });
            cx.subscribe(&state, Self::handle_input_event).detach();
            self.body = Body::Text(state);
          }
        }
      }
      Ok(FileContent::Image { .. }) => {
        self.body = Body::Image {
          path: self.path.clone(),
        };
      }
      Ok(FileContent::Binary) => self.body = Body::Binary,
      Ok(FileContent::TooLarge { bytes }) => self.body = Body::TooLarge { bytes },
      Err(err) => self.body = Body::Error(err.to_string()),
    }
    self.dirty = false;
    self.external = None;
    self.save_error = None;
    cx.notify();
  }

  fn handle_input_event(
    &mut self,
    _state: Entity<InputState>,
    event: &InputEvent,
    cx: &mut Context<Self>,
  ) {
    if !matches!(event, InputEvent::Change) {
      return;
    }
    let dirty = self.buffer_signature(cx) != self.disk_signature;
    if dirty != self.dirty {
      self.dirty = dirty;
      cx.emit(EditorViewEvent::DirtyChanged);
    }
    cx.notify();
  }

  fn buffer_signature(&self, cx: &App) -> Option<u64> {
    match &self.body {
      Body::Text(state) => Some(helix_buffer::signature::of(state.read(cx).value().as_ref())),
      _ => None,
    }
  }

  pub fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    let Body::Text(state) = &self.body else {
      return;
    };
    let text = state.read(cx).value().to_string();
    match helix_buffer::write(&self.path, &text) {
      Ok(signature) => {
        self.disk_signature = Some(signature);
        self.dirty = false;
        self.external = None;
        self.save_error = None;
        cx.emit(EditorViewEvent::Saved);
        cx.emit(EditorViewEvent::DirtyChanged);
      }
      Err(err) => self.save_error = Some(err.to_string()),
    }
    cx.notify();
  }

  pub fn note_external_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    if !self.path.exists() {
      if self.dirty {
        self.external = Some(ExternalMutation::Deleted);
        cx.notify();
      }
      return;
    }

    let disk = match helix_buffer::read(&self.path) {
      Ok(FileContent::Text { signature, .. }) => Some(signature),
      _ => None,
    };
    if disk.is_some() && disk == self.disk_signature {
      return;
    }

    if self.dirty {
      self.external = Some(ExternalMutation::Changed);
      cx.notify();
    } else {
      self.load(window, cx);
    }
  }

  pub fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.load(window, cx);
  }

  pub fn keep_local_edits(&mut self, cx: &mut Context<Self>) {
    if let Ok(FileContent::Text { signature, .. }) = helix_buffer::read(&self.path) {
      self.disk_signature = Some(signature);
      self.dirty = self.buffer_signature(cx) != self.disk_signature;
    }
    self.external = None;
    cx.notify();
  }

  fn render_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
    let mutation = self.external?;
    let (message, color) = match mutation {
      ExternalMutation::Changed => ("Changed on disk while you had unsaved edits.", theme.yellow),
      ExternalMutation::Deleted => (
        "Deleted on disk. Your unsaved edits are still here.",
        theme.red,
      ),
    };
    let mut banner = div()
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .px_3()
      .py_2()
      .border_b_1()
      .border_color(theme.panel_border)
      .bg(theme.elevated)
      .child(
        div()
          .flex_none()
          .text_color(color)
          .child(Icon::new(IconName::TriangleAlert).size_3p5()),
      )
      .child(
        div()
          .flex_1()
          .text_xs()
          .text_color(theme.text)
          .child(message),
      );

    if mutation == ExternalMutation::Changed {
      banner = banner.child(
        div()
          .id("editor-reload")
          .px_2()
          .py_1()
          .rounded_md()
          .text_xs()
          .cursor_pointer()
          .text_color(theme.text_muted)
          .hover(|s| s.bg(theme.hover).text_color(theme.text))
          .on_click(cx.listener(|this, _, window, cx| this.reload_from_disk(window, cx)))
          .child("Reload from disk"),
      );
    }

    Some(
      banner
        .child(
          div()
            .id("editor-keep")
            .px_2()
            .py_1()
            .rounded_md()
            .text_xs()
            .cursor_pointer()
            .text_color(theme.text_muted)
            .hover(|s| s.bg(theme.hover).text_color(theme.text))
            .on_click(cx.listener(|this, _, _, cx| this.keep_local_edits(cx)))
            .child("Keep my edits"),
        )
        .into_any_element(),
    )
  }

  fn placeholder(theme: &Theme, message: String) -> AnyElement {
    div()
      .flex_1()
      .flex()
      .items_center()
      .justify_center()
      .text_sm()
      .text_color(theme.text_dim)
      .child(message)
      .into_any_element()
  }
}

impl Render for EditorView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    let banner = self.render_banner(&theme, cx);

    let body: AnyElement = match &self.body {
      Body::Text(state) => div()
        .flex_1()
        .min_h_0()
        .child(
          Input::new(state)
            .h_full()
            .appearance(false)
            .bordered(false)
            .p_0(),
        )
        .into_any_element(),
      Body::Image { path } => div()
        .flex_1()
        .min_h_0()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .child(img(path.clone()).max_w_full().max_h_full())
        .into_any_element(),
      Body::Binary => Self::placeholder(&theme, "Binary file — cannot display".to_string()),
      Body::TooLarge { bytes } => Self::placeholder(
        &theme,
        format!(
          "File too large: {:.1} MB exceeds {} MB limit",
          *bytes as f64 / (1024.0 * 1024.0),
          helix_buffer::MAX_TEXT_FILE_BYTES / (1024 * 1024)
        ),
      ),
      Body::Error(err) => Self::placeholder(&theme, format!("Could not open file: {err}")),
    };

    let status = self.save_error.clone().map(|err| {
      div()
        .flex_none()
        .px_3()
        .py_1()
        .text_xs()
        .text_color(theme.red)
        .child(format!("save failed: {err}"))
    });

    div()
      .key_context("Editor")
      .track_focus(&self.focus_handle)
      .size_full()
      .flex()
      .flex_col()
      .min_h_0()
      .font_family(theme.font_mono.clone())
      .text_size(px(FONT_SIZE))
      .line_height(px(LINE_HEIGHT))
      .on_action(
        cx.listener(|this, _: &helix_commands::SaveFile, window, cx| this.save(window, cx)),
      )
      .children(banner)
      .child(body)
      .children(status)
  }
}

pub fn relative_label(root: &Path, path: &Path) -> String {
  path
    .strip_prefix(root)
    .unwrap_or(path)
    .to_string_lossy()
    .to_string()
}
