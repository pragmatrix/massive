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

use massive_geometry::SizePx;
use massive_layout::{LayoutAxis, Padding, Thickness};
use massive_shell::Scene;

use crate::{
    DesktopPresenter, Map, OrderedHierarchy,
    desktop_presenter::DesktopTarget,
    event_sourcing::Transaction,
    projects::{
        GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Launcher, Project,
        ProjectCommand,
    },
};

#[derive(Debug)]
pub enum DesktopCommand {
    Project(ProjectCommand),
}

#[derive(Debug)]
pub struct DesktopSystem {
    hierarchy: OrderedHierarchy<DesktopTarget>,
    layout_specs: Map<DesktopTarget, LayoutSpec>,
    presenter: DesktopPresenter,
    startup_profile: Option<LaunchProfileId>,
}

impl DesktopSystem {
    pub fn new(project: Project, default_panel_size: SizePx, scene: &Scene) -> Result<Self> {
        let transaction = project_to_transaction(&project).map(DesktopCommand::Project);
        let presenter = DesktopPresenter::new(project, scene);
        let mut system = Self {
            hierarchy: Default::default(),
            layout_specs: Default::default(),
            presenter,
            startup_profile: None,
        };
        system.transact(transaction)?;
        Ok(system)
    }

    fn transact(&mut self, transaction: Transaction<DesktopCommand>) -> Result<()> {
        for command in transaction {
            self.apply_command(command)?;
        }

        Ok(())
    }

    fn apply_command(&mut self, command: DesktopCommand) -> Result<()> {
        match command {
            DesktopCommand::Project(project_command) => self.apply_project_command(project_command),
        }
    }

    fn apply_project_command(&mut self, command: ProjectCommand) -> Result<()> {
        match command {
            ProjectCommand::InsertLaunchGroup {
                parent,
                id,
                properties,
            } => {
                self.hierarchy
                    .insert_mapped(id, parent, DesktopTarget::Group)?;
                let spec = properties
                    .layout
                    .axis()
                    .to_container()
                    .spacing(10)
                    .padding(10, 10);
                self.layout_specs.insert_or_update(id.into(), spec.into())?;
            }
            ProjectCommand::RemoveLaunchGroup(group_id) => {
                let target = group_id.into();
                self.layout_specs.remove(&target)?;
                self.hierarchy.remove(&target)?;
            }
            ProjectCommand::InsertLauncher { group, id, profile } => {
                self.hierarchy
                    .insert(DesktopTarget::Launcher(id), group.map(DesktopTarget::Group))?;
            }
            ProjectCommand::RemoveLauncher(launch_profile_id) => {
                self.hierarchy
                    .remove(&DesktopTarget::Launcher(launch_profile_id))?;
            }
            ProjectCommand::SetStartupProfile(launch_profile_id) => {
                self.startup_profile = launch_profile_id
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
pub enum LayoutSpec {
    Container {
        axis: LayoutAxis,
        padding: Thickness<2>,
        spacing: u32,
    },
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
    commands.push(ProjectCommand::InsertLaunchGroup {
        parent: parent.map(Into::into),
        id: group.id,
        properties: group.properties.clone(),
    });

    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for launch_group in launch_groups {
                launch_group_commands(Some(group.id), &launch_group, commands);
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
    commands.push(ProjectCommand::InsertLauncher {
        group: group.into(),
        id: launcher.id,
        profile: launcher.profile.clone(),
    })
}
