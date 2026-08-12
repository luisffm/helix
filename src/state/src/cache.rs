use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Kept apart from `config.json`: that file is the user's, written when they
/// change a setting, and a cache that rewrites itself would churn it and race
/// their edits. This one is ours to overwrite.
static CACHE: Mutex<Option<HelixCache>> = Mutex::new(None);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeStats {
  pub added: usize,
  pub removed: usize,
  /// The index mtime the counts were taken at, as seconds since the epoch. A
  /// worktree whose index has not moved since needs no diff.
  #[serde(default)]
  pub stamp: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HelixCache {
  #[serde(default)]
  pub worktrees: HashMap<PathBuf, WorktreeStats>,
}

pub fn stamp_of(time: SystemTime) -> u64 {
  time
    .duration_since(UNIX_EPOCH)
    .map(|since| since.as_secs())
    .unwrap_or(0)
}

fn cache_path() -> Option<PathBuf> {
  Some(crate::config::app_dir()?.join("cache.json"))
}

pub fn load() -> HelixCache {
  let mut held = CACHE.lock().unwrap_or_else(|err| err.into_inner());

  if let Some(cache) = held.as_ref() {
    return cache.clone();
  }

  let cache: HelixCache = cache_path()
    .and_then(|path| std::fs::read_to_string(path).ok())
    .and_then(|text| serde_json::from_str(&text).ok())
    .unwrap_or_default();

  *held = Some(cache.clone());

  cache
}

/// Records what a worktree was measured at. Returns whether anything changed, so
/// a caller can skip both the write and the redraw when it did not.
pub fn set_worktree_stats(root: &Path, stats: WorktreeStats) -> bool {
  let mut held = CACHE.lock().unwrap_or_else(|err| err.into_inner());
  let cache = held.get_or_insert_with(|| {
    cache_path()
      .and_then(|path| std::fs::read_to_string(path).ok())
      .and_then(|text| serde_json::from_str::<HelixCache>(&text).ok())
      .unwrap_or_default()
  });

  if cache.worktrees.get(root) == Some(&stats) {
    return false;
  }

  cache.worktrees.insert(root.to_path_buf(), stats);

  let snapshot = cache.clone();

  drop(held);
  write(&snapshot);

  true
}

/// Drops worktrees that are no longer listed, so the file does not grow with
/// every branch that ever existed.
pub fn retain_worktrees(listed: &[PathBuf]) {
  let mut held = CACHE.lock().unwrap_or_else(|err| err.into_inner());
  let Some(cache) = held.as_mut() else { return };

  let before = cache.worktrees.len();

  cache.worktrees.retain(|root, _| listed.contains(root));

  if cache.worktrees.len() == before {
    return;
  }

  let snapshot = cache.clone();

  drop(held);
  write(&snapshot);
}

fn write(cache: &HelixCache) {
  let Some(path) = cache_path() else { return };

  if let Some(parent) = path.parent() {
    let _ = std::fs::create_dir_all(parent);
  }

  if let Ok(json) = serde_json::to_string(cache) {
    let _ = std::fs::write(path, json);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_reading_is_only_recorded_when_it_changes() {
    let dir = std::env::temp_dir().join(format!("helix-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // SAFETY: the cache path is read through this, and the test owns the process
    // wide value only for as long as it holds the lock below.
    unsafe { std::env::set_var("HELIX_CONFIG_DIR", &dir) };

    let root = PathBuf::from("/tmp/helix-cache-test/worktree");
    let stats = WorktreeStats {
      added: 12,
      removed: 3,
      stamp: 99,
    };

    assert!(set_worktree_stats(&root, stats.clone()));
    assert!(!set_worktree_stats(&root, stats.clone()));

    let changed = WorktreeStats { added: 13, ..stats };

    assert!(set_worktree_stats(&root, changed.clone()));
    assert_eq!(load().worktrees.get(&root), Some(&changed));

    retain_worktrees(&[]);

    assert!(load().worktrees.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
  }
}
