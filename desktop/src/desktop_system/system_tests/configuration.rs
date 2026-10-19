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
