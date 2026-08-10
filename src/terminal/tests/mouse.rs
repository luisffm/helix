use alacritty_terminal::term::TermMode;
use helix_terminal::mouse::{
  BUTTON_LEFT, BUTTON_RIGHT, BUTTON_WHEEL_UP, MouseReport, alternate_scroll, encode,
};

fn press(button: u8, col: usize, row: usize) -> MouseReport {
  MouseReport {
    button,
    col,
    row,
    pressed: true,
    motion: false,
    shift: false,
    alt: false,
    ctrl: false,
  }
}

#[test]
fn no_report_without_mouse_mode() {
  assert!(encode(press(BUTTON_LEFT, 0, 0), TermMode::empty()).is_none());
}

#[test]
fn sgr_press_and_release_use_distinct_terminators() {
  let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
  let pressed = encode(press(BUTTON_LEFT, 4, 9), mode).unwrap();

  assert_eq!(pressed, b"\x1b[<0;5;10M");

  let released = encode(
    MouseReport {
      pressed: false,
      ..press(BUTTON_LEFT, 4, 9)
    },
    mode,
  )
  .unwrap();

  assert_eq!(released, b"\x1b[<0;5;10m");
}

#[test]
fn sgr_encodes_modifiers_and_motion() {
  let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
  let report = MouseReport {
    motion: true,
    ctrl: true,
    ..press(BUTTON_RIGHT, 0, 0)
  };

  assert_eq!(encode(report, mode).unwrap(), b"\x1b[<50;1;1M");
}

#[test]
fn x10_release_collapses_to_button_three() {
  let mode = TermMode::MOUSE_REPORT_CLICK;
  let released = encode(
    MouseReport {
      pressed: false,
      ..press(BUTTON_RIGHT, 2, 3)
    },
    mode,
  )
  .unwrap();

  assert_eq!(released, vec![0x1b, b'[', b'M', 32 + 3, 35, 36]);
}

#[test]
fn x10_keeps_wheel_button_on_release_path() {
  let mode = TermMode::MOUSE_REPORT_CLICK;

  assert_eq!(
    encode(press(BUTTON_WHEEL_UP, 0, 0), mode).unwrap(),
    vec![0x1b, b'[', b'M', 96, 33, 33]
  );
}

#[test]
fn x10_drops_coordinates_past_the_byte_range() {
  let mode = TermMode::MOUSE_REPORT_CLICK;

  assert!(encode(press(BUTTON_LEFT, 300, 0), mode).is_none());
  assert!(encode(press(BUTTON_LEFT, 300, 0), mode | TermMode::UTF8_MOUSE).is_some());
}

#[test]
fn alternate_scroll_only_fires_on_the_alt_screen() {
  let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL;

  assert_eq!(alternate_scroll(2, mode).unwrap(), b"\x1b[A\x1b[A");
  assert_eq!(alternate_scroll(-1, mode).unwrap(), b"\x1b[B");
  assert!(alternate_scroll(2, TermMode::ALTERNATE_SCROLL).is_none());
}

#[test]
fn alternate_scroll_follows_application_cursor_keys() {
  let mode = TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL | TermMode::APP_CURSOR;

  assert_eq!(alternate_scroll(1, mode).unwrap(), b"\x1bOA");
}
