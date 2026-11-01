use std::time::Instant;

use anyhow::Result;
use uuid::Uuid;
use winit::event::MouseButton;

use massive_animation::{Animated, AnimationProgress, Movement};
use massive_applications::prelude::*;
use massive_applications::{InstanceId, InstanceParameters, ViewEvent};
use massive_geometry::{Color, Quaternion, Rect, RectPx, Size, SizePx, SizedTransform, Vector3};
use massive_input::EventManager;
use massive_layout::{
    ChildPlacement, LayoutAxis, MeasuredLayout, Offset, Placement, Rect as LayoutRect,
    Size as LayoutSize,
};
use massive_scene::prelude::*;
use massive_shapes::{self as shapes, IntoShape, Shape, Size as SizeExt};

use super::slot_layout::SlotLayout;
use super::visor_layout;
use crate::desktop_system::change::Zoom;
use crate::desktop_system::{
    AppliedLayout, Commands, DesktopCommand, DesktopTarget, FocusIntent, KeyContext, KeyHandler,
    KeyInput, KeyOutcome, LayoutContext, McpActivation, SelectionOperation, Shortcut, TargetLayout,
    measure_container, place_container_children,
};
use crate::instance_presenter::{InstanceKind, instance_extent};
use crate::policy::colors;
use crate::policy::constants::{launcher, mcp_indicator};
use crate::policy::when::{self, PressingCommandEnter};
use crate::projects::{LaunchProfileId, LauncherMode};

use super::runtime_configuration::Params;

#[derive(Debug, Clone, Copy)]
struct VisorLayoutSummary {
    group_center_x: f64,
    flat_span: f64,
    instance_count: usize,
}

#[derive(Debug)]
pub struct LauncherPresenter {
    id: LaunchProfileId,
    movement: Movement<LauncherMovement>,

    location: Handle<Location>,
    /// Held only because dropping the handle removes the name from the scene.
    _name: Handle<Visual>,
    presents_instance: bool,

    /// The visor's focus anchor the visor centers on and that stays visible during collapse: the
    /// most recently focused instance while no mouse button was pressed. The visor centers on this
    /// anchor independent of the live keyboard focus.
    pub focus_anchor_instance: Option<InstanceId>,

    /// We need our own EventManager, because the event's positions are relative to us and even if
    /// they weren't sharing the event history isn't really possible.
    event_manager: EventManager<ViewEvent>,
}

#[derive(Debug)]
struct LauncherMovement {
    layout: SlotLayout,
    // Alpha fading of name / background.
    fader: Animated<f32>,
    /// Lives in the movement, because the movement redraws the background on every step.
    background_color: Animated<Color>,
}

impl LauncherPresenter {
    // Ergonomics: Scene can be imported from two locations, use just the shell one, or somehow
    // introduce something new that exports more ergonomic UI components.
    pub fn new(
        parent_location: Handle<Location>,
        id: LaunchProfileId,
        name: String,
        mcp_activation: McpActivation,
        size: Size,
    ) -> Self {
        let background_color = mcp_activation.launcher_background();
        // Ergonomics: I want this to look like `rect.as_shape().with_color(Color::WHITE);`
        let background_shape = background_shape(size.to_rect(), background_color);

        let (our_transform, our_location) =
            identity_location().relative_to(&parent_location).submit();

        // Instances are parented to `our_location`, so the fade needs its own child location;
        // the renderer skips it at alpha 0, keeping it out of the depth buffer under instances.
        let (_, fade_location) = identity_location().relative_to(&our_location).submit();

        let background = background_shape.at(&fade_location).submit();

        let name = name
            // Idea: To not waste so much memory here for large fonts, may use a quality index that
            // is automatically applied based on the font, small fonts high quality, large fonts,
            // lower quality, the quality index starts with 1 and is the effective pixel resolution
            // divisor: Quality 1: original size, quality 2: 1/4th the memory in use (horizontal
            // size / 2, vertical size / 2)
            //
            // Idea: No, this should be fully automatic depending of how large the font is shown I
            // guess. Make this independent of the font size, but dependent on what is visible (a
            // background optimizer).
            .size(launcher::NAME_FONT_SIZE)
            .shape()
            .map(|r| r.with_color(colors::launcher::TEXT).into_shape())
            .at(&fade_location)
            .with_decal_order(0)
            .submit();

        let scene_transform = our_transform.clone();
        let movement = movement(
            LauncherMovement::new(background_color),
            move |movement, context| {
                movement.apply_animations(context, &scene_transform, &background, &fade_location);
            },
        )
        .mount();

        Self {
            id,
            movement,
            location: our_location,
            _name: name,
            presents_instance: false,
            focus_anchor_instance: None,
            event_manager: EventManager::default(),
        }
    }

