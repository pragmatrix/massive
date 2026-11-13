use std::time::Instant;

use super::*;

#[tokio::test]
async fn cmd_enter_on_a_launcher_zooms_first_and_starts_once_fully_in() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, receiver) = system_from(CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(command.into())?;
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
                focused_slot_project: None,
                focus: KeyFocus::Desktop,
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
            Some(DesktopCommand::StartInstance(_))
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
            pointer_press_focus(Some(DesktopTarget::matrix(project))),
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
            DesktopCommand::Navigate(Direction::Right),
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
            Instant::now(),
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

/// A command that keeps the focus, as MCP issues it, leaves the user where they are (ADR 0036).
#[tokio::test]
async fn adding_content_with_a_kept_focus_leaves_the_focus_alone() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let labs_b = root_project_at(&system, 1);

        run(
            &mut system,
            &mut instance_manager,
            ConfigurationCommand::AddNextTo {
                anchor: SlotContent::Project(labs_b),
                content: SlotAssignment::Project {
                    id: ProjectId::new(),
                    name: "added".into(),
                },
                focus: FocusIntent::Keep,
            }
            .into(),
        )?;

        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );
        Ok(())
    })
    .await
}

/// Ungrouping with a kept focus leaves a focus elsewhere alone. A focused project that the
/// ungrouping removes still hands its focus to the ungrouped slots, because the focus cannot stay
/// on it (ADR 0036).
#[tokio::test]
async fn ungrouping_with_a_kept_focus_moves_it_only_off_the_removed_project() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let labs_a = root_project_at(&system, 0);
        let labs_b = root_project_at(&system, 1);
        let ungroup = |project| {
            ConfigurationCommand::Ungroup {
                project,
                focus: FocusIntent::Keep,
            }
            .into()
        };

        run(&mut system, &mut instance_manager, ungroup(labs_b))?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );

        let Some(SlotContent::Launcher(shell_a)) = system
            .aggregates
            .configuration
            .content_at(labs_a, SlotPosition { column: 0, row: 0 })
        else {
            panic!("labs-a holds its launcher");
        };
        system.transact(
            set_focus(
                Some(FocusableTarget::Project(labs_a)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        run(&mut system, &mut instance_manager, ungroup(labs_a))?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(shell_a))
        );
        Ok(())
    })
    .await
}

/// A visor follows a focus change made while a mouse button is pressed only when the button is
/// released, even if something else re-lays it out during the press (ADR 0002).
#[tokio::test]
async fn a_visor_follows_a_focus_change_during_a_press_only_on_release() -> Result<()> {
    task_context::with_context(task_context(), async {
        // Live transactions animate, which needs a frame.
        let frame = massive_applications::begin_frame();
        let (mut system, receiver) = system_from(CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in projects::to_commands(&system.aggregates.configuration) {
            run(&mut system, &mut instance_manager, command.into())?;
        }
        let launcher = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        let first = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        let second = start_instance(&mut system, &mut instance_manager, launcher, 1)?;
        // Focusing the launcher itself collapses the visor around the second instance.
        system.transact(
            set_focus(
                Some(FocusableTarget::Launcher(launcher)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        let first_visible = |system: &DesktopSystem| {
            system
                .layout_state
                .local_placement(&DesktopTarget::from(first))
                .visible
        };
        let anchor =
            |system: &DesktopSystem| system.aggregates.launchers[&launcher].focus_anchor_instance;
        assert!(!first_visible(&system));

        // The press focuses the first instance, and a resize re-lays out the visor meanwhile.
        system.transact(
            set_focus(
                Some(FocusableTarget::Instance(first)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::UserGestureActive,
        )?;
        system.transact(
            DesktopChange::ResizeAll(SizePx::new(640, 480)),
            &mut instance_manager,
            TransactionEffectsMode::UserGestureActive,
        )?;
        assert!(!first_visible(&system));
        assert_eq!(anchor(&system), Some(second));

        // The release syncs the layout focus, so the visor expands and centers on the first
        // instance.
        system.transact(
            Changes::Empty,
            &mut instance_manager,
            TransactionEffectsMode::Normal,
        )?;
        assert!(first_visible(&system));
        assert_eq!(anchor(&system), Some(first));

        drop(frame);
        Ok(())
    })
    .await
}

/// The nested project in the root's slot at `column`, in the first row.
fn root_project_at(system: &DesktopSystem, column: u32) -> ProjectId {
    match system
        .aggregates
        .configuration
        .content_at(ProjectId::ROOT, SlotPosition { column, row: 0 })
    {
        Some(SlotContent::Project(project)) => project,
        other => panic!("expected a project at column {column}, found {other:?}"),
    }
}
