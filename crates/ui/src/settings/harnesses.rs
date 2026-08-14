//! Settings → Agents: enable/disable harnesses (the t3code models-page
//! arrangement — one card row per agent with a trailing toggle).
//!
//! The state is PER-DEVICE and lives on the engine (`harness-prefs.json` in
//! its data dir): CLI installs are per-device, so enablement is too. The
//! page (the Accounts pattern) drives both the
//! `ListHarnesses` probe and the `SetHarnessEnabled` writes at any registered
//! device over the relay-forwarded RPCs.
//!
//! Only Claude Code and Codex ship enabled; the rest are opt-in. A harness
//! whose CLI is missing on the target device renders dimmed with an install
//! hint and its toggle inert — enabling an agent that can't run would only
//! manufacture NotInstalled errors at send time. The engine enforces the same
//! gate (plus "can't disable the last enabled harness") where the state
//! lives, so a raced or stale toggle self-corrects from the RPC reply.

use gpui::{Context, Entity, IntoElement, Render, SharedString, Task, Window, div, prelude::*, px};

use helix_engine::registry::{HarnessDescriptor, descriptor_enabled};
use helix_proto::HarnessId;
use helix_rpc::methods;

use crate::pickers::visible_harnesses;
use crate::popover::{self, Loadable};
use crate::settings::widgets;
use crate::state::AppState;
use crate::theme::Theme;

/// One-line blurb per agent (the t3code models page pairs every toggle row
/// with a description; the catalog descriptor doesn't carry one).
pub fn blurb(harness: HarnessId) -> &'static str {
  match harness {
    HarnessId::ClaudeCode => "Anthropic's coding agent, driven through the Claude Code CLI.",
    HarnessId::Codex => "OpenAI's coding agent, driven through the Codex CLI.",
    HarnessId::Cursor => "Cursor's coding agent, driven through the cursor-agent CLI.",
    HarnessId::Grok => "xAI's Grok Build agent (grok CLI).",
    HarnessId::Hermes => "Nous Research's Hermes Agent (hermes CLI).",
    HarnessId::Pi => "The pi coding agent (pi CLI).",
    HarnessId::Mock => "Scripted test harness.",
  }
}

/// The CLI named in the not-installed hint.
pub fn cli_name(harness: HarnessId) -> &'static str {
  match harness {
    HarnessId::ClaudeCode => "claude",
    HarnessId::Codex => "codex",
    HarnessId::Cursor => "cursor-agent",
    HarnessId::Grok => "grok",
    HarnessId::Hermes => "hermes",
    HarnessId::Pi => "pi",
    HarnessId::Mock => "mock",
  }
}

pub struct HarnessesPage {
  state: Entity<AppState>,
  harnesses: Loadable<Vec<HarnessDescriptor>>,
  /// Which device's harnesses are shown/edited; `None` = this device (no
  /// passthrough). Retargeted by the page-header device switcher.
  /// Whether the menu was open when the trigger press began — the menu's
  /// `on_mouse_down_out` closes it on that same press, so by click time a
  /// plain toggle would reopen (the [`popover::Popup`] press note, for
  /// this page's bool-state menu).
  /// Last refused/failed toggle (engine guards), shown in the error strip.
  error: Option<String>,
  load_task: Option<Task<()>>,
  toggle_task: Option<Task<()>>,
}

