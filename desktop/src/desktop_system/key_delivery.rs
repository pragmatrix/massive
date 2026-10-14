//! Key delivery (ADR 0020): a key travels down the keyboard focus path, outermost level first, and
//! the first level that consumes it ends the walk.

use std::fmt;

use anyhow::Result;

use massive_applications::{InstanceId, ViewEvent, ViewId};

use super::change::{Changes, DesktopChange};
use super::{
    Commands, DesktopFocusPath, DesktopSystem, DesktopTarget, FocusableTarget, InputFocus, KeyInput,
};
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::RuntimeConfiguration;
use crate::targeted_event::EventTransition;

/// A level of the keyboard focus path that may consume a key.
pub trait KeyHandler: fmt::Debug {
    fn handle_key(&self, input: &KeyInput, context: &KeyContext<'_>) -> KeyOutcome;
}

#[derive(Debug)]
pub enum KeyOutcome {
    /// The level has no use for the key; it continues to the next level.
    Pass,
    /// The level took the key and asks for these commands.
    Consumed(Commands),
    /// The key reached the application; the level hands it the event.
    Forward(ViewEvent),
}

#[derive(Debug)]
pub struct KeyLevel<'a> {
    pub target: FocusableTarget,
    pub handler: &'a dyn KeyHandler,
}

#[derive(Debug)]
enum KeyDelivery {
    /// The key fell off the end of the path.
    Dropped,
    Consumed(Commands),
    /// The key reached the application behind `view`.
    Forwarded {
        instance: InstanceId,
        view: ViewId,
        event: ViewEvent,
    },
}

impl DesktopSystem {
    /// Delivers a key down the keyboard focus path and plans what the receiving level asks for.
    pub(super) fn deliver_key_along_focus_path(&self, input: &KeyInput) -> Result<Changes> {
        let delivery = {
            let context = KeyContext {
                is_focused: false,
                fully_zoomed_in: self.zoom_navigation().is_fully_zoomed_in(),
                hierarchy: &self.aggregates.hierarchy,
                configuration: &self.aggregates.configuration,
            };
            let levels = self.key_levels(&self.focused_path());
            offer_key_to_levels(&levels, input, context)
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
            KeyDelivery::Forwarded {
                instance,
                view,
                event,
            } => {
                changes <<= DesktopChange::ForwardEvents(
                    EventTransition::Send(DesktopTarget::view(instance, view), event).into(),
                );
                InputFocus::Application(view)
            }
        };

        if input.is_typed() {
            changes <<= DesktopChange::SetInputFocus(input_focus);
        }
        Ok(changes)
    }

    /// The handlers of the keyboard focus path, outermost first.
    pub(super) fn key_levels<'s>(&'s self, focused_path: &DesktopFocusPath) -> Vec<KeyLevel<'s>> {
        let aggregates = &self.aggregates;
        focused_path
            .iter()
            .filter_map(DesktopTarget::as_focusable)
            .map(|target| {
                let handler: &dyn KeyHandler = match target {
                    FocusableTarget::Desktop => &self.desktop_presenter,
                    FocusableTarget::Project(project) => aggregates
                        .projects
                        .get(&project)
                        .expect("A project on the focus path has a presenter"),
                    FocusableTarget::Launcher(launcher) => aggregates
                        .launchers
                        .get(&launcher)
                        .expect("A launcher on the focus path has a presenter"),
                    FocusableTarget::Instance(instance) => aggregates
                        .instances
                        .get(&instance)
                        .expect("An instance on the focus path has a presenter"),
                    FocusableTarget::View(..) => &ApplicationKeys,
                };
                KeyLevel { target, handler }
            })
            .collect()
    }
}

/// Offers the key to `levels`, outermost first, until one consumes it.
fn offer_key_to_levels(
    levels: &[KeyLevel<'_>],
    input: &KeyInput,
    context: KeyContext<'_>,
) -> KeyDelivery {
    for (index, level) in levels.iter().enumerate() {
        // The focus ends the path and is always focusable, so it is the last level.
        let context = KeyContext {
            is_focused: index + 1 == levels.len(),
            ..context
        };
        match level.handler.handle_key(input, &context) {
            KeyOutcome::Pass => {}
            KeyOutcome::Consumed(commands) => return KeyDelivery::Consumed(commands),
            KeyOutcome::Forward(event) => {
                let FocusableTarget::View(instance, view) = level.target else {
                    unreachable!("Only the application's view level forwards a key");
                };
                return KeyDelivery::Forwarded {
                    instance,
                    view,
                    event,
                };
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
#[derive(Debug, Clone, Copy)]
pub struct KeyContext<'a> {
    /// Whether the receiving level is the keyboard focus itself; keys also pass through the levels
    /// above it.
    pub is_focused: bool,
    /// Whether the camera frames the keyboard-focused target itself (zoom level `Focus`). Cmd+Enter
    /// zooms there first and acts only when already there; presenters cannot read the zoom state.
    pub fully_zoomed_in: bool,
    /// Cmd+T on an instance starts the new instance under that instance's launcher.
    pub hierarchy: &'a DesktopTopology,
    /// A launcher starts its instances with the parameters configured for it.
    pub configuration: &'a RuntimeConfiguration,
}
