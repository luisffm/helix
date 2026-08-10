use crate::icons::HelixIcon;
use crate::theme::{Theme, c};
use gpui::Hsla;
use gpui_component::{Icon, IconName};
use helix_models::GitFileKind;
use std::path::Path;

pub fn icon(path: &Path) -> Icon {
  if helix_buffer::image_mime(path).is_some() {
    return Icon::new(HelixIcon::FileImage);
  }

  let svg = match helix_buffer::language::of(path) {
    "rust" | "typescript" | "tsx" | "javascript" | "python" | "go" | "ruby" | "java" | "c"
    | "cpp" | "c_sharp" | "swift" | "scala" | "zig" | "elixir" | "html" | "css" | "sequel"
    | "json" => HelixIcon::FileCode,
    "bash" | "make" | "cmake" => HelixIcon::FileTerminal,
    "toml" | "yaml" => HelixIcon::FileSliders,
    "markdown" => HelixIcon::FileText,
    _ => return Icon::new(IconName::File),
  };

  Icon::new(svg)
}

pub fn ignored_color() -> Hsla {
  c(0x4a4a4a)
}

pub fn folder_icon(expanded: bool) -> Icon {
  Icon::new(if expanded {
    IconName::FolderOpen
  } else {
    IconName::Folder
  })
}

pub fn status_letter(kind: GitFileKind) -> &'static str {
  match kind {
    GitFileKind::Added => "A",
    GitFileKind::Modified => "M",
    GitFileKind::Deleted => "D",
    GitFileKind::Renamed => "R",
    GitFileKind::Typechange => "T",
    GitFileKind::Untracked => "U",
    GitFileKind::Conflicted => "!",
  }
}

pub fn status_color(kind: GitFileKind, theme: &Theme) -> Hsla {
  match kind {
    GitFileKind::Modified | GitFileKind::Typechange => c(0xe2c08d),
    GitFileKind::Added => c(0x81b88b),
    GitFileKind::Deleted => c(0xc74e39),
    GitFileKind::Renamed | GitFileKind::Untracked => c(0x73c991),
    GitFileKind::Conflicted => theme.red,
  }
}

pub fn dominance(kind: GitFileKind) -> u8 {
  match kind {
    GitFileKind::Conflicted => 6,
    GitFileKind::Deleted => 5,
    GitFileKind::Modified | GitFileKind::Typechange => 4,
    GitFileKind::Added | GitFileKind::Untracked => 3,
    GitFileKind::Renamed => 2,
  }
}

#[cfg(test)]
mod tests {
  use super::dominance;
  use helix_models::GitFileKind;

  #[test]
  fn deleted_outranks_modified() {
    assert!(dominance(GitFileKind::Deleted) > dominance(GitFileKind::Modified));
  }

  #[test]
  fn modified_outranks_untracked() {
    assert!(dominance(GitFileKind::Modified) > dominance(GitFileKind::Untracked));
  }
}
