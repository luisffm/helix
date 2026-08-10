use helix_terminal::shell::quote_path;
use std::path::Path;

#[test]
fn plain_paths_stay_bare() {
  assert_eq!(
    quote_path(Path::new("/Users/luis/projects/helix/src/main.rs")),
    "/Users/luis/projects/helix/src/main.rs"
  );
}

#[test]
fn paths_with_spaces_get_quoted() {
  assert_eq!(
    quote_path(Path::new("/tmp/my notes.md")),
    "'/tmp/my notes.md'"
  );
}

#[test]
fn single_quotes_are_escaped_out_of_the_quoted_run() {
  assert_eq!(
    quote_path(Path::new("/tmp/luis's file")),
    r"'/tmp/luis'\''s file'"
  );
}

#[test]
fn shell_metacharacters_are_quoted() {
  assert_eq!(
    quote_path(Path::new("/tmp/a;rm -rf b")),
    "'/tmp/a;rm -rf b'"
  );
  assert_eq!(quote_path(Path::new("/tmp/$(id)")), "'/tmp/$(id)'");
}
