use std::{
    collections::{HashMap, hash_map},
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use log::warn;

use massive_animation::{Animated, Interpolation};
use massive_applications::ViewEvent;
use massive_geometry::{Color, PixelCamera, Rect, SizePx};
use massive_layout::Layouter;
use massive_renderer::text::FontSystem;
use massive_scene::{
    Handle, IntoVisual, Location, Object, ToCamera, ToTransform, Transform, Visual,
};
use massive_shapes::{Shape, StrokeRect};
use massive_shell::Scene;

use super::{
    LauncherPresenter, Project,
    project::{GroupId, LaunchGroup, LaunchGroupContents, LaunchProfileId},
};
use crate::{
    EventTransition,
    navigation::{NavigationNode, container},
    projects::{Id, STRUCTURAL_ANIMATION_DURATION},
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

    pub fn handle_event_transition(&mut self, event_transition: EventTransition<Id>) -> Result<()> {
        match event_transition {
            EventTransition::Directed(focus_path, view_event) => {
                if let Some(id) = focus_path.last() {
                    self.handle_directed_event(id.clone(), view_event)?;
                }
            }
            EventTransition::Broadcast(view_event) => {
                for group in self.groups.values_mut() {
                    group.handle_event(view_event.clone())?;
                }
                for launcher in self.launchers.values_mut() {
                    launcher.handle_event(view_event.clone())?;
                }
            }
        }
        Ok(())
    }

    const HOVER_ANIMATION_DURATION: Duration = Duration::from_millis(500);

    fn handle_directed_event(&mut self, id: Id, view_event: ViewEvent) -> Result<()> {
        match id {
            Id::Group(group_id) => {
                self.groups
                    .get_mut(&group_id)
                    .expect("Internal Error: Missing group")
                    .handle_event(view_event)?;
            }
            Id::Launcher(launch_profile_id) => {
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
                    ViewEvent::Focused(focused) => {
                        if focused {
                            warn!("FOCUSED {launch_profile_id:?}");
                        }
                    }
                    _ => {}
                }

                self.launchers
                    .get_mut(&launch_profile_id)
                    .expect("Internal Error: Missing launcher")
                    .handle_event(view_event)?;
            }
        }
        Ok(())
    }

    fn rect_of(&self, id: Id) -> Rect {
        match id {
            Id::Group(group_id) => self.groups[&group_id].rect.final_value(),
            Id::Launcher(launch_profile_id) => {
                self.launchers[&launch_profile_id].rect.final_value()
            }
        }
    }

    // Architecture: layout() has to be called before the navigation structure can return anything.
    // Perhaps manifest this in a better constructor.
    pub fn navigation(&self) -> NavigationNode<'_, Id> {
        let rect = self.groups[&self.project.root.id].rect.final_value();
        container(self.project.root.id, || {
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

    pub fn group_navigation<'a>(&'a self, launch_group: &'a LaunchGroup) -> NavigationNode<'a, Id> {
        let rect = self.groups[&launch_group.id].rect.final_value();
        container(launch_group.id, || {
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

    pub fn set_rect(&mut self, id: Id, rect: Rect, scene: &Scene, font_system: &mut FontSystem) {
        match id {
            Id::Group(group_id) => self.set_group_rect(group_id, rect, scene),
            Id::Launcher(launch_profile_id) => {
                self.set_launcher_rect(launch_profile_id, rect, scene, font_system)
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

    pub fn outer_camera(&self) -> PixelCamera {
        let root_group = self.project.root.id;
        if let Some(group) = self.groups.get(&root_group) {
            return group.camera();
        }
        Transform::IDENTITY.to_camera()
    }
}

/// Recursively layout a launch group and its children.
fn layout_launch_group(layout: &mut Layouter<Id, 2>, group: &LaunchGroup, default_size: SizePx) {
    match &group.contents {
        LaunchGroupContents::Groups(launch_groups) => {
            for group in launch_groups {
                let mut container = layout
                    .container(Id::Group(group.id), group.layout.axis())
                    .spacing(10)
                    .padding([10, 10], [10, 10]);
                layout_launch_group(&mut container, group, default_size);
            }
        }
        LaunchGroupContents::Launchers(launchers) => {
            for launcher in launchers {
                layout.leaf(Id::Launcher(launcher.id), default_size);
            }
        }
    }
}

fn create_hover_shapes(rect_alpha: Option<(Rect, f32)>) -> Arc<[Shape]> {
    rect_alpha
        .map(|(r, a)| {
            StrokeRect {
                rect: r,
                stroke: (10., 10.).into(),
                color: Color::rgb_u32(0xffff00).with_alpha(a),
            }
            .into()
        })
        .into_iter()
        .collect()
}

#[derive(Debug)]
struct GroupPresenter {
    // Ergonomics: Use just Location.
    location: Handle<Location>,
    rect: Animated<Rect>,
    // No background for now, we focus on the launchers.
    // background: Handle<Visual>,
}

impl GroupPresenter {
    pub fn new(location: Handle<Location>, rect: Rect, scene: &Scene) -> Self {
        // Ergonomics: I want this to look like rect.as_shape().with_color(Color::WHITE);
        //
        // Ergonomics: I need more named color constants for faster prototyping.
        // let background_shape = background_shape(rect, Color::rgb_u32(0x0000ff));

        Self {
            location: location.clone(),
            rect: scene.animated(rect),
            // background: [background_shape].at(&location).enter(scene),
        }
    }

    fn handle_event(&mut self, view_event: ViewEvent) -> Result<()> {
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

    fn camera(&self) -> PixelCamera {
        let rect = self.rect.final_value();

        rect.center()
            .to_transform()
            .to_camera()
            .with_size(rect.size())
    }
}
