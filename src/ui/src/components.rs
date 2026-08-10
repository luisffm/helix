use crate::theme::Theme;
use gpui::{
  App, Context, Div, Hsla, IntoElement, Render, SharedString, Stateful, Transformation, div,
  percentage, prelude::*, px,
};
use gpui_component::{Icon, IconName};
use std::time::Duration;

pub const HEADER_HEIGHT: f32 = 42.0;

const SPINNER_STEPS: u32 = 12;
const SPINNER_TICK: Duration = Duration::from_millis(75);

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

#[derive(Default)]
pub struct SpinnerClock {
  step: u32,
  ticking: bool,
}

impl SpinnerClock {
  pub fn step(&self) -> u32 {
    self.step
  }
}

pub trait Spinning: Render {
  fn spinner_clock(&mut self) -> &mut SpinnerClock;
  fn spinner_active(&self, cx: &App) -> bool;
}

pub fn drive_spinner<V: Spinning>(view: &mut V, cx: &mut Context<V>) {
  if view.spinner_clock().ticking || !view.spinner_active(cx) {
    return;
  }

  view.spinner_clock().ticking = true;

  cx.spawn(async move |this, cx| {
    loop {
      cx.background_executor().timer(SPINNER_TICK).await;

      let advanced = this.update(cx, |view, cx| {
        if !view.spinner_active(cx) {
          view.spinner_clock().ticking = false;

          return false;
        }

        let clock = view.spinner_clock();

        clock.step = (clock.step + 1) % SPINNER_STEPS;

        cx.notify();

        true
      });

      if !matches!(advanced, Ok(true)) {
        break;
      }
    }
  })
  .detach();
}

pub fn spinner(step: u32, color: Hsla) -> impl IntoElement {
  let turn = (step % SPINNER_STEPS) as f32 / SPINNER_STEPS as f32;

  div().flex_none().text_color(color).child(
    Icon::new(IconName::LoaderCircle)
      .size_3p5()
      .transform(Transformation::rotate(percentage(turn))),
  )
}

pub fn icon_button(
  id: impl Into<SharedString>,
  icon: impl Into<Icon>,
  theme: &Theme,
) -> Stateful<Div> {
  icon_button_base(id, theme).child(Icon::new(icon).size_3p5())
}

fn icon_button_base(id: impl Into<SharedString>, theme: &Theme) -> Stateful<Div> {
  div()
    .id(id.into())
    .size(px(22.0))
    .flex()
    .items_center()
    .justify_center()
    .rounded_sm()
    .text_color(theme.text_muted)
    .hover(|style| style.bg(theme.hover).text_color(theme.text))
    .cursor_pointer()
}

pub fn ago(epoch_seconds: i64) -> String {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);

  let delta = (now - epoch_seconds).max(0);

  match delta {
    0..=59 => format!("{delta}s"),
    60..=3599 => format!("{}m", delta / 60),
    3600..=86_399 => format!("{}h", delta / 3600),
    _ => format!("{}d", delta / 86_400),
  }
}
