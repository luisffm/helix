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

pub const HEADER_HEIGHT: f32 = 42.0;

const SPINNER_PERIOD: Duration = Duration::from_millis(900);

pub fn section_label(text: impl Into<SharedString>, theme: &Theme) -> Div {
  div()
    .px_1()
    .pt_3()
    .pb_1()
    .text_xs()
    .text_color(theme.text_dim)
    .child(text.into())
}

pub fn status_dot(color: Hsla) -> Div {
  div().size(px(7.0)).rounded_full().bg(color).flex_none()
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
    .with_size(px(22.0))
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
