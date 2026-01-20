use std::cmp::Ordering;

use anyhow::Result;
use derive_more::From;
use winit::{event::ElementState, keyboard::Key};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewEvent, ViewId, ViewRole};
use massive_geometry::{PixelCamera, Point, Vector3, VectorPx};
use massive_input::Event;
use massive_renderer::RenderGeometry;
use massive_shell::Scene;

use crate::{
    EventTransition, HitTester,
    desktop_presenter::DesktopPresenter,
    event_router, focus_path,
    instance_manager::{InstanceManager, ViewPath},
};

// Naming: Should probably get another, just Path or TargetPath / EventPath / RoutingPath?
type FocusPath = focus_path::FocusPath<FocusTarget>;
type EventRouter = event_router::EventRouter<FocusTarget>;

#[derive(Debug)]
pub struct DesktopInteraction {
    event_router: EventRouter,
    camera: Animated<PixelCamera>,
}

// Architecture: Every function here needs &InstanceManager.
impl DesktopInteraction {
    // Detail: We need a primary instance and view to initialize the UI for now.
    //
    // Detail: This function assumes that the window is focused right now.
    pub fn new(
        view_path: ViewPath,
        instance_manager: &InstanceManager,
        presenter: &DesktopPresenter,
        scene: &Scene,
    ) -> Result<Self> {
        let mut event_router = EventRouter::default();
        let initial_transitions = event_router.focus(view_path.into());

        apply_changes(initial_transitions.transitions, instance_manager)?;

        let camera =
            camera(event_router.focused(), presenter).expect("Internal error: No initial focus");

        Ok(Self {
            event_router,
            camera: scene.animated(camera),
        })
    }

    pub fn focused_instance(&self) -> Option<InstanceId> {
        self.event_router.focused().instance()
    }

    pub fn camera(&self) -> PixelCamera {
        self.camera.value()
    }

    pub fn make_foreground(
        &mut self,
        instance: InstanceId,
        instance_manager: &InstanceManager,
        presenter: &DesktopPresenter,
    ) -> Result<()> {
        // If the window is not focus, we just focus the instance.
        let primary_view = instance_manager.get_view_by_role(instance, ViewRole::Primary)?;
        let focus_path: FocusPath = (instance, primary_view).into();

        let transitions = self.event_router.focus(focus_path);
        apply_changes(transitions.transitions, instance_manager)?;

        let camera = camera(self.event_router.focused(), presenter);
        if let Some(camera) = camera {
            self.camera.animate_if_changed(
                camera,
                DesktopPresenter::INSTANCE_TRANSITION_DURATION,
                Interpolation::CubicOut,
            );
        }

        Ok(())
    }

    pub fn handle_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<UiCommand> {
        // Catch Command+t and Command+w

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.states().is_command()
            && let Some(ViewPath { instance, .. }) = self.event_router.focused().view_path()
        {
            match &key_event.logical_key {
                Key::Character(c) if c.as_str() == "t" => {
                    let application = instance_manager.get_application_name(instance)?;
                    return Ok(UiCommand::StartInstance {
                        application: application.to_string(),
                        originating_instance: instance,
                    });
                }
                Key::Character(c) if c.as_str() == "w" => {
                    return Ok(UiCommand::StopInstance { instance });
                }
                _ => {}
            }
        }

        // Create a hit tester and forward events.

        let hit_test = HitTest::from((instance_manager, render_geometry));
        let transitions = self.event_router.handle_event(event, &hit_test)?;

        apply_changes(transitions.transitions, instance_manager)?;

        // Robustness: Currently we don't check if the only the instance actually changed.
        let command = if let Some(new_focus) = transitions.focus_changed
            && let Some(instance) = new_focus.instance()
        {
            UiCommand::MakeForeground { instance }
        } else {
            UiCommand::None
        };

        Ok(command)
    }
}

#[derive(Debug, PartialEq, Clone, From)]
enum FocusTarget {
    Instance(InstanceId),
    View(ViewId),
}

fn apply_changes(
    changes: Vec<EventTransition<FocusTarget>>,
    instance_manager: &InstanceManager,
) -> Result<()> {
    for transition in changes {
        match transition {
            EventTransition::Send(focus_path, view_event) => {
                if let Some(path) = focus_path.view_path() {
                    instance_manager.send_view_event(path, view_event)?;
                }
            }
            EventTransition::Broadcast(view_event) => {
                for (view, _) in instance_manager.views() {
                    instance_manager.send_view_event(view, view_event.clone())?;
                }
            }
        }
    }

    Ok(())
}

