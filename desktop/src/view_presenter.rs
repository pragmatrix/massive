//! The desktop's state of a view its instance presents. It lives inside the instance's
//! `Presenting` state, so nothing kept about a view outlives the view.

use std::collections::HashMap;

use winit::event::ElementState;
use winit::keyboard::PhysicalKey;

use massive_applications::{KeyEvent, ViewCreationInfo, ViewEvent, ViewId};
use massive_geometry::SizePx;

use crate::instance_presenter::ViewWindowState;

#[derive(Debug)]
pub(crate) struct ViewPresenter {
    pub creation_info: ViewCreationInfo,
    pub window_state: ViewWindowState,
    pub view_size: SizePx,
    held_keys: HeldKeys,
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

/// A held key's release, delivered when the view loses the focus before the key comes up.
fn synthesized_release(press: ViewEvent) -> ViewEvent {
    let ViewEvent::KeyboardInput {
        device_id, event, ..
    } = press
    else {
        panic!("A held key is not a key event: {press:?}");
    };
    ViewEvent::KeyboardInput {
        device_id,
        event: KeyEvent {
            state: ElementState::Released,
            repeat: false,
            text: None,
            ..event
        },
        is_synthetic: true,
    }
}

/// The presses a view received and has not seen released yet.
#[derive(Debug, Default)]
struct HeldKeys(HashMap<PhysicalKey, ViewEvent>);

impl HeldKeys {
    /// A repeat replaces the press, so the release is synthesized from the latest one.
    fn pressed(&mut self, key: PhysicalKey, press: ViewEvent) {
        self.0.insert(key, press);
    }

    /// Whether the view received the press this release belongs to.
    fn released(&mut self, key: PhysicalKey) -> bool {
        self.0.remove(&key).is_some()
    }

    /// The view loses the focus, so the releases of its held keys are due now.
    fn drain(&mut self) -> Vec<ViewEvent> {
        self.0.drain().map(|(_, press)| press).collect()
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;
    use winit::event::DeviceId;
    use winit::keyboard::{Key, KeyCode, KeyLocation};

    use massive_applications::ViewRole;
    use massive_geometry::BoxPx;

    use super::*;

    const A: PhysicalKey = PhysicalKey::Code(KeyCode::KeyA);
    const T: PhysicalKey = PhysicalKey::Code(KeyCode::KeyT);

    fn presenter() -> ViewPresenter {
        ViewPresenter::new(&ViewCreationInfo {
            id: Uuid::new_v4().into(),
            role: ViewRole::Primary,
            extents: BoxPx::from_size((100, 100).into()),
        })
    }

    fn key(physical_key: PhysicalKey, state: ElementState) -> ViewEvent {
        ViewEvent::KeyboardInput {
            device_id: DeviceId::dummy(),
            event: KeyEvent {
                physical_key,
                logical_key: Key::Character("a".into()),
                text: None,
                location: KeyLocation::Standard,
                state,
                repeat: false,
                key_without_modifiers: None,
            },
            is_synthetic: false,
        }
    }

    #[test]
    fn a_release_without_its_press_is_dropped() {
        let mut view = presenter();
        // The desktop consumed the press of Cmd+T.
        assert!(
            view.paired_events(key(T, ElementState::Released))
                .is_empty()
        );

        view.paired_events(key(A, ElementState::Pressed));
        assert_eq!(view.paired_events(key(A, ElementState::Released)).len(), 1);
        assert!(
            view.paired_events(key(A, ElementState::Released))
                .is_empty()
        );
    }

    #[test]
    fn losing_the_focus_releases_the_held_keys_first() {
        let mut view = presenter();
        view.paired_events(key(A, ElementState::Pressed));

        let events = view.paired_events(ViewEvent::Focused(false));
        let [
            ViewEvent::KeyboardInput {
                event: release,
                is_synthetic: true,
                ..
            },
            ViewEvent::Focused(false),
        ] = events.as_slice()
        else {
            panic!("Expected the release before the focus loss: {events:?}");
        };
        assert_eq!(release.physical_key, A);
        assert_eq!(release.state, ElementState::Released);
        // The physical release arrives after the focus moved on.
        assert!(
            view.paired_events(key(A, ElementState::Released))
                .is_empty()
        );
    }
}
