use gpui::{App, Global, Hsla, SharedString, rgb, rgba};

#[derive(Clone)]
pub struct TermColors {
  pub fg: Hsla,
  pub cursor: Hsla,
  pub selection: Hsla,
  pub palette: [Hsla; 16],
}

#[derive(Clone)]
pub struct Theme {
  pub bg: Hsla,
  pub content: Hsla,
  pub panel: Hsla,
  pub panel_border: Hsla,
  pub elevated: Hsla,
  pub hover: Hsla,
  pub active: Hsla,
  pub text: Hsla,
  pub text_muted: Hsla,
  pub text_dim: Hsla,
  pub accent: Hsla,
  pub claude: Hsla,
  pub green: Hsla,
  pub red: Hsla,
  pub yellow: Hsla,
  pub blue: Hsla,
  pub purple: Hsla,
  pub cyan: Hsla,
  pub font_ui: SharedString,
  pub font_mono: SharedString,
  pub term: TermColors,
}

impl Global for Theme {}

pub fn c(hex: u32) -> Hsla {
  rgb(hex).into()
}

pub const BLUR_LEVELS: [(&str, &str); 4] = [
  ("off", "Off"),
  ("subtle", "Subtle"),
  ("medium", "Medium"),
  ("strong", "Strong"),
];

pub fn apply_blur_level(theme: &mut Theme, level: &str) {
  theme.bg.a = match level {
    "off" => 1.0,
    "subtle" => 0.96,
    "strong" => 0.74,
    _ => 0.88,
  };
}

pub fn sync_component_theme(cx: &mut App) {
  use gpui_component::theme::{Theme as ComponentTheme, ThemeMode};
  use std::sync::Arc;

  ComponentTheme::change(ThemeMode::Dark, None, cx);

  let ours = Theme::of(cx).clone();
  let component = ComponentTheme::global_mut(cx);

  let mut highlight = (*component.highlight_theme).clone();
  highlight.style.editor_background = Some(Hsla::transparent_black());
  highlight.style.editor_line_number = Some(ours.text_dim);
  highlight.style.editor_active_line_number = Some(ours.text_muted);

  component.highlight_theme = Arc::new(highlight);
  component.font_family = ours.font_ui.clone();
  component.mono_font_family = ours.font_mono.clone();
  component.radius = gpui::px(6.0);
  component.radius_lg = gpui::px(10.0);

  let colors = &mut component.colors;

  colors.background = ours.bg;
  colors.foreground = ours.text;
  colors.popover = ca(0x1a1a1af8);
  colors.popover_foreground = ours.text;

  colors.border = ours.panel_border;
  colors.input = ours.panel_border;
  colors.ring = ours.active;

  colors.muted = ours.elevated;
  colors.muted_foreground = ours.text_dim;
  colors.accent = ours.hover;
  colors.accent_foreground = ours.text;

  colors.primary = ours.accent;
  colors.primary_hover = ours.accent;
  colors.primary_active = ours.accent;
  colors.primary_foreground = ca(0x161616ff);

  colors.secondary = ours.elevated;
  colors.secondary_hover = ours.hover;
  colors.secondary_active = ours.active;
  colors.secondary_foreground = ours.text;

  colors.list_hover = ours.hover;
  colors.list_active = ours.active;
  colors.list_active_border = ours.active;

  // Read by the components already in use, and left at the library's own dark
  // palette until they are set here.
  colors.caret = ours.accent;
  colors.drag_border = ours.active;
  colors.selection = ours.term.selection;
  colors.scrollbar = Hsla::transparent_black();
  colors.scrollbar_thumb = ours.panel_border;
  colors.scrollbar_thumb_hover = ours.text_dim;
}

pub fn appearance_for_level(level: &str) -> gpui::WindowBackgroundAppearance {
  if level == "off" {
    gpui::WindowBackgroundAppearance::Opaque
  } else {
    gpui::WindowBackgroundAppearance::Blurred
  }
}

pub fn ca(hex: u32) -> Hsla {
  rgba(hex).into()
}

impl Theme {
  pub fn of(cx: &App) -> &Theme {
    cx.global::<Theme>()
  }

  pub fn dark() -> Self {
    Self {
      bg: ca(0x0d0d0dc9),
      content: c(0x0d0d0d),
      panel: ca(0xffffff0a),
      panel_border: ca(0xffffff14),
      elevated: ca(0xffffff12),
      hover: ca(0xffffff14),
      active: ca(0xffffff38),
      text: c(0xededed),
      text_muted: c(0xa0a0a0),
      text_dim: c(0x6e6e6e),
      accent: c(0xe5e5e5),
      claude: c(0xd97757),
      green: c(0x7bc47f),
      red: c(0xe5636a),
      yellow: c(0xd6a243),
      blue: c(0x8f9ba8),
      purple: c(0xb3a6c4),
      cyan: c(0x8fbfcf),
      font_ui: ".SystemUIFont".into(),
      font_mono: "Menlo".into(),
      term: TermColors {
        fg: c(0xd6d6d6),
        cursor: c(0xd6d6d6),
        selection: ca(0xffffff2b),
        palette: [
          c(0x1a1a1a),
          c(0xde6e6e),
          c(0x86c58b),
          c(0xd8b573),
          c(0x8aa9d6),
          c(0xb59ad0),
          c(0x83bfcf),
          c(0xb8b8b8),
          c(0x4d4d4d),
          c(0xe98787),
          c(0x9fd6a3),
          c(0xe6c688),
          c(0xa3bde3),
          c(0xc9b1de),
          c(0x9cd3e0),
          c(0xe6e6e6),
        ],
      },
    }
  }
}
