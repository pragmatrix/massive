//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

use anyhow::{Result, anyhow};
use derive_more::{Deref, DerefMut, From};

use massive_applications::CreationMode;
use massive_applications::InstanceParameters;
use massive_applications::ViewRole;
use massive_applications::{InstanceId, ViewCreationInfo, ViewEvent};
use massive_geometry::SizePx;
use massive_input::Event;
use massive_layout::{Layout, LayoutAxis, Padding, Thickness, container, leaf};
use massive_renderer::{RenderGeometry, text::FontSystem};
use massive_shell::Scene;

use crate::desktop_presenter::{DesktopFocusPath, DesktopTarget, SECTION_SPACING};
use crate::event_sourcing::{self, Transaction};
use crate::instance_manager::InstanceManager;
use crate::projects::{
    GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher, Project, ProjectCommand,
};
use crate::{DesktopEnvironment, DesktopInteraction, DesktopPresenter, Map, OrderedHierarchy};

/// The commands the desktop system can execute.
#[derive(Debug)]
pub enum DesktopCommand {
    Project(ProjectCommand),
    PresentInstance(InstanceId),
    PresentView(InstanceId, ViewCreationInfo),
    StartInstance { parameters: InstanceParameters },
    StopInstance(InstanceId),
    Focus(DesktopFocusPath),
}

pub type Cmd = event_sourcing::Cmd<DesktopCommand>;

#[derive(Debug, Deref, DerefMut)]
pub struct DesktopSystem {
    env: DesktopEnvironment,
    default_panel_size: SizePx,

    pub interaction: DesktopInteraction,

    #[deref]
    #[deref_mut]
    presenter: DesktopPresenter,

    hierarchy: OrderedHierarchy<DesktopTarget>,
    layout_specs: Map<DesktopTarget, LayoutSpec>,
    startup_profile: Option<LaunchProfileId>,
}

impl DesktopSystem {
    pub fn new(
        env: DesktopEnvironment,
        project: Project,
        (primary_instance, primary_view): (InstanceId, ViewCreationInfo),
        default_panel_size: SizePx,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
        geometry: &RenderGeometry,
    ) -> Result<Self> {
        let project_setup_transaction =
            project_to_transaction(&project).map(DesktopCommand::Project);

        // Set up static hierarchy parts and layout specs.

        let mut hierarchy = OrderedHierarchy::default();
        hierarchy.add_nested(
            DesktopTarget::Desktop,
            [
                DesktopTarget::TopBand,
                DesktopTarget::Group(project.root.id),
            ],
        )?;

        let mut layout_specs = Map::default();
        layout_specs.insert_or_update(
            DesktopTarget::Desktop,
            LayoutAxis::VERTICAL.to_container().spacing(SECTION_SPACING),
        );
        layout_specs.insert_or_update(DesktopTarget::TopBand, LayoutAxis::HORIZONTAL);

        let mut presenter = DesktopPresenter::new(project, scene);

        let initial_focus = [DesktopTarget::Desktop, DesktopTarget::TopBand];

        let interaction = DesktopInteraction::new(
            initial_focus.to_vec().into(),
            instance_manager,
            &mut presenter,
            scene,
        )?;

        let mut system = Self {
            env,
            default_panel_size,
            interaction,
            presenter,

            hierarchy,
            layout_specs,
            startup_profile: None,
        };

        let primary_view_transaction: Transaction<_> = [
            DesktopCommand::PresentInstance(primary_instance),
            DesktopCommand::PresentView(primary_instance, primary_view),
        ]
        .into_iter()
        .collect::<Vec<_>>()
        .into();

        system.transact(
            project_setup_transaction + primary_view_transaction,
            scene,
            instance_manager,
            geometry,
        )?;
        Ok(system)
    }

    // Architecture: Is it really necessary to think in terms of transaction, if we update the
    // effects explicitly?
    pub fn transact(
        &mut self,
        transaction: impl Into<Transaction<DesktopCommand>>,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
        geometry: &RenderGeometry,
    ) -> Result<()> {
        for command in transaction.into() {
            self.apply_command(command, scene, instance_manager, geometry)?;
        }

        Ok(())
    }

