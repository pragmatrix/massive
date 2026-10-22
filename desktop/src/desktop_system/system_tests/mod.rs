//! System-level tests: they boot a `DesktopSystem` from a configuration and drive it through
//! commands and changes.

use std::path::Path;

use massive_animation::{AnimationCoordinator, MovementRuntime};
use massive_applications::task_context::{self, TaskContext};
use massive_applications::{InstanceChange, InstanceEnvironment, InstanceSubmission, ViewRole};
use massive_geometry::{Rect, RectPx, SizePx};
use massive_renderer::{FontManager, ShapingEngineKind};
use massive_scene::{AnyCollector, SceneChange};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState, NamedKey};

use super::change::{
    DesktopSystemEffect, InstancePresentation, ToggleFullScreenModeTarget, TopologyChange, Zoom,
    set_focus,
};
use super::focus_input::pointer_press_focus;
use super::*;
use crate::desktop_environment::DesktopEnvironment;
use crate::instance_manager::InstanceManager;
use crate::instance_presenter::{InstanceKind, InstanceRoot, InstanceTitleBarMetrics};
use crate::projects::persistence::parse_configuration;
use crate::projects::{SlotAssignment, SlotContent, SlotPosition};
use crate::window_state::WindowState;

mod camera;
mod configuration;
mod focus;
mod fullscreen;
mod navigation;
mod selection;

/// Tests run at the scale factor of a standard display.
const TEST_SCALE_FACTOR: f64 = 1.0;

const CONFIG: &str = r#"{
    "startup": "/shell",
    "slots": [
        { "at": [0, 0], "launcher": { "name": "shell", "mode": "visor", "params": { "command": "echo" } } }
    ]
}"#;

const NESTED_SIBLINGS_CONFIG: &str = r#"{
    "startup": "/labs-a/shell-a",
    "slots": [
        { "at": [0, 0], "project": { "name": "labs-a", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell-a", "mode": "visor" } }
        ] } },
        { "at": [1, 0], "project": { "name": "labs-b", "slots": [
            { "at": [0, 0], "launcher": { "name": "shell-b", "mode": "visor" } }
        ] } }
    ]
}"#;

/// The nested siblings booted, with one instance started per sibling and `labs-a`'s focused.
struct NestedSiblings {
    system: DesktopSystem,
    instance_manager: InstanceManager,
    instance_a: InstanceId,
    instance_b: InstanceId,
}

fn nested_siblings() -> Result<NestedSiblings> {
    let (mut system, receiver) = system_from(NESTED_SIBLINGS_CONFIG);
    let mut instance_manager = instance_manager(&receiver);
    for command in crate::projects::to_commands(&system.aggregates.configuration) {
        let changes = system.plan(DesktopCommand::Configuration(command))?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
    }

    let launcher_a = system
        .aggregates
        .configuration
        .boot_launcher()
        .expect("the test configuration has a startup launcher");
    let project_a = system
        .aggregates
        .hierarchy
        .project_of_target(&DesktopTarget::Launcher(launcher_a));
    let project_b = system
        .aggregates
        .configuration
        .slots_ordered(ProjectId::ROOT)
        .find_map(|(_, content)| match content {
            SlotContent::Project(project) if project != project_a => Some(project),
            _ => None,
        })
        .expect("the root has a sibling project");
    let launcher_b = system
        .aggregates
        .configuration
        .slots_ordered(project_b)
        .find_map(|(_, content)| match content {
            SlotContent::Launcher(launcher) => Some(launcher),
            SlotContent::Project(_) => None,
        })
        .expect("the sibling project has a launcher");

    let instance_a = start_instance(&mut system, &mut instance_manager, launcher_a, 0)?;
    let instance_b = start_instance(&mut system, &mut instance_manager, launcher_b, 0)?;
    system.transact(
        set_focus(
            Some(FocusableTarget::Instance(instance_a)),
            KeyboardFocusReason::InputTransition,
        ),
        &mut instance_manager,
        TransactionEffectsMode::Setup,
    )?;

    Ok(NestedSiblings {
        system,
        instance_manager,
        instance_a,
        instance_b,
    })
}

fn run(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    command: DesktopCommand,
) -> Result<()> {
    let changes = system.plan(command)?;
    system.transact(changes, instance_manager, TransactionEffectsMode::Setup)?;
    Ok(())
}

fn task_context() -> TaskContext {
    TaskContext::new(
        AnyCollector::for_type::<SceneChange>(),
        AnimationCoordinator::new(),
        MovementRuntime::default(),
        FontManager::system(ShapingEngineKind::available()[0])
            .expect("system fonts are available")
            .new_shaping_context(),
    )
}

fn system() -> (
    DesktopSystem,
    UnboundedReceiver<(InstanceId, InstanceSubmission)>,
) {
    system_from(CONFIG)
}

