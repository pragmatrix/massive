//! Key delivery (ADR 0020): a key travels down the keyboard focus path, outermost level first, and
//! the first level that consumes it ends the walk.

use std::fmt;

use anyhow::Result;
use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState};

use massive_applications::ViewEvent;
use massive_input::Event;

use super::change::Changes;
use super::{Commands, DesktopFocusPath, DesktopSystem, DesktopTarget};
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::RuntimeConfiguration;

/// A level of the keyboard focus path that may consume a key.
pub(crate) trait KeyHandler: fmt::Debug {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome;
}

/// A key event reduced to what key handlers decide on, plus the event to hand to an application.
#[derive(Debug)]
pub(crate) struct KeyInput {
    pub logical_key: Key,
    pub state: ElementState,
    pub repeat: bool,
    pub modifiers: ModifiersState,
    view_event: ViewEvent,
}

impl KeyInput {
    #[cfg(test)]
    pub fn new(
        logical_key: Key,
        state: ElementState,
        repeat: bool,
        modifiers: ModifiersState,
    ) -> Self {
        Self {
            logical_key,
            state,
            repeat,
            modifiers,
            // Handlers in tests never forward it.
            view_event: ViewEvent::Focused(true),
        }
    }

    /// `None` if `event` is not a key event.
    pub fn from_event(event: &Event<'_, ViewEvent>) -> Option<Self> {
        let ViewEvent::KeyboardInput {
            event: key_event, ..
        } = event.event()
        else {
            return None;
        };

        Some(Self {
            logical_key: key_event.logical_key.clone(),
            state: key_event.state,
            repeat: key_event.repeat,
            modifiers: event.keyboard_modifiers(),
            view_event: event.event().clone(),
        })
    }

    pub fn is_pressed(&self) -> bool {
        self.state == ElementState::Pressed
    }

    /// `true` if the Windows key on Windows or the Command key on a Mac is pressed.
    pub fn is_command(&self) -> bool {
        self.modifiers.super_key()
    }

    pub fn is_shift(&self) -> bool {
        self.modifiers.shift_key()
    }

    pub fn is_ctrl(&self) -> bool {
        self.modifiers.control_key()
    }

    /// The event as an application receives it.
    pub fn view_event(&self) -> ViewEvent {
        self.view_event.clone()
    }
}

#[derive(Debug)]
pub(crate) enum KeyOutcome {
    /// The level has no use for the key; it continues to the next level.
    Pass,
    /// The level took the key and asks for these commands.
    Consumed(Commands),
}

#[derive(Debug)]
pub(crate) struct KeyLevel<'a> {
    pub target: DesktopTarget,
    pub handler: &'a dyn KeyHandler,
}

#[derive(Debug)]
struct KeyDelivery {
    /// The level that consumed the key, `None` if it fell off the end of the path.
    consumer: Option<DesktopTarget>,
    commands: Commands,
}

impl DesktopSystem {
    /// Delivers a key down the keyboard focus path and plans what the receiving level asks for.
    pub(super) fn deliver_key_along_focus_path(&self, input: &KeyInput) -> Result<Changes> {
        let delivery = {
            let context = KeyContext {
                focused_path: self.focused_path(),
                fully_zoomed_in: self.zoom_navigation().is_fully_zoomed_in(),
                hierarchy: &self.aggregates.hierarchy,
                configuration: &self.aggregates.configuration,
            };
            let levels = self.key_levels(&context.focused_path);
            deliver_key(&levels, input, &context)
        };

        let mut changes = Changes::Empty;
        for command in delivery.commands {
            changes += self.plan(command)?;
        }
        Ok(changes)
    }

    /// The handlers of the keyboard focus path, outermost first.
    ///
    /// Structural targets stand for the level that hosts them: a project's header and matrix are
    /// its project, and an instance's title bar and view are its instance.
    pub(super) fn key_levels<'s>(&'s self, focused_path: &DesktopFocusPath) -> Vec<KeyLevel<'s>> {
        let aggregates = &self.aggregates;
        focused_path
            .iter()
            .filter_map(|target| {
                let handler: &dyn KeyHandler = match target {
                    DesktopTarget::Desktop => &self.desktop_presenter,
                    DesktopTarget::Project(project) => aggregates
                        .projects
                        .get(project)
                        .expect("A project on the focus path has a presenter"),
                    DesktopTarget::Launcher(launcher) => aggregates
                        .launchers
                        .get(launcher)
                        .expect("A launcher on the focus path has a presenter"),
                    DesktopTarget::Instance(instance) => aggregates
                        .instances
                        .get(instance)
                        .expect("An instance on the focus path has a presenter"),
                    DesktopTarget::ProjectHeader(_)
                    | DesktopTarget::ProjectMatrix(_)
                    | DesktopTarget::InstanceTitleBar(_)
                    | DesktopTarget::View(_) => return None,
                };
                Some(KeyLevel {
                    target: target.clone(),
                    handler,
                })
            })
            .collect()
    }
}

/// Offers the key to `levels`, outermost first, until one consumes it.
fn deliver_key(levels: &[KeyLevel<'_>], input: &KeyInput, context: &KeyContext<'_>) -> KeyDelivery {
    for level in levels {
        match level.handler.handle_key(input, context) {
            KeyOutcome::Pass => {}
            KeyOutcome::Consumed(commands) => {
                return KeyDelivery {
                    consumer: Some(level.target.clone()),
                    commands,
                };
            }
        }
    }

    KeyDelivery {
        consumer: None,
        commands: Commands::Empty,
    }
}

/// What the desktop's key handlers read while deciding.
#[derive(Debug)]
pub(crate) struct KeyContext<'a> {
    /// The keyboard focus path. A level checks whether the focus is on itself, because keys also
    /// pass through it.
    pub focused_path: DesktopFocusPath,
    /// Whether the camera frames the keyboard-focused target itself (zoom level `Focus`). Cmd+Enter
    /// zooms to it first and starts an instance from a launcher only when already there.
    pub fully_zoomed_in: bool,
    /// Cmd+T on an instance starts the new instance under the launcher of the focused instance.
    pub hierarchy: &'a DesktopTopology,
    /// A launcher starts its instances with the parameters configured for it.
    pub configuration: &'a RuntimeConfiguration,
}

impl KeyContext<'_> {
    pub fn focused(&self) -> Option<&DesktopTarget> {
        self.focused_path.focused()
    }
}
