use anyhow::Result;
use winit::{event::ElementState, keyboard::Key};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewEvent};
use massive_geometry::PixelCamera;
use massive_input::Event;
use massive_renderer::RenderGeometry;
use massive_shell::Scene;

use crate::{
    desktop_presenter::{DesktopPath, DesktopPresenter, DesktopTarget},
    event_router,
    instance_manager::InstanceManager,
    navigation::NavigationHitTester,
};

// Naming: Should probably get another, just Path or TargetPath / EventPath / RoutingPath?
type EventRouter = event_router::EventRouter<DesktopTarget>;

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
        path: DesktopPath,
        instance_manager: &InstanceManager,
        presenter: &mut DesktopPresenter,
        scene: &Scene,
    ) -> Result<Self> {
        let mut event_router = EventRouter::default();
        let initial_transitions = event_router.focus(path);
        presenter.forward_event_transitions(initial_transitions.transitions, instance_manager)?;

        // We can't call apply_changes yet as it needs a mutable presenter reference
        // which we don't have. The transitions will be applied later.

        let camera = presenter
            .camera_for_focus(event_router.focused())
            .expect("Internal error: No initial focus");

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
        presenter: &mut DesktopPresenter,
    ) -> Result<()> {
        // If the window is not focus, we just focus the instance.
        // let primary_view = instance_manager.get_view_by_role(instance, ViewRole::Primary)?;
        let focus_path = DesktopPath::from_instance(instance);

        let transitions = self.event_router.focus(focus_path);
        presenter.forward_event_transitions(transitions.transitions, instance_manager)?;

        let camera = presenter.camera_for_focus(self.event_router.focused());
        if let Some(camera) = camera {
            self.camera.animate_if_changed(
                camera,
                crate::desktop_presenter::STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        }

        Ok(())
    }

    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        presenter: &mut DesktopPresenter,
        render_geometry: &RenderGeometry,
    ) -> Result<DesktopCommand> {
        // Catch Command+t and Command+w if a instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.states().is_command()
            && let Some(instance) = self.event_router.focused().instance()
        {
            match &key_event.logical_key {
                Key::Character(c) if c.as_str() == "t" => {
                    let application = instance_manager.get_application_name(instance)?;
                    return Ok(DesktopCommand::StartInstance {
                        application: application.to_string(),
                        originating_instance: instance,
                    });
                }
                Key::Character(c) if c.as_str() == "w" => {
                    return Ok(DesktopCommand::StopInstance { instance });
                }
                _ => {}
            }
        }

        // Create a hit tester and forward events.

        let transitions = {
            let navigation = presenter.navigation();
            let hit_test = NavigationHitTester::new(navigation, render_geometry);
            self.event_router.process(event, &hit_test)?
        };
        presenter.forward_event_transitions(transitions.transitions, instance_manager)?;

        // Robustness: Currently we don't check if the only the instance actually changed.
        let command = if let Some(new_focus) = transitions.focus_changed
            && let Some(instance) = new_focus.instance()
        {
            DesktopCommand::MakeForeground { instance }
        } else {
            DesktopCommand::None
        };

        Ok(command)
    }
}

#[must_use]
pub enum DesktopCommand {
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
