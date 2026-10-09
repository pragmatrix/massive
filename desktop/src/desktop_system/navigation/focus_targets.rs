//! Focus landing: which target keyboard focus lands on when a slot, launcher or project is
//! entered without a direction (ADR 0018). Shared by zoom and slot removal.

use crate::desktop_system::topology::DesktopTopology;
use crate::desktop_system::{DesktopTarget, LauncherMap, ProjectMap};
use crate::projects::{LaunchProfileId, ProjectId, RuntimeConfiguration, SlotContent};

/// Resolves the focus target a slot, launcher or project lands on.
#[derive(Debug)]
pub struct FocusTargets<'a> {
    hierarchy: &'a DesktopTopology,
    configuration: &'a RuntimeConfiguration,
    projects: &'a ProjectMap,
    launchers: &'a LauncherMap,
}

impl<'a> FocusTargets<'a> {
    pub fn new(
        hierarchy: &'a DesktopTopology,
        configuration: &'a RuntimeConfiguration,
        projects: &'a ProjectMap,
        launchers: &'a LauncherMap,
    ) -> Self {
        Self {
            hierarchy,
            configuration,
            projects,
            launchers,
        }
    }

    /// Lands focus on a slot's content without a direction: a launcher focuses its instance
    /// anchor, else its first instance, else itself; a project is focused as a slot (ADR 0018).
    pub fn slot_focus_target(&self, target: DesktopTarget) -> DesktopTarget {
        match target {
            DesktopTarget::Launcher(launcher) => self.focus_target_for_launcher(launcher),
            target => target,
        }
    }

    /// The leaf a project is entered down to: its focus slots followed through nested projects,
    /// else its first launcher depth first, else the project itself.
    pub fn focus_target_for_project(&self, project: ProjectId) -> DesktopTarget {
        if let Some(content) = self.last_focused_content(project) {
            return self.focus_target_for_slot(content);
        }

        if let Some(launcher) = self.first_launcher_depth_first(project) {
            return self.focus_target_for_launcher(launcher);
        }

        DesktopTarget::Project(project)
    }

    /// The content a zoom into `project` lands on: its focus slot, else its first slot.
    pub fn entry_content(&self, project: ProjectId) -> Option<SlotContent> {
        self.last_focused_content(project).or_else(|| {
            self.configuration
                .first_slot(project)
                .map(|(_, content)| content)
        })
    }

    fn focus_target_for_launcher(&self, launcher: LaunchProfileId) -> DesktopTarget {
        let hierarchy = self.hierarchy;
        let target = self
            .launchers
            .get(&launcher)
            .and_then(|presenter| presenter.focus_anchor_instance)
            .filter(|instance| {
                hierarchy.parent(&DesktopTarget::Instance(*instance))
                    == Some(&DesktopTarget::Launcher(launcher))
            })
            .or_else(|| hierarchy.launcher_instances(launcher).next())
            .map(DesktopTarget::Instance)
            .unwrap_or(DesktopTarget::Launcher(launcher));
        hierarchy.resolve_keyboard_focus_target(&target)
    }

    fn last_focused_content(&self, project: ProjectId) -> Option<SlotContent> {
        let placement = self.projects.get(&project)?.last_focused_placement?;
        self.configuration.content_at(project, placement)
    }

    fn focus_target_for_slot(&self, content: SlotContent) -> DesktopTarget {
        match content {
            SlotContent::Launcher(launcher) => self.focus_target_for_launcher(launcher),
            SlotContent::Project(project) => self.focus_target_for_project(project),
        }
    }

    fn first_launcher_depth_first(&self, project: ProjectId) -> Option<LaunchProfileId> {
        for (_, content) in self.configuration.slots_ordered(project) {
            match content {
                SlotContent::Launcher(launcher) => return Some(launcher),
                SlotContent::Project(nested) => {
                    if let Some(launcher) = self.first_launcher_depth_first(nested) {
                        return Some(launcher);
                    }
                }
            }
        }
        None
    }
}
