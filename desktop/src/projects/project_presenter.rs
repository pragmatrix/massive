use std::{
    collections::{HashMap, hash_map},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use log::error;

use massive_animation::{Animated, Interpolation};
use massive_applications::{InstanceId, ViewCreationInfo, ViewEvent};
use massive_geometry::{Color, Rect, SizePx};
use massive_layout::{Layout, container, leaf};
use massive_renderer::text::FontSystem;
use massive_scene::{Handle, IntoVisual, Location, Object, Visual};
use massive_shapes::{Shape, StrokeRect};
use massive_shell::Scene;

use super::{
    LauncherPresenter, Project,
    project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId},
};
use crate::{
    EventTransition, UserIntent,
    instance_manager::ViewPath,
    navigation::{self, NavigationNode},
    projects::{ProjectTarget, STRUCTURAL_ANIMATION_DURATION},
};

#[derive(Debug)]
pub struct ProjectPresenter {
    /// The project hierarchy is used for layout. It references the presenters through `GroupIds` and
    /// `LaunchProfileIds`.
    project: Project,

    location: Handle<Location>,

    groups: HashMap<GroupId, GroupPresenter>,
    launchers: HashMap<LaunchProfileId, LauncherPresenter>,

    // Idea: Use a type that combines Alpha with another Interpolatable type.
    // Robustness: Alpha should be a type.
    hover_alpha: Animated<f32>,
    hover_rect: Animated<Rect>,
    // Idea: can't we just animate a visual / Handle<Visual>?
    // Performance: This is a visual that _always_ lives inside the renderer, even though it does not contain a single shape when alpha = 0.0
    hover_visual: Handle<Visual>,
}

impl ProjectPresenter {
    pub fn new(project: Project, location: Handle<Location>, scene: &Scene) -> Self {
        Self {
            location: location.clone(),
            project,
            // Groups and slots are created when layouted.
            groups: Default::default(),
            launchers: Default::default(),
            hover_alpha: scene.animated(0.0),
            hover_rect: scene.animated(Rect::ZERO),
            hover_visual: create_hover_shapes(None)
                .into_visual()
                .at(location)
                .enter(scene),
        }
    }

    pub fn process_transition(
        &mut self,
        event_transition: EventTransition<ProjectTarget>,
    ) -> Result<UserIntent> {
        let intent = match event_transition {
            EventTransition::Directed(focus_path, view_event) => {
                if let Some(id) = focus_path.last() {
                    self.handle_directed_event(id.clone(), view_event)?
                } else {
                    UserIntent::None
                }
            }
            EventTransition::Broadcast(view_event) => {
                for group in self.groups.values_mut() {
                    group.process(view_event.clone())?;
                }
                for launcher in self.launchers.values_mut() {
                    let intent = launcher.process(view_event.clone())?;
                    if intent != UserIntent::None {
                        error!(
                            "Unsupported user intent in response to a Broadcast event: {intent:?}"
                        );
                    }
                }
                UserIntent::None
            }
        };

        Ok(intent)
    }

    const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(500);

    fn handle_directed_event(
        &mut self,
        id: ProjectTarget,
        view_event: ViewEvent,
    ) -> Result<UserIntent> {
        Ok(match id {
            ProjectTarget::Group(group_id) => {
                self.groups
                    .get_mut(&group_id)
                    .expect("Internal Error: Missing group")
                    .process(view_event)?;
                UserIntent::None
            }
            ProjectTarget::Launcher(launch_profile_id) => {
                match view_event {
                    ViewEvent::CursorEntered { .. } => {
                        // We do have to do this when the navigation structure already retrieves rects?
                        let rect = self.rect_of(id);

                        let was_visible = self.hover_alpha.final_value() == 1.0;

                        self.hover_alpha.animate_if_changed(
                            1.0,
                            Self::HOVER_ANIMATION_DURATION,
                            Interpolation::CubicOut,
                        );

                        if was_visible {
                            self.hover_rect.animate_if_changed(
                                rect,
                                Self::HOVER_ANIMATION_DURATION,
                                Interpolation::CubicOut,
                            );
                        } else {
                            self.hover_rect.set_immediately(rect);
                        }
                    }
                    ViewEvent::CursorLeft { .. } => {
                        self.hover_alpha.animate(
                            0.0,
                            Self::HOVER_ANIMATION_DURATION,
                            Interpolation::CubicOut,
                        );
                    }
                    _ => {}
                }

                self.launchers
                    .get_mut(&launch_profile_id)
                    .expect("Internal Error: Missing launcher")
                    .process(view_event)?
            }
            ProjectTarget::Band(launch_profile_id, _) => self
                .launchers
                .get_mut(&launch_profile_id)
                .expect("Internal Error: Missing launcher")
                .process_band(view_event)?,
        })
    }

