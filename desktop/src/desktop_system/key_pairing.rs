//! Views receive key presses and releases in pairs.
//!
//! The desktop consumes the presses of its shortcuts but not their releases, and the keyboard focus
//! may move while a key is held. Applications that track held keys (the Kitty keyboard protocol
//! reports releases) would otherwise see releases without presses, or keys that never come up.

use std::collections::HashMap;
use std::hash::Hash;

use winit::keyboard::PhysicalKey;

#[derive(Debug)]
pub(crate) struct KeyPairing<V, E> {
    /// The presses each view received and has not seen released yet.
    held: HashMap<V, HashMap<PhysicalKey, E>>,
}

impl<V, E> Default for KeyPairing<V, E> {
    fn default() -> Self {
        Self {
            held: HashMap::new(),
        }
    }
}

impl<V: Eq + Hash, E> KeyPairing<V, E> {
    /// A repeat replaces the press, so the release is synthesized from the latest one.
    pub fn pressed(&mut self, view: V, key: PhysicalKey, press: E) {
        self.held.entry(view).or_default().insert(key, press);
    }

    /// Whether the view received the press this release belongs to.
    pub fn released(&mut self, view: &V, key: PhysicalKey) -> bool {
        let Some(keys) = self.held.get_mut(view) else {
            return false;
        };
        let was_held = keys.remove(&key).is_some();
        if keys.is_empty() {
            self.held.remove(view);
        }
        was_held
    }

    /// The presses the view still holds while it loses the focus; their releases are due now,
    /// because the view won't receive them later.
    pub fn unfocused(&mut self, view: &V) -> Vec<E> {
        self.held
            .remove(view)
            .map(|keys| keys.into_values().collect())
            .unwrap_or_default()
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
        let mut pairing = KeyPairing::<u32, &str>::default();
        // The desktop consumed the press of Cmd+T.
        assert!(!pairing.released(&1, T));

        pairing.pressed(1, A, "a");
        // The press went to view 1; view 2 got the focus meanwhile.
        assert!(!pairing.released(&2, A));
        assert!(pairing.released(&1, A));
        assert!(!pairing.released(&1, A));
    }

    #[test]
    fn losing_the_focus_releases_the_held_keys() {
        let mut pairing = KeyPairing::<u32, &str>::default();
        pairing.pressed(1, A, "a");

        assert_eq!(pairing.unfocused(&1), vec!["a"]);
        // The physical release arrives after the focus moved on.
        assert!(!pairing.released(&1, A));
    }
}
