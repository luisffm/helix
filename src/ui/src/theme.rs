use gpui::{App, Global, Hsla, SharedString, rgb};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
  Dark,
  Light,
}

impl Mode {
  pub fn from_id(id: &str) -> Self {
    if id == "light" {
      Mode::Light
    } else {
      Mode::Dark
    }
  }

  pub fn id(self) -> &'static str {
    match self {
      Mode::Dark => "dark",
      Mode::Light => "light",
    }
  }

  pub fn label(self) -> &'static str {
    match self {
      Mode::Dark => "Dark",
      Mode::Light => "Light",
    }
  }

  pub fn toggled(self) -> Self {
    match self {
      Mode::Dark => Mode::Light,
      Mode::Light => Mode::Dark,
    }
  }
}

#[derive(Clone)]
pub struct TermColors {
  pub bg: Hsla,
  pub fg: Hsla,
  pub cursor: Hsla,
  pub selection: Hsla,
  pub palette: [Hsla; 16],
}

#[derive(Clone)]
pub struct Theme {
  pub mode: Mode,
  /// The opaque base of the window: the centre column paints it directly, and
  /// the sidebars paint it at `side`'s alpha so the window blur shows through.
  pub win_tint: Hsla,
  pub side: Hsla,
  pub win_border: Hsla,
  pub panel: Hsla,
  pub panel2: Hsla,
  pub panel_border: Hsla,
  pub elevated: Hsla,
  pub hover: Hsla,
  pub active: Hsla,
  pub overlay: Hsla,
  pub text: Hsla,
  pub text_muted: Hsla,
  pub text_dim: Hsla,
  pub accent: Hsla,
  pub accent_soft: Hsla,
  pub accent_text: Hsla,
  pub claude: Hsla,
  pub claude_soft: Hsla,
  pub claude_text: Hsla,
  pub green: Hsla,
  pub green_soft: Hsla,
  pub red: Hsla,
  pub yellow: Hsla,
  pub yellow_soft: Hsla,
  pub blue: Hsla,
  pub blue_soft: Hsla,
  pub purple: Hsla,
  pub purple_soft: Hsla,
  pub cyan: Hsla,
  pub code_bg: Hsla,
  pub add_bg: Hsla,
  pub del_bg: Hsla,
  pub syntax_kw: Hsla,
  pub syntax_str: Hsla,
  pub syntax_num: Hsla,
  pub syntax_fn: Hsla,
  pub font_ui: SharedString,
  pub font_mono: SharedString,
  /// The terminal is the one surface whose font the user picks, and a shell
  /// wants the Nerd Font glyphs the detector looks for, so it is kept apart
  /// from the mono font the rest of the UI draws with.
  pub font_term: SharedString,
  pub term: TermColors,
}

impl Global for Theme {}

pub fn c(hex: u32) -> Hsla {
  rgb(hex).into()
}

/// The design states translucent tokens as `rgba(...)` over the surface below,
/// so they are written here as a colour plus a straight alpha rather than as a
/// blended hex.
pub fn fade(hex: u32, alpha: f32) -> Hsla {
  let mut color: Hsla = rgb(hex).into();

  color.a = alpha;

  color
}

pub fn white(alpha: f32) -> Hsla {
  fade(0xffffff, alpha)
}

pub fn ink(alpha: f32) -> Hsla {
  fade(0x1e1914, alpha)
}

pub const BLUR_LEVELS: [(&str, &str); 4] = [
  ("off", "Off"),
  ("subtle", "Subtle"),
  ("medium", "Medium"),
  ("strong", "Strong"),
];

/// Only the sidebars and the status bar are translucent. The centre column
/// stays opaque at `win_tint`, so the blur level never touches it.
pub fn apply_blur_level(theme: &mut Theme, level: &str) {
  theme.side.a = match level {
    "off" => 1.0,
    "subtle" => 0.93,
    "strong" => 0.66,
    _ => 0.82,
  };
}

fn hex_string(color: Hsla) -> String {
  let rgba = gpui::Rgba::from(color);

  format!(
    "#{:02x}{:02x}{:02x}",
    (rgba.r * 255.0).round() as u8,
    (rgba.g * 255.0).round() as u8,
    (rgba.b * 255.0).round() as u8
  )
}

