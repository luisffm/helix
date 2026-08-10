use anyhow::{Result, anyhow};
use git2::{IndexAddOption, Repository, Signature};
use std::path::Path;

pub fn stage(root: &Path, relative: &str) -> Result<()> {
  let repo = Repository::discover(root)?;
  let mut index = repo.index()?;
  let path = Path::new(relative);
  if repo
    .workdir()
    .map(|workdir| workdir.join(path).exists())
    .unwrap_or(false)
  {
    index.add_path(path)?;
  } else {
    index.remove_path(path)?;
  }
  index.write()?;
  Ok(())
}

pub fn stage_all(root: &Path) -> Result<()> {
  let repo = Repository::discover(root)?;
  let mut index = repo.index()?;
  index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
  index.write()?;
  Ok(())
}

pub fn unstage(root: &Path, relative: &str) -> Result<()> {
  let repo = Repository::discover(root)?;
  let head = repo.head().and_then(|head| head.peel_to_commit());
  let mut index = repo.index()?;
  let path = Path::new(relative);
  match head {
    Ok(commit) => match commit.tree()?.get_path(path) {
      Ok(entry) => {
        let blob = repo.find_blob(entry.id())?;
        index.add_frombuffer(
          &git2::IndexEntry {
            ctime: git2::IndexTime::new(0, 0),
            mtime: git2::IndexTime::new(0, 0),
            dev: 0,
            ino: 0,
            mode: entry.filemode() as u32,
            uid: 0,
            gid: 0,
            file_size: blob.size() as u32,
            id: entry.id(),
            flags: 0,
            flags_extended: 0,
            path: relative.as_bytes().to_vec(),
          },
          blob.content(),
        )?;
      }
      Err(_) => {
        index.remove_path(path)?;
      }
    },
    Err(_) => {
      index.remove_path(path)?;
    }
  }
  index.write()?;
  Ok(())
}

pub fn unstage_all(root: &Path) -> Result<()> {
  let repo = Repository::discover(root)?;
  let mut index = repo.index()?;
  match repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .and_then(|commit| commit.tree())
  {
    Ok(tree) => index.read_tree(&tree)?,
    Err(_) => index.clear()?,
  }
  index.write()?;
  Ok(())
}

pub fn commit(root: &Path, message: &str) -> Result<String> {
  if message.trim().is_empty() {
    return Err(anyhow!("empty commit message"));
  }
  let repo = Repository::discover(root)?;
  let mut index = repo.index()?;
  let tree_id = index.write_tree()?;
  let tree = repo.find_tree(tree_id)?;
  let signature = author(&repo)?;
  let parents = match repo.head().and_then(|head| head.peel_to_commit()) {
    Ok(parent) => vec![parent],
    Err(_) => Vec::new(),
  };

  if let Some(parent) = parents.first() {
    if parent.tree_id() == tree_id {
      return Err(anyhow!("nothing to commit"));
    }
  }

  let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
  let oid = repo.commit(
    Some("HEAD"),
    &signature,
    &signature,
    message,
    &tree,
    &parent_refs,
  )?;
  Ok(oid.to_string())
}

fn author(repo: &Repository) -> Result<Signature<'static>> {
  let config = repo.config()?;
  let name = config
    .get_string("user.name")
    .map_err(|_| anyhow!("git user.name is not set"))?;
  let email = config
    .get_string("user.email")
    .map_err(|_| anyhow!("git user.email is not set"))?;
  Ok(Signature::now(&name, &email)?)
}

pub fn staged_name_status(root: &Path) -> Result<String> {
  let repo = Repository::discover(root)?;
  let head_tree = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .and_then(|commit| commit.tree())
    .ok();
  let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, None)?;
  let mut lines = Vec::new();
  diff.foreach(
    &mut |delta, _| {
      let status = match delta.status() {
        git2::Delta::Added => "A",
        git2::Delta::Deleted => "D",
        git2::Delta::Modified => "M",
        git2::Delta::Renamed => "R",
        git2::Delta::Copied => "C",
        git2::Delta::Typechange => "T",
        _ => "?",
      };
      let path = delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(|path| path.display().to_string())
        .unwrap_or_default();
      lines.push(format!("{status}\t{path}"));
      true
    },
    None,
    None,
    None,
  )?;
  Ok(lines.join("\n"))
}

