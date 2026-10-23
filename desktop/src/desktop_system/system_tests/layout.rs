use super::*;

const LONG_AND_SHORT_NAMES_CONFIG: &str = r#"{
    "startup": "/a/shell-a",
    "slots": [
        { "at": [0, 0], "project": { "name": "a", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell-a", "mode": "visor" } }
        ] } },
        { "at": [1, 0], "project": { "name": "a project name far wider than any slot", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell-b", "mode": "visor" } }
        ] } }
    ]
}"#;

/// A project's header measures no width, so its name never changes the project's size, nor,
/// through the shared scale, its siblings' (ADR 0034).
#[tokio::test]
async fn a_project_name_does_not_change_the_project_size() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(LONG_AND_SHORT_NAMES_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Configuration(command),
            )?;
        }

        let measured_sizes: Vec<_> = system
            .aggregates
            .configuration
            .slots_ordered(ProjectId::ROOT)
            .map(|(_, content)| match content {
                SlotContent::Project(project) => {
                    system
                        .layout_state
                        .measured(&DesktopTarget::Project(project, ProjectPart::Project))
                        .size
                }
                SlotContent::Launcher(_) => panic!("the root holds only projects"),
            })
            .collect();
        assert_eq!(measured_sizes.len(), 2);
        assert_eq!(measured_sizes[0], measured_sizes[1]);
        Ok(())
    })
    .await
}

const SIBLINGS_OF_DIFFERENT_SHAPES_CONFIG: &str = r#"{
    "slots": [
        { "at": [0, 0], "project": { "name": "wide", "slots": [
            { "at": [0, 0], "launcher": { "name": "wide-0", "mode": "visor" } },
            { "at": [1, 0], "launcher": { "name": "wide-1", "mode": "visor" } },
            { "at": [2, 0], "launcher": { "name": "wide-2", "mode": "visor" } }
        ] } },
        { "at": [1, 0], "project": { "name": "narrow", "slots": [
            { "at": [0, 0], "launcher": { "name": "narrow-0", "mode": "visor" } }
        ] } },
        { "at": [2, 0], "project": { "name": "tall", "slots": [
            { "at": [0, 0], "launcher": { "name": "tall-0", "mode": "visor" } },
            { "at": [0, 1], "launcher": { "name": "tall-1", "mode": "visor" } },
            { "at": [0, 2], "launcher": { "name": "tall-2", "mode": "visor" } },
            { "at": [0, 3], "launcher": { "name": "tall-3", "mode": "visor" } },
            { "at": [0, 4], "launcher": { "name": "tall-4", "mode": "visor" } }
        ] } }
    ]
}"#;

/// Sibling projects lay out the columns of the widest one, so they are equally wide whatever
/// their own columns, and the tall project, which sets the shared scale, adds no width beyond
/// those columns. Clearing the wide project's last column narrows every sibling with it, because
/// the refreshed count measures their matrices again (ADR 0035).
#[tokio::test]
async fn sibling_projects_lay_out_the_columns_of_the_widest_one() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(SIBLINGS_OF_DIFFERENT_SHAPES_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Configuration(command),
            )?;
        }
        let projects: Vec<ProjectId> = system
            .aggregates
            .configuration
            .slots_ordered(ProjectId::ROOT)
            .map(|(_, content)| match content {
                SlotContent::Project(project) => project,
                SlotContent::Launcher(_) => panic!("the root holds only projects"),
            })
            .collect();
        let widths = |system: &DesktopSystem| -> Vec<(u32, u32)> {
            projects
                .iter()
                .map(|&project| {
                    let target = DesktopTarget::Project(project, ProjectPart::Project);
                    let measured = system.layout_state.measured(&target).size[0];
                    let placed = system
                        .layout_state
                        .absolute_placement(&target, &system.aggregates.hierarchy)
                        .rect
                        .size[0];
                    (measured, placed)
                })
                .collect()
        };

        let three_columns = widths(&system);
        assert!(three_columns.iter().all(|&width| width == three_columns[0]));
        assert_eq!(three_columns[0].0, three_columns[0].1);

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Configuration(ConfigurationCommand::ClearSlot {
                parent: projects[0],
                slot: SlotPosition { column: 2, row: 0 },
                shift: SlotShift::Keep,
            }),
        )?;

        let two_columns = widths(&system);
        assert!(two_columns.iter().all(|&width| width == two_columns[0]));
        assert!(two_columns[0].0 < three_columns[0].0);
        Ok(())
    })
    .await
}
