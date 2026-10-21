use super::*;
use crate::desktop_system::selection::SelectionDelta;

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
        let columns = |columns| SelectionDelta::new(columns, 0);

        extend(&mut system, &mut instance_manager, Direction::Right)?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(a))
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

/// With a selection beyond the focused slot, the outline covers the rectangle of the selected
/// placements: for two neighboring launchers, it is centered between them and wider than either.
#[tokio::test]
async fn the_outline_covers_the_selected_placements() -> Result<()> {
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
/// placement, keeping their arrangement. Cmd+Shift+G gives them back and selects them again
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
            panic!("a project takes the selection's top-left placement");
        };
        assert_eq!(configuration.slots_ordered(ProjectId::ROOT).count(), 1);
        assert_eq!(configuration.slot_of_content(a), Some((group, at(0))));
        assert_eq!(configuration.slot_of_content(b), Some((group, at(1))));
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Project(group))
        );
        assert_eq!(system.selection.delta(), SelectionDelta::zero());

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
        assert_eq!(system.selection.delta(), SelectionDelta::new(1, 0));

        Ok(())
    })
    .await
}