    pub fn rect_of(&self, id: ProjectTarget) -> Rect {
        match id {
            ProjectTarget::Group(group_id) => self.groups[&group_id].rect.final_value(),
            ProjectTarget::Launcher(launch_profile_id)
            | ProjectTarget::Band(launch_profile_id, ..) => {
                self.launchers[&launch_profile_id].rect.final_value()
            }
        }
    }

    pub fn layout(&self, default_panel_size: SizePx) -> Layout<ProjectTarget, 2> {
        layout_launch_group(&self.project.root, default_panel_size)
    }

    // Architecture: layout() has to be called before the navigation structure can return anything.
    // Perhaps manifest this in a better constructor.
    pub fn navigation(&self) -> NavigationNode<'_, ProjectTarget> {
        let rect = self.groups[&self.project.root.id].rect.final_value();

        // Root is a navigation target and treated as a regular group for now.
        navigation::container(ProjectTarget::Group(self.project.root.id), || {
            let mut r = Vec::new();
            match &self.project.root.contents {
                LaunchGroupContents::Groups(launch_groups) => {
                    for launch_group in launch_groups.iter() {
                        r.push(self.group_navigation(launch_group));
                    }
                }
                // Robustness: Is this true, if so, can't we create a better Project type that reflects that?
                _ => panic!("Project root must be groups"),
            }
            r
        })
        .with_rect(rect)
    }

    pub fn group_navigation<'a>(
        &'a self,
        launch_group: &'a LaunchGroup,
    ) -> NavigationNode<'a, ProjectTarget> {
        let rect = self.groups[&launch_group.id].rect.final_value();
        navigation::container(ProjectTarget::Group(launch_group.id), || {
            let mut r = Vec::new();
            match &launch_group.contents {
                LaunchGroupContents::Groups(launch_groups) => {
                    for lg in launch_groups {
                        r.push(self.group_navigation(lg));
                    }
                }
                LaunchGroupContents::Launchers(launchers) => {
                    for launcher in launchers {
                        let presenter = &self.launchers[&launcher.id];
                        r.push(presenter.navigation(launcher));
                    }
                }
            }
            r
        })
        .with_rect(rect)
    }

    // layout callbacks
    // Ergonomics: Make Scene Clone.

    pub fn set_rect(
        &mut self,
        id: ProjectTarget,
        rect: Rect,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        match id {
            ProjectTarget::Group(group_id) => self.set_group_rect(group_id, rect, scene),
            ProjectTarget::Launcher(launch_profile_id) => {
                self.set_launcher_rect(launch_profile_id, rect, scene, font_system)
            }
            ProjectTarget::Band(..) => {
                panic!("Invalid set_rect on a Band inside the project")
            }
        }
    }

    fn set_group_rect(&mut self, id: GroupId, rect: Rect, scene: &Scene) {
        use hash_map::Entry;
        match self.groups.entry(id) {
            Entry::Occupied(mut entry) => entry.get_mut().set_rect(rect),
            Entry::Vacant(entry) => {
                entry.insert(GroupPresenter::new(self.location.clone(), rect, scene));
            }
        }
    }

    fn set_launcher_rect(
        &mut self,
        id: LaunchProfileId,
        rect: Rect,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        use hash_map::Entry;
        let profile = self
            .project
            .get_launch_profile(id)
            .expect("Internal Error: Launch profile not found");
        match self.launchers.entry(id) {
            Entry::Occupied(mut entry) => entry.get_mut().set_rect(rect),
            Entry::Vacant(entry) => {
                entry.insert(LauncherPresenter::new(
                    self.location.clone(),
                    profile.clone(),
                    rect,
                    scene,
                    font_system,
                ));
            }
        }
    }

    pub fn apply_animations(&mut self) {
        {
            let alpha = self.hover_alpha.value();
            let rect_alpha = (alpha != 0.0).then(|| (self.hover_rect.value(), alpha));

            // Ergonomics: What something like apply_to_if_changed(&mut self.hover_visual) or so?
            //
            // Performance: Can't be update just the shapes here with apply...
            let visual = create_hover_shapes(rect_alpha)
                .into_visual()
                .at(&self.location)
                .with_depth_bias(5);
            self.hover_visual.update_if_changed(visual);
        }

        self.groups
            .values_mut()
            .for_each(|gp| gp.apply_animations());
        self.launchers
            .values_mut()
            .for_each(|sp| sp.apply_animations());
    }

    pub fn present_instance(
        &mut self,
        launcher: LaunchProfileId,
        instance: InstanceId,
        originating_from: Option<InstanceId>,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<()> {
        self.launchers
            .get_mut(&launcher)
            .expect("Launcher does not exist")
            .present_instance(instance, originating_from, default_panel_size, scene)
    }

    pub fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        if let Some(launcher) = self
            .launchers
            .values_mut()
            .find(|launcher| launcher.is_presenting_instance(instance))
        {
            launcher.hide_instance(instance)
        } else {
            bail!("Internal error: No instance in this project")
        }
    }

    pub fn present_view(
        &mut self,
        instance: InstanceId,
        creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        let launcher = self
            .mut_launcher_for_instance(instance)
            .expect("Instance for view does not exist");

        launcher.present_view(instance, creation_info)
    }

    pub fn hide_view(&mut self, view: ViewPath) -> Result<()> {
        let launcher = self
            .mut_launcher_for_instance(view.instance)
            .expect("Instance for view does not exist");
        launcher.hide_view(view)
    }

    fn mut_launcher_for_instance(
        &mut self,
        instance: InstanceId,
    ) -> Option<&mut LauncherPresenter> {
        self.launchers
            .values_mut()
            .find(|l| l.is_presenting_instance(instance))
    }
}

