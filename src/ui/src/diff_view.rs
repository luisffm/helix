use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, FocusHandle, Focusable, Hsla, IntoElement, ParentElement, Render,
  SharedString, StyledText, TextRun, Window, div, prelude::*, px,
};
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};
use gpui_component::input::Rope;
use helix_models::{DiffBase, DiffLineKind, DiffState, FileDiff};
use std::path::PathBuf;

const GUTTER_WIDTH: f32 = 92.0;

pub struct DiffView {
  pub root: PathBuf,
  pub relative: String,
  pub base: DiffBase,
  pub title: SharedString,
  diff: Option<FileDiff>,
  error: Option<String>,
  old_styles: Vec<(std::ops::Range<usize>, Hsla)>,
  new_styles: Vec<(std::ops::Range<usize>, Hsla)>,
  load_token: u64,
  focus_handle: FocusHandle,
}

impl Focusable for DiffView {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl DiffView {
  pub fn new(root: PathBuf, relative: String, base: DiffBase, cx: &mut Context<Self>) -> Self {
    let name = relative.rsplit('/').next().unwrap_or(&relative).to_string();

    let mut view = Self {
      root,
      relative,
      base,
      title: name.into(),
      diff: None,
      error: None,
      old_styles: Vec::new(),
      new_styles: Vec::new(),
      load_token: 0,
      focus_handle: cx.focus_handle(),
    };

    view.reload(cx);

    view
  }

