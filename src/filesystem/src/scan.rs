use std::path::{Path, PathBuf};

const FILTER_MAX_MATCHES: usize = 300;
const FILTER_MAX_DIRS: usize = 4000;
const IGNORED_RANK_PENALTY: u8 = 3;

#[derive(Clone)]
pub struct FileNode {
  pub path: PathBuf,
  pub name: String,
  pub lower: String,
  pub is_dir: bool,
  pub ignored: bool,
}

/// `needle` must already be lowercase. `by_path` widens the match to the
/// workspace-relative path, which only helps once the query has a separator.
pub fn match_rank(name: &str, relative: &str, needle: &str, by_path: bool) -> Option<u8> {
  rank_lowered(&name.to_lowercase(), relative, needle, by_path)
}

fn rank_lowered(name: &str, relative: &str, needle: &str, by_path: bool) -> Option<u8> {
  if name.starts_with(needle) {
    return Some(0);
  }

  if name.contains(needle) {
    return Some(1);
  }

  if by_path && relative.to_lowercase().contains(needle) {
    return Some(2);
  }

  None
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

pub fn scan_matches(
  root: &Path,
  needle: &str,
  by_path: bool,
  show_dotfiles: bool,
  ignored: &dyn Fn(&Path, bool) -> bool,
) -> Vec<FileNode> {
  let mut ranked: Vec<(u8, FileNode)> = Vec::new();
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

      let relative = node
        .path
        .strip_prefix(root)
        .unwrap_or(&node.path)
        .to_string_lossy()
        .to_string();

      let Some(mut rank) = rank_lowered(&node.lower, &relative, needle, by_path) else {
        continue;
      };

      if node.ignored {
        rank += IGNORED_RANK_PENALTY;
      }

      ranked.push((rank, node));
    }
  }

  ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| name_cmp(&a.1, &b.1)));

  ranked.into_iter().map(|(_, node)| node).collect()
}
