use crate::components::{EMOJI_CHOICES, PROJECT_ICONS};
use crate::theme::{BLUR_LEVELS, Theme};
use gpui::{
  AnyElement, App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
  KeyDownEvent, ParentElement, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName};
use std::path::PathBuf;

pub enum SettingsEvent {
  Close,
  Changed,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Section {
  General,
  Project,
}

#[derive(Clone, Copy, PartialEq)]
enum GlyphTab {
  Icon,
  Emoji,
}

pub struct SettingsPage {
  section: Section,
  project_root: PathBuf,
  project_dir_name: String,
  name_input: Entity<InputState>,
  font_input: Entity<InputState>,
  icon_selected: Option<String>,
  emoji_selected: Option<String>,
  font_size: f32,
  blur_level: String,
  glyph_tab: GlyphTab,
  focus_handle: FocusHandle,
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

impl Focusable for SettingsPage {
  fn focus_handle(&self, _cx: &App) -> FocusHandle {
    self.focus_handle.clone()
  }
}

impl SettingsPage {
  pub fn new(
    section: Section,
    project_root: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
  ) -> Self {
    let config = helix_state::config::load();
    let project = helix_state::config::project_for(&project_root);

    let project_dir_name = project_root
      .file_name()
      .map(|n| n.to_string_lossy().to_string())
      .unwrap_or_default();

    let (display_name, icon_selected, emoji_selected) = project
      .map(|p| (p.display_name.unwrap_or_default(), p.icon, p.emoji))
      .unwrap_or_default();

    let glyph_tab = if icon_selected.is_some() {
      GlyphTab::Icon
    } else {
      GlyphTab::Emoji
    };

    let terminal_font = config.terminal_font.unwrap_or_default();

    let name_input = cx.new(|cx| {
      let mut state = InputState::new(window, cx).placeholder("Folder name is used when empty");

      if !display_name.is_empty() {
        state = state.default_value(display_name);
      }

      state
    });

    let font_input = cx.new(|cx| {
      let mut state =
        InputState::new(window, cx).placeholder("Auto-detect (terminal config / Nerd Fonts)");

      if !terminal_font.is_empty() {
        state = state.default_value(terminal_font);
      }

      state
    });

    cx.subscribe(&name_input, |this, input, event: &InputEvent, cx| {
      if let InputEvent::Change = event {
        let value = input.read(cx).value().trim().to_string();

        this.persist_display_name(value, cx);
      }
    })
    .detach();

    cx.subscribe(&font_input, |_, input, event: &InputEvent, cx| {
      if let InputEvent::Change = event {
        let value = input.read(cx).value().trim().to_string();

        helix_state::config::set_terminal_font(Some(value));
        cx.emit(SettingsEvent::Changed);
      }
    })
    .detach();

    Self {
      section,
      project_root,
      project_dir_name,
      name_input,
      font_input,
      icon_selected,
      emoji_selected,
      font_size: config.terminal_font_size.unwrap_or(13.0),
      blur_level: config.blur_level.unwrap_or_else(|| "medium".to_string()),
      glyph_tab,
      focus_handle: cx.focus_handle(),
    }
  }

  fn persist_display_name(&self, value: String, cx: &mut Context<Self>) {
    helix_state::config::set_display_name(&self.project_root, &value);
    cx.emit(SettingsEvent::Changed);
  }

  fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
    if event.keystroke.key.as_str() == "escape" {
      cx.emit(SettingsEvent::Close);
    }
  }

