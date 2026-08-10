pub mod paths;
pub mod scan;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

pub struct FsWatcher {
  _watcher: RecommendedWatcher,
}

pub fn watch(root: &Path, tx: UnboundedSender<Vec<PathBuf>>) -> notify::Result<FsWatcher> {
  let (raw_tx, raw_rx) = std_mpsc::channel::<notify::Result<Event>>();
  let mut watcher = notify::recommended_watcher(raw_tx)?;
  watcher.watch(root, RecursiveMode::Recursive)?;

  std::thread::Builder::new()
    .name("helix-fs-debounce".into())
    .spawn(move || {
      let mut pending: Vec<PathBuf> = Vec::new();

      loop {
        let timeout = if pending.is_empty() {
          Duration::from_secs(3600)
        } else {
          Duration::from_millis(180)
        };

        match raw_rx.recv_timeout(timeout) {
          Ok(Ok(event)) => {
            for path in event.paths {
              if is_relevant(&path) {
                pending.push(path);
              }
            }
          }
          Ok(Err(_)) => {}
          Err(std_mpsc::RecvTimeoutError::Timeout) => {
            if !pending.is_empty() && tx.send(std::mem::take(&mut pending)).is_err() {
              break;
            }
          }
          Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
        }
      }
    })
    .ok();

  Ok(FsWatcher { _watcher: watcher })
}

fn is_relevant(path: &Path) -> bool {
  let s = path.to_string_lossy();

  if s.contains("/.git/") {
    return s.ends_with("/HEAD") || s.ends_with("/index") || s.contains("/refs/");
  }

  if s.contains("/target/") || s.contains("/node_modules/") || s.ends_with(".DS_Store") {
    return false;
  }

  true
}