    fn place_visor_children(
        &self,
        children: &[MeasuredLayout<2>],
        center_index: usize,
        expanded: bool,
        default_panel_size: SizePx,
        placements: &mut [Placement<Transform, 2>],
    ) {
        let offset = centered_children_offset(children, default_panel_size.width as i32);

        let Some(summary) = visor_layout_summary(offset, children) else {
            place_container_children(
                LayoutAxis::HORIZONTAL,
                launcher::CHILD_SPACING,
                offset,
                children.iter().map(|child| child.size),
                placements,
            );
            return;
        };

        let mut offset = offset;

        for (child_index, (child, placement)) in children.iter().zip(placements).enumerate() {
            let child_size = child.size;
            if child_index > 0 {
                offset[0] += launcher::CHILD_SPACING;
            }

            let center_y = child_center_y(offset, child_size);
            let transform =
                visor_child_transform(child_index, center_y, summary, center_index, expanded);
            let visible = visor_child_visibility(child_index, center_index, expanded);

            *placement = Placement::new(transform, LayoutRect::new(offset, child_size))
                .with_visibility(visible);
            offset[0] += child_size[0] as i32;
        }
    }

    // Architecture: I don't want the launcher here to directly generate commands. may be
    // LauncherCommand? Not sure.
    pub fn process(&mut self, event: ViewEvent, params: &Params) -> Result<Commands> {
        let presents_instance = self.presents_instance();

        let Some(event) = self.event_manager.add_event(event, Instant::now()) else {
            return Ok(Commands::Empty);
        };

        if !presents_instance && event.detect_click(MouseButton::Left).is_some() {
            let kind = InstanceKind::from_modifiers(event.keyboard_modifiers());
            return Ok(self.start_instance(kind, params).into());
        }

        Ok(Commands::Empty)
    }

    fn presents_instance(&self) -> bool {
        self.presents_instance
    }

    /// An assistant starts plain, without the launcher's configured parameters (ADR 0014).
    fn start_instance(&self, kind: InstanceKind, params: &Params) -> DesktopCommand {
        let parameters = match kind {
            InstanceKind::Primary => params.clone(),
            InstanceKind::Assistant => InstanceParameters::new(),
        };

        // Usability: Should pass this rectangle?
        DesktopCommand::StartInstance {
            launcher: self.id,
            instance: Uuid::new_v4().into(),
            root: None,
            parameters,
            kind,
            focus: FocusIntent::Follow,
        }
    }

    pub fn set_layout(&mut self, layout: SizedTransform, animate: bool) {
        self.movement.modify(move |movement, context| {
            movement.layout.set(context, layout);
        });
        if !animate {
            self.movement.snap();
        }
    }

    pub fn location(&self) -> Handle<Location> {
        self.location.clone()
    }

    /// Converts the on-screen layout with `previous_to_current`, which maps the previous parent's
    /// coordinates into the current parent's. Nothing is animated here. The next layout then
    /// slides from the converted position.
    pub fn rebase(&self, previous_to_current: Transform) {
        self.movement.modify(move |movement, _context| {
            movement.layout.rebase(previous_to_current);
        });
    }