pub fn staged_patch(root: &Path, byte_budget: usize) -> Result<String> {
  let repo = Repository::discover(root)?;
  let head_tree = repo
    .head()
    .and_then(|head| head.peel_to_commit())
    .and_then(|commit| commit.tree())
    .ok();
  let diff = repo.diff_tree_to_index(head_tree.as_ref(), None, None)?;
  let mut patch = String::new();
  diff.print(git2::DiffFormat::Patch, |_, _, line| {
    let origin = line.origin();
    if matches!(origin, '+' | '-' | ' ') {
      patch.push(origin);
    }
    patch.push_str(&String::from_utf8_lossy(line.content()));
    true
  })?;
  Ok(truncate_patch(&patch, byte_budget))
}

pub fn truncate_patch(patch: &str, byte_budget: usize) -> String {
  if patch.len() <= byte_budget {
    return patch.to_string();
  }

  let sections = split_sections(patch);
  if sections.is_empty() {
    return clip_on_line(patch, byte_budget).to_string();
  }

  let mut budgets = water_fill(&sections, byte_budget);
  let mut out = String::with_capacity(byte_budget + 64);
  for (section, budget) in sections.iter().zip(budgets.drain(..)) {
    let kept = clip_on_line(section, budget);
    out.push_str(kept);
    let omitted = section.len() - kept.len();
    if omitted > 0 {
      out.push_str(&format!("\n...(diff truncated, {omitted} bytes omitted)\n"));
    }
  }
  out
}

fn split_sections(patch: &str) -> Vec<&str> {
  let marker = "diff --git ";
  let mut sections = Vec::new();
  let mut start = None;
  for (index, _) in patch.match_indices(marker) {
    if index == 0 || patch.as_bytes()[index - 1] == b'\n' {
      if let Some(previous) = start {
        sections.push(&patch[previous..index]);
      }
      start = Some(index);
    }
  }
  if let Some(previous) = start {
    sections.push(&patch[previous..]);
  }
  sections
}

fn water_fill(sections: &[&str], byte_budget: usize) -> Vec<usize> {
  let mut budgets = vec![0usize; sections.len()];
  let mut remaining = byte_budget;
  let mut unfilled: Vec<usize> = (0..sections.len()).collect();

  while !unfilled.is_empty() && remaining > 0 {
    let share = remaining / unfilled.len();
    if share == 0 {
      break;
    }
    let mut next_unfilled = Vec::new();
    let mut spent = 0usize;
    for index in unfilled {
      let need = sections[index].len() - budgets[index];
      if need <= share {
        budgets[index] += need;
        spent += need;
      } else {
        budgets[index] += share;
        spent += share;
        next_unfilled.push(index);
      }
    }
    remaining -= spent;
    if spent == 0 {
      break;
    }
    unfilled = next_unfilled;
  }
  budgets
}

fn clip_on_line(text: &str, budget: usize) -> &str {
  if text.len() <= budget {
    return text;
  }
  match text[..budget].rfind('\n') {
    Some(index) => &text[..index],
    None => "",
  }
}

#[cfg(test)]
mod tests {
  use super::{truncate_patch, water_fill};

  #[test]
  fn short_patch_is_untouched() {
    let patch = "diff --git a/x b/x\n+one\n";
    assert_eq!(truncate_patch(patch, 1024), patch);
  }

  #[test]
  fn small_section_slack_goes_to_big_section() {
    let sections = ["diff --git a/s b/s\n+s\n", &"x".repeat(1000)];
    let budgets = water_fill(&sections, 600);
    assert_eq!(budgets[0], sections[0].len());
    assert!(budgets[1] > 300);
  }

  #[test]
  fn truncation_marks_omitted_bytes() {
    let big = format!("diff --git a/big b/big\n{}", "+line\n".repeat(500));
    let out = truncate_patch(&big, 200);
    assert!(out.contains("diff truncated"));
    assert!(out.len() < big.len());
  }
}
