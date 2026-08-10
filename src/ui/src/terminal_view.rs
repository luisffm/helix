use crate::ansi_colors;
use crate::keymap::to_pty_bytes;
use crate::theme::Theme;
use alacritty_terminal::event::Event as AlacEvent;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor};
use gpui::{
  App, ClipboardItem, Context, EventEmitter, FocusHandle, Focusable, Font, FontFallbacks,
  FontStyle, FontWeight, Hsla, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
  MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, ScrollWheelEvent,
  SharedString, StyledText, TextRun, UnderlineStyle, Window, canvas, div, font, point, prelude::*,
  px,
};
use helix_agents::launch_spec;
use helix_models::{AgentStatus, SessionId, SessionKind};
use helix_terminal::{SpawnOptions, TerminalBackend};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const DRAIN_LIMIT: usize = 512;
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const ACTIVITY_EMIT_INTERVAL: Duration = Duration::from_secs(1);
const DETECT_INTERVAL: Duration = Duration::from_secs(5);

pub enum TerminalViewEvent {
  Activity,
  Retitled,
  Exited(i32),
}

pub struct TerminalView {
  pub id: SessionId,
  pub kind: SessionKind,
  pub title: SharedString,
  pub exited: Option<i32>,
  pub last_activity: Instant,
  pub started_at: SystemTime,
  backend: Option<Arc<TerminalBackend>>,
  claude_detected: bool,
  flush_pending: bool,
  last_activity_emit: Instant,
  spawn_error: Option<String>,
  focus_handle: FocusHandle,
  cols: u16,
  rows: u16,
  cell_width: Pixels,
  line_height: Pixels,
  font_size: Pixels,
  origin: Point<Pixels>,
  scroll_accum: f32,
  selecting: bool,
  select_anchor: Option<(usize, i32)>,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl Focusable for TerminalView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

fn is_claude_process(pgid: i32) -> bool {
  let Ok(output) = std::process::Command::new("ps")
    .args(["-o", "comm=", "-o", "args=", "-p", &pgid.to_string()])
    .output()
  else {
    return false;
  };

  let text = String::from_utf8_lossy(&output.stdout).to_lowercase();

  text
    .split_whitespace()
    .take(4)
    .any(|token| token.rsplit('/').next() == Some("claude"))
}

struct RunFrame {
  start_col: usize,
  cols: usize,
  text: String,
  fg: Hsla,
  bg: Option<Hsla>,
  font: Font,
  underline: Option<UnderlineStyle>,
  mergeable: bool,
}

struct LineFrame {
  runs: Vec<RunFrame>,
}

struct Frame {
  lines: Vec<LineFrame>,
  cursor: Option<(i32, usize)>,
  display_offset: usize,
}

impl TerminalView {
  pub fn new(
    id: SessionId,
    kind: SessionKind,
    title: String,
    cwd: std::path::PathBuf,
    cx: &mut Context<Self>,
  ) -> Self {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let spec = launch_spec(kind);

    let (backend, spawn_error) = match TerminalBackend::spawn(
      SpawnOptions {
        program: spec.program,
        args: spec.args,
        cwd,
        cols: 80,
        rows: 24,
        cell_width: 8,
        cell_height: 18,
      },
      tx,
    ) {
      Ok(backend) => (Some(Arc::new(backend)), None),
      Err(err) => (None, Some(err.to_string())),
    };

    cx.spawn(async move |this, cx| {
      while let Some(event) = rx.recv().await {
        let mut batch = vec![event];

        while batch.len() < DRAIN_LIMIT {
          let Ok(next) = rx.try_recv() else {
            break;
          };

          batch.push(next);
        }

        let applied = this.update(cx, |view, cx| {
          for event in batch {
            view.handle_alac_event(event, cx);
          }
        });

        if applied.is_err() {
          break;
        }
      }
    })
    .detach();

    if kind == SessionKind::Terminal {
      Self::spawn_claude_detector(cx);
    }

    let font_size = helix_state::config::load()
      .terminal_font_size
      .unwrap_or(13.0)
      .clamp(9.0, 22.0);

    Self {
      id,
      kind,
      title: title.into(),
      exited: None,
      last_activity: Instant::now(),
      started_at: SystemTime::now(),
      backend,
      claude_detected: false,
      flush_pending: false,
      last_activity_emit: Instant::now(),
      spawn_error,
      focus_handle: cx.focus_handle(),
      cols: 80,
      rows: 24,
      cell_width: px(8.0),
      line_height: px(font_size * 1.45),
      font_size: px(font_size),
      origin: point(px(0.0), px(0.0)),
      scroll_accum: 0.0,
      selecting: false,
      select_anchor: None,
    }
  }

