use massive_animation::Animated;
use massive_applications::ViewEvent;
use massive_geometry::PixelCamera;
use massive_input::Event;
use massive_renderer::{RenderGeometry};

use super::NavigationNode;
use crate::focus_path::FocusPath;

#[derive(Debug)]
struct Navigator<Target> {
    /// Where the pointer / cursor is pointing to.
    mouse_focus: 
    /// Where the camera is pointing to and the user is meant to interact with.
    focus: FocusPath<Target>,
    /// The current camera, may be animated.
    camera: Animated<PixelCamera>,
}

impl<Target> Navigator<Target> {
    pub fn new_context<'a>(
        &'a mut self,
        geometry: &'a RenderGeometry,
        root: &'a NavigationNode<'a, Target>,
    ) -> NavigationContext<'a, Target> {
        NavigationContext {
            geometry,
            navigator: self,
            root,
        }
    }
}

/// A Navigator bound to a root node.
#[derive(Debug)]
struct NavigationContext<'a, Target> {
    geometry: &'a RenderGeometry,
    navigator: &'a mut Navigator<Target>,
    // Performance: If we traverse the node hierarchy multiple times, a cache would be needed.
    // Later, bounding box computation might be expensive.
    root: &'a NavigationNode<'a, Target>,
}

impl<Target> NavigationContext<'_, Target> {
    pub fn handle_input_event(&mut self, event: &Event<ViewEvent>) {}
}

enum NavigatorEvent {
    HoverChanged(FocusPath<Target>, FocusPath<Target>),
    /// Focus change from a to b (this is direct, no intermediate steps in the navigation hierarchy may be used)
    UserFocusChanged(FocusPath<Target>, FocusPath<Target>),
}
