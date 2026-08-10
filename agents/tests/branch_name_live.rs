use helix_agents::branch_name::{TYPES, generate, is_valid};

#[test]
#[ignore = "spawns the real claude CLI; run with --ignored"]
fn generates_a_valid_english_branch_name_from_portuguese_context() {
  let cwd = std::env::temp_dir();
  let name = generate(
    &cwd,
    "preciso arrumar o bug do índice do git que fica travado quando dois processos escrevem juntos",
    None,
  )
  .unwrap();

  println!("--- generated ---\n{name}\n---");
  assert!(is_valid(&name), "git would reject `{name}`");
  let (kind, summary) = name.split_once('/').expect("expected <type>/<summary>");
  assert!(TYPES.contains(&kind), "unexpected type `{kind}`");
  assert!(!summary.is_empty());
  assert_eq!(name, name.to_lowercase(), "expected lowercase");
  assert!(!name.contains(' '));
  assert!(name.len() <= 48, "too long: {}", name.len());
}