  fn card(&self, theme: &Theme, title: &'static str) -> Div {
    div()
      .flex()
      .flex_col()
      .gap_3()
      .p_4()
      .rounded_lg()
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.panel)
      .child(
        div()
          .text_sm()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.text)
          .child(title),
      )
  }

  fn render_general(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let blur_buttons =
      div()
        .flex()
        .gap_1()
        .children(BLUR_LEVELS.iter().map(|(level_id, label)| {
          let selected = self.blur_level == *level_id;
          let level = level_id.to_string();

          div()
            .id(SharedString::from(format!("blur-{level_id}")))
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| {
              el.border_color(theme.active)
                .bg(theme.elevated)
                .text_color(theme.text)
            })
            .when(!selected, |el| {
              el.border_color(theme.panel_border)
                .text_color(theme.text_muted)
                .hover(|s| s.bg(theme.hover))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
              this.blur_level = level.clone();
              helix_state::config::set_blur_level(&level);
              cx.emit(SettingsEvent::Changed);
              cx.notify();
            }))
            .child(*label)
        }));

    let size_control = div()
      .flex()
      .items_center()
      .gap_2()
      .child(
        div()
          .id("font-size-minus")
          .size(px(24.0))
          .flex()
          .items_center()
          .justify_center()
          .rounded_md()
          .border_1()
          .border_color(theme.panel_border)
          .text_color(theme.text_muted)
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover))
          .on_click(cx.listener(|this, _, _, cx| {
            this.font_size = (this.font_size - 1.0).max(9.0);
            helix_state::config::set_terminal_font_size(this.font_size);
            cx.emit(SettingsEvent::Changed);
            cx.notify();
          }))
          .child("−"),
      )
      .child(
        div()
          .w(px(40.0))
          .text_sm()
          .text_color(theme.text)
          .flex()
          .justify_center()
          .child(format!("{:.0}", self.font_size)),
      )
      .child(
        div()
          .id("font-size-plus")
          .size(px(24.0))
          .flex()
          .items_center()
          .justify_center()
          .rounded_md()
          .border_1()
          .border_color(theme.panel_border)
          .text_color(theme.text_muted)
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover))
          .on_click(cx.listener(|this, _, _, cx| {
            this.font_size = (this.font_size + 1.0).min(22.0);
            helix_state::config::set_terminal_font_size(this.font_size);
            cx.emit(SettingsEvent::Changed);
            cx.notify();
          }))
          .child("+"),
      );

    div()
      .flex()
      .flex_col()
      .gap_4()
      .child(
        self
          .card(theme, "Appearance")
          .child(
            div()
              .flex()
              .items_center()
              .child(
                div()
                  .flex_1()
                  .text_sm()
                  .text_color(theme.text_muted)
                  .child("Theme"),
              )
              .child(
                div()
                  .px_2()
                  .py_0p5()
                  .rounded_md()
                  .border_1()
                  .border_color(theme.panel_border)
                  .text_sm()
                  .text_color(theme.text_dim)
                  .child("Dark"),
              ),
          )
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.text_muted)
                  .child("Background blur"),
              )
              .child(blur_buttons)
              .child(
                div()
                  .text_xs()
                  .text_color(theme.text_dim)
                  .child("Applies immediately to the window background."),
              ),
          ),
      )
      .child(
        self
          .card(theme, "Terminal")
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.text_muted)
                  .child("Font family"),
              )
              .child(Input::new(&self.font_input).w_full())
              .child(
                div()
                  .text_xs()
                  .text_color(theme.text_dim)
                  .child("Leave empty to auto-detect. Applies on restart."),
              ),
          )
          .child(
            div()
              .flex()
              .items_center()
              .child(
                div()
                  .flex_1()
                  .text_sm()
                  .text_color(theme.text_muted)
                  .child("Font size"),
              )
              .child(size_control),
          )
          .child(
            div()
              .text_xs()
              .text_color(theme.text_dim)
              .child("Font size applies to new terminal sessions."),
          ),
      )
      .into_any_element()
  }

  fn render_project(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let tabs =
      div()
        .flex()
        .gap_1()
        .children(
          [(GlyphTab::Icon, "Icon"), (GlyphTab::Emoji, "Emoji")].map(|(tab, label)| {
            let selected = self.glyph_tab == tab;
            div()
              .id(SharedString::from(format!("glyph-tab-{label}")))
              .px_3()
              .py_1()
              .rounded_md()
              .text_sm()
              .cursor_pointer()
              .when(selected, |el| el.bg(theme.elevated).text_color(theme.text))
              .when(!selected, |el| {
                el.text_color(theme.text_dim).hover(|s| s.bg(theme.hover))
              })
              .on_click(cx.listener(move |this, _, _, cx| {
                this.glyph_tab = tab;
                cx.notify();
              }))
              .child(label)
          }),
        );

    let grid: AnyElement = match self.glyph_tab {
      GlyphTab::Icon => div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(PROJECT_ICONS.iter().map(|(icon_id, icon)| {
          let selected = self.icon_selected.as_deref() == Some(*icon_id);
          let id_string = icon_id.to_string();
          div()
            .id(SharedString::from(format!("proj-icon-{icon_id}")))
            .size(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .cursor_pointer()
            .when(selected, |el| {
              el.border_color(theme.active).bg(theme.elevated)
            })
            .when(!selected, |el| {
              el.border_color(gpui::transparent_black())
                .hover(|s| s.bg(theme.hover))
            })
            .text_color(theme.text_muted)
            .on_click(cx.listener(move |this, _, _, cx| {
              helix_state::config::set_icon(&this.project_root, &id_string);
              this.icon_selected = Some(id_string.clone());
              this.emoji_selected = None;
              cx.emit(SettingsEvent::Changed);
              cx.notify();
            }))
            .child(Icon::new(icon.clone()).size_4())
        }))
        .into_any_element(),
      GlyphTab::Emoji => div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(EMOJI_CHOICES.iter().map(|emoji| {
          let selected = self.emoji_selected.as_deref() == Some(*emoji);
          div()
            .id(SharedString::from(format!("proj-emoji-{emoji}")))
            .size(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .border_1()
            .cursor_pointer()
            .when(selected, |el| {
              el.border_color(theme.active).bg(theme.elevated)
            })
            .when(!selected, |el| {
              el.border_color(gpui::transparent_black())
                .hover(|s| s.bg(theme.hover))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
              helix_state::config::set_emoji(&this.project_root, emoji);
              this.emoji_selected = Some(emoji.to_string());
              this.icon_selected = None;
              cx.emit(SettingsEvent::Changed);
              cx.notify();
            }))
            .child(*emoji)
        }))
        .into_any_element(),
    };

    div()
      .flex()
      .flex_col()
      .gap_4()
      .child(
        div()
          .text_xs()
          .text_color(theme.text_dim)
          .child(self.project_root.display().to_string()),
      )
      .child(
        self
          .card(theme, "Identity")
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.text_muted)
                  .child("Display Name"),
              )
              .child(Input::new(&self.name_input).w_full())
              .child(div().text_xs().text_color(theme.text_dim).child(format!(
                "Leave empty to use the folder name ({})",
                self.project_dir_name
              ))),
          )
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(
                div()
                  .text_sm()
                  .text_color(theme.text_muted)
                  .child("Project Icon"),
              )
              .child(tabs)
              .child(grid),
          ),
      )
      .into_any_element()
  }
}

