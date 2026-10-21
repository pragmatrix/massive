use super::*;

#[tokio::test]
async fn navigation_escapes_to_the_parent_and_restores_focus_on_return() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
            ..
        } = nested_siblings()?;

        for _ in 0..2 {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Zoom(Zoom::Out),
            )?;
        }
        assert_eq!(
            system.zoom_navigation().focused_zoom_level(),
            Some((ZoomLevel::Row, 1))
        );

        // `labs-a` holds a single slot, so there is nothing to its right inside it: the
        // navigation escapes to the sibling project, which was visited and so is re-entered
        // at its focus leaf. Navigating back restores the original focus.
        let navigate = |system: &mut DesktopSystem, manager: &mut InstanceManager, direction| {
            run(system, manager, DesktopCommand::Navigate(direction))
        };
        navigate(
            &mut system,
            &mut instance_manager,
            crate::desktop_system::Direction::Right,
        )?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_b))
        );
        assert_eq!(
            system
                .zoom_navigation()
                .focused_zoom_level()
                .map(|(level, _)| level),
            Some(ZoomLevel::Row)
        );

        navigate(
            &mut system,
            &mut instance_manager,
            crate::desktop_system::Direction::Left,
        )?;
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn zooming_out_in_the_root_keeps_focus_on_the_root_slot() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, receiver) = system();
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Configuration(command),
            )?;
        }
        let launcher = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;

        for expected in [ZoomLevel::Slot, ZoomLevel::Row, ZoomLevel::Project] {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Zoom(Zoom::Out),
            )?;
            assert_eq!(
                system.zoom_navigation().focused_zoom_level(),
                Some((expected, 0))
            );
            assert_eq!(
                system.keyboard_focus.focused(),
                Some(FocusableTarget::Instance(instance))
            );
        }
        assert!(system.plan(DesktopCommand::Zoom(Zoom::Out))?.is_empty());

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

/// Esc and Cmd+Esc climb one level at a time (ADR 0031). Plain Esc stays with an instance's
/// application, and Cmd+Esc escapes the instance to its launcher at the same zoom level. Esc then
/// escapes the nested project, which takes the focus, and in the root project it frames the whole
/// root, where it stops.
#[tokio::test]
async fn esc_climbs_from_an_instance_to_the_root_frame() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let Some(SlotContent::Launcher(launcher_a)) = system
            .aggregates
            .hierarchy
            .slot_content_of_target(&DesktopTarget::Instance(instance_a, InstancePart::Instance))
        else {
            panic!("the instance belongs to a launcher");
        };
        let labs_a = system
            .aggregates
            .hierarchy
            .project_of_target(&DesktopTarget::Launcher(launcher_a));
        let press = |system: &mut DesktopSystem,
                     instance_manager: &mut InstanceManager,
                     modifiers: ModifiersState| {
            let esc = KeyInput::new(
                Key::Named(NamedKey::Escape),
                ElementState::Pressed,
                false,
                modifiers,
            );
            let changes = system.deliver_key_along_focus_path(&esc)?;
            system.transact(changes, instance_manager, TransactionEffectsMode::Setup)
        };
        let focused = |system: &DesktopSystem| system.keyboard_focus.focused();
        let zoom_level = system.zoom_level;

        press(&mut system, &mut instance_manager, ModifiersState::empty())?;
        assert_eq!(
            focused(&system),
            Some(FocusableTarget::Instance(instance_a))
        );

        press(&mut system, &mut instance_manager, ModifiersState::SUPER)?;
        assert_eq!(
            focused(&system),
            Some(FocusableTarget::Launcher(launcher_a))
        );
        assert_eq!(system.zoom_level, zoom_level);

        press(&mut system, &mut instance_manager, ModifiersState::empty())?;
        assert_eq!(focused(&system), Some(FocusableTarget::Project(labs_a)));
        assert_eq!(system.zoom_level, ZoomLevel::Focus);

        press(&mut system, &mut instance_manager, ModifiersState::empty())?;
        assert_eq!(focused(&system), Some(FocusableTarget::Project(labs_a)));
        assert_eq!(system.zoom_level, ZoomLevel::Project);

        press(&mut system, &mut instance_manager, ModifiersState::empty())?;
        assert_eq!(focused(&system), Some(FocusableTarget::Project(labs_a)));
        assert_eq!(system.zoom_level, ZoomLevel::Project);

        drop(frame);
        Ok(())
    })
    .await
}
