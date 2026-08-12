use gpui::{App, AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Geist ships one file per weight, so the UI font is only available at the
/// weights bundled here: 400, 500 and 600 for text, 400 and 500 for mono.
const FONTS: [&[u8]; 5] = [
  include_bytes!("../../../assets/fonts/Geist-Regular.ttf"),
  include_bytes!("../../../assets/fonts/Geist-Medium.ttf"),
  include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf"),
  include_bytes!("../../../assets/fonts/GeistMono-Regular.ttf"),
  include_bytes!("../../../assets/fonts/GeistMono-Medium.ttf"),
];

pub fn register_fonts(cx: &App) {
  let fonts = FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect();

  if let Err(err) = cx.text_system().add_fonts(fonts) {
    eprintln!("helix: bundled fonts failed to load: {err}");
  }
}

const GIT_BRANCH_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"##;

const FOLDER_PLUS_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"##;

const GIT_COMPARE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="5" cy="6" r="3"/><path d="M12 6h5a2 2 0 0 1 2 2v7"/><path d="m15 9-3-3 3-3"/><circle cx="19" cy="18" r="3"/><path d="M12 18H7a2 2 0 0 1-2-2V9"/><path d="m9 15 3 3-3 3"/></svg>"##;

const FILE_CODE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="m10 13-2 2 2 2"/><path d="m14 13 2 2-2 2"/></svg>"##;

const FILE_TEXT_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 13h6"/><path d="M9 17h6"/></svg>"##;

const FILE_TERMINAL_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="m8 16 2-2-2-2"/><path d="M12 18h4"/></svg>"##;

const FILE_SLIDERS_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M8 12h8"/><path d="M11 10v4"/><path d="M8 18h8"/><path d="M14 16v4"/></svg>"##;

const FILE_IMAGE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><circle cx="10" cy="13" r="2"/><path d="m20 19-3.5-3.5a2 2 0 0 0-2.8 0L8 21"/></svg>"##;

const LIST_COLLAPSE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 10 2.5-2.5L3 5"/><path d="m3 19 2.5-2.5L3 14"/><path d="M10 6h11"/><path d="M10 12h11"/><path d="M10 18h11"/></svg>"##;

const REFRESH_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>"##;

const CLAUDE_SUNBURST_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 12h6M15.5 14l3.4 2M14 15.5l3 5.2M12 16v4M10 15.5l-3 5.2M8.5 14l-3.4 2M8 12H2M8.5 10 5.1 8M10 8.5 7 3.3M12 8V4M14 8.5l3-5.2M15.5 10l3.4-2"/></svg>"##;

const GIT_PULL_REQUEST_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="6" cy="6" r="3"/><circle cx="18" cy="18" r="3"/><path d="M6 9v6a3 3 0 0 0 3 3h6"/></svg>"##;

const SLIDERS_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 6H11M7 6H3M21 12h-8M9 12H3M21 18h-4M13 18H3"/><circle cx="9" cy="6" r="1.5"/><circle cx="11" cy="12" r="1.5"/><circle cx="15" cy="18" r="1.5"/></svg>"##;

const LIST_FILTER_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h16M7 12h10M10 18h4"/></svg>"##;

const EXTRA_ICONS: [(&str, &[u8]); 14] = [
  ("icons/claude-sunburst.svg", CLAUDE_SUNBURST_SVG),
  ("icons/git-pull-request.svg", GIT_PULL_REQUEST_SVG),
  ("icons/sliders.svg", SLIDERS_SVG),
  ("icons/list-filter.svg", LIST_FILTER_SVG),
  ("icons/git-branch.svg", GIT_BRANCH_SVG),
  ("icons/folder-plus.svg", FOLDER_PLUS_SVG),
  ("icons/git-compare.svg", GIT_COMPARE_SVG),
  ("icons/file-code.svg", FILE_CODE_SVG),
  ("icons/file-text.svg", FILE_TEXT_SVG),
  ("icons/file-terminal.svg", FILE_TERMINAL_SVG),
  ("icons/file-sliders.svg", FILE_SLIDERS_SVG),
  ("icons/file-image.svg", FILE_IMAGE_SVG),
  ("icons/list-collapse.svg", LIST_COLLAPSE_SVG),
  ("icons/refresh.svg", REFRESH_SVG),
];

pub struct HelixAssets;

impl AssetSource for HelixAssets {
  fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
    if let Some((_, bytes)) = EXTRA_ICONS.iter().find(|(name, _)| *name == path) {
      return Ok(Some(Cow::Borrowed(bytes)));
    }

    gpui_component_assets::Assets.load(path)
  }

  fn list(&self, path: &str) -> Result<Vec<SharedString>> {
    let mut entries = gpui_component_assets::Assets.list(path)?;

    for (name, _) in EXTRA_ICONS {
      if name.starts_with(path) {
        entries.push(name.into());
      }
    }

    Ok(entries)
  }
}