impl Render for SettingsPage {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();

    let header = div()
      .id("settings-header")
      .window_control_area(gpui::WindowControlArea::Drag)
      .flex()
      .flex_none()
      .items_center()
      .gap_2()
      .h(px(42.0))
      .px_3()
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .id("settings-back")
          .flex()
          .items_center()
          .gap_1()
          .px_2()
          .py_0p5()
          .rounded_md()
          .text_sm()
          .text_color(theme.text_muted)
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover).text_color(theme.text))
          .on_click(cx.listener(|_, _, _, cx| {
            cx.emit(SettingsEvent::Close);
          }))
          .child(Icon::new(IconName::ArrowLeft).size_3p5())
          .child("Back to app"),
      )
      .child(div().flex_1())
      .child(
        div()
          .text_sm()
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.text)
          .child(match self.section {
            Section::General => "Settings".to_string(),
            Section::Project => {
              format!("Project Settings · {}", self.project_dir_name)
            }
          }),
      )
      .child(div().flex_1());

    let nav = div()
      .flex()
      .flex_col()
      .flex_none()
      .w(px(170.0))
      .p_2()
      .gap_0p5()
      .border_r_1()
      .border_color(theme.panel_border)
      .children(
        [(Section::General, "General"), (Section::Project, "Project")].map(|(section, label)| {
          let selected = self.section == section;
          div()
            .id(SharedString::from(format!("settings-nav-{label}")))
            .px_2()
            .py_1()
            .rounded_md()
            .text_sm()
            .cursor_pointer()
            .when(selected, |el| el.bg(theme.elevated).text_color(theme.text))
            .when(!selected, |el| {
              el.text_color(theme.text_muted).hover(|s| s.bg(theme.hover))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
              this.section = section;
              cx.notify();
            }))
            .child(label)
        }),
      );

    let content = match self.section {
      Section::General => self.render_general(&theme, cx),
      Section::Project => self.render_project(&theme, cx),
    };

    div()
      .id("settings-page")
      .track_focus(&self.focus_handle)
      .on_key_down(cx.listener(Self::on_key_down))
      .on_mouse_down(
        gpui::MouseButton::Left,
        cx.listener(|this, _, window, _| {
          window.focus(&this.focus_handle);
        }),
      )
      .flex()
      .flex_col()
      .size_full()
      .child(header)
      .child(
        div().flex().flex_1().min_h_0().child(nav).child(
          div()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .p_6()
            .child(div().max_w(px(720.0)).child(content)),
        ),
      )
  }
}