  pub fn status(&self) -> AgentStatus {
    helix_state::activity_status(self.last_activity, self.exited)
  }

  pub fn shell_pid(&self) -> Option<u32> {
    self
      .backend
      .as_ref()
      .and_then(|backend| backend.shell_pid())
  }

  pub fn agent_kind(&self) -> SessionKind {
    if self.kind == SessionKind::ClaudeCode || self.claude_detected {
      SessionKind::ClaudeCode
    } else {
      SessionKind::Terminal
    }
  }

  fn spawn_claude_detector(cx: &mut Context<Self>) {
    cx.spawn(async move |this, cx| {
      let mut probed: Option<i32> = None;

      loop {
        cx.background_executor().timer(DETECT_INTERVAL).await;

        let Ok((alive, probe)) = this.update(cx, |view, _| {
          if view.exited.is_some() {
            return (false, None);
          }

          let probe = view.backend.as_ref().and_then(|backend| {
            let pgid = backend.foreground_pgid()?;
            let shell_pid = backend.shell_pid()?;

            (pgid as u32 != shell_pid).then_some(pgid)
          });

          (true, probe)
        }) else {
          break;
        };

        if !alive {
          break;
        }

        if probe == probed {
          continue;
        }

        probed = probe;

        let detected = match probe {
          None => false,
          Some(pgid) => {
            cx.background_executor()
              .spawn(async move { is_claude_process(pgid) })
              .await
          }
        };

        if this
          .update(cx, |view, cx| {
            if view.claude_detected != detected {
              view.claude_detected = detected;

              cx.emit(TerminalViewEvent::Activity);
              cx.notify();
            }
          })
          .is_err()
        {
          break;
        }
      }
    })
    .detach();
  }

  pub fn activity_ago(&self) -> String {
    let secs = self.last_activity.elapsed().as_secs();

    match secs {
      0..=9 => "now".to_string(),
      10..=59 => format!("{secs}s"),
      60..=3599 => format!("{}m", secs / 60),
      _ => format!("{}h", secs / 3600),
    }
  }

  fn write_bytes(&self, bytes: Vec<u8>) {
    if let Some(backend) = &self.backend {
      backend.write(bytes);
    }
  }

