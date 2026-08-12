use crate::icons::HelixIcon;
use crate::theme::Theme;
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

pub fn ignored_color(theme: &Theme) -> Hsla {
  theme.text_dim
}

pub fn folder_icon(expanded: bool) -> Icon {
  Icon::new(if expanded {
    IconName::FolderOpen
  } else {
    IconName::Folder
  })
}

pub fn status_color(kind: GitFileKind, theme: &Theme) -> Hsla {
  match kind {
    GitFileKind::Modified | GitFileKind::Typechange => theme.yellow,
    GitFileKind::Added | GitFileKind::Renamed | GitFileKind::Untracked => theme.green,
    GitFileKind::Deleted => theme.red,
    GitFileKind::Conflicted => theme.red,
  }
}
