//! Key delivery (ADR 0020): a key travels down the keyboard focus path, outermost level first, and
//! the first level that consumes it ends the walk.

use std::fmt;

use anyhow::Result;

use massive_applications::{ViewEvent, ViewId};

use super::change::{Changes, DesktopChange};
use super::{Commands, DesktopFocusPath, DesktopSystem, DesktopTarget, KeyInput};
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::RuntimeConfiguration;
use crate::targeted_event::EventTransition;

/// A level of the keyboard focus path that may consume a key.
pub(crate) trait KeyHandler: fmt::Debug {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome;
}

#[derive(Debug)]
pub(crate) enum KeyOutcome {
    /// The level has no use for the key; it continues to the next level.
    Pass,
    /// The level took the key and asks for these commands.
    Consumed(Commands),
    /// The key reached the application; the level hands it the event.
    Forward(ViewEvent),
}

#[derive(Debug)]
pub(crate) struct KeyLevel<'a> {
    pub target: DesktopTarget,
    pub handler: &'a dyn KeyHandler,
}

#[derive(Debug)]
enum KeyDelivery {
    /// The key fell off the end of the path.
    Dropped,
    Consumed(Commands),
    /// The key reached the application behind `view`.
    Forwarded {
        view: ViewId,
        event: ViewEvent,
    },
}

/// Where the last typed key went (ADR 0020). The hover outline marks the keyboard focus only while
/// the user is at the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum InputFocus {
    /// A desktop level consumed the key, or it was dropped.
    #[default]
    Desktop,
    Application(ViewId),
}

impl InputFocus {
    /// The keyboard focus can move without a key, so the application only holds the input focus
    /// while it is still the keyboard-focused view.
    pub fn is_application_at(&self, keyboard_focus: &DesktopTarget) -> bool {
        matches!(
            (self, keyboard_focus),
            (Self::Application(view), DesktopTarget::View(focused)) if view == focused
        )
    }
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
            offer_key_to_levels(&levels, input, &context)
        };

        let mut changes = Changes::Empty;
        let input_focus = match delivery {
            KeyDelivery::Dropped => InputFocus::Desktop,
            KeyDelivery::Consumed(commands) => {
                for command in commands {
                    changes += self.plan(command)?;
                }
                InputFocus::Desktop
            }
            KeyDelivery::Forwarded { view, event } => {
                changes <<= DesktopChange::ForwardEvents(
                    EventTransition::Send(DesktopTarget::View(view), event).into(),
                );
                InputFocus::Application(view)
            }
        };

        // Modifiers, releases and repeats don't move the input focus.
        if input.is_typed() {
            changes <<= DesktopChange::SetInputFocus(input_focus);
        }
        Ok(changes)
    }

    /// The handlers of the keyboard focus path, outermost first.
    ///
    /// Structural targets stand for the level that hosts them: a project's header and matrix are
    /// its project, and an instance's title bar is its instance.
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
                    DesktopTarget::View(_) => &ApplicationKeys,
                    DesktopTarget::ProjectHeader(_)
                    | DesktopTarget::ProjectMatrix(_)
                    | DesktopTarget::InstanceTitleBar(_) => return None,
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
fn offer_key_to_levels(
    levels: &[KeyLevel<'_>],
    input: &KeyInput,
    context: &KeyContext<'_>,
) -> KeyDelivery {
    for level in levels {
        match level.handler.handle_key(input, context) {
            KeyOutcome::Pass => {}
            KeyOutcome::Consumed(commands) => return KeyDelivery::Consumed(commands),
            KeyOutcome::Forward(event) => {
                let DesktopTarget::View(view) = level.target else {
                    unreachable!("Only the application's view level forwards a key");
                };
                return KeyDelivery::Forwarded { view, event };
            }
        }
    }

    KeyDelivery::Dropped
}

/// The last level of the path: the application behind a view receives every key that reaches it.
#[derive(Debug)]
struct ApplicationKeys;

impl KeyHandler for ApplicationKeys {
    fn handle_key(&self, input: &KeyInput, _context: &KeyContext<'_>) -> KeyOutcome {
        KeyOutcome::Forward(input.view_event().clone())
    }
}

/// What the desktop's key handlers read while deciding.
#[derive(Debug)]
pub(crate) struct KeyContext<'a> {
    /// The keyboard focus path. A level checks whether the focus is on itself, because keys also
    /// pass through it.
    pub focused_path: DesktopFocusPath,
    /// Whether the camera frames the keyboard-focused target itself (zoom level `Focus`). Cmd+Enter
    /// zooms there first and acts only when already there; presenters cannot read the zoom state.
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
