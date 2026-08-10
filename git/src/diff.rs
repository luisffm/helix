use anyhow::{Result, anyhow};
use git2::Repository;
use helix_models::{DiffBase, DiffHunk, DiffLine, DiffLineKind, DiffState, FileDiff};
use similar::{ChangeTag, TextDiff};
use std::path::Path;

pub const MAX_DIFF_LINES: usize = 120_000;
pub const MAX_DIFF_CHARS: usize = 6_000_000;
pub const CONTEXT_LINES: usize = 3;

pub fn file_diff(root: &Path, relative: &str, base: &DiffBase) -> Result<FileDiff> {
  let repo = Repository::discover(root)?;
  let workdir = repo
    .workdir()
    .ok_or_else(|| anyhow!("bare repository has no working tree"))?
    .to_path_buf();

  let (old, new) = match base {
    DiffBase::Unstaged => (
      blob_at_index(&repo, relative).or_else(|_| blob_at_rev(&repo, "HEAD", relative)),
      read_working_tree(&workdir, relative),
    ),
    DiffBase::Staged => (
      blob_at_rev(&repo, "HEAD", relative),
      blob_at_index(&repo, relative),
    ),
    DiffBase::Head => (
      blob_at_rev(&repo, "HEAD", relative),
      read_working_tree(&workdir, relative),
    ),
    DiffBase::Branch { merge_base, head } => (
      blob_at_rev(&repo, merge_base, relative),
      blob_at_rev(&repo, head, relative),
    ),
  };

  let language = helix_buffer::language::of(Path::new(relative)).to_string();
  let old = old.unwrap_or(Side::Missing);
  let new = new.unwrap_or(Side::Missing);

  let (old_text, new_text) = match (old, new) {
    (Side::Binary, _) | (_, Side::Binary) => {
      return Ok(empty_diff(relative, base, language, DiffState::Binary));
    }
    (old, new) => (old.into_text(), new.into_text()),
  };

  let lines = old_text.lines().count().max(new_text.lines().count());
  let chars = old_text.len() + new_text.len();
  if lines > MAX_DIFF_LINES || chars > MAX_DIFF_CHARS {
    return Ok(empty_diff(
      relative,
      base,
      language,
      DiffState::TooLarge { lines, chars },
    ));
  }

  if old_text == new_text {
    return Ok(FileDiff {
      path: relative.to_string(),
      base: base.clone(),
      language,
      state: DiffState::Identical,
      old_text,
      new_text,
      hunks: Vec::new(),
      added: 0,
      removed: 0,
    });
  }

  let (hunks, added, removed) = hunks_of(&old_text, &new_text);
  Ok(FileDiff {
    path: relative.to_string(),
    base: base.clone(),
    language,
    state: DiffState::Text,
    old_text,
    new_text,
    hunks,
    added,
    removed,
  })
}

fn empty_diff(relative: &str, base: &DiffBase, language: String, state: DiffState) -> FileDiff {
  FileDiff {
    path: relative.to_string(),
    base: base.clone(),
    language,
    state,
    old_text: String::new(),
    new_text: String::new(),
    hunks: Vec::new(),
    added: 0,
    removed: 0,
  }
}

fn hunks_of(old_text: &str, new_text: &str) -> (Vec<DiffHunk>, usize, usize) {
  let old_offsets = line_offsets(old_text);
  let new_offsets = line_offsets(new_text);
  let diff = TextDiff::from_lines(old_text, new_text);

  let mut hunks = Vec::new();
  let mut added = 0usize;
  let mut removed = 0usize;

  for group in diff.grouped_ops(CONTEXT_LINES) {
    let mut lines: Vec<DiffLine> = Vec::new();
    let mut old_start = 0u32;
    let mut new_start = 0u32;
    let mut started = false;

    for op in &group {
      for change in diff.iter_changes(op) {
        let old_index = change.old_index();
        let new_index = change.new_index();
        if !started {
          old_start = old_index.unwrap_or(0) as u32 + 1;
          new_start = new_index.unwrap_or(0) as u32 + 1;
          started = true;
        }
        let (kind, range) = match change.tag() {
          ChangeTag::Delete => {
            removed += 1;
            (
              DiffLineKind::Removed,
              old_offsets
                .get(old_index.unwrap_or(0))
                .cloned()
                .unwrap_or(0..0),
            )
          }
          ChangeTag::Insert => {
            added += 1;
            (
              DiffLineKind::Added,
              new_offsets
                .get(new_index.unwrap_or(0))
                .cloned()
                .unwrap_or(0..0),
            )
          }
          ChangeTag::Equal => (
            DiffLineKind::Context,
            new_offsets
              .get(new_index.unwrap_or(0))
              .cloned()
              .unwrap_or(0..0),
          ),
        };
        lines.push(DiffLine {
          kind,
          old_line: old_index.map(|i| i as u32 + 1),
          new_line: new_index.map(|i| i as u32 + 1),
          range,
        });
      }
    }

    if !lines.is_empty() {
      hunks.push(DiffHunk {
        old_start,
        new_start,
        lines,
      });
    }
  }

  (hunks, added, removed)
}

