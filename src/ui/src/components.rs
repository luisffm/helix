use crate::theme::Theme;
use gpui::Keystroke;
use gpui::{
  Animation, AnimationExt, Div, ElementId, Hsla, IntoElement, SharedString, Transformation, div,
  percentage, prelude::*, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::kbd::Kbd;
use gpui_component::{Icon, IconName, Sizable as _};
use std::time::Duration;

pub const HEADER_HEIGHT: f32 = 32.0;
/// What the window's own traffic lights occupy, measured rather than derived:
/// three 13px buttons on a 22.5px pitch from x=14, plus room to breathe. Any
/// header drawn level with them starts after this.
pub const TRAFFIC_LIGHTS: f32 = 84.0;
pub const STATUS_HEIGHT: f32 = 30.0;
pub const SIDEBAR_LEFT_WIDTH: f32 = 262.0;
pub const SIDEBAR_RIGHT_WIDTH: f32 = 302.0;

/// The design's type scale. gpui's `text_xs`/`text_sm` steps land between these
/// sizes, so every size is stated in px against the token it belongs to.
pub const TITLE: f32 = 13.0;
pub const BODY: f32 = 12.5;
pub const UI: f32 = 12.0;
pub const SMALL: f32 = 11.5;
pub const META: f32 = 11.0;
pub const TINY: f32 = 10.5;
pub const MICRO: f32 = 10.0;

const SPINNER_PERIOD: Duration = Duration::from_millis(900);

/// 10.5px, 600, dim. Callers pass the text already upper-cased, so drawing a
/// label costs no allocation. The design also asks for .09em of tracking, which
/// gpui has no styling for, so the labels carry every other property instead.
pub fn section_label(text: impl Into<SharedString>, theme: &Theme) -> Div {
  div()
    .text_size(px(TINY))
    .font_weight(gpui::FontWeight::SEMIBOLD)
    .text_color(theme.text_dim)
    .child(text.into())
}

pub fn status_dot(color: Hsla) -> Div {
  div().size(px(7.0)).rounded_full().bg(color).flex_none()
}

/// What an agent is waiting for, as a badge beside its mark: a ring with `?`
/// when it asked something, a solid disc when it only wants reading. Both in the
/// Claude accent, because both mean the session is holding for a person.
pub fn attention_badge(kind: helix_models::AgentAttention, theme: &Theme) -> Div {
  let badge = div()
    .size(px(11.0))
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded_full()
    .bg(theme.claude);

  match kind {
    helix_models::AgentAttention::Answer => badge
      .text_size(px(8.0))
      .font_weight(gpui::FontWeight::BOLD)
      .text_color(theme.claude_text)
      .child("?"),
    helix_models::AgentAttention::Report => badge.size(px(7.0)),
  }
}

/// How long a row takes to pick up its new emphasis when the current project
/// changes. One shot: the animator drives frames for this long and then stops, so
/// it costs nothing once it has settled — unlike a loop, which never does.
pub const EMPHASIS_MS: u64 = 220;

/// Blends between two colours in the space they are stored in, which is HSL here.
/// Interpolating hue would swing a muted grey through colours it should never
/// visit, so only saturation, lightness and alpha move.
pub fn blend(from: Hsla, to: Hsla, t: f32) -> Hsla {
  let t = t.clamp(0.0, 1.0);
  let mix = |a: f32, b: f32| a + (b - a) * t;

  Hsla {
    h: to.h,
    s: mix(from.s, to.s),
    l: mix(from.l, to.l),
    a: mix(from.a, to.a),
  }
}

pub fn claude_icon(color: Hsla, size: f32) -> impl IntoElement {
  div()
    .flex_none()
    .text_color(color)
    .child(Icon::new(crate::icons::HelixIcon::Claude).size(px(size)))
}

/// A pill: the small capsule the design uses for `primary`, `OPEN`, `worktree`
/// and the like.
pub fn pill(text: impl Into<SharedString>, fg: Hsla, bg: Hsla, size: f32) -> Div {
  div()
    .flex_none()
    .px(px(6.0))
    .py(px(1.0))
    .rounded_full()
    .bg(bg)
    .text_size(px(size))
    .font_weight(gpui::FontWeight::MEDIUM)
    .text_color(fg)
    .child(text.into())
}

/// A bordered cap: the design's own key-cap and badge shape, used where the
/// platform keystroke glyphs of `Kbd` are not what is being shown.
pub fn cap(text: impl Into<SharedString>, theme: &Theme) -> Div {
  div()
    .flex_none()
    .px(px(5.0))
    .py(px(1.0))
    .rounded(px(5.0))
    .border_1()
    .border_color(theme.panel_border)
    .text_size(px(TINY))
    .text_color(theme.text_muted)
    .child(text.into())
}

pub const PROJECT_ACCENTS: [&str; 5] = ["coral", "blue", "green", "purple", "yellow"];

/// A project's accent, as the foreground and the soft background its avatar is
/// drawn in. Unknown and unset names fall back to the neutral pair, which is
/// also what the active project is drawn in.
pub fn project_accent(name: Option<&str>, theme: &Theme) -> (Hsla, Hsla) {
  match name {
    Some("coral") => (theme.claude, theme.claude_soft),
    Some("blue") => (theme.blue, theme.blue_soft),
    Some("green") => (theme.green, theme.green_soft),
    Some("purple") => (theme.purple, theme.purple_soft),
    Some("yellow") => (theme.yellow, theme.yellow_soft),
    _ => (theme.text, theme.active),
  }
}

pub const EMOJI_CHOICES: [&str; 24] = [
  "🔥", "🚀", "⭐", "💎", "🦀", "🐍", "⚡", "🧠", "🛠️", "📦", "🌊", "🌙", "🍀", "🎯", "🧪", "🎨",
  "🐳", "🔮", "💼", "🏗️", "📱", "🌐", "🤖", "📁",
];

pub const PROJECT_ICONS: [(&str, IconName); 24] = [
  ("folder", IconName::Folder),
  ("folder-open", IconName::FolderOpen),
  ("folder-closed", IconName::FolderClosed),
  ("bot", IconName::Bot),
  ("square-terminal", IconName::SquareTerminal),
  ("star", IconName::Star),
  ("heart", IconName::Heart),
  ("globe", IconName::Globe),
  ("bell", IconName::Bell),
  ("book-open", IconName::BookOpen),
  ("chart-pie", IconName::ChartPie),
  ("calendar", IconName::Calendar),
  ("layout-dashboard", IconName::LayoutDashboard),
  ("map", IconName::Map),
  ("inbox", IconName::Inbox),
  ("frame", IconName::Frame),
  ("gallery", IconName::GalleryVerticalEnd),
  ("github", IconName::GitHub),
  ("palette", IconName::Palette),
  ("settings", IconName::Settings),
  ("user", IconName::User),
  ("sun", IconName::Sun),
  ("moon", IconName::Moon),
  ("building", IconName::Building2),
];

pub fn project_icon(name: &str) -> Option<IconName> {
  PROJECT_ICONS
    .iter()
    .find(|(id, _)| *id == name)
    .map(|(_, icon)| icon.clone())
}

/// A project's chosen glyph: the icon or emoji picked for it, drawn in its accent
/// so the two settings compose instead of replacing one another.
///
/// It sits on a soft chip, which is what separates a project from the worktrees
/// under it. Both levels are a line of text at nearly one size, so without the
/// chip the list reads as a single flat run of rows.
pub fn project_glyph(
  icon: Option<&str>,
  emoji: Option<&str>,
  accent: Option<&str>,
  theme: &Theme,
) -> Div {
  let (fg, bg) = project_accent(accent, theme);
  let chip = div()
    .size(px(20.0))
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded(px(5.0))
    .bg(bg);

  match (icon, emoji) {
    (_, Some(emoji)) if icon.is_none() => chip.text_size(px(GLYPH)).child(emoji.to_string()),
    (icon, _) => chip
      .text_color(fg)
      .child(Icon::new(icon.and_then(project_icon).unwrap_or(IconName::Folder)).size(px(GLYPH))),
  }
}

pub fn sparkline(values: &[f32], color: Hsla) -> impl IntoElement {
  let max = values.iter().copied().fold(1.0_f32, f32::max);

  div().flex().items_end().gap(px(1.0)).h(px(16.0)).children(
    values.iter().rev().take(24).rev().map(move |value| {
      let ratio = (value / max).clamp(0.05, 1.0);

      div()
        .w(px(2.0))
        .h(px(2.0 + 14.0 * ratio))
        .rounded_sm()
        .bg(color)
    }),
  )
}

/// An icon reads as part of the line it sits on, so callers give it that line's
/// text size rather than one of the library's fixed steps.
pub fn git_branch_icon(color: Hsla, size: f32) -> impl IntoElement {
  div()
    .flex_none()
    .text_color(color)
    .child(Icon::new(crate::icons::HelixIcon::GitBranch).size(px(size)))
}

/// Driven by the element animator rather than a notifier, so the rotation is
/// continuous and only costs frames while a spinner is actually on screen.
///
/// "Only while on screen" is the whole caveat: gpui has no per-element
/// invalidation, so an animated element asks the window for a frame every frame
/// and the entire element tree is rebuilt on each one. Measured at ~20% of a core
/// against ~5% with nothing animating. Worth it for a control the user is waiting
/// on; think twice before putting one somewhere it lives permanently.
pub fn spinner(id: impl Into<ElementId>, color: Hsla, size: f32) -> impl IntoElement {
  div().flex_none().text_color(color).child(
    Icon::new(IconName::LoaderCircle)
      .size(px(size))
      .with_animation(
        id,
        Animation::new(SPINNER_PERIOD).repeat(),
        |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
      ),
  )
}

/// A footer hint: the key caps for `keys`, space separated, then what they do.
/// The caps render with the platform's own glyphs.
pub fn key_hint(keys: &str, action: &'static str, theme: &Theme) -> Div {
  div()
    .flex()
    .items_center()
    .gap_1()
    .children(
      keys
        .split_whitespace()
        .filter_map(|key| Keystroke::parse(key).ok())
        .map(Kbd::new),
    )
    .child(div().text_xs().text_color(theme.text_dim).child(action))
}

/// An icon sitting inline with text takes the size of that text, so it reads as
/// part of the line rather than as a picture beside it.
pub const GLYPH: f32 = 12.0;

/// An icon standing on its own in a header or a toolbar has no text to match and
/// is a target as much as a mark, so it is drawn a step larger. Every such
/// control shares this size — nothing in a row of them is bigger than its
/// neighbour.
pub const CHROME_GLYPH: f32 = 14.0;

/// `Button` discards whatever size its icon was given and derives one from the
/// button's own box, at three quarters of it. So the glyph is sized by picking
/// the box that yields it — asking the icon does nothing.
const ICON_BUTTON_BOX: f32 = CHROME_GLYPH / 0.75;

pub fn icon_button(id: impl Into<SharedString>, icon: impl Into<Icon>, theme: &Theme) -> Button {
  Button::new(id.into())
    .icon(Icon::new(icon))
    .ghost()
    .with_size(px(ICON_BUTTON_BOX))
    .text_color(theme.text_muted)
}

pub fn elapsed_label(secs: u64) -> String {
  match secs {
    0..=9 => "now".to_string(),
    10..=59 => format!("{secs}s"),
    60..=3599 => format!("{}m", secs / 60),
    3600..=86_399 => format!("{}h", secs / 3600),
    _ => format!("{}d", secs / 86_400),
  }
}

pub fn ago(epoch_seconds: i64) -> String {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);

  elapsed_label((now - epoch_seconds).max(0) as u64)
}