/// Recursively layout a launch group and its children.
fn layout_launch_group(group: &LaunchGroup, default_size: SizePx) -> Layout<ProjectTarget, 2> {
    let group_id = group.id;

    let mut builder = container(ProjectTarget::Group(group_id), group.layout.axis())
        .spacing(10)
        .padding(10, 10);

    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for child_group in launch_groups {
                builder.child(layout_launch_group(child_group, default_size));
            }
        }
        LaunchGroupContents::Launchers(launchers) => {
            for launcher in launchers {
                builder.child(leaf(ProjectTarget::Launcher(launcher.id), default_size));
            }
        }
    }

    builder.layout()
}

fn create_hover_shapes(rect_alpha: Option<(Rect, f32)>) -> Arc<[Shape]> {
    rect_alpha
        .map(|(r, a)| {
            StrokeRect {
                rect: r,
                stroke: (10., 10.).into(),
                color: Color::rgb_u32(0xff0000).with_alpha(a),
            }
            .into()
        })
        .into_iter()
        .collect()
}

#[derive(Debug)]
struct GroupPresenter {
    // Ergonomics: Use just Location.
    // location: Handle<Location>,
    rect: Animated<Rect>,
    // No background for now, we focus on the launchers.
    // background: Handle<Visual>,
}

impl GroupPresenter {
    pub fn new(_location: Handle<Location>, rect: Rect, scene: &Scene) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        //
        // Ergonomics: I need more named color constants for faster prototyping.
        // let background_shape = background_shape(rect, Color::rgb_u32(0x0000ff));

        Self {
            // location: location.clone(),
            rect: scene.animated(rect),
            // background: [background_shape].at(&location).enter(scene),
        }
    }

    fn process(&mut self, _view_event: ViewEvent) -> Result<()> {
        Ok(())
    }

    fn set_rect(&mut self, rect: Rect) {
        self.rect
            .animate_if_changed(rect, STRUCTURAL_ANIMATION_DURATION, Interpolation::CubicOut);
    }

    fn apply_animations(&mut self) {
        // Ergonomics: Support value_mut() (wrap the mutex guard).
        // let rect = self.rect.value();
        // self.background
        //     .update_with(|v| v.shapes = [background_shape(rect, Color::rgb_u32(0x0000ff))].into());
    }
}
