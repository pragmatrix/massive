use massive_applications::InstanceId;

use super::{HorizontalDirection, VerticalDirection};
use crate::desktop_system::DesktopTarget;
use crate::desktop_system::Direction;
use crate::desktop_system::topology::DesktopTopology;
use crate::projects::{
    LaunchProfileId, MatrixPlacement, ProjectId, RuntimeConfiguration, SlotContent,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct MatrixNavigation<'a> {
    hierarchy: &'a DesktopTopology,
    configuration: &'a RuntimeConfiguration,
}

/// The outcome of a navigation lookup.
#[derive(Debug, Clone)]
pub(super) struct NavigationStep {
    pub target: DesktopTarget,
    /// The placement of the slot the search escaped to in an ancestor matrix, if the target was
    /// not found in the origin's own matrix.
    pub escaped_from: Option<MatrixPlacement>,
}

#[derive(Debug, Clone, Copy)]
struct MatrixEntry<K> {
    key: K,
    placement: MatrixPlacement,
}

impl<'a> MatrixNavigation<'a> {
    pub(super) fn new(
        hierarchy: &'a DesktopTopology,
        configuration: &'a RuntimeConfiguration,
    ) -> Self {
        Self {
            hierarchy,
            configuration,
        }
    }

    pub(super) fn navigate_from_slot(
        self,
        content: impl Into<SlotContent>,
        direction: Direction,
        preferred_column: Option<u32>,
    ) -> Option<NavigationStep> {
        let (project_id, origin_placement) = self.configuration.slot_of_content(content)?;
        self.navigate_from_matrix_slot(project_id, origin_placement, direction, preferred_column)
    }

    /// Navigates from an assigned slot of `project`'s matrix. A project-assigned
    /// slot's target is the nested project itself, so the neighbor is whatever the
    /// destination slot hosts.
    ///
    /// When the matrix has no neighbor in `direction`, the search escapes to the slot hosting
    /// the project in its parent's matrix and retries from there, recursively up to the root.
    /// `preferred_column` applies only to the starting matrix; escaped levels use the hosting
    /// slot's own column.
    pub(super) fn navigate_from_matrix_slot(
        self,
        project_id: ProjectId,
        origin_placement: MatrixPlacement,
        direction: Direction,
        preferred_column: Option<u32>,
    ) -> Option<NavigationStep> {
        let mut project = project_id;
        let mut placement = origin_placement;
        let mut preferred_column = preferred_column;
        let mut escaped_from = None;

        loop {
            if let Some(target) =
                self.navigate_within_matrix(project, placement, direction, preferred_column)
            {
                return Some(NavigationStep {
                    target,
                    escaped_from,
                });
            }

            (project, placement) = self.configuration.slot_of_content(project)?;
            preferred_column = None;
            escaped_from = Some(placement);
        }
    }

    pub(super) fn navigate_within_matrix(
        self,
        project_id: ProjectId,
        origin_placement: MatrixPlacement,
        direction: Direction,
        preferred_column: Option<u32>,
    ) -> Option<DesktopTarget> {
        let entries = self.create_project_matrix_entries(project_id);
        select_matrix_neighbor(&entries, origin_placement, direction, preferred_column)
            .map(SlotContent::target)
    }

    pub(super) fn navigate_from_child(
        self,
        launcher_id: LaunchProfileId,
        index: usize,
        direction: Direction,
        preferred_column: Option<u32>,
    ) -> Option<NavigationStep> {
        // Existence is answered from the configuration aggregate; presenters are not
        // consulted along this path.
        let _ = self.configuration.launcher(launcher_id)?;
        let instances: Vec<_> = self.hierarchy.launcher_instances(launcher_id).collect();
        if let Some(horizontal) = direction.horizontal() {
            return horizontal_child_neighbor(&instances, index, horizontal)
                .map(|instance| NavigationStep {
                    target: DesktopTarget::Instance(instance),
                    escaped_from: None,
                })
                .or_else(|| self.navigate_from_slot(launcher_id, direction, preferred_column));
        }

        self.navigate_from_slot(launcher_id, direction, preferred_column)
    }

    fn create_project_matrix_entries(self, project_id: ProjectId) -> Vec<MatrixEntry<SlotContent>> {
        self.configuration
            .slots_ordered(project_id)
            .map(|(placement, content)| MatrixEntry {
                key: content,
                placement,
            })
            .collect()
    }
}

