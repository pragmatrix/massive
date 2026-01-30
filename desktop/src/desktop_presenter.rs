use std::time::Duration;

use anyhow::{Result, bail};
use derive_more::From;

use massive_applications::{InstanceId, ViewCreationInfo, ViewId};
use massive_geometry::{PixelCamera, PointPx, Rect, SizePx};
use massive_layout as layout;
use massive_layout::LayoutAxis;
use massive_renderer::text::FontSystem;
use massive_scene::{Object, ToCamera, ToLocation, Transform};
use massive_shell::Scene;

use crate::box_to_rect;
use crate::projects::LaunchProfileId;
use crate::{
    EventTransition, UserIntent,
    band_presenter::{BandPresenter, BandTarget},
    focus_path::FocusPath,
    instance_manager::InstanceManager,
    navigation::{NavigationNode, container},
    projects::{Project, ProjectPresenter, ProjectTarget},
};

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
const SECTION_SPACING: u32 = 20;

#[derive(Debug, Clone, PartialEq, From)]
pub enum LayoutId {
    Desktop,
    TopBand,
    Instance(InstanceId),
    Project(ProjectTarget),
}

/// Architecture: We need "unified" target enums. One that encapsulate the full path, but has parent
/// / add_nested or something like that trait implementations?
#[derive(Debug, Clone, PartialEq, Eq, From)]
pub enum DesktopTarget {
    // The whole area, covering the top band and
    Desktop,
    TopBand,
    Band(BandTarget),
    // The project itself is a group inside the project (not sure yet if this is a good thing)
    Project(ProjectTarget),
}

pub type DesktopFocusPath = FocusPath<DesktopTarget>;

/// The location where the instance bands are.
#[derive(Debug)]
pub enum InstanceTarget {
    TopBand,
    LaunchProfile(LaunchProfileId),
}

/// Manages the presentation of the desktop, combining the band (instances) and projects
/// with unified vertical layout.
#[derive(Debug)]
pub struct DesktopPresenter {
    top_band: BandPresenter,
    project: ProjectPresenter,

    rect: Rect,
    top_band_rect: Rect,
}

impl DesktopPresenter {
    pub fn new(project: Project, scene: &Scene) -> Self {
        let location = Transform::IDENTITY.enter(scene).to_location().enter(scene);
        let project_presenter = ProjectPresenter::new(project, location.clone(), scene);

        Self {
            top_band: BandPresenter::default(),
            project: project_presenter,
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
        self.top_band
            .present_primary_instance(instance, view_creation_info, scene)
    }

    pub fn present_instance(
        &mut self,
        target: InstanceTarget,
        instance: InstanceId,
        originating_from: Option<InstanceId>,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<()> {
        match target {
            InstanceTarget::TopBand => self.top_band.present_instance(
                instance,
                originating_from,
                default_panel_size,
                scene,
            ),
            InstanceTarget::LaunchProfile(launch_profile_id) => self.project.present_instance(
                launch_profile_id,
                instance,
                originating_from,
                default_panel_size,
                scene,
            ),
        }
    }

    pub fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        // Here the instance does exist, so we can check where it belongs to.
        if self.top_band.presents_instance(instance) {
            return self.top_band.present_view(instance, view_creation_info);
        }
        self.project.present_view(instance, view_creation_info)
    }

    pub fn hide_view(&mut self, id: ViewId) -> Result<()> {
        self.top_band.hide_view(id)
    }

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
            self.top_band
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
            .place_inline(PointPx::origin(), |id, rect| {
                let rect_px = box_to_rect(rect);
                match id {
                    LayoutId::Desktop => {
                        self.rect = rect_px.into();
                    }
                    LayoutId::TopBand => {
                        self.top_band_rect = rect_px.into();
                    }
                    LayoutId::Instance(instance_id) => {
                        self.top_band
                            .set_instance_rect(instance_id, rect_px, animate);
                    }
                    LayoutId::Project(project_id) => {
                        self.project
                            .set_rect(project_id, rect_px.into(), scene, font_system);
                    }
                }
            });
    }

    pub fn apply_animations(&mut self) {
        self.top_band.apply_animations();
        self.project.apply_animations();
    }

    // Navigation

