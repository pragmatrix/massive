use super::*;
use crate::projects::LauncherMode;

const VISOR_AND_BAND_CONFIG: &str = r#"{
    "startup": "/visor",
    "slots": [
        { "at": [0, 0], "launcher": { "name": "visor", "mode": "visor" } },
        { "at": [1, 0], "launcher": { "name": "band", "mode": "band" } }
    ]
}"#;

/// A launcher lines up with the instances it starts, so it spans their extent, title bar
/// included (ADR 0019): a visor by its fixed measure, an empty band by its stretchable one.
#[tokio::test]
async fn launchers_span_the_extent_of_an_instance() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(VISOR_AND_BAND_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        let frame = massive_applications::begin_frame();
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Project(command))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
        }
        let launcher_of_mode = |mode| {
            system
                .aggregates
                .configuration
                .launchers()
                .find(|launcher| launcher.mode == mode)
                .expect("the test configuration has a launcher of each mode")
                .id
        };
        let visor = launcher_of_mode(LauncherMode::Visor);
        let band = launcher_of_mode(LauncherMode::Band);

        let instance = start_instance(&mut system, &mut instance_manager, visor, 0)?;

        let size_of = |target| system.placement(&target).rect.size;
        let instance_size = size_of(DesktopTarget::Instance(instance));
        assert_eq!(size_of(DesktopTarget::Launcher(visor)), instance_size);
        assert_eq!(size_of(DesktopTarget::Launcher(band)), instance_size);

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}

const ROW_WITH_PROJECT_CONFIG: &str = r#"{
    "startup": "/busy",
    "slots": [
        { "at": [0, 0], "launcher": { "name": "empty", "mode": "visor" } },
        { "at": [1, 0], "launcher": { "name": "busy", "mode": "visor" } },
        { "at": [2, 0], "project": { "name": "nested", "slots": [
            { "at": [0, 0], "launcher": { "name": "inner", "mode": "visor" } }
        ] } }
    ]
}"#;

/// Only an empty launcher stretches to its row, which a nested project makes taller than an
/// instance; instances give a launcher their intrinsic extent instead.
#[tokio::test]
async fn only_empty_launchers_stretch_to_their_row() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(ROW_WITH_PROJECT_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        let frame = massive_applications::begin_frame();
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            let changes = system.plan(DesktopCommand::Project(command))?;
            system.transact(
                changes,
                &mut instance_manager,
                TransactionEffectsMode::Setup,
            )?;
        }
        let launcher_named = |name| {
            system
                .aggregates
                .configuration
                .launchers()
                .find(|launcher| launcher.name == name)
                .expect("the test configuration names the launcher")
                .id
        };
        let empty = launcher_named("empty");
        let busy = launcher_named("busy");
        let root = system
            .aggregates
            .configuration
            .project_of_launcher(busy)
            .expect("the launcher is in a project")
            .id;

        let instance = start_instance(&mut system, &mut instance_manager, busy, 0)?;

        let size_of = |target| system.placement(&target).rect.size;
        let row_height = size_of(DesktopTarget::ProjectMatrix(root))[1];
        let instance_size = size_of(DesktopTarget::Instance(instance));
        assert!(row_height > instance_size[1]);
        assert_eq!(size_of(DesktopTarget::Launcher(empty))[1], row_height);
        assert_eq!(size_of(DesktopTarget::Launcher(busy)), instance_size);

        drop(frame.submission::<SceneChange>());
        Ok(())
    })
    .await
}
