//! The desktop policy decides the desktop's exceptional behaviors (ADR 0029).
//!
//! Each rule is named after its situation and is a pure function of the facts its caller passes in.

use crate::DirectionBias;
use crate::desktop_system::Direction;

/// What Cmd+Shift+W does to a launcher or a nested project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovingSlotContent {
    Remove,
    /// A key press never quits running processes.
    KeepWhileInstancesRun,
}

#[must_use]
pub fn removing_slot_content(instances_run_below: bool) -> RemovingSlotContent {
    if instances_run_below {
        return RemovingSlotContent::KeepWhileInstancesRun;
    }
    RemovingSlotContent::Remove
}

/// What Cmd+Shift+G does (ADR 0026).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ungrouping {
    UngroupFocusedProject,
    /// The project whose slots are all selected.
    UngroupSelectedProject,
    /// The user sees which slots move.
    RefuseUnlessFocusedOrFullySelected,
}

/// An extended selection is deliberate, so when it covers its whole project,
/// that project wins over a focused one.
#[must_use]
pub fn ungrouping(
    focus_on_project: bool,
    selection_covers_project: bool,
    selection_extended: bool,
) -> Ungrouping {
    if selection_covers_project && (selection_extended || !focus_on_project) {
        return Ungrouping::UngroupSelectedProject;
    }
    if focus_on_project {
        return Ungrouping::UngroupFocusedProject;
    }
    Ungrouping::RefuseUnlessFocusedOrFullySelected
}

/// How a nested project is ungrouped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UngroupingProject {
    MoveSlotsOut,
    /// An empty project has no slots to move.
    RemoveProject,
}

#[must_use]
pub fn ungrouping_project(is_empty: bool) -> UngroupingProject {
    if is_empty {
        return UngroupingProject::RemoveProject;
    }
    UngroupingProject::MoveSlotsOut
}

/// Where Ctrl+Cmd+F goes (ADR 0014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TogglingFullScreen {
    /// Enter or leave native window fullscreen.
    Window,
    /// Toggle the Full Screen Mode of the keyboard-focused instance.
    Mode,
}

#[must_use]
pub fn toggling_full_screen(
    window_is_fullscreen: bool,
    fully_in_on_instance: bool,
) -> TogglingFullScreen {
    if window_is_fullscreen && fully_in_on_instance {
        TogglingFullScreen::Mode
    } else {
        TogglingFullScreen::Window
    }
}

/// What Cmd+Enter on an instance or a launcher does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressingCommandEnter {
    Act,
    /// The user sees what the next Cmd+Enter acts on.
    ZoomFirst,
}

#[must_use]
pub fn pressing_command_enter(fully_zoomed_in: bool) -> PressingCommandEnter {
    if fully_zoomed_in {
        return PressingCommandEnter::Act;
    }
    PressingCommandEnter::ZoomFirst
}

/// What the load does with a configuration that defines no launcher (ADR 0012).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingWithoutLauncher {
    /// The session needs a launcher to boot into.
    AddDefaultLauncher,
}

#[must_use]
pub fn loading_without_launcher() -> LoadingWithoutLauncher {
    LoadingWithoutLauncher::AddDefaultLauncher
}

/// Which launcher the session boots into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Booting {
    StartupLauncher,
    /// A renamed or removed startup launcher never stops the boot.
    FirstLauncher,
}

#[must_use]
pub fn booting(startup_resolves: bool) -> Booting {
    if startup_resolves {
        return Booting::StartupLauncher;
    }
    Booting::FirstLauncher
}

/// A candidate for the keyboard focus after the focused instance stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceSuccessor {
    Neighbor(DirectionBias),
    Launcher,
}

/// The candidates in order of preference. The first that exists takes the focus.
#[must_use]
pub fn stopping_focused_instance() -> [InstanceSuccessor; 2] {
    [
        InstanceSuccessor::Neighbor(DirectionBias::Begin),
        InstanceSuccessor::Launcher,
    ]
}

/// A candidate for the keyboard focus after the focused slot is cleared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotSuccessor {
    Neighbor(Direction),
    HostingProject,
}

/// The candidates in order of preference. The first that exists takes the focus.
#[must_use]
pub fn clearing_focused_slot() -> [SlotSuccessor; 5] {
    [
        SlotSuccessor::Neighbor(Direction::Right),
        SlotSuccessor::Neighbor(Direction::Left),
        SlotSuccessor::Neighbor(Direction::Down),
        SlotSuccessor::Neighbor(Direction::Up),
        SlotSuccessor::HostingProject,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extended_selection_over_its_whole_project_outranks_the_focused_project() {
        assert_eq!(
            ungrouping(true, true, false),
            Ungrouping::UngroupFocusedProject
        );
        assert_eq!(
            ungrouping(true, true, true),
            Ungrouping::UngroupSelectedProject
        );
        assert_eq!(
            ungrouping(true, false, true),
            Ungrouping::UngroupFocusedProject
        );
        assert_eq!(
            ungrouping(false, true, false),
            Ungrouping::UngroupSelectedProject
        );
        assert_eq!(
            ungrouping(false, false, true),
            Ungrouping::RefuseUnlessFocusedOrFullySelected
        );
    }
}
