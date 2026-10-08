use massive_applications::prelude::*;
use massive_geometry::{Color, Rect, SizedTransform, Transform, Vector3};
use massive_scene::prelude::*;
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt};

/// Height, font size, and horizontal text indent at a scale factor of 1.0.
const BASE_HEIGHT: f64 = 24.0;
const BASE_FONT_SIZE: f64 = 14.0;
const BASE_INDENT: f64 = 8.0;

const BASE_BACKGROUND_COLOR: Color = Color::rgb_u32(0x1f4f8f);
const ASSISTANT_BACKGROUND_COLOR: Color = Color::rgb_u32(0x6a3d8f);
const INSTANCE_TITLE_BAR_TEXT_COLOR: Color = Color::WHITE;
const INSTANCE_TITLE_BAR_TEXT_DECAL_ORDER: usize = 0;
const ELLIPSIS: char = '…';
/// Separates the parts of a title, in the bar as well as in the window title.
pub const TITLE_SEPARATOR: &str = "  ·  ";

/// The bar drawn above an instance's view (ADR 0019): the primary view's title followed by the
/// launcher's label.
#[derive(Debug)]
pub struct InstanceTitleBarPresenter {
    metrics: InstanceTitleBarMetrics,
    background_color: Color,
    label: String,
    title: String,
    available_width: Option<u32>,
    scene_transform: Handle<Transform>,
    background: Handle<Visual>,
    text_transform: Handle<Transform>,
    text: Handle<Visual>,
    layout_size: Rect,
}

impl InstanceTitleBarPresenter {
    pub fn new(spec: InstanceTitleBarSpec, parent_location: Handle<Location>) -> Self {
        let InstanceTitleBarSpec {
            label,
            metrics,
            is_assistant,
        } = spec;
        let background_color = if is_assistant {
            ASSISTANT_BACKGROUND_COLOR
        } else {
            BASE_BACKGROUND_COLOR
        };
        let (scene_transform, location) =
            identity_location().relative_to(&parent_location).submit();
        let (text_transform, text_location) = identity_location().relative_to(&location).submit();

        let background = background_shape(Rect::default(), background_color)
            .at(&location)
            .submit();
        let text = text_shape(&label, metrics.font_size)
            .map(|(shape, _)| shape)
            .at(&text_location)
            .with_decal_order(INSTANCE_TITLE_BAR_TEXT_DECAL_ORDER)
            .submit();

        let mut presenter = Self {
            metrics,
            background_color,
            label,
            title: String::new(),
            available_width: None,
            scene_transform,
            background,
            text_transform,
            text,
            layout_size: Rect::default(),
        };
        presenter.update_text();
        presenter
    }

    /// The bar's height, measured at regular presentation scale.
    pub fn measured_height(&self) -> u32 {
        self.metrics.height
    }

    pub fn set_title(&mut self, title: &str) {
        if self.title == title {
            return;
        }
        self.title = title.to_string();
        self.update_text();
    }

    pub fn set_layout(&mut self, layout: SizedTransform) {
        self.scene_transform
            .update_if_changed(layout.to_origin_space());

        let rect = layout.rect();
        if self.layout_size != rect {
            self.layout_size = rect;
            self.background.update_if_changed_with(|visual| {
                visual.shapes = [background_shape(rect, self.background_color)].into()
            });
        }

        let available_width = (layout.size.width - 2.0 * self.metrics.indent).max(0.0) as u32;
        if self.available_width != Some(available_width) {
            self.available_width = Some(available_width);
            self.update_text();
        }
    }

    fn update_text(&mut self) {
        let text = fit_text(
            &self.label,
            &self.title,
            self.available_width,
            self.metrics.font_size,
        );
        let (shape, text_height) = match text_shape(&text, self.metrics.font_size) {
            Some((shape, height)) => (Some(shape), height),
            None => (None, 0.0),
        };
        self.text.update_if_changed_with(|visual| {
            visual.shapes = shape.into_iter().collect();
        });

        // The text is indented and vertically centered in the bar.
        let top = (self.metrics.height as f64 - text_height) * 0.5;
        self.text_transform
            .update_if_changed(Transform::from_translation(Vector3::new(
                self.metrics.indent,
                top,
                0.0,
            )));
    }
}

/// What an instance needs to create its title bar.
#[derive(Debug, Clone)]
pub struct InstanceTitleBarSpec {
    pub label: String,
    pub metrics: InstanceTitleBarMetrics,
    /// Assistant instances are set apart by their background color.
    pub is_assistant: bool,
}

/// The bar's sizes in pixels at regular presentation scale (ADR 0019). They follow the monitor's
/// scale factor, like the terminal's font.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceTitleBarMetrics {
    pub height: u32,
    font_size: f32,
    indent: f64,
}

impl InstanceTitleBarMetrics {
    pub fn from_scale_factor(scale_factor: f64) -> Self {
        Self {
            height: (BASE_HEIGHT * scale_factor).round() as u32,
            font_size: (BASE_FONT_SIZE * scale_factor) as f32,
            indent: (BASE_INDENT * scale_factor).round(),
        }
    }
}

/// The label and the title as one string. If it does not fit, the start of the title is elided.
fn fit_text(label: &str, title: &str, available_width: Option<u32>, font_size: f32) -> String {
    let compose = |title: &str| {
        if title.is_empty() {
            label.to_string()
        } else {
            format!("{title}{TITLE_SEPARATOR}{label}")
        }
    };

    let Some(available_width) = available_width else {
        return compose(title);
    };
    let fits =
        |text: &str| text_width(text, font_size).is_none_or(|width| width <= available_width);

    let full = compose(title);
    if fits(&full) {
        return full;
    }

    // Drop leading characters of the title until the remainder fits.
    let mut remainder = title;
    while let Some((_, rest)) = remainder.split_at_checked(first_char_len(remainder)) {
        if remainder.is_empty() {
            break;
        }
        remainder = rest;
        let candidate = compose(&format!("{ELLIPSIS}{remainder}"));
        if fits(&candidate) {
            return candidate;
        }
    }
    label.to_string()
}

fn text_width(text: &str, font_size: f32) -> Option<u32> {
    text.to_string()
        .size(font_size)
        .shape()
        .map(|run| run.metrics.size().width)
}

fn first_char_len(text: &str) -> usize {
    text.chars().next().map_or(0, char::len_utf8)
}

/// The text's shape and its height.
fn text_shape(text: &str, font_size: f32) -> Option<(Shape, f64)> {
    text.to_string().size(font_size).shape().map(|run| {
        let height = run.metrics.size().height as f64;
        (
            run.with_color(INSTANCE_TITLE_BAR_TEXT_COLOR).into_shape(),
            height,
        )
    })
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}