  fn schedule_flush(&mut self, cx: &mut Context<Self>) {
    if self.flush_pending {
      return;
    }

    self.flush_pending = true;

    cx.spawn(async move |this, cx| {
      cx.background_executor().timer(FLUSH_INTERVAL).await;

      this
        .update(cx, |view, cx| {
          view.flush_pending = false;

          if view.last_activity_emit.elapsed() >= ACTIVITY_EMIT_INTERVAL {
            view.last_activity_emit = Instant::now();

            cx.emit(TerminalViewEvent::Activity);
          }

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  fn handle_alac_event(&mut self, event: AlacEvent, cx: &mut Context<Self>) {
    match event {
      AlacEvent::Wakeup => {
        self.last_activity = Instant::now();

        self.schedule_flush(cx);
      }
      AlacEvent::Title(title) => {
        self.title = title.into();
        cx.emit(TerminalViewEvent::Retitled);
        cx.notify();
      }
      AlacEvent::ChildExit(status) => {
        let code = status.code().unwrap_or(-1);

        self.exited = Some(code);

        cx.emit(TerminalViewEvent::Exited(code));
        cx.notify();
      }
      AlacEvent::PtyWrite(text) => self.write_bytes(text.into_bytes()),
      AlacEvent::ClipboardStore(_, text) => {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
      }
      AlacEvent::ClipboardLoad(_, format) => {
        let text = cx
          .read_from_clipboard()
          .and_then(|item| item.text())
          .unwrap_or_default();

        self.write_bytes(format(&text).into_bytes());
      }
      AlacEvent::ColorRequest(index, format) => {
        let rgb = ansi_colors::color_for_osc_index(index, Theme::of(cx));

        self.write_bytes(format(rgb).into_bytes());
      }
      _ => {}
    }
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    let Some(backend) = self.backend.clone() else {
      return;
    };

    let mode = backend.mode();

    if let Some(bytes) = to_pty_bytes(&event.keystroke, mode) {
      backend.clear_selection();
      backend.scroll_to_bottom();
      backend.write(bytes);

      self.last_activity = Instant::now();

      cx.notify();
      cx.stop_propagation();
    }
  }

  fn copy(&mut self, _: &helix_commands::TerminalCopy, _: &mut Window, cx: &mut Context<Self>) {
    if let Some(text) = self.backend.as_ref().and_then(|b| b.selection_text()) {
      if !text.is_empty() {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
      }
    }
  }

  fn paste(&mut self, _: &helix_commands::TerminalPaste, _: &mut Window, cx: &mut Context<Self>) {
    let Some(backend) = self.backend.clone() else {
      return;
    };

    let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
      return;
    };

    let payload = if backend.mode().contains(TermMode::BRACKETED_PASTE) {
      format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
    } else {
      text.replace("\r\n", "\r").replace('\n', "\r")
    };

    backend.scroll_to_bottom();
    backend.write(payload.into_bytes());

    cx.notify();
  }

  fn grid_position(&self, position: Point<Pixels>) -> (usize, i32, Side) {
    let x = (position.x - self.origin.x).max(px(0.0));
    let y = (position.y - self.origin.y).max(px(0.0));
    let col_f = x / self.cell_width;
    let col = (col_f as usize).min(self.cols.saturating_sub(1) as usize);
    let row = ((y / self.line_height) as i32).min(self.rows as i32 - 1);

    let side = if col_f.fract() > 0.5 {
      Side::Right
    } else {
      Side::Left
    };

    (col, row, side)
  }

  fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    window.focus(&self.focus_handle);

    let Some(backend) = self.backend.clone() else {
      return;
    };

    let (col, row, side) = self.grid_position(event.position);
    let grid_point = backend.viewport_to_point(col, row);

    match event.click_count {
      1 => {
        backend.clear_selection();

        self.select_anchor = Some((col, row));
        self.selecting = true;
      }
      2 => {
        backend.start_selection(grid_point, side, SelectionType::Semantic);
        self.selecting = true;
      }
      _ => {
        backend.start_selection(grid_point, side, SelectionType::Lines);
        self.selecting = true;
      }
    }

    cx.notify();
  }

  fn on_mouse_move(
    &mut self,
    event: &MouseMoveEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
      return;
    }

    let Some(backend) = self.backend.clone() else {
      return;
    };

    let (col, row, side) = self.grid_position(event.position);
    let grid_point = backend.viewport_to_point(col, row);

    if !backend.has_selection() {
      if let Some((anchor_col, anchor_row)) = self.select_anchor {
        if (anchor_col, anchor_row) == (col, row) {
          return;
        }

        let anchor_point = backend.viewport_to_point(anchor_col, anchor_row);

        backend.start_selection(anchor_point, Side::Left, SelectionType::Simple);
      }
    }

    backend.update_selection(grid_point, side);

    cx.notify();
  }

  fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
    self.selecting = false;
    self.select_anchor = None;

    cx.notify();
  }

  fn on_scroll_wheel(
    &mut self,
    event: &ScrollWheelEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(backend) = self.backend.clone() else {
      return;
    };

    let delta = event.delta.pixel_delta(self.line_height).y / self.line_height;

    self.scroll_accum += delta;

    let lines = self.scroll_accum as i32;

    if lines != 0 {
      self.scroll_accum -= lines as f32;

      backend.scroll_lines(lines);
      cx.notify();
    }
  }

  fn sync_layout(
    &mut self,
    size: gpui::Size<Pixels>,
    origin: Point<Pixels>,
    cx: &mut Context<Self>,
  ) {
    self.origin = origin;

    let cols = (size.width / self.cell_width) as u16;
    let rows = (size.height / self.line_height) as u16;

    if (cols, rows) != (self.cols, self.rows) && cols >= 2 && rows >= 2 {
      self.cols = cols;
      self.rows = rows;

      if let Some(backend) = &self.backend {
        backend.resize(
          cols,
          rows,
          f32::from(self.cell_width) as u16,
          f32::from(self.line_height) as u16,
        );
      }

      cx.notify();
    }
  }

  fn base_font(&self, theme: &Theme) -> Font {
    let mut base = font(theme.font_mono.clone());

    base.fallbacks = Some(FontFallbacks::from_fonts(vec![
      "Symbols Nerd Font Mono".to_string(),
      "Apple Color Emoji".to_string(),
      "Apple Symbols".to_string(),
    ]));

    base
  }

  fn build_frame(&self, theme: &Theme) -> Option<Frame> {
    let backend = self.backend.as_ref()?;
    let base_font = self.base_font(theme);

    Some(backend.with_term(|term| {
      let content = term.renderable_content();
      let display_offset = content.display_offset;
      let selection = content.selection;

      let mut lines: Vec<LineFrame> = Vec::new();
      let mut current_row: Option<i32> = None;

      for indexed in content.display_iter {
        let row = indexed.point.line.0 + display_offset as i32;

        if current_row != Some(row) {
          current_row = Some(row);
          lines.push(LineFrame { runs: Vec::new() });
        }

        let line = lines.last_mut().unwrap();
        let cell = &indexed.cell;

        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
          continue;
        }

        let col = indexed.point.column.0;

        let width = if cell.flags.contains(Flags::WIDE_CHAR) {
          2
        } else {
          1
        };

        let ch = if cell.flags.contains(Flags::HIDDEN) {
          ' '
        } else {
          cell.c
        };

        let mut fg = ansi_colors::to_hsla(cell.fg, content.colors, theme);
        let mut bg = match cell.bg {
          AnsiColor::Named(NamedColor::Background) => None,
          other => Some(ansi_colors::to_hsla(other, content.colors, theme)),
        };

        if cell.flags.contains(Flags::INVERSE) {
          let old_fg = fg;

          fg = bg.unwrap_or(theme.bg);
          bg = Some(old_fg);
        }

        if selection.map_or(false, |range| range.contains(indexed.point)) {
          bg = Some(theme.term.selection);
        }

        if cell.flags.contains(Flags::DIM) {
          fg.a *= 0.6;
        }

        let mut run_font = base_font.clone();

        if cell.flags.contains(Flags::BOLD) {
          run_font.weight = FontWeight::BOLD;
        }

        if cell.flags.contains(Flags::ITALIC) {
          run_font.style = FontStyle::Italic;
        }

        let underline = if cell.flags.intersects(Flags::ALL_UNDERLINES) {
          Some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(fg),
            wavy: cell.flags.contains(Flags::UNDERCURL),
          })
        } else {
          None
        };

        let mergeable = width == 1;

        match line.runs.last_mut() {
          Some(run)
            if run.mergeable
              && mergeable
              && run.start_col + run.cols == col
              && run.fg == fg
              && run.bg == bg
              && run.font == run_font
              && run.underline == underline =>
          {
            run.text.push(ch);
            run.cols += 1;
          }
          _ => line.runs.push(RunFrame {
            start_col: col,
            cols: width,
            text: ch.to_string(),
            fg,
            bg,
            font: run_font,
            underline,
            mergeable,
          }),
        }
      }

      let cursor = if content.cursor.shape == CursorShape::Hidden {
        None
      } else {
        let row = content.cursor.point.line.0 + display_offset as i32;

        if row >= 0 && row < self.rows as i32 {
          Some((row, content.cursor.point.column.0))
        } else {
          None
        }
      };

      Frame {
        lines,
        cursor,
        display_offset,
      }
    }))
  }
}

