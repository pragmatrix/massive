use super::*;

/// Regression (ADR 0014): toggling Full Screen Mode must rescale the focused instance's
/// view in the same transaction, not only after focus moved away and back.
/// The fullscreen scale lives on the instance's view placement, so the
/// toggle must invalidate the whole launcher subtree (views included), not
/// just the launcher's own measurement.
#[tokio::test]
async fn toggling_full_screen_mode_rescales_the_focused_view_in_place() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system();
        let mut instance_manager = instance_manager(&receiver);
        let frame = massive_applications::begin_frame();
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            // Tests at panel size: the system's constructor already
            // committed the 800×600 window state.
            let changes = system.plan(DesktopCommand::Configuration(command))?;
            system.transact(changes, &mut instance_manager, TransactionEffectsMode::Setup)?;
        }

        // A native fullscreen 1000×800 window on the 800×600 panel: the fullscreen toggle
        // frames content at the window, so the tests need the larger state
        // committed through its change.
        system.transact(
            DesktopChange::WindowResized(WindowState::new(SizePx::new(1000, 800), true)),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        let launcher = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        let instance = uuid::Uuid::new_v4().into();
        let start = system.plan(DesktopCommand::StartInstance {
            launcher,
            instance,
            root: Some(InstanceRoot::new()),
            parameters: Default::default(),
            kind: InstanceKind::Primary,
        })?;
        let mut submission_changes = massive_util::ChangeSet::default();
        submission_changes.push(InstanceChange::CreateView(
            massive_applications::ViewCreationInfo {
                id: uuid::Uuid::new_v4().into(),
                role: ViewRole::Primary,
                extents: massive_geometry::BoxPx::new(
                    massive_geometry::PointPx::new(0, 0),
                    massive_geometry::PointPx::new(800, 600),
                ),
            },
        ));
        let initial_submission = InstanceSubmission::new(
            submission_changes,
            massive_renderer::RenderPacing::default(),
        );
        let mut changes: Changes = start;
        changes <<= DesktopChange::IntegrateInstanceSubmission(instance, initial_submission);
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup
)?;

        let view = system.aggregates.instances[&instance]
            .primary_view_id()
            .expect("the initial submission created a primary view");
        let scale_before = system
            .placement(&DesktopTarget::view(instance, view))
            .transform
            .scale;

        // Toggle through the live command path, like the desktop loop does.
        let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
        system.transact(
            changes,
            &mut instance_manager,
            Option::<TransactionEffectsMode>::None
)?;

        let scale_after = system
            .placement(&DesktopTarget::view(instance, view))
            .transform
            .scale;

        assert!(
            system.aggregates.configuration[launcher].full_screen_mode
                == crate::projects::FullScreenMode::FullScreen,
            "the toggle flipped the launcher's Full Screen Mode"
        );
        assert!(
            (scale_after - scale_before).abs() > 0.001,
            "the focused view must rescale in the toggle transaction: before {scale_before:?}, after {scale_after:?}"
        );

        // Toggle back off: the visor layout must collapse back onto panel
        // measurements in the same transaction — not only after the next
        // focus change or arc rotation (regression: stale view measurement).
        let instance_b = uuid::Uuid::new_v4().into();
        let start_b = system.plan(DesktopCommand::StartInstance {
            launcher,
            instance: instance_b,
            root: Some(InstanceRoot::new()),
            parameters: Default::default(),
            kind: InstanceKind::Primary,
        })?;
        let mut submission_b = massive_util::ChangeSet::default();
        submission_b.push(InstanceChange::CreateView(
            massive_applications::ViewCreationInfo {
                id: uuid::Uuid::new_v4().into(),
                role: ViewRole::Primary,
                extents: massive_geometry::BoxPx::new(
                    massive_geometry::PointPx::new(0, 0),
                    massive_geometry::PointPx::new(800, 600),
                ),
            },
        ));
        let mut changes_b: Changes = start_b;
        changes_b <<= DesktopChange::IntegrateInstanceSubmission(
            instance_b,
            InstanceSubmission::new(
                submission_b,
                massive_renderer::RenderPacing::default(),
            ),
        );
        system.transact(
            changes_b,
            &mut instance_manager,
            Option::<TransactionEffectsMode>::None,
        )?;

        let view_b = system.aggregates.instances[&instance_b]
            .primary_view_id()
            .expect("the second instance created a primary view");
        let view_b_size_fullscreen = system
            .placement(&DesktopTarget::view(instance_b, view_b))
            .rect
            .size;

        let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
        system.transact(
            changes,
            &mut instance_manager,
            Option::<TransactionEffectsMode>::None
)?;

        let view_b_size_after = system
            .placement(&DesktopTarget::view(instance_b, view_b))
            .rect
            .size;
        assert!(
            system.aggregates.configuration[launcher].full_screen_mode
                == crate::projects::FullScreenMode::Regular,
            "the second toggle restored the launcher's Regular mode"
        );
        assert!(
            (view_b_size_after[0] as f64 - 800.0).abs() < 0.01
                && view_b_size_fullscreen[0] as f64 >= 1000.0,
            "the visor view must be measured back to its panel size in the toggle-off transaction: fullscreen {view_b_size_fullscreen:?}, after {view_b_size_after:?}"
        );

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn fullscreen_request_enters_native_fullscreen_before_toggling_instance_mode() -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        assert_eq!(system.zoom_level, ZoomLevel::Focus);
        assert_eq!(system.focused_path().instance(), Some(instance));

        let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
        assert!(matches!(
            changes,
            Changes::One(DesktopChange::ToggleWindowFullScreen)
        ), "a windowed native window must enter fullscreen regardless of instance focus: {changes:?}");
        let output = system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Normal,
        )?;
        assert!(matches!(
            output.effects.as_slice(),
            [DesktopSystemEffect::ToggleWindowFullScreen]
        ));

        system.transact(
            DesktopChange::WindowResized(WindowState::new(SizePx::new(1000, 800), true)),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        let changes = system.plan(DesktopCommand::ToggleFullScreen)?;
        assert!(matches!(
            changes,
            Changes::One(DesktopChange::ToggleFullScreenMode(
                ToggleFullScreenModeTarget::Launcher(target)
            )) if target == launcher
        ));
        let output = system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Normal,
        )?;
        assert!(matches!(
            output.effects.as_slice(),
            [crate::desktop_system::change::DesktopSystemEffect::PersistConfiguration]
        ));

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn native_fullscreen_resize_keeps_the_presented_camera_on_the_focused_instance() -> Result<()>
{
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        deliver_view(&mut system, &mut instance_manager, instance)?;
        let focused = system.keyboard_focus.focused();

        for window_state in [
            WindowState::new(SizePx::new(1000, 864), false),
            WindowState::new(SizePx::new(2560, 1440), true),
        ] {
            system.transact(
                DesktopChange::WindowResized(window_state),
                &mut instance_manager,
                Option::<TransactionEffectsMode>::None,
            )?;
            assert_eq!(system.keyboard_focus.focused(), focused);
            assert_eq!(system.zoom_level, ZoomLevel::Focus);
            let desired = system
                .resolve_desired_camera()
                .expect("the instance is focused");
            assert_eq!(
                *system.camera(),
                desired,
                "native resize must present the focused camera without an intermediate zoom"
            );
        }

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

/// Regression (ADR 0014, ADR 0019): in Full Screen Mode the view and the title bar are measured at
/// window resolution, so a native window resize must remeasure both. A stale view keeps the
/// instance framed at the windowed size, and a stale title bar spans only part of the view once a
/// later focus change remeasures the view alone.
#[tokio::test]
async fn native_window_resize_remeasures_a_fullscreen_instance_at_window_resolution() -> Result<()>
{
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        deliver_view(&mut system, &mut instance_manager, instance)?;
        let view = system.aggregates.instances[&instance]
            .primary_view_id()
            .expect("the delivered view is the primary view");

        let window_size = SizePx::new(2560, 1440);
        system.transact(
            DesktopChange::WindowResized(WindowState::new(window_size, true)),
            &mut instance_manager,
            Option::<TransactionEffectsMode>::None,
        )?;

        let measured_width = |target: DesktopTarget| system.layout_state.measured(&target).size[0];
        assert_eq!(
            measured_width(DesktopTarget::view(instance, view)),
            window_size.width,
            "the view must be measured at the resized window's width"
        );
        assert_eq!(
            measured_width(DesktopTarget::title_bar(instance)),
            window_size.width,
            "the title bar must be measured at the resized window's width"
        );

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

/// A primary instance created in a Full Screen launcher spawns at window
/// resolution and frames fullscreen from its view-less first commit;
/// framing it at the panel distance dollies the camera out and back in when
/// the view arrives (the `Cmd+T` zoom-out bounce).
#[tokio::test]
async fn a_new_base_instance_in_a_fullscreen_launcher_frames_fullscreen_from_its_first_commit()
-> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(CONFIG)?;

        // A real spawn: root None, so plan emits SpawnInstance with the
        // seed parameters whose `size_px` this test pins.
        let start = system.plan(DesktopCommand::StartInstance {
            launcher,
            instance: uuid::Uuid::new_v4().into(),
            root: None,
            parameters: Default::default(),
            kind: InstanceKind::Primary,
        })?;
        let seed = start
            .iter()
            .find_map(|change| match change {
                DesktopChange::SpawnInstance { parameters, .. } => {
                    parameters.get("size_px").cloned()
                }
                _ => None,
            })
            .expect("SpawnInstance seeds the application canvas size");
        assert_eq!(
            seed,
            serde_json::json!([1000, 800 - system.aggregates.instance_title_bar_metrics.height]),
            "a fullscreen launcher's spawned instance must start at window resolution below the title bar, not panel"
        );

        // The plan's SpawnInstance would hit the (empty) test application
        // registry, so the instance is presented directly.
        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;

        let panel_distance = massive_geometry::PixelCamera::pixel_perfect_distance(
            massive_geometry::PixelCamera::DEFAULT_FOVY,
        );
        // The window-resolution stack (title bar and view) fits the instance extent: panel and bar.
        let fullscreen_distance = panel_distance
            * (800.0_f64 / 1000.0).min((600.0 + system.aggregates.instance_title_bar_metrics.height as f64) / 800.0);
        let view_less_distance = camera_distance(&system);
        assert!(
            (view_less_distance - fullscreen_distance).abs() < 1e-6,
            "a view-less fullscreen instance must frame at the fullscreen distance: expected {fullscreen_distance}, got {view_less_distance}"
        );

        deliver_view(&mut system, &mut instance_manager, instance)?;
        let view_distance = camera_distance(&system);
        assert!(
            (view_distance - fullscreen_distance).abs() < 1e-6,
            "the fullscreen instance keeps the fullscreen distance once its view arrives: expected {fullscreen_distance}, got {view_distance}"
        );

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}
