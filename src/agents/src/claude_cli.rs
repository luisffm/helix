use anyhow::{Result, anyhow};
use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

pub const TIMEOUT: Duration = Duration::from_secs(60);
pub const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Spec {
  pub program: String,
  pub args: Vec<String>,
  pub stdin: String,
}

pub fn print_mode(prompt: String, model: Option<&str>) -> Spec {
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

pub fn run(cwd: &Path, spec: &Spec) -> Result<String> {
  let mut child = helix_process::command(&spec.program)
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
    .name("helix-claude-cli-timeout".into())
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
    return Err(anyhow!(if stderr.is_empty() {
      format!("{} exited without a message", spec.program)
    } else {
      stderr
    }));
  }

  if output.stdout.len() > MAX_OUTPUT_BYTES {
    return Err(anyhow!("model output was unexpectedly large"));
  }

  Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(unix)]
fn kill(pid: u32) {
  unsafe {
    libc::kill(pid as i32, libc::SIGKILL);
  }
}

#[cfg(not(unix))]
fn kill(_pid: u32) {}

pub fn strip_code_fence(raw: &str) -> Vec<&str> {
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

  lines
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn print_mode_omits_model_when_unset() {
    let spec = print_mode("hi".to_string(), None);

    assert!(!spec.args.contains(&"--model".to_string()));
    assert_eq!(spec.stdin, "hi");
  }

  #[test]
  fn print_mode_passes_model_when_set() {
    let spec = print_mode("hi".to_string(), Some("opus"));
    let at = spec.args.iter().position(|arg| arg == "--model").unwrap();

    assert_eq!(spec.args[at + 1], "opus");
  }

  #[test]
  fn print_mode_ignores_blank_model() {
    let spec = print_mode("hi".to_string(), Some("  "));

    assert!(!spec.args.contains(&"--model".to_string()));
  }

  #[test]
  fn print_mode_uses_the_documented_flags() {
    let spec = print_mode("hi".to_string(), None);

    assert_eq!(spec.program, "claude");
    assert!(spec.args.contains(&"-p".to_string()));
    assert!(spec.args.contains(&"text".to_string()));
    assert!(spec.args.contains(&"plan".to_string()));
  }

  #[test]
  fn strip_code_fence_removes_wrapping_fence() {
    assert_eq!(strip_code_fence("```\nbody\n```\n"), vec!["body"]);
  }

  #[test]
  fn strip_code_fence_leaves_plain_text() {
    assert_eq!(strip_code_fence("body\nmore"), vec!["body", "more"]);
  }
}