/// Returns the camera position it should target at.
fn camera(path: &FocusPath, presenter: &DesktopPresenter) -> Option<PixelCamera> {
    path.instance()
        .and_then(|instance| presenter.instance_transform(instance))
        .map(|target| PixelCamera::look_at(target, None, PixelCamera::DEFAULT_FOVY))
}

//
// Hit testing
//

#[derive(From)]
struct HitTest<'a> {
    instance_manager: &'a InstanceManager,
    render_geometry: &'a RenderGeometry,
}

impl HitTester<FocusTarget> for HitTest<'_> {
    fn hit_test(&self, screen_pos: Point) -> (FocusPath, Vector3) {
        if let Some((view_path, hit)) =
            hit_test_at_point(screen_pos, self.instance_manager, self.render_geometry)
        {
            return (view_path.into(), hit);
        }
        (FocusPath::EMPTY, screen_pos.with_z(0.0))
    }

    fn hit_test_target(&self, screen_pos: Point, target: &FocusPath) -> Option<Vector3> {
        target.view_path().and_then(|view_path| {
            hit_test_on_view(
                screen_pos,
                view_path,
                self.instance_manager,
                self.render_geometry,
            )
        })
    }
}

fn hit_test_at_point(
    screen_pos: Point,
    instance_manager: &InstanceManager,
    geometry: &RenderGeometry,
) -> Option<(ViewPath, Vector3)> {
    let mut hits = Vec::new();

    for (view, view_info) in instance_manager.views() {
        let location = view_info.location.value();
        let transform = location.transform.value();
        let extents = view_info.extents;

        // Feature: Support parent transforms (and cache?)!
        let matrix = transform.to_matrix4();
        if let Some(local_pos) = geometry.unproject_to_model_z0(screen_pos, &matrix) {
            // Robustness: Are we leaving accuracy on the table here by converting from f64 to i32?
            let v: VectorPx = (local_pos.x as i32, local_pos.y as i32).into();
            if extents.contains(v.to_point()) {
                hits.push((view, local_pos));
            }
        }
    }

    // Sort by z (descending) to get topmost view first
    hits.sort_by(|a, b| b.1.z.partial_cmp(&a.1.z).unwrap_or(Ordering::Equal));

    hits.first().copied()
}

/// Specifically hit tests on a view without considering its boundaries.
///
/// Meaning that the hit test result may be out of bounds.
fn hit_test_on_view(
    screen_pos: Point,
    view: ViewPath,
    instance_manager: &InstanceManager,
    render_geometry: &RenderGeometry,
) -> Option<Vector3> {
    let view_info = instance_manager.get_view(view).ok()?;
    let location = view_info.location.value();
    let matrix = location.transform.value().to_matrix4();
    render_geometry.unproject_to_model_z0(screen_pos, &matrix)
}

#[must_use]
pub enum UiCommand {
    None,
    StartInstance {
        application: String,
        originating_instance: InstanceId,
    },
    StopInstance {
        instance: InstanceId,
    },
    MakeForeground {
        instance: InstanceId,
    },
}

impl focus_path::FocusPath<FocusTarget> {
    // Returns the instance that is currently focused.
    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| t.instance())
    }

    pub fn view_path(&self) -> Option<ViewPath> {
        match self.as_slice() {
            [.., FocusTarget::Instance(instance), FocusTarget::View(view)] => {
                Some((*instance, *view).into())
            }
            _ => None,
        }
    }
}

impl From<ViewPath> for FocusPath {
    fn from(view_path: ViewPath) -> Self {
        let (instance, view) = view_path.into();
        vec![instance.into(), view.into()].into()
    }
}

impl From<(InstanceId, Option<ViewId>)> for FocusPath {
    fn from((instance, view): (InstanceId, Option<ViewId>)) -> Self {
        if let Some(view) = view {
            FocusPath::EMPTY.with(instance).with(view)
        } else {
            FocusPath::EMPTY.with(instance)
        }
    }
}

impl FocusTarget {
    fn instance(&self) -> Option<InstanceId> {
        match self {
            FocusTarget::Instance(instance_id) => Some(*instance_id),
            _ => None,
        }
    }
}
