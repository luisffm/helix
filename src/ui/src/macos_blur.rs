#![allow(unexpected_cfgs)]

#[cfg(target_os = "macos")]
pub fn apply_blur_material() -> bool {
  use objc::runtime::{BOOL, Object, YES};
  use objc::{class, msg_send, sel, sel_impl};

  const UNDER_WINDOW_BACKGROUND: isize = 21;
  const STATE_ACTIVE: isize = 1;

  let mut applied = false;
  unsafe {
    let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
    let windows: *mut Object = msg_send![app, windows];
    let window_count: usize = msg_send![windows, count];

    for w in 0..window_count {
      let window: *mut Object = msg_send![windows, objectAtIndex: w];
      let content_view: *mut Object = msg_send![window, contentView];

      if content_view.is_null() {
        continue;
      }

      let subviews: *mut Object = msg_send![content_view, subviews];
      let subview_count: usize = msg_send![subviews, count];

      for s in 0..subview_count {
        let view: *mut Object = msg_send![subviews, objectAtIndex: s];
        let is_effect_view: BOOL = msg_send![view, isKindOfClass: class!(NSVisualEffectView)];

        if is_effect_view == YES {
          let _: () = msg_send![view, setMaterial: UNDER_WINDOW_BACKGROUND];
          let _: () = msg_send![view, setState: STATE_ACTIVE];
          applied = true;
        }
      }
    }
  }

  applied
}

#[cfg(not(target_os = "macos"))]
pub fn apply_blur_material() -> bool {
  true
}
