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
                Some(FocusableTarget::Launcher(launcher)),
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
                DesktopTarget::Project(project, ProjectPart::Project) => Some(*project),
                _ => None,
            })
            .expect("the focused instance is inside a project");

        system.transact(
            pointer_press_focus(Some(DesktopTarget::Project(project, ProjectPart::Matrix))),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Project(project))
        );

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Navigate(crate::desktop_system::Direction::Right),
        )?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_b))
        );

        drop(frame);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn regaining_the_window_focus_after_the_focused_instance_stopped_focuses_a_live_target()
-> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        deliver_view(&mut system, &mut instance_manager, instance)?;

        change_window_focus(&mut system, &mut instance_manager, false)?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::StopInstance(instance),
        )?;
        change_window_focus(&mut system, &mut instance_manager, true)?;

        let focused = system
            .focused_target()
            .expect("regaining the window focus focuses a target");
        assert!(
            system.aggregates.hierarchy.exists(&focused),
            "{focused:?} is not in the hierarchy"
        );

        drop(frame);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_view_destroyed_while_the_window_is_unfocused_is_not_restored() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        deliver_view(&mut system, &mut instance_manager, instance)?;
        let Some(FocusableTarget::View(_, view)) = system.keyboard_focus.focused() else {
            panic!("the delivered primary view is focused");
        };

        change_window_focus(&mut system, &mut instance_manager, false)?;
        let mut submission = massive_util::ChangeSet::default();
        submission.push(InstanceChange::DestroyView(view));
        system.transact(
            DesktopChange::IntegrateInstanceSubmission(
                instance,
                InstanceSubmission::new(submission, massive_renderer::RenderPacing::default()),
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        change_window_focus(&mut system, &mut instance_manager, true)?;

        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance))
        );

        drop(frame);
        Ok(())
    })
    .await
}

/// Routes a window focus change through input processing, as the window delivers it.
fn change_window_focus(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    focused: bool,
) -> Result<()> {
    let mut events = massive_input::EventManager::<massive_applications::ViewEvent>::default();
    let event = events
        .add_event(
            massive_applications::ViewEvent::Focused(focused),
            std::time::Instant::now(),
        )
        .expect("a window focus change is not redundant");
    let geometry = massive_renderer::RenderGeometry::new(
        SizePx::new(1000, 800),
        massive_geometry::PixelCamera::default(),
    );
    let changes = system.process_input_event(&event, &geometry)?;
    system.transact(changes, instance_manager, TransactionEffectsMode::Setup)?;
    Ok(())
}