    // Architecture: The current focus is part of the system, so DesktopInteraction should probably be embedded here.
    fn apply_command(
        &mut self,
        command: DesktopCommand,
        scene: &Scene,
        instance_manager: &mut InstanceManager,
        geometry: &RenderGeometry,
    ) -> Result<()> {
        match command {
            DesktopCommand::StartInstance { parameters } => {
                // Feature: Support starting non-primary applications.
                let application = self
                    .env
                    .applications
                    .get_named(&self.env.primary_application)
                    .ok_or(anyhow!("Internal error, application not registered"))?;

                let instance =
                    instance_manager.spawn(application, CreationMode::New(parameters))?;

                // Robustness: Should this be a real, logged event?
                // Architecture: Better to start up the primary directly, so that we can remove the PresentInstance command?
                self.apply_command(
                    DesktopCommand::PresentInstance(instance),
                    scene,
                    instance_manager,
                    geometry,
                )
            }

            DesktopCommand::StopInstance(instance) => {
                // Remove the instance from the focus first.
                //
                // Detail: This causes an unfocus event sent to the instance's view. I don't think
                // this should happen on teardown.
                let focus = self.interaction.focused();
                if let Some(focused_instance) = self.interaction.focused().instance()
                    && focused_instance == instance
                {
                    let instance_parent = focus.instance_parent().expect("Internal error: Instance parent failed even though instance() returned one.");
                    let intent = self.focus(instance_parent, instance_manager)?;
                    assert!(intent.is_none());
                }

                // Trigger the shutdown.
                instance_manager.trigger_shutdown(instance)?;

                // We hide the instance as soon we trigger a shutdown so that they can't be in the
                // navigation tree anymore.
                self.hide_instance(instance)?;

                // Refocus the cursor since it may be pointing to the removed instance.
                let cmd = self.refocus_pointer(instance_manager, geometry)?;
                // No intent on refocusing allowed.
                assert!(cmd.is_none());
                Ok(())
            }

            DesktopCommand::PresentInstance(instance) => {
                let focused = self.interaction.focused();
                let originating_from = focused.instance();
                let instance_parent_path = focused.instance_parent().ok_or(anyhow!(
                    "Failed to present instance when no parent is focused that can take on a new one"
                ))?;

                let instance_parent = instance_parent_path.last().unwrap().clone();

                let insertion_index = self.presenter.present_instance(
                    instance_parent.clone(),
                    originating_from,
                    instance,
                    self.default_panel_size,
                    scene,
                )?;

                let instance_target = DesktopTarget::Instance(instance);
                let instance_path = instance_parent_path.clone().join(instance_target.clone());

                // Add this instance to the hierarchy.
                self.hierarchy.insert_at(
                    instance_parent,
                    insertion_index,
                    instance_target.clone(),
                )?;

                // Overwrite the parent's layout (make sure this is horizontal band for now). In case of a project, this is currently set to fixed panel size.

                self.layout_specs.insert_or_update(
                    instance_parent_path.last().unwrap().clone(),
                    LayoutAxis::HORIZONTAL,
                );

                // Register the size of this instance.
                self.layout_specs
                    .insert_or_update(instance_target, self.default_panel_size);

                // Focus it.
                let cmd =
                    self.interaction
                        .focus(instance_path, instance_manager, &mut self.presenter)?;

                assert!(cmd.is_none());
                Ok(())
            }
            DesktopCommand::PresentView(instance, creation_info) => {
                self.presenter.present_view(instance, &creation_info)?;

                let focused = self.interaction.focused();
                // If this instance is currently focused and the new view is primary, make it
                // foreground so that the view is focused.
                if matches!(focused.last(), Some(DesktopTarget::Instance(i)) if *i == instance)
                    && creation_info.role == ViewRole::Primary
                {
                    let view_focus = focused.clone().join(DesktopTarget::View(creation_info.id));
                    let cmd = self.focus(view_focus, instance_manager)?;
                    assert!(cmd.is_none())
                }

                Ok(())
            }
            DesktopCommand::Project(project_command) => self.apply_project_command(project_command),

            DesktopCommand::Focus(path) => {
                assert!(self.focus(path, instance_manager)?.is_none());
                Ok(())
            }
        }
    }

    fn apply_project_command(&mut self, command: ProjectCommand) -> Result<()> {
        match command {
            ProjectCommand::AddLaunchGroup {
                parent,
                id,
                properties,
            } => {
                if let Some(parent) = parent {
                    self.hierarchy.add(parent.into(), id.into())?;
                };
                let spec = properties
                    .layout
                    .axis()
                    .to_container()
                    .spacing(10)
                    .padding(10, 10);
                self.layout_specs.insert_or_update(id.into(), spec);
            }
            ProjectCommand::RemoveLaunchGroup(group) => {
                let target = group.into();
                self.layout_specs.remove(&target)?;
                self.hierarchy.remove(&target)?;
            }
            ProjectCommand::AddLauncher {
                group,
                id,
                profile: _,
            } => {
                let target = DesktopTarget::Launcher(id);
                self.hierarchy.add(group.into(), target.clone())?;
                self.layout_specs
                    .insert_or_update(target, self.default_panel_size);
            }
            ProjectCommand::RemoveLauncher(launch_profile) => {
                let target = DesktopTarget::Launcher(launch_profile);
                self.layout_specs.remove(&target)?;
                self.hierarchy.remove(&target)?;
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                self.startup_profile = launch_profile_id
            }
        }

        Ok(())
    }

