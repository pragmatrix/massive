use anyhow::Result;
use winit::{
    event::ElementState,
    keyboard::{Key, NamedKey},
};

use massive_animation::{Animated, Interpolation};
use massive_applications::ViewEvent;
use massive_geometry::PixelCamera;
use massive_input::Event;
use massive_renderer::RenderGeometry;
use massive_shell::Scene;

use crate::desktop_presenter::{DesktopFocusPath, DesktopPresenter, DesktopTarget};
use crate::desktop_system::{Cmd, DesktopCommand};
use crate::event_router;
use crate::instance_manager::InstanceManager;
use crate::navigation::NavigationHitTester;

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
        initial_focus: DesktopFocusPath,
        instance_manager: &InstanceManager,
        presenter: &mut DesktopPresenter,
        scene: &Scene,
    ) -> Result<Self> {
        let mut event_router = EventRouter::default();
        let initial_transitions = event_router.focus(initial_focus);
        assert!(
            presenter
                .forward_event_transitions(initial_transitions.transitions, instance_manager)?
                .is_none()
        );

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

    pub fn focused(&self) -> &DesktopFocusPath {
        self.event_router.focused()
    }

    pub fn camera(&self) -> PixelCamera {
        self.camera.value()
    }

    pub fn focus(
        &mut self,
        focus_path: DesktopFocusPath,
        instance_manager: &InstanceManager,
        presenter: &mut DesktopPresenter,
    ) -> Result<Cmd> {
        let transitions = self.event_router.focus(focus_path);
        let user_intent =
            presenter.forward_event_transitions(transitions.transitions, instance_manager)?;

        let camera = presenter.camera_for_focus(self.event_router.focused());
        if let Some(camera) = camera {
            self.camera.animate_if_changed(
                camera,
                crate::desktop_presenter::STRUCTURAL_ANIMATION_DURATION,
                Interpolation::CubicOut,
            );
        }

        Ok(user_intent)
    }

    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        presenter: &mut DesktopPresenter,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let intent = self.preprocess_keyboard_commands(event)?;
        if !intent.is_none() {
            return Ok(intent);
        }

        // Create a hit tester and forward events.

        let transitions = {
            self.event_router.process(
                event,
                &NavigationHitTester::new(presenter.navigation(), render_geometry),
            )?
        };
        let intent =
            presenter.forward_event_transitions(transitions.transitions, instance_manager)?;

        if let Some(focus) = transitions.keyboard_focus_changed {
            let intent = self.focus(focus, instance_manager, presenter)?;
            assert!(intent.is_none());
        }

        Ok(intent)
    }

    /// Refocus the pointer at its current position.
    ///
    /// This is needed when navigation nodes are removed.
    pub fn refocus_pointer(
        &mut self,
        instance_manager: &InstanceManager,
        presenter: &mut DesktopPresenter,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let transitions = self
            .event_router
            .reset_pointer_focus(&NavigationHitTester::new(
                presenter.navigation(),
                render_geometry,
            ))?;

        assert!(transitions.keyboard_focus_changed.is_none());

        presenter.forward_event_transitions(transitions.transitions, instance_manager)
    }

    fn preprocess_keyboard_commands(&self, event: &Event<ViewEvent>) -> Result<Cmd> {
        // Catch CMD+t and CMD+w if an instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.device_states().is_command()
        {
            if let Some(instance) = self.event_router.focused().instance() {
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Ok(DesktopCommand::StartInstance {
                            parameters: Default::default(),
                        }
                        .into());
                    }
                    Key::Character(c) if c.as_str() == "w" => {
                        // Architecture: Shouldn't this just end the current view, and let the
                        // instance decide then?
                        return Ok(DesktopCommand::StopInstance(instance).into());
                    }
                    _ => {}
                }
            }

            if let Some(parent_focus) = self.event_router.focused().parent()
                && let Key::Named(NamedKey::Escape) = &key_event.logical_key
            {
                return Ok(DesktopCommand::Focus(parent_focus).into());
            }
        }

        Ok(Cmd::None)
    }
}