impl Render for TerminalView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let measure_run = TextRun {
      len: 1,
      font: self.base_font(&theme),
      color: theme.text,
      background_color: None,
      underline: None,
      strikethrough: None,
    };

    self.cell_width = window
      .text_system()
      .shape_line("M".into(), self.font_size, &[measure_run], None)
      .width;

    let frame = self.build_frame(&theme);
    let focused = self.focus_handle.is_focused(window);
    let entity = cx.entity().downgrade();
    let padding = px(10.0);

    let mut content = div().relative().size_full().child(
      canvas(
        move |bounds, _window, cx| {
          entity
            .update(cx, |view, cx| {
              view.sync_layout(bounds.size, bounds.origin, cx)
            })
            .ok();
        },
        |_, _, _, _| {},
      )
      .absolute()
      .size_full(),
    );

    if let Some(frame) = frame {
      let line_height = self.line_height;
      let cell_width = self.cell_width;

      content = content.child(
        div()
          .flex()
          .flex_col()
          .children(frame.lines.into_iter().map(|line| {
            div()
              .relative()
              .h(line_height)
              .w_full()
              .children(line.runs.into_iter().filter_map(|run| {
                let visible =
                  !run.text.trim().is_empty() || run.bg.is_some() || run.underline.is_some();
                if !visible {
                  return None;
                }

                let x = cell_width * run.start_col as f32;
                let width = cell_width * run.cols as f32;
                let len = run.text.len();

                let styled = StyledText::new(run.text).with_runs(vec![TextRun {
                  len,
                  font: run.font,
                  color: run.fg,
                  background_color: None,
                  underline: run.underline,
                  strikethrough: None,
                }]);

                Some(
                  div()
                    .absolute()
                    .left(x)
                    .top(px(0.0))
                    .w(width)
                    .h_full()
                    .whitespace_nowrap()
                    .when_some(run.bg, |el, bg| el.bg(bg))
                    .child(styled),
                )
              }))
          })),
      );

      if let Some((row, col)) = frame.cursor {
        if frame.display_offset == 0 {
          let mut cursor_color = theme.term.cursor;
          cursor_color.a = if focused { 0.55 } else { 0.25 };

          content = content.child(
            div()
              .absolute()
              .left(cell_width * col as f32)
              .top(line_height * row as f32)
              .w(cell_width)
              .h(line_height)
              .rounded(px(1.0))
              .bg(cursor_color),
          );
        }
      }
    }

    let mut root = div()
      .id(SharedString::from(format!("terminal-{}", self.id)))
      .key_context("Terminal")
      .track_focus(&self.focus_handle)
      .size_full()
      .overflow_hidden()
      .p(padding)
      .font_family(theme.font_mono.clone())
      .text_size(self.font_size)
      .line_height(self.line_height)
      .text_color(theme.term.fg)
      .cursor_text()
      .on_action(cx.listener(Self::copy))
      .on_action(cx.listener(Self::paste))
      .on_key_down(cx.listener(Self::on_key_down))
      .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
      .on_mouse_move(cx.listener(Self::on_mouse_move))
      .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
      .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
      .child(content);

    if let Some(error) = &self.spawn_error {
      root = root.child(
        div()
          .absolute()
          .inset_0()
          .flex()
          .items_center()
          .justify_center()
          .text_color(theme.red)
          .child(format!("failed to start shell: {error}")),
      );
    } else if let Some(code) = self.exited {
      root = root.child(
        div()
          .absolute()
          .bottom_2()
          .right_2()
          .px_2()
          .py_1()
          .rounded_md()
          .bg(theme.elevated)
          .text_color(if code == 0 {
            theme.text_muted
          } else {
            theme.red
          })
          .text_xs()
          .child(format!("process exited ({code})")),
      );
    }

    root
  }
}
