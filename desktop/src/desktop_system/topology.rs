use log::error;

use massive_applications::InstanceId;

use crate::desktop_system::{InstancePart, ProjectPart};
use crate::projects::{LaunchProfileId, ProjectId, SlotContent};
use crate::{DesktopTarget, OrderedHierarchy};

pub type DesktopTopology = OrderedHierarchy<DesktopTarget>;

impl OrderedHierarchy<DesktopTarget> {
    /// The launcher an instance belongs to, walking up until one is found.
    ///
    /// The only parent an instance is ever given is a launcher, so an instance
    /// that reaches no launcher is either not in the topology or the tree lost
    /// its shape — an invariant violation.
    pub fn launcher_of_instance(&self, instance_id: InstanceId) -> LaunchProfileId {
        self.parent_chain(&DesktopTarget::Instance(
            instance_id,
            InstancePart::Instance,
        ))
        .find_map(|target| match target {
            DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!("instance {instance_id:?} must be nested under a launcher, or not exist")
        })
    }

    /// The instance a target belongs to: `Some` for an instance and its parts, `None` otherwise.
    pub fn instance_of_target(&self, target: &DesktopTarget) -> Option<InstanceId> {
        match target {
            DesktopTarget::Instance(instance_id, _) => Some(*instance_id),
            _ => None,
        }
    }

    /// The launcher a target belongs to, when it belongs to one at all: a
    /// launcher-assigned slot's own target resolves to it, and the instances and
    /// views below it do too. `None` for project and project-descendant targets.
    pub fn launcher_of_target(&self, target: &DesktopTarget) -> Option<LaunchProfileId> {
        self.parent_chain(target).find_map(|target| match target {
            DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
            _ => None,
        })
    }

    /// The slot content a target belongs to: its nearest launcher or nested project. `None` for
    /// the desktop and the root project, which no slot hosts.
    pub fn slot_content_of_target(&self, target: &DesktopTarget) -> Option<SlotContent> {
        self.parent_chain(target).find_map(|target| match target {
            DesktopTarget::Launcher(launcher) => Some(SlotContent::Launcher(*launcher)),
            DesktopTarget::Project(project, _) if *project != ProjectId::ROOT => {
                Some(SlotContent::Project(*project))
            }
            _ => None,
        })
    }

