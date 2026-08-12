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
const PULSE_PERIOD: Duration = Duration::from_millis(1400);

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

/// A running agent. Driven by the element animator rather than a notifier, so
/// the pulse costs frames only while a session is actually working.
pub fn pulsing_dot(id: impl Into<ElementId>, color: Hsla) -> impl IntoElement {
  status_dot(color).with_animation(id, Animation::new(PULSE_PERIOD).repeat(), |dot, delta| {
    let phase = (delta * std::f32::consts::TAU).cos() * 0.5 + 0.5;

    dot.opacity(0.45 + 0.55 * phase)
  })
}

pub fn claude_icon(color: Hsla, size: f32) -> impl IntoElement {
  div()
    .flex_none()
    .text_color(color)
    .child(Icon::new(crate::icons::HelixIcon::ClaudeSunburst).size(px(size)))
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

/// The 16px rounded square carrying a project's initial.
pub fn project_avatar(label: &str, accent: Option<&str>, theme: &Theme) -> Div {
  let (fg, bg) = project_accent(accent, theme);
  let initial = label
    .chars()
    .next()
    .map(|c| c.to_uppercase().to_string())
    .unwrap_or_default();

  div()
    .size(px(16.0))
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded(px(4.0))
    .bg(bg)
    .text_size(px(9.5))
    .font_weight(gpui::FontWeight::SEMIBOLD)
    .text_color(fg)
    .child(initial)
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

pub fn git_branch_icon(color: Hsla) -> impl IntoElement {
  div()
    .flex_none()
    .text_color(color)
    .child(Icon::new(crate::icons::HelixIcon::GitBranch).size_3p5())
}

/// Driven by the element animator rather than a notifier, so the rotation is
/// continuous and only costs frames while a spinner is actually on screen.
pub fn spinner(id: impl Into<ElementId>, color: Hsla) -> impl IntoElement {
  div().flex_none().text_color(color).child(
    Icon::new(IconName::LoaderCircle).size_3p5().with_animation(
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

pub fn icon_button(id: impl Into<SharedString>, icon: impl Into<Icon>, theme: &Theme) -> Button {
  Button::new(id.into())
    .icon(Icon::new(icon).size_3p5())
    .ghost()
    .with_size(px(24.0))
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
