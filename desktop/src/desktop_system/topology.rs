use log::error;

use massive_applications::InstanceId;
use massive_layout::LayoutTopology;

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
        self.parent_chain(&DesktopTarget::Instance(instance_id))
            .find_map(|target| match target {
                DesktopTarget::Launcher(launcher_id) => Some(*launcher_id),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!("instance {instance_id:?} must be nested under a launcher, or not exist")
            })
    }

    /// The instance a target descends from, walking the parent chain: `Some` for
    /// instance and view targets, `None` for targets that are not instances.
    pub fn instance_of_target(&self, target: &DesktopTarget) -> Option<InstanceId> {
        match target {
            DesktopTarget::Instance(instance_id) => Some(*instance_id),
            _ => self
                .parent_of(target)
                .and_then(|parent| self.instance_of_target(parent)),
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
                    DesktopTarget::Project(project_id) => Some(*project_id),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("target {target:?} must belong to a project")),
        }
    }

    /// The project whose matrix hosts `project`, or `None` for the root.
    pub fn parent_project_of(&self, project: ProjectId) -> Option<ProjectId> {
        self.parent_chain(&DesktopTarget::Project(project))
            .find_map(|target| match target {
                DesktopTarget::ProjectMatrix(parent) => Some(*parent),
                _ => None,
            })
    }

    /// How many projects enclose `project`, root = 0: every enclosing project hosts the path in
    /// its matrix, and the root hangs under `Desktop`.
    pub fn project_nesting_depth(&self, project: ProjectId) -> usize {
        self.parent_chain(&DesktopTarget::Project(project))
            .filter(|target| matches!(target, DesktopTarget::ProjectMatrix(_)))
            .count()
    }

    /// The instances of a launcher, in the launcher's child order.
    pub fn launcher_instances<'a>(
        &'a self,
        launcher_id: LaunchProfileId,
    ) -> impl Iterator<Item = InstanceId> + 'a {
        self.get_nested(&DesktopTarget::Launcher(launcher_id))
            .iter()
            .filter_map(|target| match target {
                DesktopTarget::Instance(instance_id) => Some(*instance_id),
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
    /// The *placement* of each slot is configuration data, not topology state; see
    /// `DesktopConfiguration::placement_of_content`.
    pub fn matrix_slots(&self, project: ProjectId) -> Vec<SlotContent> {
        self.get_nested(&DesktopTarget::ProjectMatrix(project))
            .iter()
            .map(|target| match target {
                DesktopTarget::Launcher(launcher_id) => SlotContent::Launcher(*launcher_id),
                DesktopTarget::Project(project_id) => SlotContent::Project(*project_id),
                other => panic!(
                    "project matrix children must be launcher or project targets, found {other:?}"
                ),
            })
            .collect()
    }

    /// The launchers of `project` in matrix child order. Slot contents that are
    /// nested projects are skipped, so the layout's zip with child measurements
    /// uses [`Self::matrix_slots`].
    pub fn matrix_launchers(&self, project: ProjectId) -> impl Iterator<Item = LaunchProfileId> {
        let targets = self
            .get_nested(&DesktopTarget::ProjectMatrix(project))
            .to_vec();
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
                DesktopTarget::Project(project),
                [
                    DesktopTarget::ProjectHeader(project),
                    DesktopTarget::ProjectMatrix(project),
                ],
            )
            .unwrap();
        for slot in slots {
            topology
                .add(DesktopTarget::ProjectMatrix(project), slot.clone())
                .unwrap();
        }
        topology
    }

    #[test]
    fn matrix_slots_classify_launcher_and_project_content_in_child_order() {
        let project = ProjectId::new();
        let launcher = LaunchProfileId::new();
        let nested = ProjectId::new();
        let topology = topology_with_matrix(
            project,
            &[
                DesktopTarget::Launcher(launcher),
                DesktopTarget::Project(nested),
            ],
        );

        assert_eq!(
            topology.matrix_slots(project),
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
                DesktopTarget::Project(ProjectId::new()),
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
        let mut topology = topology_with_matrix(root, &[DesktopTarget::Project(nested)]);
        topology
            .add_nested(
                DesktopTarget::Project(nested),
                [
                    DesktopTarget::ProjectHeader(nested),
                    DesktopTarget::ProjectMatrix(nested),
                ],
            )
            .unwrap();
        topology
            .add(
                DesktopTarget::ProjectMatrix(nested),
                DesktopTarget::Launcher(launcher),
            )
            .unwrap();

        assert_eq!(
            topology.project_of_target(&DesktopTarget::Launcher(launcher)),
            nested
        );
        assert_eq!(topology.parent_project_of(nested), Some(root));
        assert_eq!(topology.parent_project_of(root), None);
        assert_eq!(
            topology.project_of_target(&DesktopTarget::Project(root)),
            root
        );
    }

    /// The documented exception: `Desktop` goes *down* the hierarchy to its
    /// single child, the root project, instead of up like every other target.
    #[test]
    fn project_of_target_resolves_the_desktop_root_to_the_root_project() {
        let root = ProjectId::ROOT;
        let mut topology = DesktopTopology::default();
        topology
            .add(DesktopTarget::Desktop, DesktopTarget::Project(root))
            .unwrap();

        assert_eq!(topology.project_of_target(&DesktopTarget::Desktop), root);
    }

    #[test]
    fn launcher_walks_answer_none_under_a_project_without_a_launcher() {
        let root = ProjectId::new();
        let nested = ProjectId::new();
        let topology = topology_with_matrix(root, &[DesktopTarget::Project(nested)]);

        assert_eq!(
            topology.launcher_of_target(&DesktopTarget::Project(nested)),
            None
        );
        assert_eq!(
            topology.project_of_target(&DesktopTarget::Project(nested)),
            nested
        );
    }
}
