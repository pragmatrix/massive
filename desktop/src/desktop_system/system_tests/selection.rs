use euclid::vec2;

use super::*;

/// Two launchers booted side by side in the root project, `a` at column 0 and `b` at column 1,
/// with nothing focused yet.
struct TwoLaunchers {
    system: DesktopSystem,
    instance_manager: InstanceManager,
    a: LaunchProfileId,
    b: LaunchProfileId,
}

fn two_launchers() -> Result<TwoLaunchers> {
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
    let a = system.aggregates.configuration.boot_launcher().unwrap();
    let Some(SlotContent::Launcher(b)) = system
        .aggregates
        .configuration
        .content_at(ProjectId::ROOT, SlotPosition { column: 1, row: 0 })
    else {
        panic!("the second launcher sits right of the first");
    };
    Ok(TwoLaunchers {
        system,
        instance_manager,
        a,
        b,
    })
}

fn focus(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    target: FocusableTarget,
) -> Result<()> {
    system.transact(
        set_focus(Some(target), KeyboardFocusReason::InputTransition),
        instance_manager,
        TransactionEffectsMode::Setup,
    )?;
    Ok(())
}

fn extend(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    direction: Direction,
) -> Result<()> {
    run(
        system,
        instance_manager,
        DesktopCommand::ExtendSelection(direction),
    )
}

/// Shift+Cmd+arrow from an instance moves the focus to its launcher, where the selection starts,
/// and the selection survives that focus change. A later focus change collapses it (ADR 0025).
#[tokio::test]
async fn extending_from_an_instance_starts_at_its_launcher_until_the_focus_moves() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            ..
        } = two_launchers()?;
        let instance = start_instance(&mut system, &mut instance_manager, a, 0)?;
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Instance(instance),
        )?;
        let columns = |columns| Selection::from_diagonal(vec2(columns, 0));

        extend(&mut system, &mut instance_manager, Direction::Right)?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(a))
        );
        assert_eq!(system.selection, columns(1));

        // The selection ends at the last column of the matrix.
        extend(&mut system, &mut instance_manager, Direction::Right)?;
        assert_eq!(system.selection, columns(1));

        // Back at the zero offsets, only the focused slot is selected.
        extend(&mut system, &mut instance_manager, Direction::Left)?;
        assert_eq!(system.selection, columns(0));

        extend(&mut system, &mut instance_manager, Direction::Right)?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Navigate(Direction::Right),
        )?;
        assert_eq!(system.selection, columns(0));

        Ok(())
    })
    .await
}

/// With a selection beyond the focused slot, the outline covers the rectangle of the selected
/// slots: for two neighboring launchers, it is centered between them and wider than either.
#[tokio::test]
async fn the_outline_covers_the_selected_slots() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            b,
        } = two_launchers()?;
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;

        extend(&mut system, &mut instance_manager, Direction::Right)?;

        let outline = system.hover_placement().expect("the selection is outlined");
        let a = system.placement(&DesktopTarget::Launcher(a));
        let b = system.placement(&DesktopTarget::Launcher(b));
        let between = (a.transform.translate + b.transform.translate) * 0.5;
        assert!((outline.transform.translate - between).length() < 1e-6);
        assert!(outline.rect.size[0] > a.rect.size[0] + b.rect.size[0]);

        Ok(())
    })
    .await
}

/// Cmd+G with a selection wraps all selected slots into one project at the selection's top-left
/// slot, keeping their arrangement. Cmd+Shift+G gives them back and selects them again
/// (ADR 0026).
#[tokio::test]
async fn grouping_a_selection_and_ungrouping_it_selects_the_slots_again() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            b,
        } = two_launchers()?;
        let at = |column| SlotPosition { column, row: 0 };
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;
        extend(&mut system, &mut instance_manager, Direction::Right)?;

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::GroupSlot,
        )?;

        let configuration = &system.aggregates.configuration;
        let Some(SlotContent::Project(group)) = configuration.content_at(ProjectId::ROOT, at(0))
        else {
            panic!("a project takes the selection's top-left slot");
        };
        assert_eq!(configuration.slots_ordered(ProjectId::ROOT).count(), 1);
        assert_eq!(configuration.slot_of_content(a), Some((group, at(0))));
        assert_eq!(configuration.slot_of_content(b), Some((group, at(1))));
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Project(group))
        );
        assert_eq!(system.selection, Selection::default());

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::UngroupSlot,
        )?;

        let configuration = &system.aggregates.configuration;
        assert_eq!(
            configuration.slot_of_content(a),
            Some((ProjectId::ROOT, at(0)))
        );
        assert_eq!(
            configuration.slot_of_content(b),
            Some((ProjectId::ROOT, at(1)))
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(a))
        );
        assert_eq!(system.selection, Selection::from_diagonal(vec2(1, 0)));

        Ok(())
    })
    .await
}