    /// Attaches the launcher's scene location to another parent location. This is used when the
    /// launcher's slot moves to the matrix of another project.
    pub fn set_parent_location(&self, parent: &Handle<Location>) {
        self.location.update_if_changed_with(|location| {
            location.parent = parent.to_ref().into();
        });
    }

    /// Shows a changed MCP activation (ADR 0036).
    pub fn set_mcp_activation(&mut self, mcp_activation: McpActivation) {
        let color = mcp_activation.launcher_background();
        self.movement.modify(move |movement, context| {
            movement.background_color.animate_if_changed_with(
                context,
                color,
                mcp_indicator::COLOR_DURATION,
                mcp_indicator::COLOR_INTERPOLATION,
            );
        });
    }

    pub fn fade_out(&mut self) {
        self.presents_instance = true;
        self.movement.modify(|movement, context| {
            movement.fader.animate_with(
                context,
                0.0,
                launcher::FADING_DURATION,
                launcher::FADING_INTERPOLATION,
            );
        });
    }

    pub fn fade_in(&mut self) {
        self.presents_instance = false;
        self.movement.modify(|movement, context| {
            movement.fader.animate_with(
                context,
                1.0,
                launcher::FADING_DURATION,
                launcher::FADING_INTERPOLATION,
            );
        });
    }
}

/// The launcher starts an instance on Enter and Cmd+T, but only when it is itself the keyboard
/// focus. Keys passing through to an instance of this launcher are not meant for it.
///
/// Cmd+Enter zooms to the launcher first and starts only once the camera is there.
impl KeyHandler for LauncherPresenter {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome {
        if !context.is_focused {
            return KeyOutcome::Pass;
        }

        match Shortcut::from_input(input) {
            Some(Shortcut::CommandEnter) => {
                match when::pressing_command_enter(context.fully_zoomed_in) {
                    PressingCommandEnter::Act => {}
                    PressingCommandEnter::ZoomFirst => {
                        return KeyOutcome::Consumed(DesktopCommand::Zoom(Zoom::Enter).into());
                    }
                }
            }
            Some(Shortcut::RemoveSlot) => {
                let command = DesktopCommand::ConfigureSelection(SelectionOperation::Remove);
                return KeyOutcome::Consumed(command.into());
            }
            Some(Shortcut::Enter | Shortcut::StartInstance) => {}
            _ => return KeyOutcome::Pass,
        }

        let kind = InstanceKind::from_modifiers(input.modifiers);
        let params = &context.configuration[self.id].params;
        KeyOutcome::Consumed(self.start_instance(kind, params).into())
    }
}

/// Bands pack their instances horizontally; visors arc them around the focus anchor and collapse
/// while none of them holds the keyboard focus.
impl TargetLayout for LauncherPresenter {
    type Part = ();