fn horizontal_child_neighbor(
    instances: &[InstanceId],
    index: usize,
    direction: HorizontalDirection,
) -> Option<InstanceId> {
    match direction {
        HorizontalDirection::Left => (index > 0).then(|| instances[index - 1]),
        HorizontalDirection::Right => (index + 1 < instances.len()).then(|| instances[index + 1]),
    }
}

fn select_matrix_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin: MatrixPlacement,
    direction: Direction,
    preferred_column: Option<u32>,
) -> Option<K> {
    if let Some(horizontal) = direction.horizontal() {
        return select_row_neighbor(entries, origin, horizontal);
    }

    if let Some(vertical) = direction.vertical() {
        return select_column_neighbor(
            entries,
            origin.row,
            preferred_column.unwrap_or(origin.column),
            vertical,
        );
    }

    None
}

fn select_row_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin: MatrixPlacement,
    direction: HorizontalDirection,
) -> Option<K> {
    match direction {
        HorizontalDirection::Left => entries
            .iter()
            .filter(|entry| {
                entry.placement.row == origin.row && entry.placement.column < origin.column
            })
            .max_by_key(|entry| entry.placement.column)
            .map(|entry| entry.key),
        HorizontalDirection::Right => entries
            .iter()
            .filter(|entry| {
                entry.placement.row == origin.row && entry.placement.column > origin.column
            })
            .min_by_key(|entry| entry.placement.column)
            .map(|entry| entry.key),
    }
}

