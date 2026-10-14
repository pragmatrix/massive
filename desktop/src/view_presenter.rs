//! The desktop's state of a view its instance presents. It lives inside the instance's
//! `Presenting` state, so nothing kept about a view outlives the view.

use std::collections::HashMap;

use winit::event::ElementState;
use winit::keyboard::PhysicalKey;

use massive_applications::{ViewCreationInfo, ViewEvent, ViewId};
use massive_geometry::SizePx;

use crate::instance_presenter::ViewWindowState;

#[derive(Debug)]
pub(crate) struct ViewPresenter {
    pub creation_info: ViewCreationInfo,
    pub window_state: ViewWindowState,
    pub view_size: SizePx,
    held_keys: HeldKeys<ViewEvent>,
}

impl ViewPresenter {
    pub fn new(creation_info: &ViewCreationInfo) -> Self {
        Self {
            creation_info: creation_info.clone(),
            window_state: ViewWindowState::default(),
            view_size: creation_info.size(),
            held_keys: HeldKeys::default(),
        }
    }

    pub fn id(&self) -> ViewId {
        self.creation_info.id
    }

    /// The events the view receives for `event`, so its key presses and releases come in pairs:
    /// the desktop consumes the presses of its shortcuts, and the focus may move while a key is
    /// held.
    pub fn paired_events(&mut self, event: ViewEvent) -> Vec<ViewEvent> {
        match &event {
            ViewEvent::KeyboardInput { event: key, .. } => {
                let physical_key = key.physical_key;
                match key.state {
                    ElementState::Pressed => {
                        self.held_keys.pressed(physical_key, event.clone());
                        vec![event]
                    }
                    ElementState::Released => {
                        if self.held_keys.released(physical_key) {
                            vec![event]
                        } else {
                            Vec::new()
                        }
                    }
                }
            }
            ViewEvent::Focused(false) => {
                let mut events: Vec<ViewEvent> = self
                    .held_keys
                    .drain()
                    .into_iter()
                    .map(synthesized_release)
                    .collect();
                events.push(event);
                events
            }
            _ => vec![event],
        }
    }
}

/// winit's `KeyEvent` can't be constructed outside winit, so the release is the press turned around.
fn synthesized_release(press: ViewEvent) -> ViewEvent {
    let ViewEvent::KeyboardInput {
        device_id,
        mut event,
        ..
    } = press
    else {
        panic!("A held key is not a key event: {press:?}");
    };
    event.state = ElementState::Released;
    event.repeat = false;
    event.text = None;
    ViewEvent::KeyboardInput {
        device_id,
        event,
        is_synthetic: true,
    }
}

/// The presses a view received and has not seen released yet; generic over the press, because
/// winit's `KeyEvent` can't be built in tests.
#[derive(Debug)]
struct HeldKeys<E>(HashMap<PhysicalKey, E>);

impl<E> Default for HeldKeys<E> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<E> HeldKeys<E> {
    /// A repeat replaces the press, so the release is synthesized from the latest one.
    fn pressed(&mut self, key: PhysicalKey, press: E) {
        self.0.insert(key, press);
    }

    /// Whether the view received the press this release belongs to.
    fn released(&mut self, key: PhysicalKey) -> bool {
        self.0.remove(&key).is_some()
    }

    /// The view loses the focus, so the releases of its held keys are due now.
    fn drain(&mut self) -> Vec<E> {
        self.0.drain().map(|(_, press)| press).collect()
    }
}

#[cfg(test)]
mod tests {
    use winit::keyboard::KeyCode;

    use super::*;

    const A: PhysicalKey = PhysicalKey::Code(KeyCode::KeyA);
    const T: PhysicalKey = PhysicalKey::Code(KeyCode::KeyT);

    #[test]
    fn a_release_without_its_press_is_dropped() {
        let mut held = HeldKeys::<&str>::default();
        // The desktop consumed the press of Cmd+T.
        assert!(!held.released(T));

        held.pressed(A, "a");
        assert!(held.released(A));
        assert!(!held.released(A));
    }

    #[test]
    fn losing_the_focus_releases_the_held_keys() {
        let mut held = HeldKeys::<&str>::default();
        held.pressed(A, "a");

        assert_eq!(held.drain(), vec!["a"]);
        // The physical release arrives after the focus moved on.
        assert!(!held.released(A));
    }
}