/// A selection set in the same transaction that adds a slot to its matrix is outlined after the
/// transaction's layout, so the outline finds the new slot measured.
#[tokio::test]
async fn the_outline_reads_a_slot_added_in_the_same_transaction() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            ..
        } = two_launchers()?;
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;

        let mut changes = system.plan(DesktopCommand::ExtendSelection(Direction::Right))?;
        changes += system.plan(DesktopCommand::AddLauncher)?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        assert_ne!(system.selection, Selection::default());
        assert!(system.hover_placement().is_some());

        Ok(())
    })
    .await
}

/// Cmd+Shift+G from inside a project ungroups it once the selection covers all of its slots
/// (ADR 0029).
#[tokio::test]
async fn selecting_all_slots_of_a_project_ungroups_it() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            b,
        } = two_launchers()?;
        let at = |column| SlotPosition { column, row: 0 };
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;
        extend(&mut system, &mut instance_manager, Direction::Right)?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::GroupSlot,
        )?;

        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;
        extend(&mut system, &mut instance_manager, Direction::Right)?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::UngroupSlot,
        )?;

        let configuration = &system.aggregates.configuration;
        assert_eq!(
            configuration.slot_of_content(a),
            Some((ProjectId::ROOT, at(0)))
        );
        assert_eq!(
            configuration.slot_of_content(b),
            Some((ProjectId::ROOT, at(1)))
        );

        Ok(())
    })
    .await
}

/// The root project cannot be ungrouped, so an extended selection over all of its slots leaves
/// Cmd+Shift+G to the focused project (ADR 0029).
#[tokio::test]
async fn selecting_all_root_slots_ungroups_the_focused_project() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            ..
        } = two_launchers()?;
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::GroupSlot,
        )?;
        let Some((group, _)) = system.aggregates.configuration.slot_of_content(a) else {
            panic!("grouping keeps the launcher in a slot");
        };

        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Project(group),
        )?;
        extend(&mut system, &mut instance_manager, Direction::Right)?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::UngroupSlot,
        )?;

        assert_eq!(
            system.aggregates.configuration.slot_of_content(a),
            Some((ProjectId::ROOT, SlotPosition { column: 0, row: 0 }))
        );

        Ok(())
    })
    .await
}

/// Cmd+A from an instance passes the desktop's levels and leaves focus and selection alone, so the
/// application keeps it. From a launcher, the project whose matrix holds the launcher's slot
/// selects all of its slots around it, without moving the focus to a corner (ADR 0030).
#[tokio::test]
async fn cmd_a_selects_all_slots_around_a_launcher_but_not_from_an_instance() -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            b,
            ..
        } = two_launchers()?;
        let instance = start_instance(&mut system, &mut instance_manager, b, 0)?;
        let cmd_a = KeyInput::new(
            Key::Character("a".into()),
            ElementState::Pressed,
            false,
            ModifiersState::SUPER,
        );
        let press_cmd_a = |system: &mut DesktopSystem, instance_manager: &mut InstanceManager| {
            let changes = system.deliver_key_along_focus_path(&cmd_a)?;
            system.transact(changes, instance_manager, TransactionEffectsMode::Setup)
        };

        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Instance(instance),
        )?;
        press_cmd_a(&mut system, &mut instance_manager)?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance))
        );
        assert_eq!(system.selection, Selection::default());

        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(b),
        )?;
        press_cmd_a(&mut system, &mut instance_manager)?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(b))
        );
        assert_eq!(system.selection, Selection::from_diagonal(vec2(-1, 0)));

        Ok(())
    })
    .await
}

/// Esc and Cmd+Esc collapse an extended selection and keep the focus. Without an extended
/// selection, the desktop passes them on, so an application below keeps them (ADR 0030).
#[tokio::test]
async fn esc_collapses_only_an_extended_selection() -> Result<()> {
    for modifiers in [ModifiersState::empty(), ModifiersState::SUPER] {
        esc_collapses_only_an_extended_selection_with(modifiers).await?;
    }
    Ok(())
}

async fn esc_collapses_only_an_extended_selection_with(modifiers: ModifiersState) -> Result<()> {
    task_context::with_context(task_context(), async {
        let TwoLaunchers {
            mut system,
            mut instance_manager,
            a,
            ..
        } = two_launchers()?;
        focus(
            &mut system,
            &mut instance_manager,
            FocusableTarget::Launcher(a),
        )?;
        let esc = KeyInput::new(
            Key::Named(NamedKey::Escape),
            ElementState::Pressed,
            false,
            modifiers,
        );
        let desktop_takes_esc = |system: &DesktopSystem| {
            let context = KeyContext {
                is_focused: false,
                fully_zoomed_in: false,
                focused_slot_project: Some(ProjectId::ROOT),
                focus_in_instance: false,
                selection_is_extended: system.selection.is_extended(),
                hierarchy: &system.aggregates.hierarchy,
                configuration: &system.aggregates.configuration,
            };
            matches!(
                system.desktop_presenter.handle_key(&esc, &context),
                KeyOutcome::Consumed(_)
            )
        };

        assert!(!desktop_takes_esc(&system));

        extend(&mut system, &mut instance_manager, Direction::Right)?;
        assert!(desktop_takes_esc(&system));
        let changes = system.deliver_key_along_focus_path(&esc)?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        assert_eq!(system.selection, Selection::default());
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(a))
        );

        Ok(())
    })
    .await
}
