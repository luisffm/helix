use gpui::{
  App, AppContext, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px,
  size,
};
use helix_commands::Quit;
use helix_ui::{HelixRoot, Theme};

mod assets;
mod single_instance;

fn main() {
  let Some(_instance_lock) = single_instance::acquire() else {
    eprintln!("helix: another instance is already running");
    return;
  };

  let path = std::env::args()
    .nth(1)
    .map(std::path::PathBuf::from)
    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));

  let (project, _worktree) = match helix_worktree::open_project(&path) {
    Ok(opened) => opened,
    Err(err) => {
      eprintln!("helix: {err}");
      std::process::exit(1);
    }
  };

  Application::new()
    .with_assets(assets::HelixAssets)
    .run(move |cx: &mut App| {
      gpui_component::init(cx);
      let blur_level = helix_state::config::load()
        .blur_level
        .unwrap_or_else(|| "medium".to_string());
      let mut theme = Theme::dark();
      helix_ui::theme::apply_blur_level(&mut theme, &blur_level);
      let installed_fonts = cx.text_system().all_font_names();
      if let Some(mono) = helix_state::terminal_font::detect(&installed_fonts) {
        theme.font_mono = mono.into();
      }
      cx.set_global(theme);
      helix_ui::theme::sync_component_theme(cx);
      cx.bind_keys(helix_commands::default_bindings());
      cx.on_action(|_: &Quit, cx| cx.quit());

      let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
      let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
          title: Some(format!("Helix — {}", project.name).into()),
          appears_transparent: true,
          traffic_light_position: Some(point(px(12.0), px(14.0))),
        }),
        window_background: helix_ui::theme::appearance_for_level(&blur_level),
        window_min_size: Some(size(px(800.0), px(500.0))),
        ..Default::default()
      };

      let project = project.clone();
      cx.open_window(options, move |window, cx| {
        let helix_root = cx.new(|cx| HelixRoot::new(project, window, cx));
        let view: gpui::AnyView = helix_root.into();
        cx.new(|cx| gpui_component::Root::new(view, window, cx))
      })
      .expect("failed to open window");
      helix_ui::window_frame::restore_and_autosave("HelixMainWindow");
      helix_ui::macos_blur::apply_blur_material();
      cx.activate(true);
    });
}