fn line_offsets(text: &str) -> Vec<std::ops::Range<usize>> {
  let mut ranges = Vec::new();
  let mut start = 0usize;
  for (index, byte) in text.bytes().enumerate() {
    if byte == b'\n' {
      let mut end = index;
      if end > start && text.as_bytes()[end - 1] == b'\r' {
        end -= 1;
      }
      ranges.push(start..end);
      start = index + 1;
    }
  }
  if start < text.len() {
    ranges.push(start..text.len());
  }
  ranges
}

enum Side {
  Text(String),
  Binary,
  Missing,
}

impl Side {
  fn into_text(self) -> String {
    match self {
      Side::Text(text) => text,
      _ => String::new(),
    }
  }
}

fn side_from_bytes(bytes: Vec<u8>) -> Side {
  match helix_buffer::from_bytes(bytes) {
    helix_buffer::FileContent::Text { text, .. } => Side::Text(text),
    _ => Side::Binary,
  }
}

fn blob_at_rev(repo: &Repository, rev: &str, relative: &str) -> Result<Side> {
  let object = repo.revparse_single(&format!("{rev}:{relative}"))?;
  let blob = object
    .as_blob()
    .ok_or_else(|| anyhow!("{rev}:{relative} is not a blob"))?;
  Ok(side_from_bytes(blob.content().to_vec()))
}

fn blob_at_index(repo: &Repository, relative: &str) -> Result<Side> {
  let index = repo.index()?;
  let entry = index
    .get_path(Path::new(relative), 0)
    .ok_or_else(|| anyhow!("{relative} not in index"))?;
  let blob = repo.find_blob(entry.id)?;
  Ok(side_from_bytes(blob.content().to_vec()))
}

fn read_working_tree(workdir: &Path, relative: &str) -> Result<Side> {
  let path = workdir.join(relative);
  if !path.exists() {
    return Ok(Side::Missing);
  }
  Ok(side_from_bytes(std::fs::read(path)?))
}

pub fn merge_base(root: &Path, base_ref: &str) -> Result<(String, String)> {
  let repo = Repository::discover(root)?;
  let head = repo.head()?.peel_to_commit()?.id();
  let base = repo.revparse_single(base_ref)?.peel_to_commit()?.id();
  let merge_base = repo.merge_base(base, head)?;
  Ok((merge_base.to_string(), head.to_string()))
}

pub fn default_base_ref(root: &Path) -> Option<String> {
  let repo = Repository::discover(root).ok()?;
  for candidate in ["origin/main", "origin/master", "main", "master"] {
    if repo.revparse_single(candidate).is_ok() {
      return Some(candidate.to_string());
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn line_offsets_handles_crlf_and_missing_trailing_newline() {
    let text = "a\r\nbb\nccc";
    let ranges = line_offsets(text);
    assert_eq!(ranges.len(), 3);
    assert_eq!(&text[ranges[0].clone()], "a");
    assert_eq!(&text[ranges[1].clone()], "bb");
    assert_eq!(&text[ranges[2].clone()], "ccc");
  }

  #[test]
  fn counts_added_and_removed() {
    let (hunks, added, removed) = hunks_of("one\ntwo\nthree\n", "one\ndeux\nthree\n");
    assert_eq!((added, removed), (1, 1));
    assert_eq!(hunks.len(), 1);
  }

  #[test]
  fn pure_insert_has_no_removals() {
    let (_, added, removed) = hunks_of("one\n", "one\ntwo\n");
    assert_eq!((added, removed), (1, 0));
  }
}
