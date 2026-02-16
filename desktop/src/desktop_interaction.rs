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

use crate::event_router;
use crate::instance_manager::InstanceManager;
use crate::navigation::NavigationHitTester;
use crate::{
    desktop_presenter::{DesktopFocusPath, DesktopPresenter, DesktopTarget},
    desktop_system::DesktopSystem,
};
use crate::{
    desktop_system::{Cmd, DesktopCommand},
    event_router::EventTransitions,
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
        initial_focus: DesktopFocusPath,
        instance_manager: &InstanceManager,
        system: &mut DesktopSystem,
        scene: &Scene,
    ) -> Result<Self> {
        let mut event_router = EventRouter::default();
        let initial_transitions = event_router.focus(initial_focus);
        assert!(
            system
                .forward_event_transitions(initial_transitions.transitions, instance_manager)?
                .is_none()
        );

        // We can't call apply_changes yet as it needs a mutable presenter reference
        // which we don't have. The transitions will be applied later.

        let camera = system
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

    // pub fn camera(&self) -> PixelCamera {
    //     self.camera.value()
    // }

    pub fn focus(&mut self, focus_path: DesktopFocusPath) -> EventTransitions<DesktopTarget> {
        self.event_router.focus(focus_path)
    }

    pub fn set_camera(&mut self, camera: PixelCamera) {
        self.camera.animate_if_changed(
            camera,
            crate::desktop_presenter::STRUCTURAL_ANIMATION_DURATION,
            Interpolation::CubicOut,
        );
    }

    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        system: &mut DesktopSystem,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        // Create a hit tester and forward events.

        let transitions = {
            self.event_router.process(
                event,
                &NavigationHitTester::new(system.navigation(), render_geometry),
            )?
        };
        let intent = system.forward_event_transitions(transitions.transitions, instance_manager)?;

        if let Some(focus) = transitions.keyboard_focus_changed {
            let intent = self.focus(focus, instance_manager, system)?;
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
        system: &mut DesktopSystem,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        let transitions = self
            .event_router
            .reset_pointer_focus(&NavigationHitTester::new(
                presenter.navigation(),
                render_geometry,
            ))?;

        assert!(transitions.keyboard_focus_changed.is_none());

        system.forward_event_transitions(transitions.transitions, instance_manager)
    }
}
