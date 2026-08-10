use helix_ui::sidebar_right::match_rank;

#[test]
fn prefix_beats_substring() {
  let prefix = match_rank("config.rs", "src/state/config.rs", "con", false);
  let substring = match_rank("reconfigure.rs", "src/reconfigure.rs", "con", false);

  assert!(
    prefix < substring,
    "{prefix:?} should sort before {substring:?}"
  );

  assert_eq!(prefix, Some(0));
  assert_eq!(substring, Some(1));
}

#[test]
fn matching_is_case_insensitive() {
  assert_eq!(
    match_rank("Cargo.toml", "Cargo.toml", "cargo", false),
    Some(0)
  );

  assert_eq!(
    match_rank("README.md", "README.md", "readme", false),
    Some(0)
  );
}

#[test]
fn a_plain_query_never_matches_only_the_directory() {
  assert_eq!(
    match_rank("lib.rs", "src/github/src/lib.rs", "github", false),
    None
  );
}

#[test]
fn a_query_with_a_separator_reaches_the_path() {
  assert_eq!(
    match_rank("lib.rs", "src/github/src/lib.rs", "github/src", true),
    Some(2)
  );

  assert_eq!(
    match_rank("lib.rs", "src/git/src/lib.rs", "github/src", true),
    None
  );
}

#[test]
fn a_slash_query_is_matched_against_the_path_alone() {
  assert_eq!(
    match_rank("git.rs", "src/git/src/git.rs", "git/src", true),
    Some(2),
    "no bare filename can contain a separator, so only the path can match"
  );
}

#[test]
fn a_plain_query_ranks_by_the_name_only() {
  assert_eq!(
    match_rank("git.rs", "src/git/src/git.rs", "git", false),
    Some(0)
  );

  assert_eq!(
    match_rank("lib.rs", "src/git/src/lib.rs", "git", false),
    None
  );
}

#[test]
fn no_match_returns_none() {
  assert_eq!(match_rank("lib.rs", "src/ui/src/lib.rs", "zzz", true), None);
}
