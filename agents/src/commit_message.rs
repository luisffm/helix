use anyhow::{Result, anyhow};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const PATCH_BYTE_BUDGET: usize = 200 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

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

#[derive(Clone, Debug)]
pub struct Spec {
  pub program: String,
  pub args: Vec<String>,
  pub stdin: String,
}

pub fn plan(prompt: String, model: Option<&str>) -> Spec {
  let mut args = vec![
    "-p".to_string(),
    "--output-format".to_string(),
    "text".to_string(),
    "--permission-mode".to_string(),
    "plan".to_string(),
  ];
  if let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) {
    args.push("--model".to_string());
    args.push(model.to_string());
  }
  Spec {
    program: "claude".to_string(),
    args,
    stdin: prompt,
  }
}

pub fn generate(cwd: &Path, spec: &Spec) -> Result<String> {
  let mut child = Command::new(&spec.program)
    .args(&spec.args)
    .current_dir(cwd)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|err| anyhow!("could not run {}: {err}", spec.program))?;

  child
    .stdin
    .take()
    .ok_or_else(|| anyhow!("could not open stdin"))?
    .write_all(spec.stdin.as_bytes())?;

  let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
  let pid = child.id();
  std::thread::Builder::new()
    .name("helix-commit-message-timeout".into())
    .spawn(move || {
      if done_rx.recv_timeout(TIMEOUT).is_err() {
        kill(pid);
      }
    })
    .ok();

  let output = child.wait_with_output();
  let _ = done_tx.send(());
  let output = output?;

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let message = if stderr.is_empty() {
      format!("{} exited without a message", spec.program)
    } else {
      stderr
    };
    return Err(anyhow!(message));
  }
  if output.stdout.len() > MAX_OUTPUT_BYTES {
    return Err(anyhow!("generated message was unexpectedly large"));
  }

  let message = sanitize(&String::from_utf8_lossy(&output.stdout));
  if message.is_empty() {
    return Err(anyhow!("model returned an empty commit message"));
  }
  Ok(message)
}

#[cfg(unix)]
fn kill(pid: u32) {
  unsafe {
    libc::kill(pid as i32, libc::SIGKILL);
  }
}

#[cfg(not(unix))]
fn kill(_pid: u32) {}

pub fn sanitize(raw: &str) -> String {
  let mut lines: Vec<&str> = raw.lines().collect();

  if lines
    .first()
    .map(|line| line.trim_start().starts_with("```"))
    .unwrap_or(false)
  {
    lines.remove(0);
    if lines
      .last()
      .map(|line| line.trim() == "```")
      .unwrap_or(false)
    {
      lines.pop();
    }
  }

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
  fn plan_omits_model_when_unset() {
    let spec = plan("hi".to_string(), None);
    assert!(!spec.args.contains(&"--model".to_string()));
    assert_eq!(spec.stdin, "hi");
  }

  #[test]
  fn plan_passes_model_when_set() {
    let spec = plan("hi".to_string(), Some("opus"));
    let position = spec.args.iter().position(|arg| arg == "--model").unwrap();
    assert_eq!(spec.args[position + 1], "opus");
  }

  #[test]
  fn plan_ignores_blank_model() {
    let spec = plan("hi".to_string(), Some("   "));
    assert!(!spec.args.contains(&"--model".to_string()));
  }

  #[test]
  fn plan_uses_print_mode() {
    let spec = plan("hi".to_string(), None);
    assert_eq!(spec.program, "claude");
    assert!(spec.args.contains(&"-p".to_string()));
    assert!(spec.args.contains(&"plan".to_string()));
  }

  #[test]
  fn sanitize_strips_code_fences() {
    let raw = "```\nfix: repair parser\n```\n";
    assert_eq!(sanitize(raw), "fix: repair parser");
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
