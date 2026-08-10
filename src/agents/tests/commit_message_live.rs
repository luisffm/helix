use helix_agents::commit_message::{Context, build_prompt, generate, plan};
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(cwd: &Path, args: &[&str]) {
  let output = Command::new("git")
    .args(args)
    .current_dir(cwd)
    .output()
    .unwrap();

  assert!(
    output.status.success(),
    "git {args:?} failed: {}",
    String::from_utf8_lossy(&output.stderr)
  );
}

#[test]
#[ignore = "spawns the real claude CLI; run with --ignored"]
fn generates_a_message_from_a_real_staged_diff() {
  let path: PathBuf = std::env::temp_dir().join(format!("helix-commit-msg-{}", std::process::id()));
  let _ = std::fs::remove_dir_all(&path);

  std::fs::create_dir_all(&path).unwrap();

  git(&path, &["init", "--initial-branch=main"]);
  git(&path, &["config", "user.name", "Helix Test"]);
  git(&path, &["config", "user.email", "test@helix.local"]);

  std::fs::write(
    path.join("timeout.rs"),
    "pub fn wait(secs: u64) -> bool {\n    secs < 30\n}\n",
  )
  .unwrap();

  git(&path, &["add", "-A"]);
  git(&path, &["commit", "-m", "initial"]);

  std::fs::write(
    path.join("timeout.rs"),
    "pub fn wait(secs: u64) -> bool {\n    secs <= 30\n}\n",
  )
  .unwrap();

  git(&path, &["add", "-A"]);

  let context = Context {
    branch: "main".to_string(),
    name_status: helix_git::index::staged_name_status(&path).unwrap(),
    patch: helix_git::index::staged_patch(&path, 200 * 1024).unwrap(),
  };
  assert!(context.patch.contains("secs <= 30"));

  let spec = plan(build_prompt(&context, None), None);
  let message = generate(&path, &spec).unwrap();

  println!("--- generated ---\n{message}\n---");

  assert!(!message.is_empty());
  assert!(!message.contains("```"));
  assert!(!message.to_lowercase().contains("co-authored-by"));

  let subject = message.lines().next().unwrap();

  assert!(
    subject.len() <= 72,
    "subject too long ({}): {subject}",
    subject.len()
  );

  let _ = std::fs::remove_dir_all(&path);
}
