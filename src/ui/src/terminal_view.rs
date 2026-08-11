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
  App, ClipboardItem, Context, EventEmitter, ExternalPaths, FocusHandle, Focusable, Font,
  FontFallbacks, FontStyle, FontWeight, Hsla, IntoElement, KeyDownEvent, Modifiers, MouseButton,
  MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Pixels, Point, Render,
  ScrollWheelEvent, SharedString, StyledText, TextRun, UnderlineStyle, Window, canvas, div, font,
  point, prelude::*, px,
};
use helix_agents::launch_spec;
use helix_models::{AgentStatus, SessionId, SessionKind};
use helix_terminal::mouse::{
  BUTTON_LEFT, BUTTON_MIDDLE, BUTTON_RIGHT, BUTTON_WHEEL_DOWN, BUTTON_WHEEL_UP, MouseReport,
  alternate_scroll, encode as encode_mouse, reports_motion,
};
use helix_terminal::shell::quote_path;
use helix_terminal::{SpawnOptions, TerminalBackend};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

const DRAIN_LIMIT: usize = 512;
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
const ACTIVITY_EMIT_INTERVAL: Duration = Duration::from_secs(1);
const DETECT_INTERVAL: Duration = Duration::from_secs(5);
const MOTION_WITHOUT_BUTTON: u8 = 3;

fn pty_mouse_button(button: MouseButton) -> Option<u8> {
  match button {
    MouseButton::Left => Some(BUTTON_LEFT),
    MouseButton::Middle => Some(BUTTON_MIDDLE),
    MouseButton::Right => Some(BUTTON_RIGHT),
    _ => None,
  }
}

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
  last_flush: Instant,
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
  last_motion_cell: Option<(usize, i32)>,
  frame: Option<Frame>,
  frame_key: Option<FrameKey>,
  frame_stale: bool,
}

impl EventEmitter<TerminalViewEvent> for TerminalView {}

impl Focusable for TerminalView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

#[derive(Default)]
struct RunFrame {
  start_col: usize,
  cols: usize,
  text: String,
  fg: Hsla,
  bg: Option<Hsla>,
  style: u8,
  underline: Option<UnderlineStyle>,
  mergeable: bool,
}

/// Rows and runs are reused across frames and `used` marks how much of them the
/// current frame filled, so a streaming terminal reallocates nothing per repaint.
#[derive(Default)]
struct LineFrame {
  runs: Vec<RunFrame>,
  used: usize,
}

impl LineFrame {
  fn runs(&self) -> &[RunFrame] {
    &self.runs[..self.used]
  }

  fn last_run(&mut self) -> Option<&mut RunFrame> {
    self.used.checked_sub(1).map(|ix| &mut self.runs[ix])
  }

  fn next_run(&mut self) -> &mut RunFrame {
    if self.runs.len() == self.used {
      self.runs.push(RunFrame::default());
    }

    self.used += 1;

    &mut self.runs[self.used - 1]
  }
}

#[derive(Default)]
struct Frame {
  lines: Vec<LineFrame>,
  used: usize,
  cursor: Option<(i32, usize)>,
  display_offset: usize,
  mouse_reporting: bool,
}

impl Frame {
  fn lines(&self) -> &[LineFrame] {
    &self.lines[..self.used]
  }

  fn last_line(&mut self) -> &mut LineFrame {
    &mut self.lines[self.used - 1]
  }

  fn next_line(&mut self) -> &mut LineFrame {
    if self.lines.len() == self.used {
      self.lines.push(LineFrame::default());
    }

    self.used += 1;

    let line = &mut self.lines[self.used - 1];
    line.used = 0;

    line
  }
}

/// The four styles a cell can ask for, built once per frame instead of cloned
/// and compared per cell.
struct FontSet([Font; 4]);

impl FontSet {
  fn new(base: Font) -> Self {
    let mut bold = base.clone();
    bold.weight = FontWeight::BOLD;

    let mut italic = base.clone();
    italic.style = FontStyle::Italic;

    let mut bold_italic = italic.clone();
    bold_italic.weight = FontWeight::BOLD;

    Self([base, bold, italic, bold_italic])
  }

  fn get(&self, style: u8) -> Font {
    self.0[style as usize].clone()
  }
}

fn style_of(flags: Flags) -> u8 {
  u8::from(flags.contains(Flags::BOLD)) | (u8::from(flags.contains(Flags::ITALIC)) << 1)
}

