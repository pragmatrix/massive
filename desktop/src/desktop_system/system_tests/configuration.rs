use massive_applications::{AccessScope, McpConfiguration};

use super::*;

#[tokio::test]
async fn a_second_parentless_assign_slot_is_rejected() -> Result<()> {
    task_context::with_context(task_context(), async {
        let (mut system, receiver) = system();
        let mut instance_manager = instance_manager(&receiver);

        let changes = system.plan(DesktopCommand::Configuration(
            ConfigurationCommand::AssignSlot {
                parent: None,
                slot: SlotPosition { column: 0, row: 0 },
                content: SlotAssignment::Project {
                    id: ProjectId::ROOT,
                    name: crate::projects::ROOT_PROJECT_NAME.into(),
                },
                shift: SlotShift::default(),
            },
        ))?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        for id in [ProjectId::ROOT, ProjectId::new()] {
            assert!(
                system
                    .plan(DesktopCommand::Configuration(
                        ConfigurationCommand::AssignSlot {
                            parent: None,
                            slot: SlotPosition { column: 0, row: 0 },
                            content: SlotAssignment::Project {
                                id,
                                name: "again".into(),
                            },
                            shift: SlotShift::default(),
                        }
                    ))
                    .is_err(),
                "a second parentless AssignSlot must be rejected"
            );
        }

        Ok(())
    })
    .await
}

/// Cmd+N inserts between the focused launcher and its right neighbour, which shifts on, and the
/// new launcher takes the focus.
#[tokio::test]
async fn add_launcher_inserts_right_of_the_focused_slot_and_focuses_it() -> Result<()> {
    task_context::with_context(task_context(), async {
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
        let at = |column| SlotPosition { column, row: 0 };
        let a = system.aggregates.configuration.boot_launcher().unwrap();
        let b = system
            .aggregates
            .configuration
            .content_at(ProjectId::ROOT, at(1))
            .unwrap();
        system.transact(
            set_focus(
                Some(FocusableTarget::Launcher(a)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::AddLauncher),
        )?;

        let configuration = &system.aggregates.configuration;
        let Some(SlotContent::Launcher(added)) = configuration.content_at(ProjectId::ROOT, at(1))
        else {
            panic!("a launcher is added right of the focused one");
        };
        assert_ne!(SlotContent::Launcher(added), b);
        assert_eq!(configuration.content_at(ProjectId::ROOT, at(2)), Some(b));
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(added))
        );

        Ok(())
    })
    .await
}

/// A slot that moves to another project keeps its running instances and the keyboard focus. The
/// launcher is re-parented together with everything below it, not removed and assigned again.
#[tokio::test]
async fn move_slot_across_projects_keeps_instances_and_focus() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let (project_a, slot_a) = system
            .aggregates
            .configuration
            .slot_of_content(launcher_a)
            .unwrap();
        let project_b = system
            .aggregates
            .configuration
            .slots_ordered(ProjectId::ROOT)
            .find_map(|(_, content)| match content {
                SlotContent::Project(project) if project != project_a => Some(project),
                _ => None,
            })
            .unwrap();
        let dest = SlotPosition { column: 1, row: 0 };

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Configuration(ConfigurationCommand::MoveSlot {
                source: (project_a, slot_a),
                dest: (project_b, dest),
            }),
        )?;

        assert_eq!(
            system.aggregates.configuration.slot_of_content(launcher_a),
            Some((project_b, dest))
        );
        assert_eq!(
            system
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::Launcher(launcher_a)),
            Some(&DesktopTarget::matrix(project_b))
        );
        assert_eq!(
            system.aggregates.hierarchy.launcher_of_instance(instance_a),
            launcher_a
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );

        Ok(())
    })
    .await
}

/// A project slot that moves into another project carries its own slots and their instances along.
#[tokio::test]
async fn move_slot_across_projects_carries_a_nested_project() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
        } = nested_siblings()?;
        let hierarchy = &system.aggregates.hierarchy;
        let project_of = |instance| {
            hierarchy.project_of_target(&DesktopTarget::Launcher(
                hierarchy.launcher_of_instance(instance),
            ))
        };
        let (project_a, project_b) = (project_of(instance_a), project_of(instance_b));
        let (_, slot_b) = system
            .aggregates
            .configuration
            .slot_of_content(project_b)
            .unwrap();
        let dest = SlotPosition { column: 1, row: 0 };

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::Configuration(ConfigurationCommand::MoveSlot {
                source: (ProjectId::ROOT, slot_b),
                dest: (project_a, dest),
            }),
        )?;

        let hierarchy = &system.aggregates.hierarchy;
        assert_eq!(
            hierarchy.parent(&DesktopTarget::Project(project_b, ProjectPart::Project)),
            Some(&DesktopTarget::matrix(project_a))
        );
        assert_eq!(hierarchy.parent_project_of(project_b), Some(project_a));
        assert_eq!(
            hierarchy.project_of_target(&DesktopTarget::Launcher(
                hierarchy.launcher_of_instance(instance_b)
            )),
            project_b
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Instance(instance_a))
        );

        Ok(())
    })
    .await
}

