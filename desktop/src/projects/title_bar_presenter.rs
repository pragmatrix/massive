use massive_animation::{Animated, Interpolation, Movement};
use massive_applications::prelude::*;
use massive_geometry::{Color, Rect, SizePx, SizedTransform, Transform, Vector3};
use massive_scene::prelude::*;
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt};

use crate::instance_presenter::STRUCTURAL_ANIMATION_DURATION;

const TEXT_COLOR: Color = Color::WHITE;
const TEXT_DECAL_ORDER: usize = 0;
const ELLIPSIS: char = '…';

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TitleBarStyle {
    pub background_color: Color,
    pub font_size: f32,
    /// Horizontal text indent in pixels.
    pub indent: f64,
}

/// A colored bar with a single line of vertically centered text. If the text does not fit, its
/// start is elided. Layout changes are always animated, unless the caller asks to snap.
#[derive(Debug)]
pub struct TitleBarPresenter {
    style: TitleBarStyle,
    content: String,
    available_width: Option<u32>,
    layout_height: f64,
    measured_size: SizePx,
    has_layout: bool,
    movement: Movement<TitleBarMovement>,
    text_transform: Handle<Transform>,
    text: Handle<Visual>,
}

impl TitleBarPresenter {
    pub fn new(style: TitleBarStyle, parent_location: Handle<Location>) -> Self {
        let (scene_transform, location) =
            identity_location().relative_to(&parent_location).submit();
        let (text_transform, text_location) = identity_location().relative_to(&location).submit();

        let background = background_shape(Rect::default(), style.background_color)
            .at(&location)
            .submit();
        let text = Option::<Shape>::None
            .at(&text_location)
            .with_decal_order(TEXT_DECAL_ORDER)
            .submit();

        let background_color = style.background_color;
        let movement = movement(TitleBarMovement::default(), move |movement, progress| {
            let layout = *movement.layout.proceed_with(progress);
            scene_transform.update_if_changed(layout.to_origin_space());
            background.update_if_changed_with(|visual| {
                visual.shapes = [background_shape(layout.rect(), background_color)].into()
            });
        })
        .mount();

        Self {
            style,
            content: String::new(),
            available_width: None,
            layout_height: 0.0,
            measured_size: SizePx::default(),
            has_layout: false,
            movement,
            text_transform,
            text,
        }
    }

    /// The size the bar needs to show its whole text.
    pub fn measured_size(&self) -> SizePx {
        self.measured_size
    }

    pub fn set_text(&mut self, text: &str) {
        if self.content == text {
            return;
        }
        self.content = text.to_string();
        self.update_text();
    }

    pub fn set_layout(&mut self, layout: SizedTransform, animate: bool) {
        self.movement.modify(move |movement, context| {
            movement.layout.animate_if_changed_with(
                context,
                layout,
                STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        });
        if !animate || !self.has_layout {
            self.movement.snap();
        }
        self.has_layout = true;

        // The text is fitted to the target layout once, not to every animation step.
        let available_width = (layout.size.width - 2.0 * self.style.indent).max(0.0) as u32;
        if self.available_width != Some(available_width) || self.layout_height != layout.size.height
        {
            self.available_width = Some(available_width);
            self.layout_height = layout.size.height;
            self.update_text();
        }
    }

    fn update_text(&mut self) {
        let fitted = fitted_text(&self.content, self.available_width, self.style.font_size);
        let full_size = fitted.full_size;
        self.measured_size = SizePx::new(
            full_size.width + (2.0 * self.style.indent) as u32,
            full_size.height,
        );

        let (shape, text_height) = match fitted.text {
            Some((shape, height)) => (Some(shape), height),
            None => (None, 0.0),
        };
        self.text.update_if_changed_with(|visual| {
            visual.shapes = shape.into_iter().collect();
        });

        // Centered on the shaped line height, not on glyph bounds, and snapped to whole pixels.
        let top = ((self.layout_height - text_height) * 0.5).round();
        self.text_transform
            .update_if_changed(Transform::from_translation(Vector3::new(
                self.style.indent,
                top,
                0.0,
            )));
    }
}

#[derive(Debug)]
struct TitleBarMovement {
    layout: Animated<SizedTransform>,
}

impl Default for TitleBarMovement {
    fn default() -> Self {
        Self {
            layout: SizedTransform::default().into(),
        }
    }
}

struct FittedText {
    /// The shaped text that fits and its line height.
    text: Option<(Shape, f64)>,
    /// The size of the whole, non-elided text.
    full_size: SizePx,
}

/// The text as shaped text. If it does not fit, the start is elided character by character until
/// it does. If even a single character does not fit, that is shown anyway.
fn fitted_text(text: &str, available_width: Option<u32>, font_size: f32) -> FittedText {
    let elided = text
        .char_indices()
        .skip(1)
        .map(|(start, _)| format!("{ELLIPSIS}{}", &text[start..]));
    let candidates = std::iter::once(text.to_string()).chain(elided);

    let mut shaped = None;
    let mut full_size = None;
    for candidate in candidates {
        let run = candidate.size(font_size).shape();
        full_size.get_or_insert_with(|| {
            run.as_ref()
                .map_or(SizePx::default(), |run| run.metrics.size())
        });
        let fits = available_width.is_none_or(|width| {
            run.as_ref()
                .is_none_or(|run| run.metrics.size().width <= width)
        });
        shaped = run;
        if fits {
            break;
        }
    }

    FittedText {
        text: shaped.map(|run| {
            let height = run.metrics.size().height as f64;
            (run.with_color(TEXT_COLOR).into_shape(), height)
        }),
        full_size: full_size.unwrap_or_default(),
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}