    /// The nearest enclosing project, including a project target itself.
    /// `Desktop` resolves to the root; other targets must belong to a live project.
    pub fn project_of_target(&self, target: &DesktopTarget) -> ProjectId {
        match target {
            DesktopTarget::Desktop => {
                error!("project_of_target reached the Desktop root; resolving to the root project");
                ProjectId::ROOT
            }
            _ => self
                .parent_chain(target)
                .find_map(|target| match target {
                    DesktopTarget::Project(project_id, ProjectPart::Project) => Some(*project_id),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("target {target:?} must belong to a project")),
        }
    }

    /// The project whose matrix hosts `project`, or `None` for the root.
    pub fn parent_project_of(&self, project: ProjectId) -> Option<ProjectId> {
        self.parent_chain(&project.into())
            .find_map(|target| match target {
                DesktopTarget::Project(parent, ProjectPart::Matrix) => Some(*parent),
                _ => None,
            })
    }

    /// How many projects enclose `project`, root = 0: every enclosing project hosts the path in
    /// its matrix, and the root hangs under `Desktop`.
    pub fn project_nesting_depth(&self, project: ProjectId) -> usize {
        self.parent_chain(&project.into())
            .filter(|target| matches!(target, DesktopTarget::Project(_, ProjectPart::Matrix)))
            .count()
    }

    /// Whether an instance runs anywhere below `target`, at any depth. Removing slot content would
    /// quit those instances, so the planner refuses the removal while any run.
    pub fn has_instances_below(&self, target: &DesktopTarget) -> bool {
        let mut pending = vec![target];
        while let Some(current) = pending.pop() {
            for child in self.get_nested(current) {
                if matches!(child, DesktopTarget::Instance(_, InstancePart::Instance)) {
                    return true;
                }
                pending.push(child);
            }
        }
        false
    }

    /// The instances of a launcher, in the launcher's child order.
    pub fn launcher_instances<'a>(
        &'a self,
        launcher_id: LaunchProfileId,
    ) -> impl Iterator<Item = InstanceId> + 'a {
        self.get_nested(&DesktopTarget::Launcher(launcher_id))
            .iter()
            .filter_map(|target| match target {
                DesktopTarget::Instance(instance_id, InstancePart::Instance) => Some(*instance_id),
                _ => None,
            })
    }

    /// The matrix children of `project`, classified as slot contents.
    ///
    /// Slots are implicit — there is no slot node in the topology and no `SlotId`
    /// — so a slot is exactly a matrix-placed child of `ProjectMatrix` that hosts
    /// content. The order is the matrix's child order, which the layout algorithm
    /// zips against its child measurements, so callers that place slots may rely
    /// on it. Slots the matrix has no child for are simply absent.
    ///
    /// The *position* of each slot is configuration data, not topology state; see
    /// `DesktopConfiguration::position_of_content`.
    pub fn matrix_slots(
        &self,
        project: ProjectId,
    ) -> impl Iterator<Item = SlotContent> + Clone + '_ {
        self.get_nested(&DesktopTarget::matrix(project))
            .iter()
            .map(|target| match target {
                DesktopTarget::Launcher(launcher_id) => SlotContent::Launcher(*launcher_id),
                DesktopTarget::Project(project_id, ProjectPart::Project) => {
                    SlotContent::Project(*project_id)
                }
                other => panic!(
                    "project matrix children must be launcher or project targets, found {other:?}"
                ),
            })
    }

    /// The launchers of `project` in matrix child order. Slot contents that are
    /// nested projects are skipped, so the layout's zip with child measurements
    /// uses [`Self::matrix_slots`].
    pub fn matrix_launchers(&self, project: ProjectId) -> impl Iterator<Item = LaunchProfileId> {
        let targets = self.get_nested(&DesktopTarget::matrix(project)).to_vec();
        targets.into_iter().filter_map(|target| match target {
            DesktopTarget::Launcher(launcher_id) => Some(launcher_id),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology_with_matrix(project: ProjectId, slots: &[DesktopTarget]) -> DesktopTopology {
        let mut topology = DesktopTopology::default();
        topology
            .add_nested(
                project.into(),
                [
                    DesktopTarget::header(project),
                    DesktopTarget::matrix(project),
                ],
            )
            .unwrap();
        for slot in slots {
            topology
                .add(DesktopTarget::matrix(project), slot.clone())
                .unwrap();
        }
        topology
    }

    #[test]
    fn matrix_slots_classify_launcher_and_project_content_in_child_order() {
        let project = ProjectId::new();
        let launcher = LaunchProfileId::new();
        let nested = ProjectId::new();
        let topology =
            topology_with_matrix(project, &[DesktopTarget::Launcher(launcher), nested.into()]);

        assert_eq!(
            topology.matrix_slots(project).collect::<Vec<_>>(),
            vec![
                SlotContent::Launcher(launcher),
                SlotContent::Project(nested),
            ]
        );
    }

    #[test]
    fn matrix_launchers_skips_nested_project_slots() {
        let project = ProjectId::new();
        let launcher = LaunchProfileId::new();
        let topology = topology_with_matrix(
            project,
            &[
                DesktopTarget::Project(ProjectId::new(), ProjectPart::Project),
                DesktopTarget::Launcher(launcher),
            ],
        );

        assert_eq!(
            topology.matrix_launchers(project).collect::<Vec<_>>(),
            vec![launcher]
        );
    }

    #[test]
    fn project_of_target_answers_the_nearest_enclosing_project() {
        let root = ProjectId::new();
        let nested = ProjectId::new();
        let launcher = LaunchProfileId::new();
        let mut topology = topology_with_matrix(root, &[nested.into()]);
        topology
            .add_nested(
                nested.into(),
                [DesktopTarget::header(nested), DesktopTarget::matrix(nested)],
            )
            .unwrap();
        topology
            .add(
                DesktopTarget::matrix(nested),
                DesktopTarget::Launcher(launcher),
            )
            .unwrap();

        assert_eq!(
            topology.project_of_target(&DesktopTarget::Launcher(launcher)),
            nested
        );
        assert_eq!(topology.parent_project_of(nested), Some(root));
        assert_eq!(topology.parent_project_of(root), None);
        assert_eq!(topology.project_of_target(&root.into()), root);
    }

    /// The documented exception: `Desktop` goes *down* the hierarchy to its
    /// single child, the root project, instead of up like every other target.
    #[test]
    fn project_of_target_resolves_the_desktop_root_to_the_root_project() {
        let root = ProjectId::ROOT;
        let mut topology = DesktopTopology::default();
        topology.add(DesktopTarget::Desktop, root.into()).unwrap();

        assert_eq!(topology.project_of_target(&DesktopTarget::Desktop), root);
    }

    #[test]
    fn launcher_walks_answer_none_under_a_project_without_a_launcher() {
        let root = ProjectId::new();
        let nested = ProjectId::new();
        let topology = topology_with_matrix(root, &[nested.into()]);

        assert_eq!(topology.launcher_of_target(&nested.into()), None);
        assert_eq!(topology.project_of_target(&nested.into()), nested);
    }
}
