use alacritty_terminal::term::TermMode;

pub const BUTTON_LEFT: u8 = 0;
pub const BUTTON_MIDDLE: u8 = 1;
pub const BUTTON_RIGHT: u8 = 2;
pub const BUTTON_WHEEL_UP: u8 = 64;
pub const BUTTON_WHEEL_DOWN: u8 = 65;

const X10_RELEASE: u8 = 3;
const COORD_OFFSET: usize = 32;
const X10_MAX_BYTE: usize = 255;

#[derive(Clone, Copy)]
pub struct MouseReport {
  pub button: u8,
  pub col: usize,
  pub row: usize,
  pub pressed: bool,
  pub motion: bool,
  pub shift: bool,
  pub alt: bool,
  pub ctrl: bool,
}

impl MouseReport {
  fn modifier_bits(&self) -> u8 {
    let mut bits = 0;

    if self.shift {
      bits |= 4;
    }

    if self.alt {
      bits |= 8;
    }

    if self.ctrl {
      bits |= 16;
    }

    bits
  }

  fn code(&self) -> u8 {
    let motion = if self.motion { 32 } else { 0 };

    self.button + self.modifier_bits() + motion
  }
}

pub fn reports_motion(mode: TermMode, pressed: bool) -> bool {
  mode.contains(TermMode::MOUSE_MOTION) || (pressed && mode.contains(TermMode::MOUSE_DRAG))
}

pub fn encode(report: MouseReport, mode: TermMode) -> Option<Vec<u8>> {
  if !mode.intersects(TermMode::MOUSE_MODE) {
    return None;
  }

  if mode.contains(TermMode::SGR_MOUSE) {
    let action = if report.pressed { 'M' } else { 'm' };

    return Some(
      format!(
        "\x1b[<{};{};{}{}",
        report.code(),
        report.col + 1,
        report.row + 1,
        action
      )
      .into_bytes(),
    );
  }

  let is_wheel = report.button >= BUTTON_WHEEL_UP;
  let code = if report.pressed || is_wheel {
    report.code()
  } else {
    X10_RELEASE + report.modifier_bits()
  };

  let utf8 = mode.contains(TermMode::UTF8_MOUSE);
  let mut bytes = b"\x1b[M".to_vec();

  bytes.push(COORD_OFFSET as u8 + code);

  push_coord(&mut bytes, report.col, utf8)?;
  push_coord(&mut bytes, report.row, utf8)?;

  Some(bytes)
}

pub fn alternate_scroll(lines: i32, mode: TermMode) -> Option<Vec<u8>> {
  if !mode.contains(TermMode::ALT_SCREEN) || !mode.contains(TermMode::ALTERNATE_SCROLL) {
    return None;
  }

  if lines == 0 {
    return None;
  }

  let arrow: &[u8] = if mode.contains(TermMode::APP_CURSOR) {
    if lines > 0 { b"\x1bOA" } else { b"\x1bOB" }
  } else if lines > 0 {
    b"\x1b[A"
  } else {
    b"\x1b[B"
  };

  Some(arrow.repeat(lines.unsigned_abs() as usize))
}

fn push_coord(bytes: &mut Vec<u8>, value: usize, utf8: bool) -> Option<()> {
  let encoded = value + 1 + COORD_OFFSET;

  if utf8 {
    let mut buffer = [0u8; 4];
    let ch = char::from_u32(encoded as u32)?;

    bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
  } else if encoded <= X10_MAX_BYTE {
    bytes.push(encoded as u8);
  } else {
    return None;
  }

  Some(())
}
