use crate::claude_cli;
use anyhow::{Result, anyhow};
use std::path::Path;

pub use claude_cli::Spec;

pub const PATCH_BYTE_BUDGET: usize = 200 * 1024;

const BASE_PROMPT: &str = "\
You are writing a git commit message for the staged changes below.

Rules:
- First line: imperative mood, at most 72 characters, no trailing period.
- Leave the body out unless the reason for the change is not obvious from the diff.
- When there is a body, wrap it at 72 characters and explain WHY, not what.
- Output only the commit message. No preamble, no code fences, no explanation.
- Do not add Co-authored-by or any other trailer.";

#[derive(Clone, Debug, Default)]
pub struct Context {
  pub branch: String,
  pub name_status: String,
  pub patch: String,
}

pub fn build_prompt(context: &Context, extra: Option<&str>) -> String {
  let mut prompt = String::with_capacity(context.patch.len() + 1024);
  prompt.push_str(BASE_PROMPT);

  if let Some(extra) = extra.map(str::trim).filter(|extra| !extra.is_empty()) {
    prompt.push_str("\n\nAdditional user prompt:\n");
    prompt.push_str(extra);
  }

  if !context.branch.is_empty() {
    prompt.push_str("\n\nBranch:\n");
    prompt.push_str(&context.branch);
  }
  if !context.name_status.is_empty() {
    prompt.push_str("\n\nChanged files:\n");
    prompt.push_str(&context.name_status);
  }
  if !context.patch.is_empty() {
    prompt.push_str("\n\nStaged diff:\n");
    prompt.push_str(&context.patch);
  }
  prompt
}

pub fn plan(prompt: String, model: Option<&str>) -> Spec {
  claude_cli::print_mode(prompt, model)
}

pub fn generate(cwd: &Path, spec: &Spec) -> Result<String> {
  let message = sanitize(&claude_cli::run(cwd, spec)?);
  if message.is_empty() {
    return Err(anyhow!("model returned an empty commit message"));
  }
  Ok(message)
}

pub fn sanitize(raw: &str) -> String {
  let mut lines = claude_cli::strip_code_fence(raw);
  lines.retain(|line| {
    let lower = line.trim().to_ascii_lowercase();
    !lower.starts_with("co-authored-by:") && !lower.starts_with("generated with")
  });
  lines
    .join("\n")
    .trim_matches(|c: char| c == '\n' || c == '\r')
    .trim_end()
    .to_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sanitize_strips_code_fences() {
    assert_eq!(
      sanitize("```\nfix: repair parser\n```\n"),
      "fix: repair parser"
    );
  }

  #[test]
  fn sanitize_drops_trailers() {
    let raw = "fix: repair parser\n\nBody line.\n\nCo-authored-by: Someone <a@b.c>\n";
    assert_eq!(sanitize(raw), "fix: repair parser\n\nBody line.");
  }

  #[test]
  fn sanitize_keeps_multiline_body() {
    let raw = "feat: add cache\n\nThe old path re-read the file on every keystroke.\n";
    assert_eq!(
      sanitize(raw),
      "feat: add cache\n\nThe old path re-read the file on every keystroke."
    );
  }

  #[test]
  fn prompt_includes_context_sections() {
    let context = Context {
      branch: "feature".to_string(),
      name_status: "M\tsrc/lib.rs".to_string(),
      patch: "diff --git a/src/lib.rs b/src/lib.rs".to_string(),
    };
    let prompt = build_prompt(&context, None);
    assert!(prompt.contains("Branch:\nfeature"));
    assert!(prompt.contains("Changed files:\nM\tsrc/lib.rs"));
    assert!(prompt.contains("Staged diff:\ndiff --git"));
  }

  #[test]
  fn prompt_appends_user_suffix() {
    let prompt = build_prompt(&Context::default(), Some("Use conventional commits."));
    assert!(prompt.contains("Additional user prompt:\nUse conventional commits."));
  }

  #[test]
  fn prompt_skips_blank_user_suffix() {
    let prompt = build_prompt(&Context::default(), Some("  "));
    assert!(!prompt.contains("Additional user prompt"));
  }
}
