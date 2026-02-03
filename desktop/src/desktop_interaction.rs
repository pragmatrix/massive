use anyhow::Result;
use winit::{
    event::ElementState,
    keyboard::{Key, NamedKey},
};

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, InstanceParameters, ViewEvent, ViewRole};
use massive_geometry::PixelCamera;
use massive_input::Event;
use massive_renderer::RenderGeometry;
use massive_shell::Scene;

use crate::{
    desktop_presenter::{BandLocation, DesktopFocusPath, DesktopPresenter, DesktopTarget},
    event_router,
    instance_manager::InstanceManager,
    navigation::NavigationHitTester,
};

#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub enum UserIntent {
    // Architecture: Review if `None` is such a good idea here, it almost never is.
    None,
    // Architecture: Could just always Focus an explicit thing?
    Focus(DesktopFocusPath),
    StartInstance { parameters: InstanceParameters },
    StopInstance { instance: InstanceId },
}

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
                == UserIntent::None
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
    ) -> Result<UserIntent> {
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
    ) -> Result<UserIntent> {
        let intent = self.preprocess_keyboard_commands(event)?;
        if intent != UserIntent::None {
            return Ok(intent);
        }

        // Create a hit tester and forward events.

        let transitions = {
            let navigation = presenter.navigation();
            let hit_test = NavigationHitTester::new(navigation, render_geometry);
            self.event_router.process(event, &hit_test)?
        };
        presenter.forward_event_transitions(transitions.transitions, instance_manager)
    }

    fn preprocess_keyboard_commands(&self, event: &Event<ViewEvent>) -> Result<UserIntent> {
        // Catch Command+t and Command+w if a instance has the keyboard focus.

        if let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
            && key_event.state == ElementState::Pressed
            && !key_event.repeat
            && event.states().is_command()
        {
            if let Some(instance) = self.event_router.focused().instance() {
                match &key_event.logical_key {
                    Key::Character(c) if c.as_str() == "t" => {
                        return Ok(UserIntent::StartInstance {
                            parameters: Default::default(),
                        });
                    }
                    Key::Character(c) if c.as_str() == "w" => {
                        return Ok(UserIntent::StopInstance { instance });
                    }
                    _ => {}
                }
            }

            if let Some(parent_focus) = self.event_router.focused().parent()
                && let Key::Named(NamedKey::Escape) = &key_event.logical_key
            {
                return Ok(UserIntent::Focus(parent_focus));
            }
        }

        Ok(UserIntent::None)
    }
}