    /// Update all effects.
    pub fn update_effects(
        &mut self,
        animate: bool,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) -> Result<()> {
        let layout = self.desktop_layout();
        self.presenter
            .apply_layout(layout, animate, scene, font_system);
        Ok(())
    }

    // Architecture: We should probably not go through the old layout engine and think of something
    // more incremental.
    fn desktop_layout(&self) -> Layout<DesktopTarget, 2> {
        self.build_layout_for(DesktopTarget::Desktop)
    }

    fn build_layout_for(&self, target: DesktopTarget) -> Layout<DesktopTarget, 2> {
        match self.layout_specs[&target] {
            LayoutSpec::Container {
                axis,
                padding,
                spacing,
            } => {
                let mut container = container(target.clone(), axis)
                    .padding(padding)
                    .spacing(spacing);

                for nested in self.hierarchy.nested(&target) {
                    container.nested(self.build_layout_for(nested.clone()));
                }

                container.layout()
            }
            LayoutSpec::Leaf(size) => leaf(target, size),
        }
    }

    pub fn process_input_event(
        &mut self,
        event: &Event<ViewEvent>,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        self.interaction.process_input_event(
            event,
            instance_manager,
            &mut self.presenter,
            render_geometry,
        )
    }

    pub fn focus(
        &mut self,
        focus_path: DesktopFocusPath,
        instance_manager: &InstanceManager,
    ) -> Result<Cmd> {
        self.interaction
            .focus(focus_path, instance_manager, &mut self.presenter)
    }

    pub fn refocus_pointer(
        &mut self,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<Cmd> {
        self.interaction
            .refocus_pointer(instance_manager, &mut self.presenter, render_geometry)
    }
}

#[derive(Debug, From)]
pub enum LayoutSpec {
    Container {
        axis: LayoutAxis,
        padding: Thickness<2>,
        spacing: u32,
    },
    #[from]
    Leaf(SizePx),
}

impl From<LayoutAxis> for LayoutSpec {
    fn from(axis: LayoutAxis) -> Self {
        Self::Container {
            axis,
            padding: Default::default(),
            spacing: 0,
        }
    }
}

impl From<ContainerBuilder> for LayoutSpec {
    fn from(value: ContainerBuilder) -> Self {
        LayoutSpec::Container {
            axis: value.axis,
            padding: value.padding,
            spacing: value.spacing,
        }
    }
}

#[derive(Debug)]
struct ContainerBuilder {
    axis: LayoutAxis,
    padding: Thickness<2>,
    spacing: u32,
}

impl ContainerBuilder {
    pub fn new(axis: LayoutAxis) -> Self {
        Self {
            axis,
            padding: Default::default(),
            spacing: 0,
        }
    }

    pub fn padding(
        mut self,
        leading: impl Into<Padding<2>>,
        trailing: impl Into<Padding<2>>,
    ) -> Self {
        self.padding = (leading.into(), trailing.into()).into();
        self
    }

    pub fn spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }
}

// We seem to benefit from .into() and to_container() invocations. to_container is useful for
// chaining follow ups to the builder.

impl From<LayoutAxis> for ContainerBuilder {
    fn from(axis: LayoutAxis) -> Self {
        ContainerBuilder::new(axis)
    }
}

trait ToContainer {
    fn to_container(self) -> ContainerBuilder;
}

impl ToContainer for LayoutAxis {
    fn to_container(self) -> ContainerBuilder {
        ContainerBuilder::new(self)
    }
}

fn project_to_transaction(project: &Project) -> Transaction<ProjectCommand> {
    let mut commands = Vec::new();

    commands.push(ProjectCommand::SetStartupProfile(project.start));

    launch_group_commands(None, &project.root, &mut commands);

    commands.into()
}

fn launch_group_commands(
    parent: Option<GroupId>,
    group: &LaunchGroup,
    commands: &mut Vec<ProjectCommand>,
) {
    commands.push(ProjectCommand::AddLaunchGroup {
        parent,
        id: group.id,
        properties: group.properties.clone(),
    });

    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for launch_group in launch_groups {
                launch_group_commands(Some(group.id), launch_group, commands);
            }
        }
        LaunchGroupContents::Launchers(launchers) => {
            for launcher in launchers {
                launcher_commands(group.id, launcher, commands)
            }
        }
    }
}

fn launcher_commands(group: GroupId, launcher: &Launcher, commands: &mut Vec<ProjectCommand>) {
    commands.push(ProjectCommand::AddLauncher {
        group,
        id: launcher.id,
        profile: launcher.profile.clone(),
    })
}
