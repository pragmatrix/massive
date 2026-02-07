//! The Desktop as an event sourced user interface system.
//!
//! The presenter hierarchy is treated as an aggregate built up from the events.
//!
//! The decision to use event sourcing stems from the fact that we want to run everything as
//! incrementally as possible, because we want to add projects, etc.
//!
//! The goal here is to remove as much as possible from the specific instances into separate systems
//! and aggregates that are event driven.

use anyhow::Result;
use derive_more::{Deref, DerefMut, From};

use massive_applications::{InstanceId, ViewCreationInfo, ViewEvent};
use massive_geometry::SizePx;
use massive_input::Event;
use massive_layout::{Layout, LayoutAxis, Padding, Thickness, container, leaf};
use massive_renderer::{RenderGeometry, text::FontSystem};
use massive_shell::Scene;

use crate::{
    DesktopInteraction, DesktopPresenter, Map, OrderedHierarchy, UserIntent,
    desktop_presenter::{DesktopFocusPath, DesktopTarget, SECTION_SPACING},
    event_sourcing::Transaction,
    instance_manager::InstanceManager,
    projects::{
        GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher, Project,
        ProjectCommand,
    },
};

#[derive(Debug)]
pub enum DesktopCommand {
    PresentInstance(InstanceId),
    Project(ProjectCommand),
}

#[derive(Debug, Deref, DerefMut)]
pub struct DesktopSystem {
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
        primary_instance: InstanceId,
        primary_view: ViewCreationInfo,
        project: Project,
        default_panel_size: SizePx,
        scene: &Scene,
        instance_manager: &InstanceManager,
    ) -> Result<Self> {
        let transaction = project_to_transaction(&project).map(DesktopCommand::Project);

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
        layout_specs.insert_or_update(
            DesktopTarget::TopBand,
            LayoutAxis::HORIZONTAL.to_container(),
        );

        let mut presenter = DesktopPresenter::new(project, scene);

        // Present the default terminal inside of the top band.
        {
            presenter.present_instance(
                &[DesktopTarget::Desktop, DesktopTarget::TopBand]
                    .to_vec()
                    .into(),
                primary_instance,
                default_panel_size,
                scene,
            )?;
            presenter.present_view(primary_instance, &primary_view)?;
        }

        // Architecture: This is the wrong way around, we need to create the desktop interaction
        // with the focus on the TopBand first and then add the primary instance.
        let interaction = DesktopInteraction::new(
            [
                DesktopTarget::Desktop,
                DesktopTarget::TopBand,
                DesktopTarget::Instance(primary_instance),
                DesktopTarget::View(primary_view.id),
            ]
            .to_vec()
            .into(),
            instance_manager,
            // This pushes the initial focus events to the presenter. Not sure if this makes sense, because Instance and View does not exist yet.
            &mut presenter,
            scene,
        )?;

        let mut system = Self {
            default_panel_size,
            interaction,
            presenter,

            hierarchy,
            layout_specs,
            startup_profile: None,
        };

        system.transact(transaction)?;
        Ok(system)
    }

    // Architecture: Is it really necessary to think in terms of transaction, if we update the
    // effects explicitly?
    fn transact(&mut self, transaction: Transaction<DesktopCommand>) -> Result<()> {
        for command in transaction {
            self.apply_command(command)?;
        }

        Ok(())
    }

    // Architecture: The current focus is part of the system, so DesktopInteraction should probably be embedded here.
    fn apply_command(&mut self, command: DesktopCommand) -> Result<()> {
        match command {
            DesktopCommand::PresentInstance(instance) => {
                todo!()
            }
            DesktopCommand::Project(project_command) => self.apply_project_command(project_command),
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
    ) -> Result<UserIntent> {
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
    ) -> Result<UserIntent> {
        self.interaction
            .focus(focus_path, instance_manager, &mut self.presenter)
    }

    pub fn refocus_pointer(
        &mut self,
        instance_manager: &InstanceManager,
        render_geometry: &RenderGeometry,
    ) -> Result<UserIntent> {
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
