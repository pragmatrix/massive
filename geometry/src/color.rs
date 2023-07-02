use std::ops::{Add, Div, Mul};

// TODO: WGPU uses f64 for colors, should we do the same?
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Color {
    pub alpha: f32,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Color {
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::new(1.0, 0.0, 0.0, 0.0);

    pub const fn new(alpha: f32, red: f32, green: f32, blue: f32) -> Self {
        Self {
            alpha,
            red,
            green,
            blue,
        }
    }

    pub fn from_rgb_u32(rgb: u32) -> Self {
        let r = (rgb & 0xff0000) >> 16;
        let g = (rgb & 0xff00) >> 8;
        let b = rgb & 0xff;
        let r = r as f32 / 255.0;
        let g = g as f32 / 255.0;
        let b = b as f32 / 255.0;
        Color::new(1.0, r, g, b)
    }

    // http://stackoverflow.com/questions/359612/how-to-change-rgb-color-to-hsv
    pub fn from_hsv(hue: f32, saturation: f32, value: f32) -> Color {
        let hf = (hue / 60.0).floor();
        let hi = hf.round() as i32 % 6;
        let f = hue / 60.0 - hf;

        let v = value;
        let p = value * (1.0 - saturation);
        let q = value * (1.0 - f * saturation);
        let t = value * (1.0 - (1.0 - f) * saturation);

        match hi {
            0 => Color::new(1.0, v, t, p),
            1 => Color::new(1.0, q, v, p),
            2 => Color::new(1.0, p, v, t),
            3 => Color::new(1.0, p, q, v),
            4 => Color::new(1.0, t, p, v),
            _ => Color::new(1.0, v, p, q),
        }
    }

    pub fn mix(self, other: Self) -> Self {
        (self + other) / 2.0
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self { alpha, ..self }
    }
}

impl Add for Color {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(
            self.alpha + rhs.alpha,
            self.red + rhs.red,
            self.green + rhs.green,
            self.blue + rhs.blue,
        )
    }
}

impl Div<f32> for Color {
    type Output = Self;
    fn div(self, rhs: f32) -> Self {
        Self::new(
            self.alpha / rhs,
            self.red / rhs,
            self.green / rhs,
            self.blue / rhs,
        )
    }
}

impl Mul<f32> for Color {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(
            self.alpha * rhs,
            self.red * rhs,
            self.green * rhs,
            self.blue * rhs,
        )
    }
}

pub struct HSV {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}
