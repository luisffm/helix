use gpui::SharedString;
use gpui_component::IconNamed;

/// The icons helix ships itself, alongside `gpui_component::IconName`. The paths
/// resolve against the asset source registered in the app crate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelixIcon {
  ClaudeSunburst,
  FileCode,
  FileImage,
  FileSliders,
  FileTerminal,
  FileText,
  FolderPlus,
  GitBranch,
  GitCompare,
  GitMerge,
  GitPullRequest,
  GitPullRequestClosed,
  ListCollapse,
  ListFilter,
  Refresh,
  Sliders,
}

impl IconNamed for HelixIcon {
  fn path(self) -> SharedString {
    match self {
      HelixIcon::ClaudeSunburst => "icons/claude-sunburst.svg",
      HelixIcon::FileCode => "icons/file-code.svg",
      HelixIcon::FileImage => "icons/file-image.svg",
      HelixIcon::FileSliders => "icons/file-sliders.svg",
      HelixIcon::FileTerminal => "icons/file-terminal.svg",
      HelixIcon::FileText => "icons/file-text.svg",
      HelixIcon::FolderPlus => "icons/folder-plus.svg",
      HelixIcon::GitBranch => "icons/git-branch.svg",
      HelixIcon::GitCompare => "icons/git-compare.svg",
      HelixIcon::GitMerge => "icons/git-merge.svg",
      HelixIcon::GitPullRequest => "icons/git-pull-request.svg",
      HelixIcon::GitPullRequestClosed => "icons/git-pull-request-closed.svg",
      HelixIcon::ListCollapse => "icons/list-collapse.svg",
      HelixIcon::ListFilter => "icons/list-filter.svg",
      HelixIcon::Refresh => "icons/refresh.svg",
      HelixIcon::Sliders => "icons/sliders.svg",
    }
    .into()
  }
}
