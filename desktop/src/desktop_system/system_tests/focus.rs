use super::*;

#[tokio::test]
async fn cmd_enter_on_a_launcher_zooms_first_and_starts_once_fully_in() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, receiver) = system_from(CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Project(command))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
        }
        let launcher = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        system.transact(
            set_focus(
                Some(DesktopTarget::Launcher(launcher)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        let cmd_enter = KeyInput::new(
            Key::Named(NamedKey::Enter),
            ElementState::Pressed,
            false,
            ModifiersState::SUPER,
        );
        let first_command = |fully_zoomed_in| {
            let context = KeyContext {
                is_focused: true,
                fully_zoomed_in,
                hierarchy: &system.aggregates.hierarchy,
                configuration: &system.aggregates.configuration,
            };
            let handler = system.aggregates.launchers.get(&launcher).unwrap();
            match handler.handle_key(&cmd_enter, &context) {
                KeyOutcome::Consumed(commands) => commands.into_iter().next(),
                KeyOutcome::Pass | KeyOutcome::Forward(_) => None,
            }
        };

        assert!(matches!(
            first_command(false),
            Some(DesktopCommand::Zoom(Zoom::Enter))
        ));
        assert!(matches!(
            first_command(true),
            Some(DesktopCommand::StartInstance { .. })
        ));

        drop(frame);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_click_on_an_empty_matrix_focuses_its_project_and_navigation_continues() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_b,
            ..
        } = nested_siblings()?;
        let project = system
            .focused_path()
            .iter()
            .rev()
            .find_map(|target| match target {
                DesktopTarget::Project(project) => Some(*project),
                _ => None,
            })
            .expect("the focused instance is inside a project");

        system.transact(
            keyboard_focus_request(
                Some(DesktopTarget::ProjectMatrix(project)),
                FocusRequestSource::PointerPress,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        assert_eq!(
            system.event_router.keyboard_focus(),
            Some(&DesktopTarget::Project(project))
        );

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Navigate(crate::desktop_system::Direction::Right),
        )?;
        assert_eq!(
            system.event_router.keyboard_focus(),
            Some(&DesktopTarget::Instance(instance_b))
        );

        drop(frame);
        Ok(())
    })
    .await
}
