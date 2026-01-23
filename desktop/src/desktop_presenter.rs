use std::time::Duration;

use anyhow::Result;
use derive_more::From;
use uuid::Uuid;

use massive_applications::{InstanceId, ViewCreationInfo, ViewId};
use massive_geometry::{PixelCamera, Point, PointPx, Rect, SizePx};
use massive_layout::{Box as LayoutBox, LayoutAxis};
use massive_renderer::text::FontSystem;
use massive_scene::{Handle, Location, Object, ToLocation, Transform};
use massive_shell::Scene;

use crate::{
    EventTransition,
    band_presenter::BandPresenter,
    focus_path::FocusPath,
    instance_manager::{InstanceManager, ViewPath},
    navigation::{NavigationNode, container, leaf},
    projects::{
        self, GroupId, Id as ProjectId, LaunchGroup, LaunchGroupContents, LaunchProfileId, Project,
        ProjectPresenter,
    },
};

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const SECTION_SPACING: u32 = 20;

#[derive(Debug, Clone, PartialEq, From)]
pub enum Id {
    BandSection,
    Projects,
    Instance(InstanceId),
    Project(projects::Id),
}

#[derive(Debug, Clone, PartialEq, From)]
pub enum DesktopTarget {
    Band(BandTarget),
    Project(ProjectId),
}

#[derive(Debug, Clone, PartialEq, From)]
pub enum BandTarget {
    Instance(InstanceId),
    View(ViewId),
}

type Layouter<'a> = massive_layout::Layouter<'a, Id, 2>;
pub type DesktopFocusPath = FocusPath<DesktopTarget>;

/// Manages the presentation of the desktop, combining the band (instances) and projects
/// with unified vertical layout.
#[derive(Debug)]
pub struct DesktopPresenter {
    pub band: BandPresenter,
    pub project: ProjectPresenter,

    /// The root location for the desktop layout.
    location: Handle<Location>,

    /// Band section rect (for navigation).
    band_rect: Rect,
    /// Projects section rect (for navigation).
    project_rect: Rect,
}

impl DesktopPresenter {
    pub fn new(project: Project, scene: &Scene) -> Self {
        let location = Transform::IDENTITY.enter(scene).to_location().enter(scene);
        let project_presenter = ProjectPresenter::new(project, location.clone(), scene);

        Self {
            band: BandPresenter::default(),
            project: project_presenter,
            location,
            band_rect: Rect::ZERO,
            project_rect: Rect::ZERO,
        }
    }

    // BandPresenter delegation

    pub fn present_primary_instance(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
        scene: &Scene,
    ) -> Result<()> {
        self.band
            .present_primary_instance(instance, view_creation_info, scene)
    }

    pub fn present_instance(
        &mut self,
        instance: InstanceId,
        originating_from: InstanceId,
        scene: &Scene,
    ) -> Result<()> {
        self.band
            .present_instance(instance, originating_from, scene)
    }

    pub fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        self.band.present_view(instance, view_creation_info)
    }

    pub fn hide_view(&mut self, id: ViewId) -> Result<()> {
        self.band.hide_view(id)
    }

    pub fn instance_transform(&self, instance: InstanceId) -> Option<Transform> {
        self.band.instance_transform(instance)
    }

    // Unified Layout

    /// Compute the unified vertical layout for band (top) and projects (bottom).
    pub fn layout(
        &mut self,
        default_panel_size: SizePx,
        animate: bool,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        let mut layout = Layouter::root(None, LayoutAxis::VERTICAL).spacing(SECTION_SPACING);

        // Band section (instances horizontally)
        {
            let mut band_container = layout.container(Id::BandSection, LayoutAxis::HORIZONTAL);

            for instance_id in &self.band.ordered {
                let presenter = &self.band.instances[instance_id];
                band_container.leaf(Id::Instance(*instance_id), presenter.panel_size);
            }
        }

        // Project section
        {
            let mut project_container = layout
                .container(Id::Projects, LayoutAxis::HORIZONTAL)
                .spacing(10)
                .padding([10, 10], [10, 10]);

            // layout_launch_group(
            //     &mut project_container,
            //     &self.project.project.root,
            //     default_panel_size,
            // );
        }

        layout.place_inline(PointPx::origin(), |(id, rect)| {
            let rect_px = box_to_rect(rect);
            match id {
                Id::BandSection => {
                    self.band_rect = rect_px.cast().into();
                }
                Id::Instance(instance_id) => {
                    self.band.set_instance_rect(instance_id, rect_px, animate);
                }
                Id::Projects => {
                    self.project_rect = rect_px.cast().into();
                }
                Id::Project(project_id) => {
                    self.project
                        .set_rect(project_id, rect_px.cast().into(), scene, font_system);
                }
            }
        });
    }

    pub fn apply_animations(&mut self) {
        self.band.apply_animations();
        self.project.apply_animations();
    }

    // Navigation

    pub fn navigation<'a>(
        &'a self,
        instance_manager: &'a InstanceManager,
    ) -> NavigationNode<'a, DesktopTarget> {
        container(
            DesktopTarget::Band(BandTarget::Instance(InstanceId::from(Uuid::nil()))),
            || {
                [
                    // Band navigation (instances/views).
                    self.band_navigation(instance_manager),
                    // Project navigation.
                    self.project_navigation(),
                ]
                .into()
            },
        )
    }

    fn band_navigation<'a>(
        &'a self,
        instance_manager: &'a InstanceManager,
    ) -> NavigationNode<'a, DesktopTarget> {
        container(
            DesktopTarget::Band(BandTarget::Instance(InstanceId::from(Uuid::nil()))),
            || {
                let mut nodes = Vec::new();

                for (view_path, view_info) in instance_manager.views() {
                    let size = view_info.extents.size().cast::<f64>();
                    let extents = Rect::new(
                        Point::ORIGIN,
                        massive_geometry::Size::new(size.width, size.height),
                    );
                    let target = DesktopTarget::Band(BandTarget::View(view_path.view));
                    let transform = *view_info.location.value().transform.value();
                    nodes.push(leaf(target, extents).with_transform(transform));
                }

                nodes
            },
        )
        .with_rect(self.band_rect)
    }

    fn project_navigation(&self) -> NavigationNode<'_, DesktopTarget> {
        // Map project navigation to desktop targets.
        let project_nav = self.project.navigation();
        map_project_navigation(project_nav).with_rect(self.project_rect)
    }

    // Camera

    pub fn camera_for_focus(&self, focus: &DesktopFocusPath) -> Option<PixelCamera> {
        // Track instance focus only
        focus
            .instance()
            .and_then(|instance| self.band.instance_transform(instance))
            .map(|target| PixelCamera::look_at(target, None, PixelCamera::DEFAULT_FOVY))
    }
}

