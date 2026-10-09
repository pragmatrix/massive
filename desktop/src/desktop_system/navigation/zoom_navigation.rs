//! Zoom planning (ADR 0018): steps the zoom level for `Zoom` commands, moves focus where a step
//! crosses a project boundary, and answers what the camera currently points at.

use super::focus_targets::FocusTargets;
use super::zoom_geometry::ZoomGeometry;
use crate::desktop_system::change::{Changes, DesktopChange, Zoom, set_focus};
use crate::desktop_system::{DesktopTarget, KeyboardFocusReason, ZoomLevel};
use crate::projects::ProjectId;

/// Plans zoom commands and resolves the zoom camera for the keyboard-focused target (ADR 0018).
#[derive(Debug)]
pub struct ZoomNavigation<'a> {
    geometry: ZoomGeometry<'a>,
    focus_targets: FocusTargets<'a>,
    focused: Option<&'a DesktopTarget>,
    zoom_level: ZoomLevel,
}

impl<'a> ZoomNavigation<'a> {
    pub fn new(
        geometry: ZoomGeometry<'a>,
        focus_targets: FocusTargets<'a>,
        focused: Option<&'a DesktopTarget>,
        zoom_level: ZoomLevel,
    ) -> Self {
        Self {
            geometry,
            focus_targets,
            focused,
            zoom_level,
        }
    }

    /// Plans a zoom command (ADR 0018): steps the zoom level, and moves focus where the step
    /// crosses a project boundary.
    pub fn plan_zoom(&self, zoom: Zoom) -> Changes {
        let Some(focused) = self.focused else {
            return Changes::Empty;
        };
        let instance_target = focused.is_instance_target();
        let project = self.geometry.project_of(focused);
        // A target without a slot — the root project — has only one frame.
        let in_slot = self.geometry.slot_in_project(project, focused).is_some();

        match zoom {
            Zoom::Out if !in_slot => Changes::Empty,
            Zoom::Out => {
                let parent = self.geometry.parent_project_of(project);
                match (self.zoom_level.zoom_out(instance_target), parent) {
                    // Leaving a nested project focuses it as a slot of its parent.
                    (Some(ZoomLevel::Project) | None, Some(_)) => {
                        self.focus_at_zoom_level(project, ZoomLevel::Focus)
                    }
                    (Some(level), _) => self.set_zoom_level(level),
                    // The root project is the outermost frame.
                    (None, None) => Changes::Empty,
                }
            }
            Zoom::In => match self.zoom_level.zoom_in(instance_target).filter(|_| in_slot) {
                Some(level) => self.set_zoom_level(level),
                None => match focused.stands_for_project() {
                    Some(entered) => self.plan_enter_project_slot(entered),
                    None => Changes::Empty,
                },
            },
            Zoom::Enter => {
                let target = match focused.stands_for_project() {
                    Some(entered) => self.focus_targets.focus_target_for_project(entered),
                    None => focused.clone(),
                };
                self.focus_at_zoom_level(target, ZoomLevel::Focus)
            }
        }
    }

    /// Whether the camera points at the focused target itself at the zoom level.
    pub fn is_fully_zoomed_in(&self) -> bool {
        self.focused.is_some_and(|focused| {
            self.zoom_level.normalized(focused.is_instance_target()) == ZoomLevel::Focus
        })
    }

    /// What the camera points at and the nesting depth of the zoom project (root = 0).
    ///
    /// `Focus` on a target other than an instance points at its slot and reads as `Slot`; a target
    /// without a slot points at its project.
    pub fn focused_zoom_level(&self) -> Option<(ZoomLevel, usize)> {
        self.focused.map(|focused| {
            let project = self.geometry.project_of(focused);
            let level = match self.zoom_level.normalized(focused.is_instance_target()) {
                _ if self.geometry.slot_in_project(project, focused).is_none() => {
                    ZoomLevel::Project
                }
                ZoomLevel::Focus if !focused.is_instance_target() => ZoomLevel::Slot,
                level => level,
            };
            (level, self.geometry.project_nesting_depth(project))
        })
    }

    /// Zooms into `project` by one level: focuses its focus slot, else its first slot, at `Row`.
    fn plan_enter_project_slot(&self, project: ProjectId) -> Changes {
        let Some(content) = self.focus_targets.entry_content(project) else {
            return Changes::Empty;
        };
        // Enter at Row so one zoom step reveals the project's contents before narrowing to a
        // slot or instance. Restore the last focused slot to preserve the user's position.
        self.focus_at_zoom_level(
            self.focus_targets.slot_focus_target(content.target()),
            ZoomLevel::Row,
        )
    }

    fn focus_at_zoom_level(&self, target: impl Into<DesktopTarget>, level: ZoomLevel) -> Changes {
        let target = target.into();
        let mut changes = Changes::Empty;
        if self.focused != Some(&target) {
            changes += set_focus(Some(target), KeyboardFocusReason::Zoom);
        }
        changes += self.set_zoom_level(level);
        changes
    }

    fn set_zoom_level(&self, level: ZoomLevel) -> Changes {
        if level == self.zoom_level {
            return Changes::Empty;
        }
        DesktopChange::SetZoomLevel(level).into()
    }
}
