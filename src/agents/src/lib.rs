pub mod branch_name;
pub mod claude_cli;
pub mod commit_message;

use helix_models::SessionKind;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// `ps` is cheap but not guaranteed to answer, and the probe runs on the
/// background executor where a stuck child would hold a slot forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct LaunchSpec {
  pub program: String,
  pub args: Vec<String>,
}

pub fn launch_spec(kind: SessionKind) -> LaunchSpec {
  let shell = default_shell();

  match kind {
    SessionKind::Terminal => LaunchSpec {
      program: shell,
      args: vec!["-l".into()],
    },
    SessionKind::ClaudeCode => LaunchSpec {
      program: shell,
      args: vec!["-l".into(), "-c".into(), "claude".into()],
    },
  }
}

pub fn default_shell() -> String {
  user_login_shell()
    .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()))
    .unwrap_or_else(|| "/bin/zsh".to_string())
}

#[cfg(unix)]
fn user_login_shell() -> Option<String> {
  unsafe {
    let passwd = libc::getpwuid(libc::getuid());

    if passwd.is_null() {
      return None;
    }

    let shell = (*passwd).pw_shell;

    if shell.is_null() {
      return None;
    }

    let shell = std::ffi::CStr::from_ptr(shell)
      .to_string_lossy()
      .to_string();

    (!shell.is_empty()).then_some(shell)
  }
}

#[cfg(not(unix))]
fn user_login_shell() -> Option<String> {
  None
}

pub fn shell_display_name(program: &str) -> String {
  Path::new(program)
    .file_name()
    .map(|n| n.to_string_lossy().to_string())
    .unwrap_or_else(|| program.to_string())
}

/// Claude paints a spinner glyph into the terminal title while it works; the
/// title is only stable once that prefix is gone.
pub fn strip_spinner(title: &str) -> &str {
  title.trim_start_matches(|c: char| "✳✻✶✽*⁕ ".contains(c))
}

pub fn is_claude_process(pgid: i32) -> bool {
  let mut command = Command::new("ps");

  command.args(["-o", "comm=", "-o", "args=", "-p", &pgid.to_string()]);

  let Ok(output) = helix_process::output(command, None, PROBE_TIMEOUT) else {
    return false;
  };

  let text = String::from_utf8_lossy(&output.stdout).to_lowercase();

  text
    .split_whitespace()
    .take(4)
    .any(|token| token.rsplit('/').next() == Some("claude"))
}