    pub fn navigation<'a>(&'a self) -> NavigationNode<'a, DesktopTarget> {
        container(DesktopTarget::Desktop, || {
            [
                // Band navigation instances.
                self.top_band
                    .navigation()
                    .map_target(DesktopTarget::Band)
                    .with_target(DesktopTarget::TopBand),
                // Project navigation.
                self.project.navigation().map_target(DesktopTarget::Project),
            ]
            .into()
        })
    }

    // Camera

    pub fn camera_for_focus(&self, focus: &DesktopFocusPath) -> Option<PixelCamera> {
        Some(match focus.last()? {
            // Desktop and TopBand are constrained to their size.
            DesktopTarget::Desktop => self.rect.center().to_camera().with_size(self.rect.size()),
            DesktopTarget::TopBand => self
                .top_band_rect
                .center()
                .to_camera()
                .with_size(self.top_band_rect.size()),
            DesktopTarget::Band(BandTarget::Instance(instance_id)) => {
                // Architecture: The Band should be responsible for resolving at least the rects, if
                // not the camera?
                self.top_band.instance_transform(*instance_id)?.to_camera()
            }
            DesktopTarget::Band(BandTarget::View(..)) => {
                // Forward this to the parent (which is a BandTarget::Instance).
                self.camera_for_focus(&focus.parent()?)?
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
    ) -> Result<UserIntent> {
        let mut user_intent = UserIntent::None;

        // Robustness: While we need to forward all transitions we currently process only one intent.
        for transition in transitions {
            user_intent = self.forward_event_transition(transition, instance_manager)?;
        }

        Ok(user_intent)
    }

    /// Forward event transitions to the appropriate handler based on the target type.
    pub fn forward_event_transition(
        &mut self,
        transition: EventTransition<DesktopTarget>,
        instance_manager: &InstanceManager,
    ) -> Result<UserIntent> {
        let band_presenter = &self.top_band;
        let project_presenter = &mut self.project;
        let mut user_intent = UserIntent::None;
        match transition {
            EventTransition::Directed(path, _) if path.is_empty() => {
                // This happens if hit testing hits no presenter and a CursorMove event gets
                // forwarded: FocusPath::EMPTY represents the Window itself.
            }
            EventTransition::Directed(focus_path, view_event) => {
                // Route to the appropriate handler based on the last target in the path
                match focus_path.last().expect("Internal Error") {
                    DesktopTarget::Desktop => {}
                    DesktopTarget::TopBand => {
                        band_presenter.process(view_event)?;
                    }
                    DesktopTarget::Band(BandTarget::Instance(..)) => {
                        // Shouldn't we forward this to the band here?
                    }
                    DesktopTarget::Band(BandTarget::View(view_id)) => {
                        let Some(instance) = instance_manager.instance_of_view(*view_id) else {
                            bail!("Internal error: Instance of view {view_id:?} not found");
                        };
                        instance_manager.send_view_event((instance, *view_id), view_event)?;
                    }
                    DesktopTarget::Project(project_id) => {
                        // Forward to project presenter
                        let project_transition =
                            EventTransition::Directed(vec![project_id.clone()].into(), view_event);
                        user_intent = project_presenter.process_transition(project_transition)?;
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
                user_intent = project_presenter.process_transition(project_transition)?;
            }
        }
        Ok(user_intent)
    }
}

// Path utilities

impl DesktopFocusPath {
    /// Focus the primary view. Currently only on the TopBand.
    pub fn from_instance_and_view(instance: InstanceId, view: impl Into<Option<ViewId>>) -> Self {
        // Ergonomics: what about supporting .join directly on a target?
        let instance = Self::new(DesktopTarget::Desktop)
            .join(DesktopTarget::TopBand)
            .join(DesktopTarget::Band(BandTarget::Instance(instance)));
        let Some(view) = view.into() else {
            return instance;
        };
        instance.join(DesktopTarget::Band(BandTarget::View(view)))
    }

    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Band(BandTarget::Instance(id)) => Some(*id),
            _ => None,
        })
    }

    /// A target that can take on more instances. This defines the locations where new instances can be created.
    pub fn instance_target(&self) -> Option<InstanceTarget> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Desktop => {
                // This could be useful for spawning a instance in the top band.
                None
            }
            DesktopTarget::TopBand | DesktopTarget::Band(..) => Some(InstanceTarget::TopBand),
            DesktopTarget::Project(ProjectTarget::Launcher(launcher_id)) => {
                Some(InstanceTarget::LaunchProfile(*launcher_id))
            }
            DesktopTarget::Project(ProjectTarget::Group(_)) => {
                // Idea: Spawn for each member of the group?
                None
            }

            DesktopTarget::Project(ProjectTarget::Band(..)) => {
                // Covered by ProjectTarget::Launcher already.
                None
            }
        })
    }
}
