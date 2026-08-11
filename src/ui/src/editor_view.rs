use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
  ParentElement, Render, SharedString, Window, div, img, prelude::*, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName, Sizable as _};
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
  disk_len: Option<usize>,
  disk_stamp: Option<(std::time::SystemTime, u64)>,
  load_token: u64,
  dirty: bool,
  external: Option<ExternalMutation>,
  save_error: Option<String>,
  focus_handle: FocusHandle,
}

/// Cheap stand-in for hashing the file: a watcher batch that did not move the
/// modification time or the length cannot have changed the content.
fn disk_stamp(path: &Path) -> Option<(std::time::SystemTime, u64)> {
  let metadata = std::fs::metadata(path).ok()?;

  Some((metadata.modified().ok()?, metadata.len()))
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
      disk_len: None,
      disk_stamp: None,
      load_token: 0,
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
    self.disk_stamp = disk_stamp(&self.path);

    let token = self.begin_read();
    let task = self.spawn_read(cx);
    let this = cx.entity().downgrade();

    window
      .spawn(cx, async move |cx| {
        let content = task.await;

        this
          .update_in(cx, |view, window, cx| {
            if view.load_token != token {
              return;
            }

            view.apply_content(content, window, cx);
          })
          .ok();
      })
      .detach();
  }

  fn begin_read(&mut self) -> u64 {
    self.load_token = self.load_token.wrapping_add(1);

    self.load_token
  }

  fn spawn_read(&self, cx: &mut Context<Self>) -> gpui::Task<anyhow::Result<FileContent>> {
    let path = self.path.clone();

    cx.background_executor()
      .spawn(async move { helix_buffer::read(&path) })
  }

  fn apply_content(
    &mut self,
    content: anyhow::Result<FileContent>,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    match content {
      Ok(FileContent::Text { text, signature }) => {
        self.disk_signature = Some(signature);
        self.disk_len = Some(text.len());

        match &self.body {
          Body::Text(state) => {
            state.update(cx, |state, cx| state.set_value(text, window, cx));
          }
          _ => {
            let language = helix_buffer::language::of(&self.path);

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

    let dirty = self.differs_from_disk(cx);

    if dirty != self.dirty {
      self.dirty = dirty;
      cx.emit(EditorViewEvent::DirtyChanged);
    }

    cx.notify();
  }

  /// Hashing the buffer is the one step here that grows with the file, and it
  /// runs on every keystroke, so a byte length that already moved answers the
  /// question without touching the content.
  fn differs_from_disk(&self, cx: &App) -> bool {
    match (self.buffer_len(cx), self.disk_len) {
      (Some(len), Some(disk_len)) if len != disk_len => true,
      _ => self.buffer_signature(cx) != self.disk_signature,
    }
  }

  fn buffer_len(&self, cx: &App) -> Option<usize> {
    match &self.body {
      Body::Text(state) => Some(state.read(cx).text().len()),
      _ => None,
    }
  }

  fn buffer_signature(&self, cx: &App) -> Option<u64> {
    match &self.body {
      Body::Text(state) => Some(helix_buffer::signature::of_chunks(
        state.read(cx).text().chunks(),
      )),
      _ => None,
    }
  }

  pub fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
    let Body::Text(state) = &self.body else {
      return;
    };

    let state = state.clone();
    let (len, written) = {
      let input = state.read(cx);

      (
        input.text().len(),
        helix_buffer::write_chunks(&self.path, input.text().chunks()),
      )
    };

    match written {
      Ok(signature) => {
        self.disk_signature = Some(signature);
        self.disk_len = Some(len);
        self.disk_stamp = disk_stamp(&self.path);
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
    let Some(stamp) = disk_stamp(&self.path) else {
      if self.dirty {
        self.external = Some(ExternalMutation::Deleted);
        cx.notify();
      }

      return;
    };

    if Some(stamp) == self.disk_stamp {
      return;
    }

    self.disk_stamp = Some(stamp);

    let token = self.begin_read();
    let task = self.spawn_read(cx);
    let this = cx.entity().downgrade();

    window
      .spawn(cx, async move |cx| {
        let content = task.await;

        this
          .update_in(cx, |view, window, cx| {
            if view.load_token != token {
              return;
            }

            let signature = match &content {
              Ok(FileContent::Text { signature, .. }) => Some(*signature),
              _ => None,
            };

            if signature.is_some() && signature == view.disk_signature {
              return;
            }

            if view.dirty {
              view.external = Some(ExternalMutation::Changed);
              cx.notify();
            } else {
              view.apply_content(content, window, cx);
            }
          })
          .ok();
      })
      .detach();
  }

  pub fn reload_from_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
    self.load(window, cx);
  }

  pub fn keep_local_edits(&mut self, cx: &mut Context<Self>) {
    if let Ok(FileContent::Text { text, signature }) = helix_buffer::read(&self.path) {
      self.disk_signature = Some(signature);
      self.disk_len = Some(text.len());
      self.dirty = self.differs_from_disk(cx);
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
        Button::new("editor-reload")
          .label("Reload from disk")
          .ghost()
          .xsmall()
          .text_color(theme.text_muted)
          .on_click(cx.listener(|this, _, window, cx| this.reload_from_disk(window, cx))),
      );
    }

    Some(
      banner
        .child(
          Button::new("editor-keep")
            .label("Keep my edits")
            .ghost()
            .xsmall()
            .text_color(theme.text_muted)
            .on_click(cx.listener(|this, _, _, cx| this.keep_local_edits(cx))),
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
