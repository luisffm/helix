use gpui::SharedString;
use gpui_component::IconNamed;

/// The icons helix ships itself, alongside `gpui_component::IconName`. The paths
/// resolve against the asset source registered in the app crate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelixIcon {
  CircleSlash,
  FileCode,
  FileImage,
  FileSliders,
  FileTerminal,
  FileText,
  FolderPlus,
  GitBranch,
  GitCompare,
  ListCollapse,
  Refresh,
}

impl IconNamed for HelixIcon {
  fn path(self) -> SharedString {
    match self {
      HelixIcon::CircleSlash => "icons/circle-slash.svg",
      HelixIcon::FileCode => "icons/file-code.svg",
      HelixIcon::FileImage => "icons/file-image.svg",
      HelixIcon::FileSliders => "icons/file-sliders.svg",
      HelixIcon::FileTerminal => "icons/file-terminal.svg",
      HelixIcon::FileText => "icons/file-text.svg",
      HelixIcon::FolderPlus => "icons/folder-plus.svg",
      HelixIcon::GitBranch => "icons/git-branch.svg",
      HelixIcon::GitCompare => "icons/git-compare.svg",
      HelixIcon::ListCollapse => "icons/list-collapse.svg",
      HelixIcon::Refresh => "icons/refresh.svg",
    }
    .into()
  }
}
