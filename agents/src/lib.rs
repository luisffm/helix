pub mod commit_message;

use helix_models::SessionKind;
use std::path::Path;

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