impl HarnessesPage {
  pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
    let mut page = Self {
      state,
      harnesses: Loadable::Idle,
      error: None,
      load_task: None,
      toggle_task: None,
    };
    page.load(cx);
    page
  }

  /// `ListHarnesses` against the target device (installed probe + enabled
  /// set both come from where the CLIs actually live).
  fn load(&mut self, cx: &mut Context<Self>) {
    let Some(engine) = self.state.read(cx).engine().cloned() else {
      return;
    };
    let params = serde_json::json!({});
    self.harnesses = Loadable::Loading;
    self.load_task = Some(cx.spawn(async move |this, cx| {
      let result = engine.client().call(methods::LIST_HARNESSES, params).await;
      this
        .update(cx, |page, cx| {
          page.harnesses = match result {
            Ok(value) => match serde_json::from_value::<Vec<HarnessDescriptor>>(value) {
              Ok(list) => Loadable::Ready(list),
              Err(err) => Loadable::Error(err.to_string()),
            },
            Err(err) => Loadable::Error(err.to_string()),
          };
          cx.notify();
        })
        .ok();
    }));
  }

  /// Flip one harness on the target device. The reply carries the device's
  /// fresh catalog, so the rows repaint from the authoritative state in one
  /// round trip; refusals (engine guards) land in the error strip.
  fn toggle(&mut self, harness: HarnessId, enabled: bool, cx: &mut Context<Self>) {
    let Some(engine) = self.state.read(cx).engine().cloned() else {
      return;
    };
    let params = serde_json::json!({
        "harness": harness,
        "enabled": enabled,
    });
    self.error = None;
    self.toggle_task = Some(cx.spawn(async move |this, cx| {
      let result = engine
        .client()
        .call(methods::SET_HARNESS_ENABLED, params)
        .await;
      this
        .update(cx, |page, cx| {
          match result {
            Ok(value) => {
              if let Ok(list) = serde_json::from_value::<Vec<HarnessDescriptor>>(value) {
                page.harnesses = Loadable::Ready(list);
              }
              // The composer caches its catalog per space — poke
              // every Pickers to re-fetch, or the rail keeps the
              // old set until restart.
              crate::pickers::bump_harness_catalog(cx);
            }
            Err(err) => page.error = Some(err.to_string()),
          }
          cx.notify();
        })
        .ok();
    }));
  }

  fn rows(&self, cx: &mut Context<Self>) -> Vec<gpui::AnyElement> {
    let theme = Theme::of(cx).clone();
    let Loadable::Ready(list) = &self.harnesses else {
      return Vec::new();
    };
    let descriptors = visible_harnesses(list);
    let enabled_count = descriptors.iter().filter(|d| descriptor_enabled(d)).count();
    descriptors
      .into_iter()
      .enumerate()
      .map(|(ix, descriptor)| {
        let harness = descriptor.id;
        let installed = descriptor.installed;
        let enabled = descriptor_enabled(&descriptor);
        // The one enabled harness left can't be switched off — the
        // composer needs something to run (mirrors the engine guard).
        let last_enabled = enabled && enabled_count == 1;
        let interactive = installed && !last_enabled;
        let (icon_path, tint) = crate::pickers::harness_brand_icon(harness);
        let mut meta: Vec<gpui::AnyElement> = vec![
          div()
            .child(SharedString::from(blurb(harness)))
            .into_any_element(),
        ];
        if !installed {
          meta.push(
            div()
              .text_color(theme.warning_muted.opacity(0.9))
              .child(SharedString::from(format!(
                "Install the {} CLI to enable",
                cli_name(harness)
              )))
              .into_any_element(),
          );
        }
        // widgets::row_tile with the brand tint honored (the Claude
        // mark keeps its orange, like the picker rail).
        let tile = div()
          .flex_none()
          .size(px(36.0))
          .rounded(px(10.0))
          .border_1()
          .border_color(theme.border)
          .bg(crate::theme::ink(0.03))
          .flex()
          .items_center()
          .justify_center()
          .child(
            crate::icons::icon(icon_path)
              .size(px(16.0))
              .text_color(tint.unwrap_or(theme.text_muted)),
          );
        widgets::card_row(&theme, ix == 0)
          .id(("harness-row", ix))
          .when(!installed, |el| el.opacity(0.55))
          .child(tile)
          .child(
            div()
              .flex_1()
              .min_w_0()
              .flex()
              .flex_col()
              .child(widgets::row_title(&theme, descriptor.name.clone()))
              .child(widgets::meta_line(&theme, meta)),
          )
          .child(
            widgets::toggle_switch(&theme, enabled)
              .id(("harness-toggle", ix))
              .when(interactive, |el| {
                el.cursor_pointer()
                  .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle(harness, !enabled, cx);
                  }))
              }),
          )
          .into_any_element()
      })
      .collect()
  }
}

impl Render for HarnessesPage {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let theme = Theme::of(cx).clone();
    let body: gpui::AnyElement = match &self.harnesses {
      Loadable::Idle | Loadable::Loading => widgets::section_card(&theme)
        .p(px(16.0))
        .child(popover::skeleton_rows(
          "harnesses-skeleton",
          &theme,
          4,
          cx.entity_id(),
          cx,
        ))
        .into_any_element(),
      Loadable::Error(message) => {
        let message = message.clone();
        div()
          .child(widgets::error_strip(&theme, message))
          .child(
            widgets::ghost_action(&theme)
              .id("harnesses-retry")
              .mt(px(8.0))
              .hover(|s| widgets::ghost_hover(&theme, s))
              .on_click(cx.listener(|page, _, _, cx| {
                page.load(cx);
                cx.notify();
              }))
              .child(SharedString::from("Retry")),
          )
          .into_any_element()
      }
      Loadable::Ready(_) => {
        let rows = self.rows(cx);
        widgets::section_card(&theme)
          .children(rows)
          .into_any_element()
      }
    };
    let error = self
      .error
      .clone()
      .map(|message| widgets::error_strip(&theme, message).into_any_element());

    div()
      .id("harnesses-page")
      .size_full()
      .overflow_y_scroll()
      .child(
        widgets::page_column()
          .child(
            div()
              .flex()
              .flex_row()
              .items_center()
              .justify_between()
              .child(widgets::page_header(&theme, "Agents", None)),
          )
          .child(
            widgets::page_subtitle(
              &theme,
              "Choose which coding agents the composer offers. The setting is per \
                             device — switch devices in the header. Agents whose CLI isn't \
                             installed on a device can't be enabled there.",
            )
            .max_w(px(512.0))
            .line_height(px(20.0)),
          )
          .children(error)
          .child(body),
      )
  }
}