/// Everything outside the terminal's own content that changes what a frame
/// looks like. While it holds and no wakeup arrived, the cached frame stands.
#[derive(PartialEq)]
struct FrameKey {
  font: SharedString,
  font_size: Pixels,
  cell_width: Pixels,
  line_height: Pixels,
  bg: Hsla,
  fg: Hsla,
  selection: Hsla,
  cols: u16,
  rows: u16,
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
      last_flush: Instant::now(),
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
      last_motion_cell: None,
      frame: None,
      frame_key: None,
      frame_stale: true,
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
              .spawn(async move { helix_agents::is_claude_process(pgid) })
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
    crate::components::elapsed_label(self.last_activity.elapsed().as_secs())
  }

  fn write_bytes(&self, bytes: Vec<u8>) {
    if let Some(backend) = &self.backend {
      backend.write(bytes);
    }
  }

  /// A burst of pty output is worth one repaint per frame, but the first byte
  /// after an idle stretch should not wait for the window to elapse, so the
  /// leading edge paints at once and only the rest is coalesced.
  fn schedule_flush(&mut self, cx: &mut Context<Self>) {
    if self.flush_pending {
      return;
    }

    if self.last_flush.elapsed() >= FLUSH_INTERVAL {
      self.flush(cx);

      return;
    }

    self.flush_pending = true;

    cx.spawn(async move |this, cx| {
      cx.background_executor().timer(FLUSH_INTERVAL).await;

      this
        .update(cx, |view, cx| {
          view.flush_pending = false;
          view.flush(cx);
        })
        .ok();
    })
    .detach();
  }

  fn flush(&mut self, cx: &mut Context<Self>) {
    self.last_flush = Instant::now();
    self.frame_stale = true;

    if self.last_activity_emit.elapsed() >= ACTIVITY_EMIT_INTERVAL {
      self.last_activity_emit = Instant::now();

      cx.emit(TerminalViewEvent::Activity);
    }

    cx.notify();
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
      self.frame_stale = true;

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

    self.frame_stale = true;

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

  fn reports_mouse(&self, backend: &TerminalBackend, modifiers: &Modifiers) -> bool {
    !modifiers.shift && backend.mode().intersects(TermMode::MOUSE_MODE)
  }

  fn report_mouse(
    &self,
    backend: &TerminalBackend,
    button: u8,
    position: Point<Pixels>,
    pressed: bool,
    motion: bool,
    modifiers: &Modifiers,
  ) {
    let (col, row, _) = self.grid_position(position);
    let report = MouseReport {
      button,
      col,
      row: row.max(0) as usize,
      pressed,
      motion,
      shift: modifiers.shift,
      alt: modifiers.alt,
      ctrl: modifiers.control,
    };

    if let Some(bytes) = encode_mouse(report, backend.mode()) {
      backend.write(bytes);
    }
  }

  fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
    window.focus(&self.focus_handle);

    let Some(backend) = self.backend.clone() else {
      return;
    };

    if let Some(button) = pty_mouse_button(event.button) {
      if self.reports_mouse(&backend, &event.modifiers) {
        self.report_mouse(
          &backend,
          button,
          event.position,
          true,
          false,
          &event.modifiers,
        );

        cx.stop_propagation();

        return;
      }
    }

    if event.button != MouseButton::Left {
      return;
    }

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

    self.frame_stale = true;

    cx.notify();
  }

  fn on_mouse_move(
    &mut self,
    event: &MouseMoveEvent,
    _window: &mut Window,
    cx: &mut Context<Self>,
  ) {
    let Some(backend) = self.backend.clone() else {
      return;
    };

    if self.reports_mouse(&backend, &event.modifiers) {
      let pressed = event.pressed_button.is_some();

      if !reports_motion(backend.mode(), pressed) {
        return;
      }

      let cell = self.grid_position(event.position);

      if self.last_motion_cell == Some((cell.0, cell.1)) {
        return;
      }

      self.last_motion_cell = Some((cell.0, cell.1));

      let button = event
        .pressed_button
        .and_then(pty_mouse_button)
        .unwrap_or(MOTION_WITHOUT_BUTTON);

      self.report_mouse(
        &backend,
        button,
        event.position,
        true,
        true,
        &event.modifiers,
      );

      return;
    }

    if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
      return;
    }

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

    self.frame_stale = true;

    cx.notify();
  }

  fn on_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
    if let Some(backend) = self.backend.clone() {
      if let Some(button) = pty_mouse_button(event.button) {
        if self.reports_mouse(&backend, &event.modifiers) {
          self.report_mouse(
            &backend,
            button,
            event.position,
            false,
            false,
            &event.modifiers,
          );

          self.last_motion_cell = None;

          cx.stop_propagation();

          return;
        }
      }

      if self.selecting {
        if let Some(text) = backend.selection_text().filter(|text| !text.is_empty()) {
          cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
      }
    }

    self.selecting = false;
    self.select_anchor = None;

    cx.notify();
  }

  fn on_drop_paths(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
    let Some(backend) = self.backend.clone() else {
      return;
    };

    let text = paths
      .paths()
      .iter()
      .map(|path| quote_path(path))
      .collect::<Vec<_>>()
      .join(" ");

    if text.is_empty() {
      return;
    }

    window.focus(&self.focus_handle);

    let payload = if backend.mode().contains(TermMode::BRACKETED_PASTE) {
      format!("\x1b[200~{text}\x1b[201~")
    } else {
      text
    };

    backend.scroll_to_bottom();
    backend.write(payload.into_bytes());

    self.frame_stale = true;

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

    if lines == 0 {
      return;
    }

    self.scroll_accum -= lines as f32;

    let mode = backend.mode();

    if self.reports_mouse(&backend, &event.modifiers) {
      let button = if lines > 0 {
        BUTTON_WHEEL_UP
      } else {
        BUTTON_WHEEL_DOWN
      };

      for _ in 0..lines.unsigned_abs() {
        self.report_mouse(
          &backend,
          button,
          event.position,
          true,
          false,
          &event.modifiers,
        );
      }

      return;
    }

    if let Some(bytes) = alternate_scroll(lines, mode) {
      backend.write(bytes);

      return;
    }

    backend.scroll_lines(lines);

    self.frame_stale = true;

    cx.notify();
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

  fn build_frame(&mut self, theme: &Theme) {
    let Some(backend) = self.backend.clone() else {
      self.frame = None;

      return;
    };

    let rows = self.rows as i32;
    let mut frame = self.frame.take().unwrap_or_default();

    frame.used = 0;

    backend.with_term(|term| {
      let content = term.renderable_content();
      let content_mode = content.mode;
      let display_offset = content.display_offset;
      let selection = content.selection;
      let colors = ansi_colors::ColorTable::new(theme, content.colors);

      let mut current_row: Option<i32> = None;

      for indexed in content.display_iter {
        let row = indexed.point.line.0 + display_offset as i32;

        let line = if current_row != Some(row) {
          current_row = Some(row);

          frame.next_line()
        } else {
          frame.last_line()
        };

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

        let mut fg = colors.resolve(cell.fg);
        let mut bg = match cell.bg {
          AnsiColor::Named(NamedColor::Background) => None,
          other => Some(colors.resolve(other)),
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

        let style = style_of(cell.flags);

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

        let merged = match line.last_run() {
          Some(run)
            if run.mergeable
              && mergeable
              && run.start_col + run.cols == col
              && run.fg == fg
              && run.bg == bg
              && run.style == style
              && run.underline == underline =>
          {
            run.text.push(ch);
            run.cols += 1;

            true
          }
          _ => false,
        };

        if !merged {
          let run = line.next_run();

          run.start_col = col;
          run.cols = width;
          run.fg = fg;
          run.bg = bg;
          run.style = style;
          run.underline = underline;
          run.mergeable = mergeable;

          run.text.clear();
          run.text.push(ch);
        }
      }

      frame.cursor = if content.cursor.shape == CursorShape::Hidden {
        None
      } else {
        let row = content.cursor.point.line.0 + display_offset as i32;

        if row >= 0 && row < rows {
          Some((row, content.cursor.point.column.0))
        } else {
          None
        }
      };

      frame.display_offset = display_offset;
      frame.mouse_reporting = content_mode.intersects(TermMode::MOUSE_MODE);
    });

    self.frame = Some(frame);
  }
}

impl Render for TerminalView {
  fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let fonts = FontSet::new(self.base_font(&theme));

    let measure_run = TextRun {
      len: 1,
      font: fonts.get(0),
      color: theme.text,
      background_color: None,
      underline: None,
      strikethrough: None,
    };

    self.cell_width = window
      .text_system()
      .shape_line("M".into(), self.font_size, &[measure_run], None)
      .width;

    let key = FrameKey {
      font: theme.font_mono.clone(),
      font_size: self.font_size,
      cell_width: self.cell_width,
      line_height: self.line_height,
      bg: theme.bg,
      fg: theme.term.fg,
      selection: theme.term.selection,
      cols: self.cols,
      rows: self.rows,
    };

    if self.frame_stale || self.frame.is_none() || self.frame_key.as_ref() != Some(&key) {
      self.build_frame(&theme);

      self.frame_stale = false;
      self.frame_key = Some(key);
    }

    let focused = self.focus_handle.is_focused(window);
    let mouse_reporting = self
      .frame
      .as_ref()
      .is_some_and(|frame| frame.mouse_reporting);
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

    if let Some(frame) = &self.frame {
      let line_height = self.line_height;
      let cell_width = self.cell_width;

      content = content.child(
        div()
          .flex()
          .flex_col()
          .children(frame.lines().iter().map(|line| {
            div()
              .relative()
              .h(line_height)
              .w_full()
              .children(line.runs().iter().filter_map(|run| {
                let visible =
                  !run.text.trim().is_empty() || run.bg.is_some() || run.underline.is_some();
                if !visible {
                  return None;
                }

                let x = cell_width * run.start_col as f32;
                let width = cell_width * run.cols as f32;
                let len = run.text.len();

                let styled = StyledText::new(run.text.clone()).with_runs(vec![TextRun {
                  len,
                  font: fonts.get(run.style),
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
      .when(mouse_reporting, |el| el.cursor_default())
      .when(!mouse_reporting, |el| el.cursor_text())
      .on_action(cx.listener(Self::copy))
      .on_action(cx.listener(Self::paste))
      .on_key_down(cx.listener(Self::on_key_down))
      .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
      .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
      .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
      .on_mouse_move(cx.listener(Self::on_mouse_move))
      .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
      .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
      .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
      .on_scroll_wheel(cx.listener(Self::on_scroll_wheel))
      .on_drop(cx.listener(Self::on_drop_paths))
      .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(theme.hover))
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
