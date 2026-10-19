use super::*;

#[tokio::test]
async fn startup_setup_snaps_camera_to_the_focused_instance() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system();
        let mut instance_manager = instance_manager(&receiver);
        let launcher = system
            .aggregates
            .configuration
            .boot_launcher()
            .expect("the test configuration has a startup launcher");
        let frame = massive_applications::begin_frame();
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Configuration(command))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
        }

        let instance = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        deliver_view(&mut system, &mut instance_manager, instance)?;

        let expected = system
            .resolve_desired_camera()
            .expect("a focused target must have a camera");
        assert_eq!(
            *system.camera(),
            expected,
            "setup should snap to the focused instance camera without an initial transition"
        );

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

/// The camera and the hover read a target's absolute placement, so the scene must
/// resolve that target to the same transform — a project nested in a parent slot
/// included.
#[tokio::test]
async fn nested_project_launcher_scene_transform_matches_its_placement() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(NESTED_PROJECT_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        let frame = massive_applications::begin_frame();
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Configuration(command))?;
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
        // Submitting the frame runs the queued movements that move the launcher's scene transform.
        drop(frame.submission::<SceneChange>());
        let location = system.aggregates.launchers[&launcher].location();
        let resolved = massive_scene::TransformResolver::default()
            .resolve(&location.to_ref())
            .transform;

        let placement = system.placement(&DesktopTarget::Launcher(launcher));
        let rect_px: RectPx = placement.rect.into();
        let local_center = Rect::from(rect_px).size().to_rect().center();
        let expected = placement.transform.to_origin_space(local_center);

        assert_eq!(resolved.scale, expected.scale);
        let offset = resolved.translate - expected.translate;
        assert!(
            offset.x.abs() < 0.001 && offset.y.abs() < 0.001 && offset.z.abs() < 0.001,
            "the scene draws the launcher at {resolved:?}, but the model places it at {expected:?}"
        );

        Ok(())
    })
    .await
}

/// The same while the launcher sits in a nested project's scaled slot.
#[tokio::test]
async fn cmd_t_in_a_nested_slot_keeps_camera_stable() -> Result<()> {
    cmd_t_keeps_camera_stable(NESTED_PROJECT_CONFIG).await
}

/// The startup launcher nested in a project, so it sits inside a project slot.
const NESTED_PROJECT_CONFIG: &str = r#"{
    "startup": "/labs/shell",
    "slots": [
        { "at": [0, 0], "project": { "name": "labs", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor" } }
        ] } }
    ]
}"#;

async fn cmd_t_keeps_camera_stable(config: &str) -> Result<()> {
    task_context::with_context(task_context(), async {
        let frame = massive_applications::begin_frame();
        let (mut system, mut instance_manager, launcher) = fullscreen_system(config)?;

        // Instance A's framing is the baseline: in a nested slot it carries
        // the slot scale.
        let instance_a = start_instance(&mut system, &mut instance_manager, launcher, 0)?;
        deliver_view(&mut system, &mut instance_manager, instance_a)?;
        let distance = camera_distance(&system);

        // `Cmd+T`: instance B is focused one commit before its view exists.
        let instance_b = start_instance(&mut system, &mut instance_manager, launcher, 1)?;
        let view_less_distance = camera_distance(&system);
        assert!(
            (view_less_distance - distance).abs() < 1e-6,
            "the camera must not zoom out while the new instance is view-less: expected {distance}, got {view_less_distance}"
        );

        deliver_view(&mut system, &mut instance_manager, instance_b)?;
        let view_distance = camera_distance(&system);
        assert!(
            (view_distance - distance).abs() < 1e-6,
            "the camera must not move once the view arrives: expected {distance}, got {view_distance}"
        );

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}
