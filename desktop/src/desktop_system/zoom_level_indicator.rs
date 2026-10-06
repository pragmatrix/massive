use std::sync::Arc;
use std::time::Duration;

use massive_animation::{
    Animated, AnimationAllocator, AnimationProgress, Ease, Interpolation, Movement,
};
use massive_applications::prelude::*;
use massive_geometry::{Color, Rect, SizePx, Transform, Vector3};
use massive_scene::LocationSpace;
use massive_scene::prelude::*;
use massive_shapes::{GlyphRun, IntoShape, Shape, Size as SizeExt};

use super::ZoomLevel;

const INDICATOR_DURATION: Duration = Duration::from_millis(1375);
const FADE_IN_END: f32 = 125.0 / 1375.0;
const FADE_OUT_START: f32 = 1125.0 / 1375.0;
const MARGIN: f64 = 16.0;
const FONT_SIZE: f32 = 36.0;
const PADDING: (u32, u32) = (16, 12);
const CORNER_RADIUS: f32 = 8.0;
// The renderer draws decal layers in ascending order, so the maximum stays above other visuals.
const DECAL_ORDER: usize = usize::MAX;
/// Digits reserved for the project nesting depth so the badge does not resize between depths.
const RESERVED_DEPTH: &str = "00";
/// The badge label per zoom level, ordered from outermost to innermost.
const ZOOM_LEVEL_LABELS: [(ZoomLevel, &str); 4] = [
    (ZoomLevel::Project, "Project"),
    (ZoomLevel::Row, "Row"),
    (ZoomLevel::Slot, "Slot"),
    (ZoomLevel::Instance, "Instance"),
];

#[derive(Debug)]
pub struct ZoomLevelIndicatorPresenter {
    scene_transform: Handle<Transform>,
    movement: Movement<ZoomLevelIndicatorMovement>,
    size: SizePx,
    presentation: Option<SizePx>,
}

impl ZoomLevelIndicatorPresenter {
    pub fn new() -> Self {
        let size = badge_size();
        // Camera space: the indicator is positioned relative to the camera, so no inverse
        // camera translation is needed to keep it fixed on screen.
        let (scene_transform, location) =
            identity_location().in_space(LocationSpace::Camera).submit();
        let visual = Arc::<[Shape]>::default()
            .into_visual()
            .at(&location)
            .with_decal_order(DECAL_ORDER)
            .submit();
        let movement = movement(
            ZoomLevelIndicatorMovement::new(),
            move |movement, progress| movement.apply(progress, &location, &visual),
        )
        .mount();

        Self {
            scene_transform,
            movement,
            size,
            presentation: None,
        }
    }

    /// Shows the framed level and the nesting depth of the framed project (ADR 0017).
    pub fn show(&mut self, zoom_level: ZoomLevel, project_depth: usize) {
        let badge = ZoomLevelBadge::new(
            &format!("{} {project_depth}", zoom_level_label(zoom_level)),
            self.size,
        );
        self.movement.modify(move |movement, context| {
            movement.show(context, badge);
        });
    }

    pub fn sync_layout(&mut self, window_size: SizePx) {
        if self.presentation == Some(window_size) {
            return;
        }
        self.presentation = Some(window_size);

        let (window_width, window_height) = window_size.into();
        // Position the badge in the top-right corner of the camera's pixel plane.
        let camera_position = Transform::from_xy(
            window_width as f64 * 0.5 - self.size.width as f64 - MARGIN,
            -(window_height as f64) * 0.5 + MARGIN,
        );
        self.scene_transform.update_if_changed(camera_position);
    }
}

#[derive(Debug)]
struct ZoomLevelIndicatorMovement {
    badge: Option<ZoomLevelBadge>,
    timeline: Animated<f32>,
}

impl ZoomLevelIndicatorMovement {
    fn new() -> Self {
        Self {
            badge: None,
            timeline: 1.0.into(),
        }
    }

    fn show(&mut self, context: &mut dyn AnimationAllocator, badge: ZoomLevelBadge) {
        self.badge = Some(badge);
        self.timeline.snap(0.0);
        self.timeline
            .animate_with(context, 1.0, INDICATOR_DURATION, Interpolation::Linear);
    }

    fn apply(
        &mut self,
        progress: AnimationProgress,
        location: &Handle<Location>,
        visual: &Handle<Visual>,
    ) {
        let timeline = *self.timeline.proceed_with(progress);
        let alpha = if timeline < FADE_IN_END {
            (timeline / FADE_IN_END).interpolate(Interpolation::CubicOut)
        } else {
            let fade_progress = ((timeline - FADE_OUT_START) / (1.0 - FADE_OUT_START))
                .clamp(0.0, 1.0)
                .interpolate(Interpolation::CubicOut);
            1.0 - fade_progress
        };
        let shapes = self
            .badge
            .as_ref()
            .map(|badge| badge.shapes(alpha))
            .unwrap_or_default();
        visual.update_if_changed(Visual::new(location, shapes).with_decal_order(DECAL_ORDER));
    }
}

#[derive(Debug)]
struct ZoomLevelBadge {
    glyph_run: GlyphRun,
    size: SizePx,
}

impl ZoomLevelBadge {
    /// Shapes `label` right-aligned in a badge of `size`.
    fn new(label: &str, size: SizePx) -> Self {
        let (horizontal_padding, vertical_padding) = PADDING;
        let mut glyph_run = shape_label(label);
        glyph_run.translation = Vector3::new(
            size.width
                .saturating_sub(horizontal_padding + glyph_run.metrics.width) as f64,
            vertical_padding as f64,
            0.0,
        );
        Self { glyph_run, size }
    }

    fn shapes(&self, alpha: f32) -> Arc<[Shape]> {
        if alpha == 0.0 {
            return Arc::default();
        }

        let background = massive_shapes::RoundRect::new(
            Rect::from_size((self.size.width as f64, self.size.height as f64)),
            CORNER_RADIUS,
            Color::rgb_u32(0x181818).with_alpha(0.85 * alpha),
        )
        .into_shape();
        let text = self
            .glyph_run
            .clone()
            .with_color(Color::rgb_u32(0xf5f5f5).with_alpha(alpha))
            .into_shape();
        [background, text].into()
    }
}

fn zoom_level_label(zoom_level: ZoomLevel) -> &'static str {
    ZOOM_LEVEL_LABELS
        .iter()
        .find_map(|(level, label)| (*level == zoom_level).then_some(*label))
        .expect("Every zoom level must have a label")
}

/// The badge fits the widest level label followed by the reserved depth digits.
fn badge_size() -> SizePx {
    let glyph_runs =
        ZOOM_LEVEL_LABELS.map(|(_, label)| shape_label(&format!("{label} {RESERVED_DEPTH}")));
    let (horizontal_padding, vertical_padding) = PADDING;
    let width = glyph_runs
        .iter()
        .map(|glyph_run| glyph_run.metrics.width)
        .max()
        .expect("Zoom-level labels must not be empty")
        + horizontal_padding * 2;
    let height = glyph_runs
        .iter()
        .map(|glyph_run| glyph_run.metrics.size().height)
        .max()
        .expect("Zoom-level labels must not be empty")
        + vertical_padding * 2;
    SizePx::new(width, height)
}

fn shape_label(label: &str) -> GlyphRun {
    label
        .size(FONT_SIZE)
        .shape()
        .expect("Zoom-level labels must produce glyphs")
}
