use std::time::Duration;

use anyhow::Result;
use derive_more::From;

use massive_applications::{InstanceId, ViewCreationInfo, ViewId, ViewRole};
use massive_geometry::{PixelCamera, PointPx, Rect, SizePx};
use massive_layout as layout;
use massive_layout::{Box as LayoutBox, LayoutAxis};
use massive_renderer::text::FontSystem;
use massive_scene::{Handle, Location, Object, ToCamera, ToLocation, Transform};
use massive_shell::Scene;

use crate::{
    EventTransition,
    band_presenter::BandPresenter,
    focus_path::FocusPath,
    instance_manager::InstanceManager,
    navigation::{NavigationNode, container},
    projects::{self, Id as ProjectId, Project, ProjectPresenter},
};

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const SECTION_SPACING: u32 = 20;

#[derive(Debug, Clone, PartialEq, From)]
pub enum LayoutId {
    Desktop,
    TopBand,
    Instance(InstanceId),
    Project(projects::Id),
}

/// Architecture: We need "unified" target enums. One that encapsulate the full path, but has parent
/// / add_nested or something like that trait implementations?
#[derive(Debug, Clone, PartialEq, From)]
pub enum DesktopTarget {
    // The whole area, covering the top band and
    Desktop,
    TopBand,
    Instance(InstanceId),
    // The project itself is a group inside the project (not sure yet if this is a good thing)
    Project(ProjectId),
}

pub type DesktopPath = FocusPath<DesktopTarget>;

/// Manages the presentation of the desktop, combining the band (instances) and projects
/// with unified vertical layout.
#[derive(Debug)]
pub struct DesktopPresenter {
    pub band: BandPresenter,
    pub project: ProjectPresenter,

    /// The root location for the desktop layout.
    location: Handle<Location>,

    rect: Rect,
    top_band_rect: Rect,
}

impl DesktopPresenter {
    pub fn new(project: Project, scene: &Scene) -> Self {
        let location = Transform::IDENTITY.enter(scene).to_location().enter(scene);
        let project_presenter = ProjectPresenter::new(project, location.clone(), scene);

        Self {
            band: BandPresenter::default(),
            project: project_presenter,
            location,
            // Ergonomics: We need to push the layout results somewhere outside of the presenters.
            // Perhaps a `HashMap<LayoutId, Rect>` or so?
            rect: Rect::ZERO,
            top_band_rect: Rect::ZERO,
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

    // Unified Layout

    /// Compute the unified vertical layout for band (top) and projects (bottom).
    pub fn layout(
        &mut self,
        default_panel_size: SizePx,
        animate: bool,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        let mut root_builder =
            layout::container(LayoutId::Desktop, LayoutAxis::VERTICAL).spacing(SECTION_SPACING);

        // Band section (instances layouted horizontally)
        root_builder.child(
            self.band
                .layout()
                .map_id(LayoutId::Instance)
                .with_id(LayoutId::TopBand),
        );

        // Project section
        {
            let project_layout = self
                .project
                .layout(default_panel_size)
                .map_id(LayoutId::Project);
            root_builder.child(project_layout);
        }

        root_builder
            .layout()
            .place_inline(PointPx::origin(), |(id, rect)| {
                let rect_px = box_to_rect(rect);
                match id {
                    LayoutId::Desktop => {
                        self.rect = rect_px.into();
                    }
                    LayoutId::TopBand => {
                        self.top_band_rect = rect_px.into();
                    }
                    LayoutId::Instance(instance_id) => {
                        self.band.set_instance_rect(instance_id, rect_px, animate);
                    }
                    LayoutId::Project(project_id) => {
                        self.project
                            .set_rect(project_id, rect_px.into(), scene, font_system);
                    }
                }
            });
    }

    pub fn apply_animations(&mut self) {
        self.band.apply_animations();
        self.project.apply_animations();
    }

    // Navigation

    pub fn navigation<'a>(&'a self) -> NavigationNode<'a, DesktopTarget> {
        container(DesktopTarget::Desktop, || {
            [
                // Band navigation instances.
                self.band
                    .navigation()
                    .map_target(&DesktopTarget::Instance)
                    .with_target(DesktopTarget::TopBand),
                // Project navigation.
                self.project
                    .navigation()
                    .map_target(&DesktopTarget::Project),
            ]
            .into()
        })
    }

    // Camera