/// The highlighter resolves capture names against a `HighlightTheme`, whose
/// style fields are private, so the design's four syntax colours are applied by
/// deserializing them over the library's theme. Every capture the design does
/// not name is left unstyled and falls through to the line's own colour, which
/// is how the diff in the prototype reads. Built once per theme change.
pub fn syntax_theme(theme: &Theme) -> std::sync::Arc<gpui_component::highlighter::HighlightTheme> {
  use gpui_component::highlighter::HighlightTheme;

  let base = match theme.mode {
    Mode::Dark => HighlightTheme::default_dark(),
    Mode::Light => HighlightTheme::default_light(),
  };

  let mut built = (*base).clone();

  built.style.editor_background = Some(Hsla::transparent_black());
  built.style.editor_line_number = Some(theme.text_dim);
  built.style.editor_active_line_number = Some(theme.text_muted);

  let kw = hex_string(theme.syntax_kw);
  let string = hex_string(theme.syntax_str);
  let num = hex_string(theme.syntax_num);
  let func = hex_string(theme.syntax_fn);
  let comment = hex_string(theme.text_dim);

  let described = serde_json::json!({
    "keyword": { "color": kw },
    "operator": { "color": kw },
    "boolean": { "color": kw },
    "preproc": { "color": kw },
    "label": { "color": kw },
    "tag": { "color": kw },
    "string": { "color": string },
    "string.escape": { "color": string },
    "string.regex": { "color": string },
    "string.special": { "color": string },
    "string.special.symbol": { "color": string },
    "number": { "color": num },
    "constant": { "color": num },
    "function": { "color": func },
    "type": { "color": func },
    "constructor": { "color": func },
    "enum": { "color": func },
    "attribute": { "color": func },
    "comment": { "color": comment },
    "comment.doc": { "color": comment },
  });

  if let Ok(syntax) = serde_json::from_value(described) {
    built.style.syntax = syntax;
  }

  std::sync::Arc::new(built)
}

pub fn sync_component_theme(cx: &mut App) {
  use gpui_component::theme::{Theme as ComponentTheme, ThemeMode};

  let ours = Theme::of(cx).clone();

  ComponentTheme::change(
    match ours.mode {
      Mode::Dark => ThemeMode::Dark,
      Mode::Light => ThemeMode::Light,
    },
    None,
    cx,
  );

  let component = ComponentTheme::global_mut(cx);

  component.highlight_theme = syntax_theme(&ours);
  component.font_family = ours.font_ui.clone();
  component.mono_font_family = ours.font_mono.clone();
  component.radius = gpui::px(7.0);
  component.radius_lg = gpui::px(10.0);

  let colors = &mut component.colors;

  colors.background = ours.win_tint;
  colors.foreground = ours.text;
  colors.popover = ours.win_tint;
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
  colors.primary_foreground = ours.accent_text;

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
  colors.selection = ours.term.selection;
  colors.scrollbar = Hsla::transparent_black();
  colors.scrollbar_thumb = ours.panel_border;
  colors.scrollbar_thumb_hover = ours.text_dim;
}

/// The palette and the blur level both come from the config file, and the level
/// is needed again for the window background, so they are read together.
pub fn configured() -> (Theme, String) {
  let config = helix_state::config::load();

  let level = config.blur_level.unwrap_or_else(|| "medium".to_string());

  let mut theme = Theme::for_mode(Mode::from_id(config.theme.as_deref().unwrap_or("dark")));

  apply_blur_level(&mut theme, &level);

  (theme, level)
}

pub fn appearance_for_level(level: &str) -> gpui::WindowBackgroundAppearance {
  if level == "off" {
    gpui::WindowBackgroundAppearance::Opaque
  } else {
    gpui::WindowBackgroundAppearance::Blurred
  }
}

impl Theme {
  pub fn of(cx: &App) -> &Theme {
    cx.global::<Theme>()
  }

  pub fn for_mode(mode: Mode) -> Self {
    match mode {
      Mode::Dark => Self::dark(),
      Mode::Light => Self::light(),
    }
  }

  pub fn is_dark(&self) -> bool {
    self.mode == Mode::Dark
  }

