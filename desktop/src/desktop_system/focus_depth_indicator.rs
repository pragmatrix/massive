use std::sync::Arc;
use std::time::{Duration, Instant};

use massive_animation::{
    Animated, AnimationAllocator, AnimationProgress, Ease, Interpolation, Movement, MovementRuntime,
};
use massive_geometry::{Color, Rect, SizePx, Transform, Vector3};
use massive_renderer::text::FontSystem;
use massive_scene::{
    Handle, IdentityLocation, IntoVisual, Location, LocationSpace, Object, Visual,
};
use massive_shapes::{GlyphRun, IntoShape, Shape, Size as SizeExt};
use massive_shell::Scene;

use super::FocusDepth;

const INDICATOR_DURATION: Duration = Duration::from_millis(1375);
const FADE_IN_END: f32 = 125.0 / 1375.0;
const FADE_OUT_START: f32 = 1125.0 / 1375.0;
const MARGIN: f64 = 16.0;
const FONT_SIZE: f32 = 36.0;
const PADDING: (u32, u32) = (16, 12);
const CORNER_RADIUS: f32 = 8.0;
const DECAL_ORDER: usize = 10;
const FOCUS_DEPTH_LABELS: [(FocusDepth, &str); 6] = [
    (FocusDepth::InstanceFullScreen, "Full Screen"),
    (FocusDepth::Instance, "Instance"),
    (FocusDepth::Launcher, "Launcher"),
    (FocusDepth::Row, "Row"),
    (FocusDepth::Project, "Project"),
    (FocusDepth::Desktop, "Desktop"),
];

#[derive(Debug)]
pub struct FocusDepthIndicatorPresenter {
    scene_transform: Handle<Transform>,
    movement: Movement<FocusDepthIndicatorMovement>,
    size: SizePx,
    active_until: Option<Instant>,
    presentation: Option<SizePx>,
}

impl FocusDepthIndicatorPresenter {
    pub fn new(
        scene: &Scene,
        font_system: &mut FontSystem,
        movement_runtime: &mut MovementRuntime,
    ) -> Self {
        let (badges, size) = FocusDepthIndicatorMovement::create_badges(font_system);
        // Camera space: the indicator is positioned relative to the camera, so no inverse
        // camera translation is needed to keep it fixed on screen.
        let (scene_transform, location) =
            LocationSpace::Camera.identity_location().enter(scene);
        let visual = Arc::<[Shape]>::default()
            .into_visual()
            .at(&location)
            .with_decal_order(DECAL_ORDER)
            .enter(scene);
        let movement = movement_runtime
            .movement(
                FocusDepthIndicatorMovement::new(badges),
                move |movement, progress| movement.apply(progress, &location, &visual),
            )
            .mount();

        Self {
            scene_transform,
            movement,
            size,
            active_until: None,
            presentation: None,
        }
    }

    pub fn show(&mut self, focus_depth: FocusDepth, instant: Instant) {
        self.active_until = Some(instant + INDICATOR_DURATION);
        self.movement.modify(move |movement, context| {
            movement.show(context, focus_depth);
        });
    }

    pub fn sync_layout(&mut self, window_size: SizePx, instant: Instant) {
        if !self
            .active_until
            .is_some_and(|active_until| instant <= active_until)
        {
            self.active_until = None;
            return;
        }
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
struct FocusDepthIndicatorMovement {
    badges: [FocusDepthBadge; FOCUS_DEPTH_LABELS.len()],
    focus_depth: FocusDepth,
    timeline: Animated<f32>,
}

impl FocusDepthIndicatorMovement {
    fn new(badges: [FocusDepthBadge; FOCUS_DEPTH_LABELS.len()]) -> Self {
        Self {
            badges,
            focus_depth: FocusDepth::default(),
            timeline: 1.0.into(),
        }
    }

    fn create_badges(
        font_system: &mut FontSystem,
    ) -> ([FocusDepthBadge; FOCUS_DEPTH_LABELS.len()], SizePx) {
        let glyph_runs = FOCUS_DEPTH_LABELS.map(|(_, label)| {
            label
                .size(FONT_SIZE)
                .shape(font_system)
                .expect("FocusDepth labels must produce glyphs")
        });
        let (horizontal_padding, vertical_padding) = PADDING;
        let width = glyph_runs
            .iter()
            .map(|glyph_run| glyph_run.metrics.width)
            .max()
            .expect("FocusDepth labels must not be empty")
            + horizontal_padding * 2;
        let height = glyph_runs
            .iter()
            .map(|glyph_run| glyph_run.metrics.size().height)
            .max()
            .expect("FocusDepth labels must not be empty")
            + vertical_padding * 2;
        let size = SizePx::new(width, height);
        let badges = glyph_runs.map(|mut glyph_run| {
            glyph_run.translation = Vector3::new(
                (width - horizontal_padding - glyph_run.metrics.width) as f64,
                vertical_padding as f64,
                0.0,
            );
            FocusDepthBadge { glyph_run, size }
        });

        (badges, size)
    }

    fn show(&mut self, context: &mut dyn AnimationAllocator, focus_depth: FocusDepth) {
        self.focus_depth = focus_depth;
        self.timeline.snap(0.0);
        self.timeline
            .animate(context, 1.0, INDICATOR_DURATION, Interpolation::Linear);
    }

    fn apply(
        &mut self,
        progress: AnimationProgress,
        location: &Handle<Location>,
        visual: &Handle<Visual>,
    ) {
        let timeline = *self.timeline.proceed(progress);
        let alpha = if timeline < FADE_IN_END {
            (timeline / FADE_IN_END).interpolate(Interpolation::CubicOut)
        } else {
            let fade_progress = ((timeline - FADE_OUT_START) / (1.0 - FADE_OUT_START))
                .clamp(0.0, 1.0)
                .interpolate(Interpolation::CubicOut);
            1.0 - fade_progress
        };
        let shapes = self.badges[self.focus_depth as usize].shapes(alpha);
        visual.update_if_changed(Visual::new(location, shapes).with_decal_order(DECAL_ORDER));
    }
}

#[derive(Debug)]
struct FocusDepthBadge {
    glyph_run: GlyphRun,
    size: SizePx,
}

impl FocusDepthBadge {
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
