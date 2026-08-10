use alacritty_terminal::event::{Event as TermEvent, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct EventProxy(UnboundedSender<TermEvent>);

impl EventListener for EventProxy {
  fn send_event(&self, event: TermEvent) {
    let _ = self.0.send(event);
  }
}

pub struct SpawnOptions {
  pub program: String,
  pub args: Vec<String>,
  pub cwd: PathBuf,
  pub cols: u16,
  pub rows: u16,
  pub cell_width: u16,
  pub cell_height: u16,
}

pub struct TerminalBackend {
  term: Arc<FairMutex<Term<EventProxy>>>,
  notifier: Mutex<Notifier>,
  shell_pid: Option<u32>,
  #[cfg(unix)]
  master: Option<std::fs::File>,
}

impl TerminalBackend {
  pub fn spawn(opts: SpawnOptions, event_tx: UnboundedSender<TermEvent>) -> Result<Self> {
    let mut env = HashMap::new();
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());

    let pty_options = tty::Options {
      shell: Some(tty::Shell::new(opts.program, opts.args)),
      working_directory: Some(opts.cwd),
      drain_on_exit: false,
      env,
      ..Default::default()
    };

    let window_size = WindowSize {
      num_lines: opts.rows.max(2),
      num_cols: opts.cols.max(2),
      cell_width: opts.cell_width,
      cell_height: opts.cell_height,
    };

    let proxy = EventProxy(event_tx);
    let term = Arc::new(FairMutex::new(Term::new(
      Config::default(),
      &TermSize::new(opts.cols.max(2) as usize, opts.rows.max(2) as usize),
      proxy.clone(),
    )));

    let pty = tty::new(&pty_options, window_size, 0)?;
    let shell_pid = Some(pty.child().id());
    #[cfg(unix)]
    let master = pty.file().try_clone().ok();

    let event_loop = EventLoop::new(Arc::clone(&term), proxy, pty, false, false)?;
    let notifier = Notifier(event_loop.channel());

    event_loop.spawn();

    Ok(Self {
      term,
      notifier: Mutex::new(notifier),
      shell_pid,
      #[cfg(unix)]
      master,
    })
  }

  pub fn shell_pid(&self) -> Option<u32> {
    self.shell_pid
  }

  #[cfg(unix)]
  pub fn foreground_pgid(&self) -> Option<i32> {
    use std::os::unix::io::AsRawFd;
    let fd = self.master.as_ref()?.as_raw_fd();
    let pgid = unsafe { libc::tcgetpgrp(fd) };

    (pgid > 0).then_some(pgid)
  }

  #[cfg(not(unix))]
  pub fn foreground_pgid(&self) -> Option<i32> {
    None
  }

  pub fn write(&self, bytes: Vec<u8>) {
    if let Ok(notifier) = self.notifier.lock() {
      notifier.notify(bytes);
    }
  }

  pub fn resize(&self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) {
    if cols < 2 || rows < 2 {
      return;
    }

    let window_size = WindowSize {
      num_lines: rows,
      num_cols: cols,
      cell_width,
      cell_height,
    };

    if let Ok(mut notifier) = self.notifier.lock() {
      notifier.on_resize(window_size);
    }

    self
      .term
      .lock()
      .resize(TermSize::new(cols as usize, rows as usize));
  }

  pub fn scroll_lines(&self, delta: i32) {
    self.term.lock().scroll_display(Scroll::Delta(delta));
  }

  pub fn scroll_to_bottom(&self) {
    self.term.lock().scroll_display(Scroll::Bottom);
  }

  pub fn display_offset(&self) -> usize {
    self.term.lock().grid().display_offset()
  }

  pub fn mode(&self) -> TermMode {
    *self.term.lock().mode()
  }

  pub fn with_term<R>(&self, f: impl FnOnce(&Term<EventProxy>) -> R) -> R {
    f(&self.term.lock())
  }

  pub fn start_selection(&self, point: GridPoint, side: Side, kind: SelectionType) {
    self.term.lock().selection = Some(Selection::new(kind, point, side));
  }

  pub fn update_selection(&self, point: GridPoint, side: Side) {
    if let Some(selection) = self.term.lock().selection.as_mut() {
      selection.update(point, side);
    }
  }

  pub fn has_selection(&self) -> bool {
    self.term.lock().selection.is_some()
  }

  pub fn clear_selection(&self) {
    self.term.lock().selection = None;
  }

  pub fn selection_text(&self) -> Option<String> {
    self.term.lock().selection_to_string()
  }

  pub fn viewport_to_point(&self, col: usize, row: i32) -> GridPoint {
    let term = self.term.lock();
    let grid = term.grid();
    let display_offset = grid.display_offset() as i32;
    let last_column = grid.columns().saturating_sub(1);
    let clamped_row = row.clamp(0, grid.screen_lines() as i32 - 1);
    let line = (clamped_row - display_offset).max(-(grid.history_size() as i32));

    GridPoint::new(Line(line), Column(col.min(last_column)))
  }
}

impl Drop for TerminalBackend {
  fn drop(&mut self) {
    if let Ok(notifier) = self.notifier.lock() {
      let _ = notifier.0.send(Msg::Shutdown);
    }
  }
}