/// Maps a project navigation node to desktop targets.
fn map_project_navigation(
    node: NavigationNode<'_, ProjectId>,
) -> NavigationNode<'_, DesktopTarget> {
    match node {
        NavigationNode::Leaf {
            target,
            transform,
            rect,
        } => NavigationNode::Leaf {
            target: DesktopTarget::Project(target),
            transform,
            rect,
        },
        NavigationNode::Container {
            id,
            transform,
            rect,
            nested,
        } => NavigationNode::Container {
            id: DesktopTarget::Project(id),
            transform,
            rect,
            nested: Box::new(move || nested().into_iter().map(map_project_navigation).collect()),
        },
    }
}

fn box_to_rect(([x, y], [w, h]): LayoutBox<2>) -> massive_geometry::RectPx {
    massive_geometry::RectPx::new((x, y).into(), (w as i32, h as i32).into())
}

// Event forwarding

/// Forward event transitions to the appropriate handler based on the target type.
pub fn forward_event_transition(
    transition: EventTransition<DesktopTarget>,
    instance_manager: &InstanceManager,
    project_presenter: &mut ProjectPresenter,
) -> Result<()> {
    match transition {
        EventTransition::Directed(focus_path, view_event) => {
            // Route to the appropriate handler based on the last target in the path
            if let Some(target) = focus_path.last() {
                match target {
                    DesktopTarget::Band(BandTarget::View(_)) => {
                        // Send to instance view
                        if let Some(view_path) = focus_path.view_path() {
                            instance_manager.send_view_event(view_path, view_event)?;
                        }
                    }
                    DesktopTarget::Band(BandTarget::Instance(_)) => {
                        // Send to instance if it has a view
                        if let Some(view_path) = focus_path.view_path() {
                            instance_manager.send_view_event(view_path, view_event)?;
                        }
                    }
                    DesktopTarget::Project(project_id) => {
                        // Forward to project presenter
                        let project_transition =
                            EventTransition::Directed(vec![project_id.clone()].into(), view_event);
                        project_presenter.handle_event_transition(project_transition)?;
                    }
                }
            }
        }
        EventTransition::Broadcast(view_event) => {
            // Broadcast to all views in instance manager
            for (view_path, _) in instance_manager.views() {
                instance_manager.send_view_event(view_path, view_event.clone())?;
            }

            // Also broadcast to project presenter
            let project_transition = EventTransition::Broadcast(view_event);
            project_presenter.handle_event_transition(project_transition)?;
        }
    }
    Ok(())
}

// Focus path utilities

impl DesktopFocusPath {
    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Band(BandTarget::Instance(id)) => Some(*id),
            _ => None,
        })
    }

    pub fn view_path(&self) -> Option<ViewPath> {
        let mut instance = None;
        let mut view = None;

        for target in self.iter() {
            match target {
                DesktopTarget::Band(BandTarget::Instance(id)) => instance = Some(*id),
                DesktopTarget::Band(BandTarget::View(id)) => view = Some(*id),
                _ => {}
            }
        }

        match (instance, view) {
            (Some(inst), Some(v)) => Some((inst, v).into()),
            _ => None,
        }
    }
}

impl From<ViewPath> for DesktopFocusPath {
    fn from(view_path: ViewPath) -> Self {
        let (instance, view) = view_path.into();
        vec![
            DesktopTarget::Band(BandTarget::Instance(instance)),
            DesktopTarget::Band(BandTarget::View(view)),
        ]
        .into()
    }
}

impl From<(InstanceId, Option<ViewId>)> for DesktopFocusPath {
    fn from((instance, view): (InstanceId, Option<ViewId>)) -> Self {
        let mut path = vec![DesktopTarget::Band(BandTarget::Instance(instance))];
        if let Some(view) = view {
            path.push(DesktopTarget::Band(BandTarget::View(view)));
        }
        path.into()
    }
}