/// Cmd+G wraps the content of the focused slot into a new project in the content's slot.
/// The content moves into the project together with its running instance. Afterwards, the parent
/// holds only the new project, and the new project has the focus.
#[tokio::test]
async fn group_wraps_the_focused_slot_in_a_new_project_in_its_slot() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let (project_a, slot_a) = system
            .aggregates
            .configuration
            .slot_of_content(launcher_a)
            .unwrap();

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::Group),
        )?;

        let configuration = &system.aggregates.configuration;
        let Some(SlotContent::Project(group)) = configuration.content_at(project_a, slot_a) else {
            panic!("a project takes the grouped content's slot");
        };
        assert_eq!(
            configuration.slot_of_content(launcher_a),
            Some((group, SlotPosition { column: 0, row: 0 }))
        );
        assert_eq!(configuration.slots_ordered(project_a).count(), 1);
        assert_eq!(
            system.aggregates.hierarchy.launcher_of_instance(instance_a),
            launcher_a
        );
        assert_eq!(
            system
                .aggregates
                .hierarchy
                .parent(&DesktopTarget::Launcher(launcher_a)),
            Some(&DesktopTarget::matrix(group))
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Project(group))
        );

        Ok(())
    })
    .await
}

/// Cmd+Shift+G on a project that Cmd+G created gives back exactly what was grouped. The content
/// returns to its slot with its running instance and takes the focus, and the project is gone.
#[tokio::test]
async fn ungroup_reverses_group() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let slot_a = system
            .aggregates
            .configuration
            .slot_of_content(launcher_a)
            .unwrap();
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::Group),
        )?;
        let Some(FocusableTarget::Project(group)) = system.keyboard_focus.focused() else {
            panic!("grouping focuses the new project");
        };

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::Ungroup),
        )?;

        let configuration = &system.aggregates.configuration;
        assert_eq!(configuration.slot_of_content(launcher_a), Some(slot_a));
        assert!(configuration.project(group).is_none());
        assert!(
            !system
                .aggregates
                .hierarchy
                .exists(&DesktopTarget::Project(group, ProjectPart::Project))
        );
        assert_eq!(
            system.aggregates.hierarchy.launcher_of_instance(instance_a),
            launcher_a
        );
        assert_eq!(
            system.keyboard_focus.focused(),
            Some(FocusableTarget::Launcher(launcher_a))
        );

        Ok(())
    })
    .await
}

/// Cmd+Shift+W removes a project only if no instance runs anywhere below it. A project whose
/// launcher runs an instance is kept, while a project with an idle launcher is removed together
/// with that launcher.
#[tokio::test]
async fn remove_slot_keeps_projects_with_instances_at_any_depth() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let project_a = system
            .aggregates
            .hierarchy
            .project_of_target(&DesktopTarget::Launcher(launcher_a));

        system.transact(
            set_focus(
                Some(FocusableTarget::Project(project_a)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::Remove),
        )?;
        assert!(system.aggregates.configuration.project(project_a).is_some());

        // An idle launcher, grouped into a project of its own.
        system.transact(
            set_focus(
                Some(FocusableTarget::Launcher(launcher_a)),
                KeyboardFocusReason::InputTransition,
            ),
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::AddLauncher),
        )?;
        let Some(FocusableTarget::Launcher(idle)) = system.keyboard_focus.focused() else {
            panic!("the added launcher takes the focus");
        };
        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::Group),
        )?;
        let Some(FocusableTarget::Project(idle_project)) = system.keyboard_focus.focused() else {
            panic!("grouping focuses the new project");
        };

        run(
            &mut system,
            &mut instance_manager,
            DesktopCommand::ConfigureSelection(SelectionOperation::Remove),
        )?;

        assert!(
            system
                .aggregates
                .configuration
                .slot_of_content(idle_project)
                .is_none()
        );
        // The removed project and its launcher leave the configuration, and their presenters go too.
        assert!(
            system
                .aggregates
                .configuration
                .project(idle_project)
                .is_none()
        );
        assert!(system.aggregates.configuration.launcher(idle).is_none());
        assert!(system.aggregates.projects.get(&idle_project).is_none());
        assert!(system.aggregates.launchers.get(&idle).is_none());
        let hierarchy = &system.aggregates.hierarchy;
        assert!(!hierarchy.exists(&DesktopTarget::Project(idle_project, ProjectPart::Project)));
        assert!(!hierarchy.exists(&DesktopTarget::Launcher(idle)));
        assert_eq!(
            system.aggregates.hierarchy.launcher_of_instance(instance_a),
            launcher_a
        );

        Ok(())
    })
    .await
}

