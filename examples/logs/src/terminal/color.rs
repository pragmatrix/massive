use std::{ops::Mul, str::FromStr};

// use super::NamedColor;

// pub const COUNT: usize = 269;

// pub const RED: Rgb = Rgb {
//     r: 0xff,
//     g: 0x0,
//     b: 0x0,
// };
// pub const YELLOW: Rgb = Rgb {
//     r: 0xff,
//     g: 0xff,
//     b: 0x0,
// };

#[derive(Debug, Eq, PartialEq, Copy, Clone, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

// a multiply function for Rgb, as the default dim is just *2/3
impl Mul<f32> for Rgb {
    type Output = Rgb;

    fn mul(self, rhs: f32) -> Rgb {
        Rgb {
            r: (f32::from(self.r) * rhs).clamp(0.0, 255.0) as u8,
            g: (f32::from(self.g) * rhs).clamp(0.0, 255.0) as u8,
            b: (f32::from(self.b) * rhs).clamp(0.0, 255.0) as u8,
        }
    }
}

impl FromStr for Rgb {
    type Err = ();

    fn from_str(s: &str) -> ::std::result::Result<Rgb, ()> {
        let mut chars = s.chars();
        let mut rgb = Rgb::default();

        macro_rules! component {
            ($($c:ident),*) => {
                $(
                    match chars.next().and_then(|c| c.to_digit(16)) {
                        Some(val) => rgb.$c = (val as u8) << 4,
                        None => return Err(())
                    }

                    match chars.next().and_then(|c| c.to_digit(16)) {
                        Some(val) => rgb.$c |= val as u8,
                        None => return Err(())
                    }
                )*
            }
        }

        match chars.next() {
            Some('0') => {
                if chars.next() != Some('x') {
                    return Err(());
                }
            }
            Some('#') => (),
            _ => return Err(()),
        }

        component!(r, g, b);

        Ok(rgb)
    }
}

// /// List of indexed colors
// ///
// /// The first 16 entries are the standard ansi named colors. Items 16..232 are
// /// the color cube.  Items 233..256 are the grayscale ramp. Item 256 is
// /// the configured foreground color, item 257 is the configured background
// /// color, item 258 is the cursor color. Following that are 8 positions for dim colors.
// /// Item 267 is the bright foreground color, 268 the dim foreground.
// #[derive(Copy, Clone)]
// pub struct List([Rgb; COUNT]);

// impl<'a> From<&'a Colors> for List {
//     fn from(colors: &Colors) -> List {
//         // Type inference fails without this annotation
//         let mut list = List([Rgb::default(); COUNT]);

//         list.fill_named(colors);
//         list.fill_cube(colors);
//         list.fill_gray_ramp(colors);

//         list
//     }
// }

// impl List {
//     pub fn fill_named(&mut self, colors: &Colors) {
//         // Normals
//         self[NamedColor::Black] = colors.normal().black;
//         self[NamedColor::Red] = colors.normal().red;
//         self[NamedColor::Green] = colors.normal().green;
//         self[NamedColor::Yellow] = colors.normal().yellow;
//         self[NamedColor::Blue] = colors.normal().blue;
//         self[NamedColor::Magenta] = colors.normal().magenta;
//         self[NamedColor::Cyan] = colors.normal().cyan;
//         self[NamedColor::White] = colors.normal().white;

//         // Br
//         self[NamedColor::BrightBlack] = colors.bright().black;
//         self[NamedColor::BrightRed] = colors.bright().red;
//         self[NamedColor::BrightGreen] = colors.bright().green;
//         self[NamedColor::BrightYellow] = colors.bright().yellow;
//         self[NamedColor::BrightBlue] = colors.bright().blue;
//         self[NamedColor::BrightMagenta] = colors.bright().magenta;
//         self[NamedColor::BrightCyan] = colors.bright().cyan;
//         self[NamedColor::BrightWhite] = colors.bright().white;
//         self[NamedColor::BrightForeground] = colors
//             .primary
//             .bright_foreground
//             .unwrap_or(colors.primary.foreground);

//         // Foreground and background
//         self[NamedColor::Foreground] = colors.primary.foreground;
//         self[NamedColor::Background] = colors.primary.background;

//         // Background for custom cursor colors
//         self[NamedColor::Cursor] = colors.cursor.cursor.unwrap_or_else(Rgb::default);

