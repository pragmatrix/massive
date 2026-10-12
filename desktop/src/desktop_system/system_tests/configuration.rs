use super::*;

#[tokio::test]
async fn a_second_parentless_assign_slot_is_rejected() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system();
        let mut instance_manager = instance_manager(&receiver);

        let changes = system.plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
            parent: None,
            placement: MatrixPlacement { column: 0, row: 0 },
            content: SlotAssignment::Project {
                id: ProjectId::ROOT,
                name: crate::projects::ROOT_PROJECT_NAME.into(),
            },
            shift: SlotShift::default(),
        }))?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        for id in [ProjectId::ROOT, ProjectId::new()] {
            assert!(
                system
                    .plan(DesktopCommand::Project(ProjectCommand::AssignSlot {
                        parent: None,
                        placement: MatrixPlacement { column: 0, row: 0 },
                        content: SlotAssignment::Project {
                            id,
                            name: "again".into(),
                        },
                        shift: SlotShift::default(),
                    }))
                    .is_err(),
                "a second parentless AssignSlot must be rejected"
            );
        }

        Ok(())
    })
    .await
}