/// Cmd+Shift+W keeps a project while an instance runs below it, but the CLI's `remove` stops the
/// instance and removes the project.
#[tokio::test]
async fn a_removal_request_stops_the_instances_below() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let project_a = system
            .aggregates
            .hierarchy
            .project_of_target(&DesktopTarget::Launcher(launcher_a));

        request(
            &mut system,
            &mut instance_manager,
            instance_b,
            massive_applications::ConfigurationRequest::Remove {
                target: massive_applications::ConfigurationTarget::Project,
                name: Some("labs-a".into()),
            },
        )?;

        assert!(system.aggregates.configuration.project(project_a).is_none());
        assert!(!system.aggregates.hierarchy.exists(&instance_a.into()));

        Ok(())
    })
    .await
}

/// The CLI's `move` onto an assigned neighbor swaps the two, so the launcher still moves by one
/// slot.
#[tokio::test]
async fn a_move_request_onto_an_assigned_neighbor_swaps() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            ..
        } = nested_siblings()?;
        let launcher_a = system.aggregates.hierarchy.launcher_of_instance(instance_a);
        let (project_a, slot_a) = system
            .aggregates
            .configuration
            .slot_of_content(launcher_a)
            .unwrap();
        request(
            &mut system,
            &mut instance_manager,
            instance_a,
            massive_applications::ConfigurationRequest::Add {
                target: massive_applications::ConfigurationTarget::Launcher,
                name: None,
            },
        )?;
        let neighbor_slot = SlotPosition {
            column: slot_a.column + 1,
            row: slot_a.row,
        };
        let neighbor = system
            .aggregates
            .configuration
            .content_at(project_a, neighbor_slot)
            .expect("the added launcher sits right of the instance's launcher");

        request(
            &mut system,
            &mut instance_manager,
            instance_a,
            massive_applications::ConfigurationRequest::MoveLauncher {
                direction: Direction::Right,
            },
        )?;

        let configuration = &system.aggregates.configuration;
        assert_eq!(
            configuration.slot_of_content(launcher_a),
            Some((project_a, neighbor_slot))
        );
        assert_eq!(configuration.content_at(project_a, slot_a), Some(neighbor));

        Ok(())
    })
    .await
}

/// The root project is never an MCP root, MCP roots never nest, and a process inside an MCP root cannot configure MCP, not even for an MCP
/// root that does not cover it (ADR 0036).
#[tokio::test]
async fn mcp_is_configured_only_from_outside_every_mcp_root_and_roots_never_nest() -> Result<()> {
    task_context::with_context(task_context(), async {
        let NestedSiblings {
            mut system,
            mut instance_manager,
            instance_a,
            instance_b,
        } = nested_siblings()?;
        let configure_mcp = |path: &str, mcp: Option<McpConfiguration>| {
            massive_applications::ConfigurationRequest::ConfigureMcp {
                path: Some(path.into()),
                mcp,
            }
        };
        let observe = McpConfiguration {
            access_scopes: [AccessScope::Observe].into(),
        };
        request(
            &mut system,
            &mut instance_manager,
            instance_a,
            massive_applications::ConfigurationRequest::Assign {
                target: massive_applications::ConfigurationTarget::Project,
                name: "inner".into(),
                column: 1,
                row: 0,
                under: None,
                shift: SlotShift::default(),
            },
        )?;
        let configuration = &system.aggregates.configuration;
        let labs_a = configuration
            .resolve_project_path(ProjectId::ROOT, "/labs-a")
            .expect("labs-a resolves");
        let inner = configuration
            .resolve_project_path(ProjectId::ROOT, "/labs-a/inner")
            .expect("inner resolves");
        let labs_b = configuration
            .resolve_project_path(ProjectId::ROOT, "/labs-b")
            .expect("labs-b resolves");

        request(
            &mut system,
            &mut instance_manager,
            instance_b,
            configure_mcp("/labs-a/inner", Some(observe.clone())),
        )?;
        request(
            &mut system,
            &mut instance_manager,
            instance_b,
            configure_mcp(
                "/labs-a",
                Some(McpConfiguration {
                    access_scopes: [AccessScope::Act].into(),
                }),
            ),
        )?;
        request(
            &mut system,
            &mut instance_manager,
            instance_a,
            configure_mcp("/", Some(observe.clone())),
        )?;
        let configuration = &system.aggregates.configuration;
        assert_eq!(configuration[inner].mcp, Some(observe.clone()));
        assert_eq!(configuration[labs_a].mcp, None);
        assert_eq!(configuration[ProjectId::ROOT].mcp, None);

        request(
            &mut system,
            &mut instance_manager,
            instance_a,
            configure_mcp("/labs-b", Some(observe.clone())),
        )?;
        assert_eq!(
            system.aggregates.configuration[labs_b].mcp,
            Some(observe.clone())
        );

        request(
            &mut system,
            &mut instance_manager,
            instance_b,
            configure_mcp("/labs-a/inner", None),
        )?;
        assert_eq!(
            system.aggregates.configuration[inner].mcp,
            Some(observe.clone())
        );

        Ok(())
    })
    .await
}
