use anyhow::Result;

use massive_applications::ViewEvent;
use massive_input::Event;
use massive_renderer::RenderGeometry;

use super::Id;
use crate::{
    EventRouter,
    event_router::EventTransitions,
    navigation::{NavigationHitTester, NavigationNode},
};

#[derive(Debug, Default)]
pub struct ProjectInteraction {
    event_router: EventRouter<Id>,
}

impl ProjectInteraction {
    pub fn handle_input_event<'a>(
        &'a mut self,
        event: &Event<ViewEvent>,
        navigation: NavigationNode<'a, Id>,
        geometry: &'a RenderGeometry,
    ) -> Result<EventTransitions<Id>> {
        let hit_tester = NavigationHitTester::new(navigation, geometry);
        self.event_router.handle_event(event, &hit_tester)
    }
}
