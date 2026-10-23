use super::*;

const LONG_AND_SHORT_NAMES_CONFIG: &str = r#"{
    "startup": "/a/shell-a",
    "slots": [
        { "at": [0, 0], "project": { "name": "a", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell-a", "mode": "visor" } }
        ] } },
        { "at": [1, 0], "project": { "name": "a project name far wider than any slot", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell-b", "mode": "visor" } }
        ] } }
    ]
}"#;

/// A project's header measures no width, so its name never changes the project's size, nor,
/// through the shared scale, its siblings' (ADR 0034).
#[tokio::test]
async fn a_project_name_does_not_change_the_project_size() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system_from(LONG_AND_SHORT_NAMES_CONFIG);
        let mut instance_manager = instance_manager(&receiver);
        for command in crate::projects::to_commands(&system.aggregates.configuration) {
            run(
                &mut system,
                &mut instance_manager,
                DesktopCommand::Configuration(command),
            )?;
        }

        let measured_sizes: Vec<_> = system
            .aggregates
            .configuration
            .slots_ordered(ProjectId::ROOT)
            .map(|(_, content)| match content {
                SlotContent::Project(project) => {
                    system
                        .layout_state
                        .measured(&DesktopTarget::Project(project, ProjectPart::Project))
                        .size
                }
                SlotContent::Launcher(_) => panic!("the root holds only projects"),
            })
            .collect();
        assert_eq!(measured_sizes.len(), 2);
        assert_eq!(measured_sizes[0], measured_sizes[1]);
        Ok(())
    })
    .await
}
