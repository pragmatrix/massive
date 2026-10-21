use super::*;
use crate::desktop_system::selection::SelectionDelta;

/// Shift+Cmd+arrow from an instance moves the focus to its launcher, where the selection starts,
/// and the selection survives that focus change. A later focus change collapses it (ADR 0025).
#[tokio::test]
async fn extending_from_an_instance_starts_at_its_launcher_until_the_focus_moves() -> Result<()> {
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
        let launcher_a = system.aggregates.configuration.boot_launcher().unwrap();
        let instance = start_instance(&mut system, &mut instance_manager, launcher_a, 0)?;
        system.transact(
            set_focus(
                Some(FocusableTarget::Instance(instance)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        let columns = |columns| SelectionDelta { columns, rows: 0 };
        let extend =
            |system: &mut DesktopSystem, instance_manager: &mut InstanceManager, direction| {
                run(
                    system,
                    instance_manager,
                    DesktopCommand::ExtendSelection(direction),
                )
            };

        extend(&mut system, &mut instance_manager, Direction::Right)?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(launcher_a))
        );
        assert_eq!(system.selection.delta(), columns(1));

        // The selection ends at the last column of the matrix.
        extend(&mut system, &mut instance_manager, Direction::Right)?;
        assert_eq!(system.selection.delta(), columns(1));

        // Back at the zero delta, only the focused slot is selected.
        extend(&mut system, &mut instance_manager, Direction::Left)?;
        assert_eq!(system.selection.delta(), columns(0));

        extend(&mut system, &mut instance_manager, Direction::Right)?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Navigate(Direction::Right),
        )?;
        assert_eq!(system.selection.delta(), columns(0));

        Ok(())
    })
    .await
}
