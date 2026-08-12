use crate::components::{
  BODY, HEADER_HEIGHT, META, PROJECT_ACCENTS, TITLE, TRAFFIC_LIGHTS, UI, project_accent,
};
use crate::theme::{BLUR_LEVELS, Mode, Theme};
use gpui::{
  AnyElement, App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, Hsla, IntoElement,
  KeyDownEvent, ParentElement, Render, SharedString, Window, div, prelude::*, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::{Icon, IconName};
use std::path::PathBuf;

const NAV_WIDTH: f32 = 180.0;
const CONTENT_WIDTH: f32 = 640.0;
const SWATCH: f32 = 26.0;

pub enum SettingsEvent {
  Close,
  Changed,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Section {
  General,
  Project,
}

pub struct SettingsPage {
  section: Section,
  project_root: PathBuf,
  project_dir_name: String,
  project_label: String,
  name_input: Entity<InputState>,
  font_input: Entity<InputState>,
  accent_selected: Option<String>,
  font_size: f32,
  blur_level: String,
  mode: Mode,
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

    let project_label = project
      .as_ref()
      .map(|p| p.label())
      .unwrap_or_else(|| project_dir_name.clone());

    let (display_name, accent_selected) = project
      .map(|p| (p.display_name.unwrap_or_default(), p.accent))
      .unwrap_or_default();

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
      project_label,
      name_input,
      font_input,
      accent_selected,
      font_size: config.terminal_font_size.unwrap_or(13.0),
      blur_level: config.blur_level.unwrap_or_else(|| "medium".to_string()),
      mode: Mode::from_id(config.theme.as_deref().unwrap_or("dark")),
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
      .gap(px(14.0))
      .p(px(16.0))
      .rounded(px(12.0))
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.panel)
      .child(
        div()
          .text_size(px(TITLE))
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.text)
          .child(title),
      )
  }

  fn field_label(&self, theme: &Theme, text: &'static str) -> Div {
    div()
      .text_size(px(BODY))
      .text_color(theme.text_muted)
      .child(text)
  }

  fn hint(&self, theme: &Theme, text: impl Into<SharedString>) -> Div {
    div()
      .text_size(px(META))
      .text_color(theme.text_dim)
      .child(text.into())
  }

  fn segment(
    &self,
    id: String,
    label: &'static str,
    selected: bool,
    theme: &Theme,
  ) -> gpui::Stateful<Div> {
    div()
      .id(SharedString::from(id))
      .px(px(12.0))
      .py(px(4.0))
      .rounded(px(7.0))
      .border_1()
      .cursor_pointer()
      .text_size(px(UI))
      .when(selected, |el| {
        el.border_color(theme.active)
          .bg(theme.active)
          .text_color(theme.text)
      })
      .when(!selected, |el| {
        el.border_color(theme.panel_border)
          .text_color(theme.text_muted)
          .hover(|s| s.bg(theme.hover))
      })
      .child(label)
  }

  fn stepper(&self, id: &'static str, glyph: &'static str, theme: &Theme) -> gpui::Stateful<Div> {
    div()
      .id(id)
      .size(px(24.0))
      .flex()
      .flex_none()
      .items_center()
      .justify_center()
      .rounded(px(7.0))
      .border_1()
      .border_color(theme.panel_border)
      .cursor_pointer()
      .text_size(px(UI))
      .text_color(theme.text_muted)
      .hover(|s| s.bg(theme.hover).text_color(theme.text))
      .child(glyph)
  }

  fn field(&self, theme: &Theme, input: &Entity<InputState>) -> Div {
    div()
      .h(px(30.0))
      .flex()
      .items_center()
      .px(px(10.0))
      .rounded(px(8.0))
      .border_1()
      .border_color(theme.panel_border)
      .bg(theme.panel2)
      .text_size(px(BODY))
      .child(Input::new(input).appearance(false).w_full())
  }

  fn render_general(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let theme_options = div()
      .flex()
      .gap_1()
      .children([Mode::Dark, Mode::Light].map(|mode| {
        self
          .segment(
            format!("theme-{}", mode.id()),
            mode.label(),
            self.mode == mode,
            theme,
          )
          .on_click(cx.listener(move |this, _, _, cx| {
            if this.mode == mode {
              return;
            }

            this.mode = mode;

            helix_state::config::set_theme(mode.id());
            cx.emit(SettingsEvent::Changed);
            cx.notify();
          }))
      }));

    let blur_options =
      div()
        .flex()
        .gap_1()
        .children(BLUR_LEVELS.iter().map(|(level_id, label)| {
          let level = level_id.to_string();

          self
            .segment(
              format!("blur-{level_id}"),
              label,
              self.blur_level == *level_id,
              theme,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
              this.blur_level = level.clone();

              helix_state::config::set_blur_level(&level);
              cx.emit(SettingsEvent::Changed);
              cx.notify();
            }))
        }));

    let size_control = div()
      .flex()
      .items_center()
      .gap_2()
      .child(
        self
          .stepper("font-size-minus", "\u{2212}", theme)
          .on_click(cx.listener(|this, _, _, cx| {
            this.font_size = (this.font_size - 1.0).max(9.0);

            helix_state::config::set_terminal_font_size(this.font_size);
            cx.emit(SettingsEvent::Changed);
            cx.notify();
          })),
      )
      .child(
        div()
          .w(px(28.0))
          .flex()
          .justify_center()
          .text_size(px(BODY))
          .text_color(theme.text)
          .child(format!("{:.0}", self.font_size)),
      )
      .child(
        self
          .stepper("font-size-plus", "+", theme)
          .on_click(cx.listener(|this, _, _, cx| {
            this.font_size = (this.font_size + 1.0).min(22.0);

            helix_state::config::set_terminal_font_size(this.font_size);
            cx.emit(SettingsEvent::Changed);
            cx.notify();
          })),
      );

    div()
      .flex()
      .flex_col()
      .gap(px(16.0))
      .child(
        self
          .card(theme, "Appearance")
          .child(
            div()
              .flex()
              .items_center()
              .child(div().flex_1().child(self.field_label(theme, "Theme")))
              .child(theme_options),
          )
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(self.field_label(theme, "Background blur"))
              .child(blur_options)
              .child(self.hint(theme, "Applies immediately to the window background.")),
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
              .child(self.field_label(theme, "Font family"))
              .child(self.field(theme, &self.font_input))
              .child(self.hint(theme, "Leave empty to auto-detect. Applies on restart.")),
          )
          .child(
            div()
              .flex()
              .items_center()
              .child(div().flex_1().child(self.field_label(theme, "Font size")))
              .child(size_control),
          )
          .child(self.hint(theme, "Font size applies to new terminal sessions.")),
      )
      .into_any_element()
  }

  fn render_project(&mut self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
    let initial = self
      .project_label
      .chars()
      .next()
      .map(|c| c.to_uppercase().to_string())
      .unwrap_or_default();

    let swatches = div()
      .flex()
      .gap(px(6.0))
      .children(PROJECT_ACCENTS.map(|accent| {
        let selected = self.accent_selected.as_deref() == Some(accent);
        let (fg, bg) = project_accent(Some(accent), theme);
        let id = accent.to_string();

        div()
          .id(SharedString::from(format!("proj-accent-{accent}")))
          .size(px(SWATCH))
          .flex()
          .items_center()
          .justify_center()
          .rounded(px(8.0))
          .bg(bg)
          .border(px(1.5))
          .cursor_pointer()
          .text_size(px(META))
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .when(selected, |el| {
            el.border_color(theme.accent).text_color(theme.accent)
          })
          .when(!selected, |el| {
            el.border_color(Hsla::transparent_black()).text_color(fg)
          })
          .on_click(cx.listener(move |this, _, _, cx| {
            helix_state::config::set_accent(&this.project_root, &id);

            this.accent_selected = Some(id.clone());

            cx.emit(SettingsEvent::Changed);
            cx.notify();
          }))
          .child(initial.clone())
      }));

    div()
      .flex()
      .flex_col()
      .gap(px(16.0))
      .child(
        div()
          .font_family(theme.font_mono.clone())
          .text_size(px(META))
          .text_color(theme.text_dim)
          .child(helix_filesystem::paths::abbreviate_home(&self.project_root)),
      )
      .child(
        self
          .card(theme, "Identity")
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(self.field_label(theme, "Display Name"))
              .child(self.field(theme, &self.name_input))
              .child(self.hint(
                theme,
                format!(
                  "Leave empty to use the folder name ({})",
                  self.project_dir_name
                ),
              )),
          )
          .child(
            div()
              .flex()
              .flex_col()
              .gap_2()
              .child(self.field_label(theme, "Accent"))
              .child(swatches),
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
      .gap(px(10.0))
      .h(px(HEADER_HEIGHT))
      .pl(px(TRAFFIC_LIGHTS))
      .pr(px(14.0))
      .border_b_1()
      .border_color(theme.panel_border)
      .child(
        div()
          .id("settings-back")
          .flex()
          .flex_none()
          .items_center()
          .gap(px(6.0))
          .px(px(10.0))
          .py(px(4.0))
          .rounded(px(7.0))
          .text_size(px(UI))
          .text_color(theme.text_muted)
          .cursor_pointer()
          .hover(|s| s.bg(theme.hover).text_color(theme.text))
          .on_click(cx.listener(|_, _, _, cx| {
            cx.emit(SettingsEvent::Close);
          }))
          .child(Icon::new(IconName::ArrowLeft).size(px(12.0)))
          .child("Back to app"),
      )
      .child(div().flex_1())
      .child(
        div()
          .text_size(px(TITLE))
          .font_weight(gpui::FontWeight::SEMIBOLD)
          .text_color(theme.text)
          .child(match self.section {
            Section::General => "Settings".to_string(),
            Section::Project => {
              format!("Project Settings \u{b7} {}", self.project_label)
            }
          }),
      )
      .child(div().flex_1());

    let nav = div()
      .flex()
      .flex_col()
      .flex_none()
      .w(px(NAV_WIDTH))
      .p(px(10.0))
      .gap(px(2.0))
      .border_r_1()
      .border_color(theme.panel_border)
      .children(
        [(Section::General, "General"), (Section::Project, "Project")].map(|(section, label)| {
          let selected = self.section == section;

          div()
            .id(SharedString::from(format!("settings-nav-{label}")))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(7.0))
            .text_size(px(BODY))
            .cursor_pointer()
            .when(selected, |el| {
              el.bg(theme.active)
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.text)
            })
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
      .bg(theme.win_tint)
      .text_size(px(BODY))
      .child(header)
      .child(
        div().flex().flex_1().min_h_0().child(nav).child(
          div()
            .id("settings-content")
            .flex_1()
            .min_w_0()
            .overflow_y_scroll()
            .p(px(26.0))
            .child(div().max_w(px(CONTENT_WIDTH)).child(content)),
        ),
      )
  }
}
