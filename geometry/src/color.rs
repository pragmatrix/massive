use std::ops::{Add, Div, Mul};

// TODO: WGPU uses f64 for colors, should we do the same?
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Color {
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);

    pub const fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self::new(red, green, blue, 1.0)
    }

    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            alpha,
            red,
            green,
            blue,
        }
    }

    pub fn rgb_u32(rgb: u32) -> Self {
        let r = (rgb & 0xff0000) >> 16;
        let g = (rgb & 0xff00) >> 8;
        let b = rgb & 0xff;
        let r = r as f32 / 255.0;
        let g = g as f32 / 255.0;
        let b = b as f32 / 255.0;
        Color::rgb(r, g, b)
    }

    // http://stackoverflow.com/questions/359612/how-to-change-rgb-color-to-hsv
    pub fn hsv(hue: f32, saturation: f32, value: f32) -> Color {
        let hf = (hue / 60.0).floor();
        let hi = hf.round() as i32 % 6;
        let f = hue / 60.0 - hf;

        let v = value;
        let p = value * (1.0 - saturation);
        let q = value * (1.0 - f * saturation);
        let t = value * (1.0 - (1.0 - f) * saturation);

        match hi {
            0 => Color::rgb(v, t, p),
            1 => Color::rgb(q, v, p),
            2 => Color::rgb(p, v, t),
            3 => Color::rgb(p, q, v),
            4 => Color::rgb(t, p, v),
            _ => Color::rgb(v, p, q),
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
            self.red + rhs.red,
            self.green + rhs.green,
            self.blue + rhs.blue,
            self.alpha + rhs.alpha,
        )
    }
}

impl Div<f32> for Color {
    type Output = Self;
    fn div(self, rhs: f32) -> Self {
        Self::new(
            self.red / rhs,
            self.green / rhs,
            self.blue / rhs,
            self.alpha / rhs,
        )
    }
}

impl Mul<f32> for Color {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(
            self.red * rhs,
            self.green * rhs,
            self.blue * rhs,
            self.alpha * rhs,
        )
    }
}

pub struct HSV {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}