    fn measure(
        &self,
        (): (),
        context: &LayoutContext,
        children: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2> {
        measure_launcher(self.id, context, children)
    }

    fn place_children(
        &self,
        (): (),
        context: &LayoutContext,
        children: ChildPlacement<'_, Transform, 2>,
    ) {
        match context.configuration[self.id].mode {
            LauncherMode::Band => place_container_children(
                LayoutAxis::HORIZONTAL,
                launcher::CHILD_SPACING,
                Offset::default(),
                children.measurements.iter().map(|child| child.size),
                children.placements,
            ),
            LauncherMode::Visor => {
                let index_of = |instance: Option<InstanceId>| {
                    let instance = instance?;
                    context
                        .hierarchy
                        .launcher_instances(self.id)
                        .position(|child| child == instance)
                };
                let expanded = index_of(context.focused_instance).is_some();
                let center_index = index_of(self.focus_anchor_instance).unwrap_or_default();
                self.place_visor_children(
                    children.measurements,
                    center_index,
                    expanded,
                    context.default_panel_size,
                    children.placements,
                );
            }
        }
    }

    /// A visor collapses around the focus anchor, which matters only when more than the anchor
    /// instance exists.
    fn relayouts_on_focus(&self, (): (), context: &LayoutContext) -> bool {
        context.configuration[self.id].mode == LauncherMode::Visor
            && context.hierarchy.launcher_instances(self.id).count() > 1
    }

    fn apply_layout(
        &mut self,
        (): (),
        layout: SizedTransform,
        _visible: bool,
        animate: bool,
    ) -> Result<AppliedLayout> {
        self.set_layout(layout, animate);
        Ok(AppliedLayout::Applied)
    }
}

/// A launcher spans an instance's extent, title bar included (ADR 0019). An empty one stretches to
/// its matrix row, while instances give it an intrinsic size, which the matrix centers in its slot
/// (ADR 0023); bands measure from their instances like any horizontal container.
fn measure_launcher(
    launcher: LaunchProfileId,
    context: &LayoutContext,
    children: &[MeasuredLayout<2>],
) -> MeasuredLayout<2> {
    let instance_extent: LayoutSize<2> =
        instance_extent(context.default_panel_size, context.title_bar_metrics.height).into();
    if context
        .hierarchy
        .get_nested(&DesktopTarget::Launcher(launcher))
        .is_empty()
    {
        return MeasuredLayout::new(instance_extent, [false, true]);
    }
    match context.configuration[launcher].mode {
        LauncherMode::Visor => instance_extent.into(),
        LauncherMode::Band => measure_container(
            LayoutAxis::HORIZONTAL,
            Default::default(),
            launcher::CHILD_SPACING as u32,
            children.iter().map(|child| child.size),
        )
        .into(),
    }
}

impl LauncherMovement {
    fn new(background_color: Color) -> Self {
        Self {
            layout: SlotLayout::default(),
            fader: 1.0.into(),
            background_color: background_color.into(),
        }
    }

