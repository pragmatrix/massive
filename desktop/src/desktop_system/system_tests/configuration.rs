use super::*;

#[tokio::test]
async fn a_second_parentless_assign_slot_is_rejected() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system();
        let mut instance_manager = instance_manager(&receiver);

        let changes = system.plan(DesktopCommand::Configuration(
            ConfigurationCommand::AssignSlot {
                parent: None,
                placement: MatrixPlacement { column: 0, row: 0 },
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    name: crate::projects::ROOT_PROJECT_NAME.into(),
                },
                shift: SlotShift::default(),
            },
        ))?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        for id in [ProjectId::ROOT, ProjectId::new()] {
            assert!(
                system
                    .plan(DesktopCommand::Configuration(
                        ConfigurationCommand::AssignSlot {
                            parent: None,
                            placement: MatrixPlacement { column: 0, row: 0 },
                            content: SlotAssignment::Project {
                                id,
                                name: "again".into(),
                            },
                            shift: SlotShift::default(),
                        }
                    ))
                    .is_err(),
                "a second parentless AssignSlot must be rejected"
            );
        }

        Ok(())
    })
    .await
}

/// Cmd+N inserts between the focused launcher and its right neighbour, which shifts on, and the
/// new launcher takes the focus.
#[tokio::test]
async fn add_launcher_inserts_right_of_the_focused_slot_and_focuses_it() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(
            r#"{
                "startup": "/a",
                "slots": [
                    { "at": [0, 0], "launcher": { "name": "a", "mode": "visor" } },
                    { "at": [1, 0], "launcher": { "name": "b", "mode": "visor" } }
                ]
            }"#,
        );
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Configuration(command),
            )?;
        }
        let at = |column| MatrixPlacement { column, row: 0 };
        let a = system.aggregates.configuration.boot_launcher().unwrap();
        let b = system
            .aggregates
            .configuration
            .content_at(ProjectId::ROOT, at(1))
            .unwrap();
        system.transact(
            set_focus(
                Some(FocusableTarget::Launcher(a)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::AddLauncher,
        )?;

        let configuration = &system.aggregates.configuration;
        let Some(SlotContent::Launcher(added)) = configuration.content_at(ProjectId::ROOT, at(1))
        else {
            panic!("a launcher is added right of the focused one");
        };
        assert_ne!(SlotContent::Launcher(added), b);
        assert_eq!(configuration.content_at(ProjectId::ROOT, at(2)), Some(b));
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(added))
        );

        Ok(())
    })
    .await
}

/// A slot that moves to another project keeps its running instances and the keyboard focus: the
/// launcher is re-parented with its subtree, not cleared and re-assigned.
#[tokio::test]
async fn move_slot_across_projects_keeps_instances_and_focus() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let (project_a, placement_a) = system
            .aggregates
            .configuration
            .slot_of_content(launcher_a)
            .unwrap();
        let project_b = system
            .aggregates
            .configuration
            .slots_ordered(ProjectId::ROOT)
            .find_map(|(_, content)| match content {
                SlotContent::Project(project) if project != project_a => Some(project),
                _ => None,
            })
            .unwrap();
        let dest = MatrixPlacement { column: 1, row: 0 };

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Configuration(ConfigurationCommand::MoveSlot {
                source: (project_a, placement_a),
                dest: (project_b, dest),
            }),
        )?;

        assert_eq!(
            system.aggregates.configuration.slot_of_content(launcher_a),
            Some((project_b, dest))
        );
        assert_eq!(
            system
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::Launcher(launcher_a)),
            Some(&DesktopTarget::Project(project_b, ProjectPart::Matrix))
        );
        assert_eq!(
            system.aggregates.hierarchy.launcher_of_instance(instance_a),
            launcher_a
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );

        Ok(())
    })
    .await
}

/// A project slot that moves into another project carries its own slots and their instances along.
#[tokio::test]
async fn move_slot_across_projects_carries_a_nested_project() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
        } = nested_siblings()?;
        let hierarchy = &system.aggregates.hierarchy;
        let project_of = |instance| {
            hierarchy.project_of_target(&DesktopTarget::Launcher(
                hierarchy.launcher_of_instance(instance),
            ))
        };
        let (project_a, project_b) = (project_of(instance_a), project_of(instance_b));
        let (_, placement_b) = system
            .aggregates
            .configuration
            .slot_of_content(project_b)
            .unwrap();
        let dest = MatrixPlacement { column: 1, row: 0 };

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Configuration(ConfigurationCommand::MoveSlot {
                source: (ProjectId::ROOT, placement_b),
                dest: (project_a, dest),
            }),
        )?;

        let hierarchy = &system.aggregates.hierarchy;
        assert_eq!(
            hierarchy.parent(&DesktopTarget::Project(project_b, ProjectPart::Project)),
            Some(&DesktopTarget::Project(project_a, ProjectPart::Matrix))
        );
        assert_eq!(hierarchy.parent_project_of(project_b), Some(project_a));
        assert_eq!(
            hierarchy.project_of_target(&DesktopTarget::Launcher(
                hierarchy.launcher_of_instance(instance_b)
            )),
            project_b
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );

        Ok(())
    })
    .await
}