    pub fn camera_for_focus(&self, focus: &DesktopPath) -> Option<PixelCamera> {
        Some(match focus.last()? {
            // Desktop and TopBand are constrained to their size.
            DesktopTarget::Desktop => self.rect.center().to_camera().with_size(self.rect.size()),
            DesktopTarget::TopBand => self
                .top_band_rect
                .center()
                .to_camera()
                .with_size(self.top_band_rect.size()),
            DesktopTarget::Instance(instance_id) => {
                self.band.instance_transform(*instance_id)?.to_camera()
            }
            DesktopTarget::Project(id) => self.project.rect_of(id.clone()).center().to_camera(),
        })
    }

    // Event forwarding Architecture: This also seems to be wrong here. Used from the
    // DesktopInteraction, DesktopInteraction should probably own DesktopPresenter?

    pub fn forward_event_transitions(
        &mut self,
        // Don't use EventTransitions here for now, it contains more information than we need.
        transitions: Vec<EventTransition<DesktopTarget>>,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        for transition in transitions {
            self.forward_event_transition(transition, instance_manager)?;
        }
        Ok(())
    }

    /// Forward event transitions to the appropriate handler based on the target type.
    pub fn forward_event_transition(
        &mut self,
        transition: EventTransition<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<()> {
        let band_presenter = &self.band;
        let project_presenter = &mut self.project;
        match transition {
            EventTransition::Directed(focus_path, view_event) => {
                // Route to the appropriate handler based on the last target in the path
                if let Some(target) = focus_path.last() {
                    match target {
                        DesktopTarget::Desktop => {}
                        DesktopTarget::TopBand => {
                            band_presenter.process(view_event)?;
                        }
                        DesktopTarget::Instance(instance) => {
                            // Shouldn't we forward this to the band here?

                            // Send to instance if it has a view
                            if let Some(view) =
                                instance_manager.get_view_by_role(*instance, ViewRole::Primary)?
                            {
                                instance_manager.send_view_event((*instance, view), view_event)?;
                            }
                        }
                        DesktopTarget::Project(project_id) => {
                            // Forward to project presenter
                            let project_transition = EventTransition::Directed(
                                vec![project_id.clone()].into(),
                                view_event,
                            );
                            project_presenter.process_transition(project_transition)?;
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
                project_presenter.process_transition(project_transition)?;
            }
        }
        Ok(())
    }
}

fn box_to_rect(([x, y], [w, h]): LayoutBox<2>) -> massive_geometry::RectPx {
    massive_geometry::RectPx::new((x, y).into(), (w as i32, h as i32).into())
}

// Focus path utilities

impl DesktopPath {
    /// Focus the primary view. Currently only on the TopBand.
    pub fn from_instance(instance: InstanceId) -> Self {
        // Ergonomics: what about supporting .join directly on a target?
        Self::new(DesktopTarget::Desktop)
            .join(DesktopTarget::TopBand)
            .join(DesktopTarget::Instance(instance))
    }

    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Instance(id) => Some(*id),
            _ => None,
        })
    }

    // pub fn view_path(&self) -> Option<ViewPath> {
    //     let mut instance = None;
    //     let mut view = None;

    //     for target in self.iter() {
    //         match target {
    //             DesktopTarget::Band(BandTarget::Instance(id)) => instance = Some(*id),
    //             DesktopTarget::Band(BandTarget::View(id)) => view = Some(*id),
    //             _ => {}
    //         }
    //     }

    //     match (instance, view) {
    //         (Some(inst), Some(v)) => Some((inst, v).into()),
    //         _ => None,
    //     }
    // }
}

//impl From<ViewPath> for DesktopFocusPath {
// fn from(view_path: ViewPath) -> Self {
//     let (instance, view) = view_path.into();
//     vec![
//         DesktopTarget::Band(BandTarget::Instance(instance)),
//         DesktopTarget::Band(BandTarget::View(view)),
//     ]
//     .into()
// }
//}

// impl From<(InstanceId, Option<ViewId>)> for DesktopFocusPath {
//     fn from((instance, view): (InstanceId, Option<ViewId>)) -> Self {
//         let mut path = vec![DesktopTarget::Band(BandTarget::Instance(instance))];
//         if let Some(view) = view {
//             path.push(DesktopTarget::Band(BandTarget::View(view)));
//         }
//         path.into()
//     }
// }