  pub fn dark() -> Self {
    Self {
      mode: Mode::Dark,
      win_tint: c(0x19191d),
      side: c(0x19191d),
      win_border: white(0.14),
      panel: white(0.045),
      panel2: white(0.03),
      panel_border: white(0.09),
      elevated: white(0.09),
      hover: white(0.07),
      active: white(0.12),
      overlay: fade(0x0a0a0c, 0.3),
      text: c(0xf0eeea),
      text_muted: c(0xa49f97),
      text_dim: c(0x6b6761),
      accent: c(0xe8e6e2),
      accent_soft: white(0.1),
      accent_text: c(0x1a1a1c),
      claude: c(0xd97757),
      claude_soft: fade(0xd97757, 0.15),
      claude_text: c(0x1a1512),
      green: c(0x7ec584),
      green_soft: fade(0x7ec584, 0.14),
      red: c(0xe5636a),
      yellow: c(0xd9a94a),
      yellow_soft: fade(0xd9a94a, 0.14),
      blue: c(0x8fa7c9),
      blue_soft: fade(0x8fa7c9, 0.16),
      purple: c(0xc29fd8),
      purple_soft: fade(0xc29fd8, 0.16),
      cyan: c(0x8fbfcf),
      code_bg: fade(0x09090b, 0.38),
      add_bg: fade(0x7ec584, 0.08),
      del_bg: fade(0xe5636a, 0.08),
      syntax_kw: c(0xc29fd8),
      syntax_str: c(0x8fca94),
      syntax_num: c(0xd8b573),
      syntax_fn: c(0x8ab0dd),
      font_ui: "Geist".into(),
      font_mono: "Geist Mono".into(),
      font_term: "Geist Mono".into(),
      term: TermColors {
        bg: c(0x19191d),
        fg: c(0xd6d6d6),
        cursor: c(0xd6d6d6),
        selection: white(0.17),
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

  /// The design fixes the light palette down to the syntax colours but says
  /// nothing about the ANSI palette, so the sixteen slots are derived from the
  /// light accents: normal slots readable on paper, bright slots darker still
  /// rather than lighter, which on a light ground is the legible direction.
  pub fn light() -> Self {
    Self {
      mode: Mode::Light,
      win_tint: c(0xf5f3f0),
      side: c(0xf5f3f0),
      win_border: white(0.6),
      panel: white(0.55),
      panel2: white(0.35),
      panel_border: ink(0.1),
      elevated: ink(0.07),
      hover: ink(0.05),
      active: ink(0.1),
      overlay: fade(0x281e19, 0.18),
      text: c(0x2a2722),
      text_muted: c(0x6e6961),
      text_dim: c(0xa39d93),
      accent: c(0x2a2722),
      accent_soft: ink(0.08),
      accent_text: c(0xf5f3f0),
      claude: c(0xc96442),
      claude_soft: fade(0xc96442, 0.13),
      claude_text: c(0xfdf6f2),
      green: c(0x3d8a4d),
      green_soft: fade(0x3d8a4d, 0.12),
      red: c(0xc04350),
      yellow: c(0xa3781f),
      yellow_soft: fade(0xa3781f, 0.13),
      blue: c(0x4a6b9c),
      blue_soft: fade(0x4a6b9c, 0.14),
      purple: c(0x8a4fa8),
      purple_soft: fade(0x8a4fa8, 0.14),
      cyan: c(0x2f7d80),
      code_bg: white(0.5),
      add_bg: fade(0x3d8a4d, 0.09),
      del_bg: fade(0xc04350, 0.07),
      syntax_kw: c(0x8a4fa8),
      syntax_str: c(0x3d8a4d),
      syntax_num: c(0xa3781f),
      syntax_fn: c(0x3565b0),
      font_ui: "Geist".into(),
      font_mono: "Geist Mono".into(),
      font_term: "Geist Mono".into(),
      term: TermColors {
        bg: c(0xf5f3f0),
        fg: c(0x2a2722),
        cursor: c(0x2a2722),
        selection: ink(0.16),
        palette: [
          c(0x2a2722),
          c(0xc04350),
          c(0x3d8a4d),
          c(0xa3781f),
          c(0x4a6b9c),
          c(0x8a4fa8),
          c(0x2f7d80),
          c(0x6e6961),
          c(0x615c54),
          c(0xa33845),
          c(0x2f6f3d),
          c(0x84621a),
          c(0x3a5680),
          c(0x71408b),
          c(0x25666a),
          c(0x2a2722),
        ],
      },
    }
  }
}
