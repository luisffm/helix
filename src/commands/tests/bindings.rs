use gpui::{KeyContext, Keymap, Keystroke};
use helix_commands::{ActivateTab, ActivateWorkspace, SLOTS, default_bindings};

fn actions_for(keys: &str, contexts: &[&str]) -> Vec<String> {
  let keymap = Keymap::new(default_bindings());

  let typed: Vec<Keystroke> = keys
    .split_whitespace()
    .map(|key| Keystroke::parse(key).expect("keystroke should parse"))
    .collect();

  let stack: Vec<KeyContext> = contexts
    .iter()
    .map(|context| KeyContext::parse(context).expect("context should parse"))
    .collect();

  let (matched, _pending) = keymap.bindings_for_input(&typed, &stack);

  matched
    .iter()
    .map(|binding| binding.action().name().to_string())
    .collect()
}

#[test]
fn cmd_t_opens_a_terminal() {
  assert_eq!(
    actions_for("cmd-t", &[]),
    vec!["helix::NewTerminal".to_string()]
  );
}

#[test]
fn cmd_shift_t_opens_a_claude_session() {
  assert_eq!(
    actions_for("cmd-shift-t", &[]),
    vec!["helix::NewClaudeSession".to_string()]
  );
}

#[test]
fn terminal_bindings_only_apply_inside_a_terminal() {
  assert!(actions_for("cmd-c", &[]).is_empty());
  assert_eq!(
    actions_for("cmd-c", &["Terminal"]),
    vec!["helix::TerminalCopy".to_string()]
  );

  assert!(actions_for("cmd-s", &[]).is_empty());
  assert_eq!(
    actions_for("cmd-s", &["Editor"]),
    vec!["helix::SaveFile".to_string()]
  );
}

#[test]
fn cmd_l_toggles_the_right_sidebar() {
  assert_eq!(
    actions_for("cmd-l", &[]),
    vec!["helix::ToggleRightSidebar".to_string()]
  );
}

#[test]
fn digits_carry_their_slot() {
  let keymap = Keymap::new(default_bindings());

  for slot in 0..SLOTS {
    let digit = slot + 1;

    let tab = keymap.bindings_for_input(&[Keystroke::parse(&format!("cmd-{digit}")).unwrap()], &[]);
    let tab = tab.0.first().expect("cmd-{digit} should be bound");

    assert_eq!(
      tab
        .action()
        .as_any()
        .downcast_ref::<ActivateTab>()
        .map(|action| action.index),
      Some(slot)
    );

    let workspace =
      keymap.bindings_for_input(&[Keystroke::parse(&format!("ctrl-{digit}")).unwrap()], &[]);
    let workspace = workspace.0.first().expect("ctrl-{digit} should be bound");

    assert_eq!(
      workspace
        .action()
        .as_any()
        .downcast_ref::<ActivateWorkspace>()
        .map(|action| action.index),
      Some(slot)
    );
  }
}

#[test]
fn every_default_binding_parses_and_matches_itself() {
  for binding in default_bindings() {
    let keys = binding
      .keystrokes()
      .iter()
      .map(|keystroke| keystroke.inner().unparse())
      .collect::<Vec<_>>()
      .join(" ");

    let name = binding.action().name().to_string();

    let contexts: &[&str] = if binding.predicate().is_some() {
      &["Terminal", "Editor", "Input", "FileTree"]
    } else {
      &[]
    };

    assert!(
      actions_for(&keys, contexts).contains(&name),
      "`{keys}` did not match {name}"
    );
  }
}
