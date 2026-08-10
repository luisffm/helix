pub mod usage;

use std::io::{Result, Write};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

/// Runs `command` to completion and kills it if it outlives `timeout`, so a
/// network operation that never answers cannot pin a task forever.
pub fn output(mut command: Command, stdin: Option<&[u8]>, timeout: Duration) -> Result<Output> {
  let mut child = command
    .stdin(if stdin.is_some() {
      Stdio::piped()
    } else {
      Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()?;

  if let Some(bytes) = stdin {
    if let Some(mut pipe) = child.stdin.take() {
      pipe.write_all(bytes)?;
    }
  }

  let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
  let pid = child.id();

  std::thread::Builder::new()
    .name("helix-process-timeout".into())
    .spawn(move || {
      if done_rx.recv_timeout(timeout).is_err() {
        kill(pid);
      }
    })
    .ok();

  let output = child.wait_with_output();

  let _ = done_tx.send(());

  output
}

pub fn open_path(path: &Path) {
  let _ = Command::new("open").arg(path).spawn();
}

pub fn open_with(app: &str, path: &Path) {
  let _ = Command::new("open").args(["-a", app]).arg(path).spawn();
}

pub fn open_url(url: &str) {
  let _ = Command::new("open").arg(url).spawn();
}

#[cfg(unix)]
fn kill(pid: u32) {
  unsafe {
    libc::kill(pid as i32, libc::SIGKILL);
  }
}

#[cfg(not(unix))]
fn kill(_pid: u32) {}