  /// The diff plus both syntax passes are built in the background, and the view
  /// keeps showing the content it already has until the newest load lands.
  pub fn reload(&mut self, cx: &mut Context<Self>) {
    self.load_token = self.load_token.wrapping_add(1);

    let token = self.load_token;
    let root = self.root.clone();
    let relative = self.relative.clone();
    let base = self.base.clone();

    let task = cx.background_executor().spawn(async move {
      let diff = helix_git::diff::file_diff(&root, &relative, &base)?;
      let old_styles = highlight(&diff.language, &diff.old_text);
      let new_styles = highlight(&diff.language, &diff.new_text);

      anyhow::Ok((diff, old_styles, new_styles))
    });

    cx.spawn(async move |this, cx| {
      let result = task.await;

      this
        .update(cx, |view, cx| {
          if view.load_token != token {
            return;
          }

          match result {
            Ok((diff, old_styles, new_styles)) => {
              view.old_styles = old_styles;
              view.new_styles = new_styles;
              view.diff = Some(diff);
              view.error = None;
            }
            Err(err) => {
              view.diff = None;
              view.error = Some(err.to_string());
            }
          }

          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  pub fn stats(&self) -> (usize, usize) {
    self
      .diff
      .as_ref()
      .map(|diff| (diff.added, diff.removed))
      .unwrap_or((0, 0))
  }

  fn styles_for(&self, kind: DiffLineKind) -> &[(std::ops::Range<usize>, Hsla)] {
    match kind {
      DiffLineKind::Removed => &self.old_styles,
      _ => &self.new_styles,
    }
  }

  fn render_line(
    &self,
    diff: &FileDiff,
    line: &helix_models::DiffLine,
    theme: &Theme,
  ) -> AnyElement {
    let text = diff.line_text(line);

    let (bg, marker, marker_color) = match line.kind {
      DiffLineKind::Added => (Some(with_alpha(theme.green, 0.14)), "+", theme.green),
      DiffLineKind::Removed => (Some(with_alpha(theme.red, 0.14)), "-", theme.red),
      DiffLineKind::Context => (None, " ", theme.text_dim),
    };

    let runs = text_runs(
      text,
      line.range.start,
      self.styles_for(line.kind),
      theme,
      line.kind,
    );

    div()
      .flex()
      .items_start()
      .w_full()
      .when_some(bg, |el, bg| el.bg(bg))
      .child(
        div()
          .flex()
          .flex_none()
          .w(px(GUTTER_WIDTH))
          .gap_2()
          .px_2()
          .text_color(theme.text_dim)
          .child(
            div()
              .w(px(30.0))
              .flex_none()
              .text_right()
              .child(number(line.old_line)),
          )
          .child(
            div()
              .w(px(30.0))
              .flex_none()
              .text_right()
              .child(number(line.new_line)),
          )
          .child(
            div()
              .w(px(8.0))
              .flex_none()
              .text_color(marker_color)
              .child(marker),
          ),
      )
      .child(
        div()
          .flex_1()
          .min_w_0()
          .pr_3()
          .whitespace_nowrap()
          .overflow_hidden()
          .child(StyledText::new(text.to_string()).with_runs(runs)),
      )
      .into_any_element()
  }

  fn render_body(&mut self, theme: &Theme) -> AnyElement {
    if let Some(error) = &self.error {
      return message(format!("Could not diff: {error}"), theme.red);
    }

    let Some(diff) = &self.diff else {
      return message("Loading diff…".to_string(), theme.text_dim);
    };

    match &diff.state {
      DiffState::Binary => return message("Binary file — no diff".to_string(), theme.text_dim),
      DiffState::TooLarge { lines, chars } => {
        return message(
          format!("Diff too large to render: {lines} lines, {chars} chars"),
          theme.text_dim,
        );
      }
      DiffState::Identical => {
        return message("No changes".to_string(), theme.text_dim);
      }
      DiffState::Text => {}
    }

    let mut body = div()
      .id("diff-scroll")
      .flex_1()
      .min_h_0()
      .overflow_scroll()
      .flex()
      .flex_col()
      .font_family(theme.font_mono.clone())
      .text_size(px(12.0))
      .line_height(px(18.0));

    for hunk in &diff.hunks {
      body = body.child(
        div()
          .flex()
          .w_full()
          .px_2()
          .py(px(2.0))
          .bg(theme.elevated)
          .text_color(theme.text_dim)
          .child(format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start,
            hunk
              .lines
              .iter()
              .filter(|l| l.kind != DiffLineKind::Added)
              .count(),
            hunk.new_start,
            hunk
              .lines
              .iter()
              .filter(|l| l.kind != DiffLineKind::Removed)
              .count(),
          )),
      );

      for line in &hunk.lines {
        body = body.child(self.render_line(diff, line, theme));
      }
    }

    body.into_any_element()
  }
}

impl Render for DiffView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let (added, removed) = self.stats();
    let body = self.render_body(&theme);

    div()
      .key_context("Diff")
      .track_focus(&self.focus_handle)
      .size_full()
      .flex()
      .flex_col()
      .min_h_0()
      .child(
        div()
          .flex()
          .flex_none()
          .items_center()
          .gap_2()
          .h(px(30.0))
          .px_3()
          .border_b_1()
          .border_color(theme.panel_border)
          .text_xs()
          .child(
            div()
              .flex_1()
              .overflow_hidden()
              .text_color(theme.text_muted)
              .child(self.relative.clone()),
          )
          .child(
            div()
              .flex_none()
              .text_color(theme.text_dim)
              .child(self.base.label()),
          )
          .child(
            div()
              .flex_none()
              .text_color(theme.green)
              .child(format!("+{added}")),
          )
          .child(
            div()
              .flex_none()
              .text_color(theme.red)
              .child(format!("-{removed}")),
          ),
      )
      .child(body)
  }
}

fn number(line: Option<u32>) -> String {
  line.map(|n| n.to_string()).unwrap_or_default()
}

fn with_alpha(mut color: Hsla, alpha: f32) -> Hsla {
  color.a = alpha;

  color
}

fn message(text: String, color: Hsla) -> AnyElement {
  div()
    .flex_1()
    .flex()
    .items_center()
    .justify_center()
    .text_sm()
    .text_color(color)
    .child(text)
    .into_any_element()
}

fn highlight(language: &str, text: &str) -> Vec<(std::ops::Range<usize>, Hsla)> {
  if text.is_empty() {
    return Vec::new();
  }

  let rope = Rope::from(text);
  let mut highlighter = SyntaxHighlighter::new(language);

  highlighter.update(None, &rope);

  let theme = HighlightTheme::default_dark();

  highlighter
    .styles(&(0..text.len()), theme.as_ref())
    .into_iter()
    .filter_map(|(range, style)| style.color.map(|color| (range, color)))
    .collect()
}

fn text_runs(
  text: &str,
  offset: usize,
  styles: &[(std::ops::Range<usize>, Hsla)],
  theme: &Theme,
  kind: DiffLineKind,
) -> Vec<TextRun> {
  let base_color = match kind {
    DiffLineKind::Context => theme.text_muted,
    _ => theme.text,
  };

  let font = gpui::font(theme.font_mono.clone());
  let mut runs: Vec<TextRun> = Vec::new();
  let mut cursor = 0usize;

  let push = |len: usize, color: Hsla, runs: &mut Vec<TextRun>| {
    if len == 0 {
      return;
    }

    match runs.last_mut() {
      Some(last) if last.color == color => last.len += len,
      _ => runs.push(TextRun {
        len,
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
      }),
    }
  };

  for (range, color) in styles {
    if range.end <= offset {
      continue;
    }

    if range.start >= offset + text.len() {
      break;
    }

    let start = range.start.saturating_sub(offset).min(text.len());
    let end = (range.end - offset).min(text.len());

    if start > cursor {
      push(start - cursor, base_color, &mut runs);
      cursor = start;
    }

    if end > cursor {
      push(end - cursor, *color, &mut runs);
      cursor = end;
    }
  }

  if cursor < text.len() {
    push(text.len() - cursor, base_color, &mut runs);
  }

  if runs.is_empty() {
    push(text.len(), base_color, &mut runs);
  }

  runs
}