fn system_from(
    config: &str,
) -> (
    DesktopSystem,
    UnboundedReceiver<(InstanceId, InstanceSubmission)>,
) {
    let (_sender, receiver) = unbounded_channel();
    let environment = DesktopEnvironment {
        primary_application: "terminal".into(),
        applications: crate::application_registry::ApplicationRegistry::new(Vec::new()),
        projects_dir: None,
    };
    let path = Path::new("/config/desktop.json");
    let aggregate = parse_configuration(path, config).unwrap();
    let system = DesktopSystem::new(
        environment,
        SizePx::new(800, 600),
        InstanceTitleBarMetrics::from_scale_factor(TEST_SCALE_FACTOR),
        aggregate,
    )
    .unwrap();
    (system, receiver)
}

fn instance_manager(
    _receiver: &UnboundedReceiver<(InstanceId, InstanceSubmission)>,
) -> InstanceManager {
    let (sender, _receiver) = unbounded_channel();
    InstanceManager::new(InstanceEnvironment::new(sender, 1.0))
}

/// A system booted from `config` in a 1000×800 window, its startup launcher
/// in Full Screen Mode. On the 800×600 panel, fullscreen framing pins the
/// window size and scales content by 0.75.
fn fullscreen_system(config: &str) -> Result<(DesktopSystem, InstanceManager, LaunchProfileId)> {
    let (mut system, receiver) = system_from(config);
    let mut instance_manager = instance_manager(&receiver);
    for command in crate::projects::to_commands(&system.aggregates.configuration) {
        let changes = system.plan(DesktopCommand::Configuration(command))?;
        system.transact(
            changes,
            &mut instance_manager,
            TransactionEffectsMode::Setup,
        )?;
    }
    system.transact(
        DesktopChange::WindowResized(WindowState::new(SizePx::new(1000, 800), false)),
        &mut instance_manager,
        TransactionEffectsMode::Setup,
    )?;

    let launcher = system
        .aggregates
        .configuration
        .boot_launcher()
        .expect("the test configuration has a startup launcher");
    let launcher_config = system
        .aggregates
        .configuration
        .launcher_mut(launcher)
        .unwrap();
    launcher_config.full_screen_mode = launcher_config.full_screen_mode.toggled();
    Ok((system, instance_manager, launcher))
}

/// The live StartInstance commit: a focused primary instance whose view has not
/// arrived yet.
fn start_instance(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    launcher: LaunchProfileId,
    index: usize,
) -> Result<InstanceId> {
    let instance = uuid::Uuid::new_v4().into();
    let mut changes: Changes = [
        DesktopChange::PresentInstance(InstancePresentation {
            launcher,
            initial_center_translation: None,
            instance,
            root: InstanceRoot::new(),
            parameters: Default::default(),
            kind: InstanceKind::Primary,
        }),
        DesktopChange::Topology(TopologyChange::Insert {
            what: instance.into(),
            at_index: index,
            under: launcher.into(),
        }),
        DesktopChange::Topology(TopologyChange::Add {
            what: DesktopTarget::Instance(instance, InstancePart::TitleBar),
            under: instance.into(),
            after: None,
        }),
    ]
    .into();
    changes += set_focus(
        Some(FocusableTarget::Instance(instance)),
        KeyboardFocusReason::PresentInstance,
    );
    system.transact(changes, instance_manager, TransactionEffectsMode::Setup)?;
    Ok(instance)
}

/// The instance's first submission, which creates its primary view.
fn deliver_view(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    instance: InstanceId,
) -> Result<()> {
    let creation = massive_applications::ViewCreationInfo {
        id: uuid::Uuid::new_v4().into(),
        role: ViewRole::Primary,
        extents: massive_geometry::BoxPx::new(
            massive_geometry::PointPx::new(0, 0),
            massive_geometry::PointPx::new(800, 600),
        ),
    };
    submit(
        system,
        instance_manager,
        instance,
        InstanceChange::CreateView(creation),
    )
}

/// Sends `request` from `instance`, the way the CLI running in it does.
fn request(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    instance: InstanceId,
    request: massive_applications::ConfigurationRequest,
) -> Result<()> {
    submit(
        system,
        instance_manager,
        instance,
        InstanceChange::Configuration(request),
    )
}

/// Integrates `change` as a submission of `instance`.
fn submit(
    system: &mut DesktopSystem,
    instance_manager: &mut InstanceManager,
    instance: InstanceId,
    change: InstanceChange,
) -> Result<()> {
    let mut submission = massive_util::ChangeSet::default();
    submission.push(change);
    system.transact(
        DesktopChange::IntegrateInstanceSubmission(
            instance,
            InstanceSubmission::new(submission, massive_renderer::RenderPacing::default()),
        ),
        instance_manager,
        TransactionEffectsMode::Setup,
    )?;
    Ok(())
}

fn camera_distance(system: &DesktopSystem) -> f64 {
    system
        .resolve_desired_camera()
        .expect("a focused instance resolves a camera")
        .distance
}
