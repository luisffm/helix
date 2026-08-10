use crate::claude_cli;
use anyhow::{Result, anyhow};
use std::path::Path;

pub const TYPES: [&str; 7] = ["feat", "fix", "chore", "refactor", "docs", "test", "perf"];
pub const MAX_LEN: usize = 48;

const PROMPT: &str = "\
You are naming a git branch.

Rules:
- Output exactly one line: <type>/<summary>. Nothing else.
- <type> is one of: feat, fix, chore, refactor, docs, test, perf.
- <summary> is lowercase English, kebab-case, 2 to 5 words, no articles.
- The whole name must be at most 48 characters.
- Use only a-z, 0-9, hyphen and the single separating slash.
- English only, even when the description is in another language.
- No preamble, no code fences, no quotes, no explanation.

Examples:
feat/streaming-diff-view
fix/stale-index-lock
refactor/split-session-store";

pub fn build_prompt(context: &str) -> String {
  format!("{PROMPT}\n\nWhat the work is about:\n{}", context.trim())
}

pub fn generate(cwd: &Path, context: &str, model: Option<&str>) -> Result<String> {
  if context.trim().is_empty() {
    return Err(anyhow!("describe the work first"));
  }
  let spec = claude_cli::print_mode(build_prompt(context), model);
  let raw = claude_cli::run(cwd, &spec)?;
  let name = sanitize(&raw);
  if name.is_empty() {
    return Err(anyhow!("model returned an unusable branch name"));
  }
  Ok(name)
}

pub fn sanitize(raw: &str) -> String {
  let Some(line) = claude_cli::strip_code_fence(raw)
    .into_iter()
    .map(str::trim)
    .find(|line| !line.is_empty())
  else {
    return String::new();
  };

  let lowered = line
    .trim_matches(|c| c == '"' || c == '\'' || c == '`')
    .to_lowercase();

  let mut cleaned = String::with_capacity(lowered.len());
  for ch in lowered.chars() {
    match ch {
      'a'..='z' | '0'..='9' | '/' => cleaned.push(ch),
      // Models often answer `fix: something`; treat the colon as the separator.
      ':' => cleaned.push('/'),
      '-' | '_' | ' ' | '\t' | '.' => cleaned.push('-'),
      _ => {}
    }
  }

  let (kind, rest) = match cleaned.split_once('/') {
    Some((kind, rest)) if TYPES.contains(&kind) => (kind.to_string(), rest.to_string()),
    _ => ("feat".to_string(), cleaned.replace('/', "-")),
  };

  let summary = collapse(&rest.replace('/', "-"));
  if summary.is_empty() {
    return String::new();
  }

  let mut name = format!("{kind}/{summary}");
  if name.len() > MAX_LEN {
    name.truncate(MAX_LEN);
  }
  name.trim_end_matches('-').to_string()
}

fn collapse(value: &str) -> String {
  let mut out = String::with_capacity(value.len());
  for ch in value.chars() {
    if ch == '-' && out.ends_with('-') {
      continue;
    }
    out.push(ch);
  }
  out.trim_matches('-').to_string()
}

/// Rejects names git itself would refuse, so a bad suggestion fails in the
/// dialog instead of inside `git worktree add`.
pub fn is_valid(name: &str) -> bool {
  !name.is_empty()
    && name.len() <= MAX_LEN
    && !name.contains("..")
    && !name.contains("@{")
    && !name.contains("//")
    && !name.ends_with(".lock")
    && !name.starts_with('/')
    && !name.ends_with('/')
    && !name.starts_with('-')
    && name
      .chars()
      .all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '/' | '.'))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keeps_a_well_formed_name() {
    assert_eq!(
      sanitize("feat/streaming-diff-view\n"),
      "feat/streaming-diff-view"
    );
  }

  #[test]
  fn strips_fences_and_quotes() {
    assert_eq!(
      sanitize("```\n\"fix/stale-index-lock\"\n```"),
      "fix/stale-index-lock"
    );
  }

  #[test]
  fn takes_only_the_first_line() {
    assert_eq!(
      sanitize("feat/add-cache\n\nExplanation here."),
      "feat/add-cache"
    );
  }

  #[test]
  fn prefixes_feat_when_type_is_missing_or_unknown() {
    assert_eq!(sanitize("add-cache"), "feat/add-cache");
    assert_eq!(sanitize("wip/add-cache"), "feat/wip-add-cache");
  }

  #[test]
  fn converts_spaces_and_underscores_to_hyphens() {
    assert_eq!(sanitize("fix: Stale Index Lock"), "fix/stale-index-lock");
  }

  #[test]
  fn collapses_repeated_separators() {
    assert_eq!(sanitize("feat///a---b"), "feat/a-b");
  }

  #[test]
  fn drops_characters_git_rejects() {
    assert_eq!(sanitize("feat/add~cache^now:?*"), "feat/addcachenow");
  }

  #[test]
  fn truncates_without_leaving_a_trailing_hyphen() {
    let long = format!("feat/{}", "very-long-summary-".repeat(6));
    let name = sanitize(&long);
    assert!(name.len() <= MAX_LEN);
    assert!(!name.ends_with('-'));
  }

  #[test]
  fn empty_summary_is_rejected() {
    assert_eq!(sanitize("feat/"), "");
    assert_eq!(sanitize("~~~"), "");
    assert_eq!(sanitize(""), "");
  }

  #[test]
  fn validity_matches_git_ref_rules() {
    assert!(is_valid("feat/add-cache"));
    assert!(!is_valid(""));
    assert!(!is_valid("feat/a..b"));
    assert!(!is_valid("feat/a@{b"));
    assert!(!is_valid("feat//b"));
    assert!(!is_valid("feat/b.lock"));
    assert!(!is_valid("/feat"));
    assert!(!is_valid("feat/"));
    assert!(!is_valid("feat/a b"));
  }

  #[test]
  fn prompt_carries_the_context() {
    let prompt = build_prompt("  refatorar o parser de diff  ");
    assert!(prompt.contains("refatorar o parser de diff"));
    assert!(prompt.contains("English only"));
  }
}
