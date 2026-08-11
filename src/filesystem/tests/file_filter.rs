use helix_filesystem::scan::scan_matches;
use std::path::{Path, PathBuf};

struct TempTree {
  root: PathBuf,
}

impl TempTree {
  fn new(name: &str) -> Self {
    let root = std::env::temp_dir().join(format!("helix-filter-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let tree = Self { root };

    for relative in [
      "Cargo.toml",
      "README.md",
      "src/reconfigure.rs",
      "src/state/config.rs",
      "src/github/src/lib.rs",
      "src/git/src/lib.rs",
    ] {
      tree.write(relative);
    }

    tree
  }

  fn write(&self, relative: &str) {
    let path = self.root.join(relative);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "contents\n").unwrap();
  }

  fn matches(&self, query: &str) -> Vec<String> {
    self.matches_with(query, &|_, _| false)
  }

  fn matches_with(&self, query: &str, ignored: &dyn Fn(&Path, bool) -> bool) -> Vec<String> {
    scan_matches(&self.root, query, false, ignored)
      .into_iter()
      .map(|node| {
        node
          .path
          .strip_prefix(&self.root)
          .unwrap_or(&node.path)
          .to_string_lossy()
          .to_string()
      })
      .collect()
  }
}

impl Drop for TempTree {
  fn drop(&mut self) {
    let _ = std::fs::remove_dir_all(&self.root);
  }
}

#[test]
fn prefix_beats_substring() {
  let tree = TempTree::new("prefix");
  let found = tree.matches("con");

  let prefix = found.iter().position(|path| path.ends_with("config.rs"));
  let substring = found
    .iter()
    .position(|path| path.ends_with("reconfigure.rs"));

  assert!(prefix.is_some() && substring.is_some(), "{found:?}");
  assert!(prefix < substring, "{found:?}");
}

#[test]
fn matching_is_case_insensitive() {
  let tree = TempTree::new("case");

  assert!(tree.matches("cargo").contains(&"Cargo.toml".to_string()));
  assert!(tree.matches("readme").contains(&"README.md".to_string()));
}

#[test]
fn a_plain_query_never_matches_only_the_directory() {
  let tree = TempTree::new("plain");

  assert!(tree.matches("github").is_empty());
}

#[test]
fn a_query_with_a_separator_reaches_the_path() {
  let tree = TempTree::new("separator");
  let found = tree.matches("github/src");

  assert_eq!(found, vec!["src/github/src/lib.rs".to_string()]);
}

#[test]
fn a_query_matches_across_gaps_in_the_name() {
  let tree = TempTree::new("fuzzy");
  let found = tree.matches("cfg");

  assert!(
    found.iter().any(|path| path.ends_with("config.rs")),
    "{found:?}"
  );
}

#[test]
fn no_match_returns_nothing() {
  let tree = TempTree::new("nomatch");

  assert!(tree.matches("zzzz").is_empty());
}

#[test]
fn ignored_files_sort_after_tracked_ones() {
  let tree = TempTree::new("ignored");
  let found = tree.matches_with("lib", &|path, is_dir| {
    !is_dir && path.to_string_lossy().contains("/github/")
  });

  let tracked = found.iter().position(|path| path.contains("/git/"));
  let ignored = found.iter().position(|path| path.contains("/github/"));

  assert!(tracked.is_some() && ignored.is_some(), "{found:?}");
  assert!(tracked < ignored, "{found:?}");
}
