use crate::components::{SMALL, UI};
use crate::theme::Theme;
use gpui::{
  AnyElement, App, Context, FocusHandle, Focusable, Hsla, IntoElement, ParentElement, Render,
  SharedString, StyledText, TextRun, Window, div, prelude::*, px, uniform_list,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::highlighter::{HighlightTheme, SyntaxHighlighter};
use gpui_component::input::Rope;
use gpui_component::menu::DropdownMenu as _;
use gpui_component::{Icon, IconName};
use helix_commands::SetDiffBase;
use helix_models::{DiffBase, DiffLineKind, DiffState, FileDiff};
use std::path::PathBuf;
use std::sync::Arc;

const NUMBER_WIDTH: f32 = 40.0;
const MARKER_WIDTH: f32 = 18.0;
const ROW_HEIGHT: f32 = 22.0;

/// Hunks flattened into one row per rendered line so the body can be
/// virtualized; headers carry their own label because their counts only change
/// when the diff is reloaded.
enum DiffRow {
  Header(SharedString),
  Line { hunk: usize, line: usize },
}

fn diff_rows(diff: &FileDiff) -> Vec<DiffRow> {
  let mut rows = Vec::new();

  for (hunk_ix, hunk) in diff.hunks.iter().enumerate() {
    let old = hunk
      .lines
      .iter()
      .filter(|line| line.kind != DiffLineKind::Added)
      .count();

    let new = hunk
      .lines
      .iter()
      .filter(|line| line.kind != DiffLineKind::Removed)
      .count();

    rows.push(DiffRow::Header(
      format!(
        "@@ -{},{} +{},{} @@",
        hunk.old_start, old, hunk.new_start, new
      )
      .into(),
    ));

    rows.extend((0..hunk.lines.len()).map(|line| DiffRow::Line {
      hunk: hunk_ix,
      line,
    }));
  }

  rows
}

pub struct DiffView {
  pub root: PathBuf,
  pub relative: String,
  pub base: DiffBase,
  pub title: SharedString,
  diff: Option<FileDiff>,
  error: Option<String>,
  old_styles: Vec<(std::ops::Range<usize>, Hsla)>,
  new_styles: Vec<(std::ops::Range<usize>, Hsla)>,
  rows: Vec<DiffRow>,
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
      rows: Vec::new(),
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
    let syntax = gpui_component::theme::Theme::global(cx)
      .highlight_theme
      .clone();

    let task = cx.background_executor().spawn(async move {
      let diff = helix_git::diff::file_diff(&root, &relative, &base)?;
      let old_styles = highlight(&diff.language, &diff.old_text, &syntax);
      let new_styles = highlight(&diff.language, &diff.new_text, &syntax);

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
              view.rows = diff_rows(&diff);
              view.diff = Some(diff);
              view.error = None;
            }
            Err(err) => {
              view.diff = None;
              view.rows = Vec::new();
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

    let (bg, marker, accent) = match line.kind {
      DiffLineKind::Added => (Some(theme.add_bg), "+", theme.green),
      DiffLineKind::Removed => (Some(theme.del_bg), "\u{2212}", theme.red),
      DiffLineKind::Context => (None, "", theme.text_dim),
    };

    let runs = text_runs(
      text,
      line.range.start,
      self.styles_for(line.kind),
      theme,
      line.kind,
    );

    let number = |value: Option<u32>, color: Hsla| {
      div()
        .w(px(NUMBER_WIDTH))
        .flex_none()
        .pr_2()
        .text_right()
        .text_color(color)
        .child(value.map(|n| n.to_string()).unwrap_or_default())
    };

    div()
      .flex()
      .items_start()
      .w_full()
      .h(px(ROW_HEIGHT))
      .when_some(bg, |el, bg| el.bg(bg))
      .child(number(
        line.old_line,
        if line.kind == DiffLineKind::Removed {
          accent
        } else {
          theme.text_dim
        },
      ))
      .child(number(
        line.new_line,
        if line.kind == DiffLineKind::Added {
          accent
        } else {
          theme.text_dim
        },
      ))
      .child(
        div()
          .w(px(MARKER_WIDTH))
          .flex_none()
          .flex()
          .justify_center()
          .text_color(accent)
          .child(marker),
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

  fn render_row(&self, ix: usize, theme: &Theme) -> Option<AnyElement> {
    let diff = self.diff.as_ref()?;

    match self.rows.get(ix)? {
      DiffRow::Header(label) => Some(
        div()
          .flex()
          .items_center()
          .w_full()
          .h(px(ROW_HEIGHT))
          .pl(px(2.0 * NUMBER_WIDTH + MARKER_WIDTH + 8.0))
          .bg(theme.panel2)
          .text_color(theme.text_dim)
          .child(label.clone())
          .into_any_element(),
      ),
      DiffRow::Line { hunk, line } => {
        let line = diff.hunks.get(*hunk)?.lines.get(*line)?;

        Some(self.render_line(diff, line, theme))
      }
    }
  }

  fn render_body(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
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

    let entity = cx.entity();

    uniform_list("diff-scroll", self.rows.len(), move |range, _, cx| {
      entity.update(cx, |view, cx| {
        let theme = Theme::of(cx).clone();

        range.filter_map(|ix| view.render_row(ix, &theme)).collect()
      })
    })
    .flex_1()
    .min_h_0()
    .bg(theme.code_bg)
    .font_family(theme.font_mono.clone())
    .text_size(px(12.5))
    .line_height(px(ROW_HEIGHT))
    .into_any_element()
  }
}

impl Render for DiffView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let (added, removed) = self.stats();
    let body = self.render_body(&theme, cx);

    let (directory, name) = match self.relative.rsplit_once('/') {
      Some((directory, name)) => (Some(directory.replace('/', " / ")), name.to_string()),
      None => (None, self.relative.clone()),
    };

    let breadcrumb = div()
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .h(px(34.0))
      .px(px(14.0))
      .border_b_1()
      .border_color(theme.panel_border)
      .text_size(px(UI))
      .when_some(directory, |el, directory| {
        el.child(
          div()
            .flex_none()
            .text_color(theme.text_dim)
            .child(format!("{directory} /")),
        )
      })
      .child(
        div()
          .flex_none()
          .font_weight(gpui::FontWeight::MEDIUM)
          .text_color(theme.text)
          .child(name),
      )
      .child(div().flex_1())
      .child(
        Button::new("diff-base")
          .ghost()
          .label(base_label(&self.base))
          .icon(Icon::new(IconName::ChevronDown).size(px(9.0)))
          .text_color(theme.text_muted)
          .text_size(px(SMALL))
          .rounded_full()
          .border_1()
          .border_color(theme.panel_border)
          .tooltip("Diff against")
          .dropdown_menu(|menu, _window, _cx| {
            menu
              .menu(
                "vs working tree",
                Box::new(SetDiffBase {
                  base: "unstaged".to_string(),
                }),
              )
              .menu(
                "vs staged",
                Box::new(SetDiffBase {
                  base: "staged".to_string(),
                }),
              )
              .menu(
                "vs HEAD",
                Box::new(SetDiffBase {
                  base: "head".to_string(),
                }),
              )
          }),
      )
      .child(
        div()
          .flex_none()
          .flex()
          .gap_1()
          .font_family(theme.font_mono.clone())
          .text_size(px(SMALL))
          .child(div().text_color(theme.green).child(format!("+{added}")))
          .child(
            div()
              .text_color(theme.red)
              .child(format!("\u{2212}{removed}")),
          ),
      );

    div()
      .key_context("Diff")
      .track_focus(&self.focus_handle)
      .size_full()
      .flex()
      .flex_col()
      .min_h_0()
      .bg(theme.code_bg)
      .on_action(cx.listener(|this, action: &SetDiffBase, _, cx| {
        let next = match action.base.as_str() {
          "staged" => DiffBase::Staged,
          "head" => DiffBase::Head,
          _ => DiffBase::Unstaged,
        };

        if this.base == next {
          return;
        }

        this.base = next;

        this.reload(cx);
        cx.notify();
      }))
      .child(breadcrumb)
      .child(body)
  }
}

/// The pill in the breadcrumb names the side being compared against, which reads
/// differently from the short label the tab list uses.
fn base_label(base: &DiffBase) -> &'static str {
  match base {
    DiffBase::Unstaged => "vs working tree",
    DiffBase::Staged => "vs staged",
    DiffBase::Head => "vs HEAD",
    DiffBase::Branch { .. } => "vs merge-base",
  }
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

fn highlight(
  language: &str,
  text: &str,
  theme: &Arc<HighlightTheme>,
) -> Vec<(std::ops::Range<usize>, Hsla)> {
  if text.is_empty() {
    return Vec::new();
  }

  let rope = Rope::from(text);
  let mut highlighter = SyntaxHighlighter::new(language);

  highlighter.update(None, &rope);

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
