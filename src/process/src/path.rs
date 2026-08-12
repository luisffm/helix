use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

const MARKER: &str = "__helix_path__";

static RESOLVED: OnceLock<Option<String>> = OnceLock::new();

/// Whether `program` can be found with the `PATH` this process carries.
pub fn resolvable(program: &str) -> bool {
  if program.contains('/') {
    return is_executable(Path::new(program));
  }

  let Ok(path) = std::env::var("PATH") else {
    return false;
  };

  path
    .split(':')
    .filter(|dir| !dir.is_empty())
    .any(|dir| is_executable(&Path::new(dir).join(program)))
}

/// The `PATH` a login shell would have given us.
///
/// A GUI launch inherits launchd's `PATH`, which carries little more than the
/// system directories, so `gh`, `claude` and everything else living under
/// Homebrew or a version manager is invisible. Exactly what it carries varies,
/// which is why the caller asks whether a program resolved rather than trying
/// to recognise the launchd `PATH` by its shape. The login shell is asked at
/// most once.
pub fn inherited() -> Option<&'static str> {
  RESOLVED.get_or_init(login_shell_path).as_deref()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
  use std::os::unix::fs::PermissionsExt;

  std::fs::metadata(path)
    .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
  path.is_file()
}

/// `-i` costs an interactive startup, but `PATH` is set in `.zshrc` far more
/// often than in a login-only profile, so a login-only shell would come back
/// missing exactly what it was asked for.
fn login_shell_path() -> Option<String> {
  let shell = std::env::var("SHELL").ok()?;

  let mut command = Command::new(shell);

  command
    .args([
      "-l",
      "-i",
      "-c",
      &format!("printf {MARKER}; /usr/bin/printenv PATH"),
    ])
    .env("TERM", "dumb");

  let output = crate::output(command, None, TIMEOUT).ok()?;

  if !output.status.success() {
    return None;
  }

  parse(&String::from_utf8_lossy(&output.stdout))
}

/// An interactive shell prints whatever its rc files print, so the answer is
/// read from the marker rather than from the start of stdout.
fn parse(stdout: &str) -> Option<String> {
  let tail = stdout.rsplit_once(MARKER)?.1;
  let path = tail.lines().next()?.trim();

  (!path.is_empty()).then(|| path.to_string())
}

#[cfg(test)]
mod tests {
  use super::{parse, resolvable};

  #[test]
  fn reads_past_rc_noise() {
    let stdout = "welcome to your shell\n__helix_path__/opt/homebrew/bin:/usr/bin\n";

    assert_eq!(parse(stdout).as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
  }

  #[test]
  fn rejects_a_missing_marker() {
    assert!(parse("some plugin failed to load\n").is_none());
  }

  #[test]
  fn rejects_an_empty_path() {
    assert!(parse("__helix_path__\n").is_none());
  }

  #[test]
  fn finds_a_system_binary() {
    assert!(resolvable("ls"));
    assert!(resolvable("/bin/ls"));
  }

  #[test]
  fn misses_what_is_not_there() {
    assert!(!resolvable("helix-definitely-not-a-binary"));
    assert!(!resolvable("/bin/helix-definitely-not-a-binary"));
  }
}
