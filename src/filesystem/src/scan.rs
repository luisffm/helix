use helix_fuzzy::Ranker;
use std::path::{Path, PathBuf};

const FILTER_MAX_MATCHES: usize = 300;
const FILTER_MAX_DIRS: usize = 4000;

/// A grep over a workspace has to stay bounded or one vendored bundle stalls the
/// whole scan, so files above this size and anything holding a NUL byte early on
/// are skipped rather than read.
const GREP_MAX_BYTES: u64 = 256 * 1024;
const GREP_SNIFF_BYTES: usize = 1024;

#[derive(Clone)]
pub struct FileNode {
  pub path: PathBuf,
  pub name: String,
  pub lower: String,
  pub is_dir: bool,
  pub ignored: bool,
}

fn name_cmp(a: &FileNode, b: &FileNode) -> std::cmp::Ordering {
  a.lower.cmp(&b.lower).then_with(|| a.name.cmp(&b.name))
}

pub fn scan_dir(
  dir: &Path,
  show_dotfiles: bool,
  ignored: &dyn Fn(&Path, bool) -> bool,
) -> Vec<FileNode> {
  let mut nodes: Vec<FileNode> = std::fs::read_dir(dir)
    .into_iter()
    .flatten()
    .flatten()
    .filter_map(|entry| {
      let name = entry.file_name().to_string_lossy().to_string();

      if name == ".git" || name == "node_modules" {
        return None;
      }

      if !show_dotfiles && name.starts_with('.') {
        return None;
      }

      let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
      let path = entry.path();
      let ignored = ignored(&path, is_dir);

      Some(FileNode {
        lower: name.to_lowercase(),
        path,
        name,
        is_dir,
        ignored,
      })
    })
    .collect();

  nodes.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| name_cmp(a, b)));

  nodes
}

/// A query holding a separator is scored against the workspace-relative path,
/// anything else against the file name alone. Ignored files stay eligible but
/// always sort below the tracked ones.
pub fn scan_matches(
  root: &Path,
  query: &str,
  show_dotfiles: bool,
  ignored: &dyn Fn(&Path, bool) -> bool,
) -> Vec<FileNode> {
  let mut ranker = Ranker::for_paths();

  ranker.set_query(query);

  let by_path = query.contains('/');
  let mut ranked: Vec<(u32, FileNode)> = Vec::new();
  let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
  let mut dirs = 0usize;

  while let Some(dir) = queue.pop_front() {
    if dirs >= FILTER_MAX_DIRS || ranked.len() >= FILTER_MAX_MATCHES {
      break;
    }

    dirs += 1;

    for node in scan_dir(&dir, show_dotfiles, ignored) {
      if node.is_dir {
        if !node.ignored {
          queue.push_back(node.path.clone());
        }

        continue;
      }

      let score = if by_path {
        let relative = node.path.strip_prefix(root).unwrap_or(&node.path);

        ranker.score(&relative.to_string_lossy())
      } else {
        ranker.score(&node.name)
      };

      let Some(score) = score else {
        continue;
      };

      ranked.push((score, node));
    }
  }

  ranked.sort_by(|a, b| {
    a.1
      .ignored
      .cmp(&b.1.ignored)
      .then_with(|| b.0.cmp(&a.0))
      .then_with(|| name_cmp(&a.1, &b.1))
  });

  ranked.into_iter().map(|(_, node)| node).collect()
}

fn holds_text(bytes: &[u8]) -> bool {
  !bytes.iter().take(GREP_SNIFF_BYTES).any(|byte| *byte == 0)
}

/// Files whose contents hold `needle`, matched case-insensitively. Ranking is by
/// how many times the needle occurs, so the file that talks about it most sorts
/// first, and ignored files stay below tracked ones as they do by name.
pub fn scan_contents(
  root: &Path,
  query: &str,
  show_dotfiles: bool,
  ignored: &dyn Fn(&Path, bool) -> bool,
) -> Vec<FileNode> {
  let needle = query.to_lowercase();

  if needle.is_empty() {
    return Vec::new();
  }

  let mut ranked: Vec<(usize, FileNode)> = Vec::new();
  let mut queue = std::collections::VecDeque::from([root.to_path_buf()]);
  let mut dirs = 0usize;

  while let Some(dir) = queue.pop_front() {
    if dirs >= FILTER_MAX_DIRS || ranked.len() >= FILTER_MAX_MATCHES {
      break;
    }

    dirs += 1;

    for node in scan_dir(&dir, show_dotfiles, ignored) {
      if node.is_dir {
        if !node.ignored {
          queue.push_back(node.path.clone());
        }

        continue;
      }

      let too_big = std::fs::metadata(&node.path)
        .map(|meta| meta.len() > GREP_MAX_BYTES)
        .unwrap_or(true);

      if too_big {
        continue;
      }

      let Ok(bytes) = std::fs::read(&node.path) else {
        continue;
      };

      if !holds_text(&bytes) {
        continue;
      }

      let Ok(text) = String::from_utf8(bytes) else {
        continue;
      };

      let hits = text.to_lowercase().matches(&needle).count();

      if hits > 0 {
        ranked.push((hits, node));
      }
    }
  }

  ranked.sort_by(|a, b| {
    a.1
      .ignored
      .cmp(&b.1.ignored)
      .then_with(|| b.0.cmp(&a.0))
      .then_with(|| name_cmp(&a.1, &b.1))
  });

  ranked.into_iter().map(|(_, node)| node).collect()
}