fn select_column_neighbor<K: Copy>(
    entries: &[MatrixEntry<K>],
    origin_row: u32,
    column: u32,
    direction: VerticalDirection,
) -> Option<K> {
    let target_row = match direction {
        VerticalDirection::Up => entries
            .iter()
            .filter(|entry| entry.placement.row < origin_row)
            .map(|entry| entry.placement.row)
            .max()?,
        VerticalDirection::Down => entries
            .iter()
            .filter(|entry| entry.placement.row > origin_row)
            .map(|entry| entry.placement.row)
            .min()?,
    };

    entries
        .iter()
        .filter(|entry| entry.placement.row == target_row)
        .min_by_key(|entry| {
            let distance = u32::abs_diff(entry.placement.column, column);
            (distance, entry.placement.column)
        })
        .map(|entry| entry.key)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::projects::persistence::parse_configuration;

    #[test]
    fn matrix_horizontal_navigation_skips_empty_slots() {
        let entries = sample_entries();

        let left = select_matrix_neighbor(&entries, (2, 0).into(), Direction::Left, None);
        let right = select_matrix_neighbor(&entries, (0, 0).into(), Direction::Right, None);

        assert_eq!(left, Some(1));
        assert_eq!(right, Some(2));
    }

    #[test]
    fn matrix_vertical_navigation_skips_empty_slots() {
        let entries = sample_entries();

        let down = select_matrix_neighbor(&entries, (0, 0).into(), Direction::Down, None);
        let up = select_matrix_neighbor(&entries, (0, 2).into(), Direction::Up, None);

        assert_eq!(down, Some(3));
        assert_eq!(up, Some(1));
    }

    #[test]
    fn row_neighbor_returns_none_when_no_candidate_exists() {
        let entries = sample_entries();

        let left = select_row_neighbor(&entries, (0, 0).into(), HorizontalDirection::Left);
        let right = select_row_neighbor(&entries, (2, 2).into(), HorizontalDirection::Right);

        assert_eq!(left, None);
        assert_eq!(right, None);
    }

    #[test]
    fn side_row_neighbor_selection_works_for_horizontal_navigation() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                placement: (2, 1).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (4, 1).into(),
            },
        ];

        let side = select_row_neighbor(&entries, (2, 1).into(), HorizontalDirection::Right);

        assert_eq!(side, Some(2));
    }

    #[test]
    fn matrix_vertical_navigation_uses_preferred_column_when_provided() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                placement: (0, 0).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (2, 0).into(),
            },
            MatrixEntry {
                key: 3,
                placement: (0, 2).into(),
            },
            MatrixEntry {
                key: 4,
                placement: (2, 2).into(),
            },
        ];

        let up = select_matrix_neighbor(&entries, (0, 2).into(), Direction::Up, Some(2));

        assert_eq!(up, Some(2));
    }

    #[test]
    fn matrix_vertical_navigation_uses_next_non_empty_row_and_nearest_column() {
        let entries = vec![
            MatrixEntry {
                key: 1,
                placement: (0, 0).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (3, 1).into(),
            },
            MatrixEntry {
                key: 3,
                placement: (1, 2).into(),
            },
        ];

        let down = select_matrix_neighbor(&entries, (0, 0).into(), Direction::Down, None);

        assert_eq!(down, Some(2));
    }

    const ESCAPE_CONFIGURATION: &str = r#"{ "slots": [
        { "at": [0, 0], "launcher": { "name": "a", "mode": "visor" } },
        { "at": [3, 0], "launcher": { "name": "d", "mode": "visor" } },
        { "at": [3, 1], "project": { "name": "labs", "slots": [
            { "at": [0, 0], "launcher": { "name": "b", "mode": "visor" } },
            { "at": [1, 1], "project": { "name": "inner", "slots": [
                { "at": [0, 0], "launcher": { "name": "c", "mode": "visor" } }
            ] } }
        ] } }
    ] }"#;

    struct EscapeFixture {
        configuration: RuntimeConfiguration,
        hierarchy: DesktopTopology,
        labs: ProjectId,
        inner: ProjectId,
    }

    impl EscapeFixture {
        fn new() -> Self {
            let configuration =
                parse_configuration(Path::new("test.json"), ESCAPE_CONFIGURATION).unwrap();
            let labs = configuration
                .child_project_at(ProjectId::ROOT, (3, 1).into())
                .unwrap();
            let inner = configuration.child_project_at(labs, (1, 1).into()).unwrap();
            Self {
                configuration,
                hierarchy: DesktopTopology::default(),
                labs,
                inner,
            }
        }

        fn navigate(
            &self,
            project: ProjectId,
            placement: (u32, u32),
            direction: Direction,
            preferred_column: Option<u32>,
        ) -> Option<NavigationStep> {
            MatrixNavigation::new(&self.hierarchy, &self.configuration).navigate_from_matrix_slot(
                project,
                placement.into(),
                direction,
                preferred_column,
            )
        }

        fn target_at(&self, project: ProjectId, placement: (u32, u32)) -> DesktopTarget {
            self.configuration
                .content_at(project, placement.into())
                .unwrap()
                .target()
        }
    }

    #[test]
    fn navigation_escapes_recursively_through_nested_projects() {
        let fixture = EscapeFixture::new();

        let step = fixture
            .navigate(fixture.inner, (0, 0), Direction::Up, None)
            .unwrap();

        // `inner` at (1, 1) in labs has nothing above in column 1 or any row, so it
        // escapes into labs, which finds `b` at (0, 0).
        assert_eq!(step.target, fixture.target_at(fixture.labs, (0, 0)));
        assert_eq!(step.escaped_from, Some((1, 1).into()));

        let step = fixture
            .navigate(fixture.labs, (0, 0), Direction::Up, None)
            .unwrap();

        assert_eq!(step.target, fixture.target_at(ProjectId::ROOT, (3, 0)));
        assert_eq!(step.escaped_from, Some((3, 1).into()));
    }

    #[test]
    fn escaped_levels_use_the_hosting_slots_column_instead_of_the_preferred_one() {
        let fixture = EscapeFixture::new();

        // A latched column 0 from the nested matrix would pick `a`; the hosting slot's column
        // 3 picks `d`.
        let step = fixture
            .navigate(fixture.labs, (0, 0), Direction::Up, Some(0))
            .unwrap();

        assert_eq!(step.target, fixture.target_at(ProjectId::ROOT, (3, 0)));
    }

    #[test]
    fn navigation_without_a_neighbor_at_any_level_finds_nothing() {
        let fixture = EscapeFixture::new();

        assert!(
            fixture
                .navigate(fixture.inner, (0, 0), Direction::Right, None)
                .is_none()
        );
        assert!(
            fixture
                .navigate(fixture.inner, (0, 0), Direction::Down, None)
                .is_none()
        );
    }

    #[test]
    fn navigation_inside_a_matrix_does_not_escape() {
        let fixture = EscapeFixture::new();

        let step = fixture
            .navigate(fixture.labs, (0, 0), Direction::Down, None)
            .unwrap_or_else(|| panic!("labs (0,0) has `inner` below"));

        assert_eq!(step.target, DesktopTarget::Project(fixture.inner));
        assert_eq!(step.escaped_from, None);
    }

    fn sample_entries() -> Vec<MatrixEntry<usize>> {
        vec![
            MatrixEntry {
                key: 1,
                placement: (0, 0).into(),
            },
            MatrixEntry {
                key: 2,
                placement: (2, 0).into(),
            },
            MatrixEntry {
                key: 3,
                placement: (0, 2).into(),
            },
            MatrixEntry {
                key: 4,
                placement: (2, 2).into(),
            },
            MatrixEntry {
                key: 5,
                placement: (1, 3).into(),
            },
        ]
    }
}