    fn apply_animations(
        &mut self,
        progress: AnimationProgress,
        scene_transform_handle: &Handle<Transform>,
        background: &Handle<Visual>,
        fade_location: &Handle<Location>,
    ) {
        let alpha = *self.fader.proceed_with(progress);
        let background_color = *self.background_color.proceed_with(progress);
        fade_location.update_if_changed_with(|location| location.alpha = alpha);

        let Some(layout) = self.layout.proceed_with(progress) else {
            return;
        };
        scene_transform_handle.update_if_changed(layout.pivoted_at_origin());

        // Performance: How can we not call this if `self.size` is not animating. `is_animating()`
        // is perhaps not reliable.
        background.update_if_changed_with(|visual| {
            visual.shapes = [background_shape(layout.rect(), background_color)].into()
        });
    }
}

fn background_shape(rect: Rect, color: Color) -> Shape {
    shapes::Rect::new(rect, color).into()
}

fn centered_children_offset(children: &[MeasuredLayout<2>], panel_width: i32) -> Offset<2> {
    Offset::from([(panel_width - children_span(children)) / 2, 0])
}

fn children_span(children: &[MeasuredLayout<2>]) -> i32 {
    children
        .iter()
        .map(|child| child.size[0] as i32)
        .sum::<i32>()
        + launcher::CHILD_SPACING * children.len().saturating_sub(1) as i32
}

fn visor_layout_summary(
    mut offset: Offset<2>,
    children: &[MeasuredLayout<2>],
) -> Option<VisorLayoutSummary> {
    let mut first_center_x = None;
    let mut last_center_x = None;

    for (child_index, child) in children.iter().enumerate() {
        let child_size = child.size;
        if child_index > 0 {
            offset[0] += launcher::CHILD_SPACING;
        }

        let center_x = child_rect(offset, child_size).center().cast::<f64>().x;
        first_center_x.get_or_insert(center_x);
        last_center_x = Some(center_x);

        offset[0] += child_size[0] as i32;
    }

    let instance_count = children.len();
    if instance_count <= 1 {
        return None;
    }

    let first_center_x = first_center_x.expect("Internal error: Expected at least one instance");
    let last_center_x = last_center_x.expect("Internal error: Expected at least one instance");

    Some(VisorLayoutSummary {
        group_center_x: (first_center_x + last_center_x) * 0.5,
        flat_span: (last_center_x - first_center_x).abs(),
        instance_count,
    })
}

fn child_rect(offset: Offset<2>, child_size: LayoutSize<2>) -> RectPx {
    LayoutRect::new(offset, child_size).into()
}

fn child_center_y(offset: Offset<2>, child_size: LayoutSize<2>) -> f64 {
    (offset[1] + child_size[1] as i32 / 2) as f64
}

fn visor_child_transform(
    instance_index: usize,
    center_y: f64,
    summary: VisorLayoutSummary,
    center_index: usize,
    expanded: bool,
) -> Transform {
    let expansion_factor = if expanded { 1.0 } else { 0.0 };
    let placement = visor_layout::placement(
        instance_index,
        summary.instance_count,
        summary.flat_span,
        center_index,
        expansion_factor,
    )
    .expect("Internal error: Visor placement requires at least two instances");
    let mut transform = Transform::new(
        Vector3::new(
            summary.group_center_x + placement.center_x_offset,
            center_y,
            placement.center_z,
        ),
        Quaternion::from_rotation_y(placement.yaw),
        1.0,
    );

    if instance_index != center_index {
        transform.translate.z +=
            launcher::COLLAPSED_NON_ANCHOR_Z_OFFSET * if expanded { 0.0 } else { 1.0 };
    }

    transform
}

fn visor_child_visibility(instance_index: usize, center_index: usize, expanded: bool) -> bool {
    expanded || instance_index == center_index
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::OrderedHierarchy;
    use crate::desktop_system::InstancePart;
    use crate::instance_presenter::InstanceTitleBarMetrics;
    use crate::projects::SharedColumnCounts;
    use crate::projects::persistence::parse_configuration;

    /// A launcher spans an instance's extent, title bar included, and only an empty one stretches
    /// vertically, so one holding instances keeps their intrinsic size (ADR 0019).
    #[test]
    fn launchers_measure_with_the_instance_extent() {
        let configuration = parse_configuration(
            Path::new("/config/desktop.json"),
            r#"{
                "startup": "/busy",
                "slots": [
                    { "at": [0, 0], "launcher": { "name": "empty", "mode": "visor" } },
                    { "at": [1, 0], "launcher": { "name": "busy", "mode": "visor" } }
                ]
            }"#,
        )
        .unwrap();
        let launcher_named = |name| {
            configuration
                .launchers()
                .find(|launcher| launcher.name == name)
                .unwrap()
                .id
        };
        let (empty, busy) = (launcher_named("empty"), launcher_named("busy"));
        let mut hierarchy = OrderedHierarchy::default();
        hierarchy
            .add(
                DesktopTarget::Launcher(busy),
                DesktopTarget::Instance(Uuid::new_v4().into(), InstancePart::Instance),
            )
            .unwrap();
        let shared_column_counts = SharedColumnCounts::new(&configuration);
        let context = LayoutContext {
            hierarchy: &hierarchy,
            configuration: &configuration,
            shared_column_counts: &shared_column_counts,
            default_panel_size: SizePx::new(800, 600),
            window_size: SizePx::new(800, 600),
            focused_instance: None,
            title_bar_metrics: InstanceTitleBarMetrics::from_scale_factor(1.0),
        };
        let instance_extent: LayoutSize<2> =
            instance_extent(context.default_panel_size, context.title_bar_metrics.height).into();
        assert!(
            instance_extent.0[1] > 600,
            "the extent includes the title bar"
        );

        let empty = measure_launcher(empty, &context, &[]);
        assert_eq!(empty.size, instance_extent);
        assert_eq!(empty.expandable_axes, [false, true]);

        let busy = measure_launcher(busy, &context, &[instance_extent.into()]);
        assert_eq!(busy.size, instance_extent);
        assert_eq!(busy.expandable_axes, [false, false]);
    }
}
