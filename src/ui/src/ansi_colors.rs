use crate::theme::Theme;
use alacritty_terminal::term::color::Colors as TermPalette;
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb as AnsiRgb};
use gpui::{Hsla, Rgba};

pub fn rgb_to_hsla(rgb: AnsiRgb) -> Hsla {
  Rgba {
    r: rgb.r as f32 / 255.0,
    g: rgb.g as f32 / 255.0,
    b: rgb.b as f32 / 255.0,
    a: 1.0,
  }
  .into()
}

pub fn hsla_to_rgb(color: Hsla) -> AnsiRgb {
  let rgba = Rgba::from(color);

  AnsiRgb {
    r: (rgba.r * 255.0) as u8,
    g: (rgba.g * 255.0) as u8,
    b: (rgba.b * 255.0) as u8,
  }
}

/// Resolving an indexed colour means cube arithmetic and an RGB→HSL conversion,
/// which is far too much to redo for every cell of every frame. The 256 indexed
/// slots are resolved once per frame build and looked up by index after that.
pub struct ColorTable<'a> {
  indexed: [Hsla; 256],
  overrides: &'a TermPalette,
  theme: &'a Theme,
}

impl<'a> ColorTable<'a> {
  pub fn new(theme: &'a Theme, overrides: &'a TermPalette) -> Self {
    let mut table = [theme.term.fg; 256];

    for (i, slot) in table.iter_mut().enumerate() {
      *slot = overrides[i]
        .map(rgb_to_hsla)
        .unwrap_or_else(|| indexed(theme, i as u8));
    }

    Self {
      indexed: table,
      overrides,
      theme,
    }
  }

  pub fn resolve(&self, color: AnsiColor) -> Hsla {
    match color {
      AnsiColor::Spec(rgb) => rgb_to_hsla(rgb),
      AnsiColor::Indexed(i) => self.indexed[i as usize],
      AnsiColor::Named(named) => self.overrides[named as usize]
        .map(rgb_to_hsla)
        .unwrap_or_else(|| named_color(self.theme, named)),
    }
  }
}

pub fn named_color(theme: &Theme, named: NamedColor) -> Hsla {
  let p = &theme.term.palette;

  match named {
    NamedColor::Black | NamedColor::DimBlack => p[0],
    NamedColor::Red | NamedColor::DimRed => p[1],
    NamedColor::Green | NamedColor::DimGreen => p[2],
    NamedColor::Yellow | NamedColor::DimYellow => p[3],
    NamedColor::Blue | NamedColor::DimBlue => p[4],
    NamedColor::Magenta | NamedColor::DimMagenta => p[5],
    NamedColor::Cyan | NamedColor::DimCyan => p[6],
    NamedColor::White | NamedColor::DimWhite => p[7],
    NamedColor::BrightBlack => p[8],
    NamedColor::BrightRed => p[9],
    NamedColor::BrightGreen => p[10],
    NamedColor::BrightYellow => p[11],
    NamedColor::BrightBlue => p[12],
    NamedColor::BrightMagenta => p[13],
    NamedColor::BrightCyan => p[14],
    NamedColor::BrightWhite => p[15],
    NamedColor::Cursor => theme.term.cursor,
    NamedColor::Background => theme.term.bg,
    _ => theme.term.fg,
  }
}

pub fn indexed(theme: &Theme, i: u8) -> Hsla {
  match i {
    0..=15 => theme.term.palette[i as usize],
    16..=231 => {
      let n = i - 16;
      let r = n / 36;
      let g = (n % 36) / 6;
      let b = n % 6;

      let level = |v: u8| -> f32 {
        if v == 0 {
          0.0
        } else {
          (v as f32 * 40.0 + 55.0) / 255.0
        }
      };

      Rgba {
        r: level(r),
        g: level(g),
        b: level(b),
        a: 1.0,
      }
      .into()
    }
    232..=255 => {
      let v = (8 + 10 * (i - 232) as u16) as f32 / 255.0;

      Rgba {
        r: v,
        g: v,
        b: v,
        a: 1.0,
      }
      .into()
    }
  }
}

pub fn color_for_osc_index(index: usize, theme: &Theme) -> AnsiRgb {
  match index {
    0..=255 => hsla_to_rgb(indexed(theme, index as u8)),
    256 => hsla_to_rgb(theme.term.fg),
    257 => hsla_to_rgb(theme.term.bg),
    258 => hsla_to_rgb(theme.term.cursor),
    _ => hsla_to_rgb(theme.term.fg),
  }
}