//         // Dims
//         self[ansi::NamedColor::DimForeground] = colors
//             .primary
//             .dim_foreground
//             .unwrap_or(colors.primary.foreground * 0.66);
//         match colors.dim {
//             Some(ref dim) => {
//                 self[ansi::NamedColor::DimBlack] = dim.black;
//                 self[ansi::NamedColor::DimRed] = dim.red;
//                 self[ansi::NamedColor::DimGreen] = dim.green;
//                 self[ansi::NamedColor::DimYellow] = dim.yellow;
//                 self[ansi::NamedColor::DimBlue] = dim.blue;
//                 self[ansi::NamedColor::DimMagenta] = dim.magenta;
//                 self[ansi::NamedColor::DimCyan] = dim.cyan;
//                 self[ansi::NamedColor::DimWhite] = dim.white;
//             }
//             None => {
//                 self[ansi::NamedColor::DimBlack] = colors.normal().black * 0.66;
//                 self[ansi::NamedColor::DimRed] = colors.normal().red * 0.66;
//                 self[ansi::NamedColor::DimGreen] = colors.normal().green * 0.66;
//                 self[ansi::NamedColor::DimYellow] = colors.normal().yellow * 0.66;
//                 self[ansi::NamedColor::DimBlue] = colors.normal().blue * 0.66;
//                 self[ansi::NamedColor::DimMagenta] = colors.normal().magenta * 0.66;
//                 self[ansi::NamedColor::DimCyan] = colors.normal().cyan * 0.66;
//                 self[ansi::NamedColor::DimWhite] = colors.normal().white * 0.66;
//             }
//         }
//     }

//     pub fn fill_cube(&mut self, colors: &Colors) {
//         let mut index: usize = 16;
//         // Build colors
//         for r in 0..6 {
//             for g in 0..6 {
//                 for b in 0..6 {
//                     // Override colors 16..232 with the config (if present)
//                     if let Some(indexed_color) = colors
//                         .indexed_colors
//                         .iter()
//                         .find(|ic| ic.index == index as u8)
//                     {
//                         self[index] = indexed_color.color;
//                     } else {
//                         self[index] = Rgb {
//                             r: if r == 0 { 0 } else { r * 40 + 55 },
//                             b: if b == 0 { 0 } else { b * 40 + 55 },
//                             g: if g == 0 { 0 } else { g * 40 + 55 },
//                         };
//                     }
//                     index += 1;
//                 }
//             }
//         }

//         debug_assert!(index == 232);
//     }

//     pub fn fill_gray_ramp(&mut self, colors: &Colors) {
//         let mut index: usize = 232;

//         for i in 0..24 {
//             // Index of the color is number of named colors + number of cube colors + i
//             let color_index = 16 + 216 + i;

//             // Override colors 232..256 with the config (if present)
//             if let Some(indexed_color) = colors
//                 .indexed_colors
//                 .iter()
//                 .find(|ic| ic.index == color_index)
//             {
//                 self[index] = indexed_color.color;
//                 index += 1;
//                 continue;
//             }

//             let value = i * 10 + 8;
//             self[index] = Rgb {
//                 r: value,
//                 g: value,
//                 b: value,
//             };
//             index += 1;
//         }

//         debug_assert!(index == 256);
//     }
// }

// impl fmt::Debug for List {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.write_str("List[..]")
//     }
// }

// impl Index<ansi::NamedColor> for List {
//     type Output = Rgb;

//     #[inline]
//     fn index(&self, idx: ansi::NamedColor) -> &Self::Output {
//         &self.0[idx as usize]
//     }
// }

// impl IndexMut<ansi::NamedColor> for List {
//     #[inline]
//     fn index_mut(&mut self, idx: ansi::NamedColor) -> &mut Self::Output {
//         &mut self.0[idx as usize]
//     }
// }

// impl Index<usize> for List {
//     type Output = Rgb;

//     #[inline]
//     fn index(&self, idx: usize) -> &Self::Output {
//         &self.0[idx]
//     }
// }

// impl IndexMut<usize> for List {
//     #[inline]
//     fn index_mut(&mut self, idx: usize) -> &mut Self::Output {
//         &mut self.0[idx]
//     }
// }

// impl Index<u8> for List {
//     type Output = Rgb;

//     #[inline]
//     fn index(&self, idx: u8) -> &Self::Output {
//         &self.0[idx as usize]
//     }
// }

// impl IndexMut<u8> for List {
//     #[inline]
//     fn index_mut(&mut self, idx: u8) -> &mut Self::Output {
//         &mut self.0[idx as usize]
//     }
// }
