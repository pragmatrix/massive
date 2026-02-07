use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use derive_more::From;
use log::error;

use massive_applications::{InstanceId, ViewCreationInfo, ViewId};
use massive_geometry::{PixelCamera, PointPx, Rect, RectPx, SizePx};
use massive_layout::LayoutAxis;
use massive_layout::{self as layout, Layout};
use massive_renderer::text::FontSystem;
use massive_scene::{Object, ToCamera, ToLocation, Transform};
use massive_shell::Scene;

use crate::{
    EventTransition, UserIntent,
    band_presenter::{BandPresenter, BandTarget},
    focus_path::FocusPath,
    instance_manager::{InstanceManager, ViewPath},
    navigation::{NavigationNode, container},
    projects::{GroupId, LaunchProfileId, Project, ProjectPresenter, ProjectTarget},
};

pub const STRUCTURAL_ANIMATION_DURATION: Duration = Duration::from_millis(500);
pub const SECTION_SPACING: u32 = 20;

/// Architecture: We need "unified" target enums. One that encapsulate the full path, but has parent
/// / add_nested or something like that trait implementations?
#[derive(Debug, Clone, PartialEq, Eq, Hash, From)]
pub enum DesktopTarget {
    // The whole area, covering the top band and
    Desktop,
    TopBand,

    Group(GroupId),
    Launcher(LaunchProfileId),

    Instance(InstanceId),
    View(ViewId),
}

impl From<BandTarget> for DesktopTarget {
    fn from(value: BandTarget) -> Self {
        match value {
            BandTarget::Instance(instance_id) => Self::Instance(instance_id),
            BandTarget::View(view_id) => Self::View(view_id),
        }
    }
}

impl From<ProjectTarget> for DesktopTarget {
    fn from(value: ProjectTarget) -> Self {
        match value {
            ProjectTarget::Group(group_id) => Self::Group(group_id),
            ProjectTarget::Launcher(launch_profile_id) => Self::Launcher(launch_profile_id),
            ProjectTarget::Band(_, band_target) => band_target.into(),
        }
    }
}

pub type DesktopFocusPath = FocusPath<DesktopTarget>;

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

    //
    // BandPresenter delegation
    //

    // Ergonomics: Perhaps pass the instance parent directly herein, or just return it? See how
    // clients use this function.
    pub fn present_instance(
        &mut self,
        focused: &DesktopFocusPath,
        new_instance: InstanceId,
        default_panel_size: SizePx,
        scene: &Scene,
    ) -> Result<DesktopFocusPath> {
        let originating_from = focused.instance();
        let instance_parent = focused.instance_parent().ok_or(anyhow!(
            "Failed to present instance when no parent is focused that can take on a new one"
        ))?;

        match instance_parent.last().unwrap() {
            DesktopTarget::TopBand => self.top_band.present_instance(
                new_instance,
                originating_from,
                default_panel_size,
                scene,
            )?,
            DesktopTarget::Launcher(launch_profile_id) => self.project.present_instance(
                *launch_profile_id,
                new_instance,
                originating_from,
                default_panel_size,
                scene,
            )?,
            invalid => {
                bail!("Invalid instance parent: {invalid:?}");
            }
        }

        Ok(instance_parent.join(DesktopTarget::Instance(new_instance)))
    }

    /// The instance is shutting down. Begin hiding them.
    pub fn hide_instance(&mut self, instance: InstanceId) -> Result<()> {
        if self.top_band.presents_instance(instance) {
            return self.top_band.hide_instance(instance);
        }

        self.project.hide_instance(instance)
    }

    pub fn present_view(
        &mut self,
        instance: InstanceId,
        view_creation_info: &ViewCreationInfo,
    ) -> Result<()> {
        // Here the instance does exist, so we can check where it belongs to.
        if self.top_band.presents_instance(instance) {
            self.top_band.present_view(instance, view_creation_info)
        } else {
            self.project.present_view(instance, view_creation_info)
        }
    }

    pub fn hide_view(&mut self, view: ViewPath) -> Result<()> {
        if self.top_band.presents_instance(view.instance) {
            self.top_band.hide_view(view)
        } else {
            self.project.hide_view(view)
        }
    }

    pub fn layout(
        &mut self,
        default_panel_size: SizePx,
        animate: bool,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        let mut root_builder = layout::container(DesktopTarget::Desktop, LayoutAxis::VERTICAL)
            .spacing(SECTION_SPACING);

        // Band section (instances layouted horizontally)
        root_builder.nested(
            self.top_band
                .layout()
                .map_id(DesktopTarget::Instance)
                .with_id(DesktopTarget::TopBand),
        );

        // Project section
        {
            let project_layout = self
                .project
                .layout(default_panel_size)
                .map_id(|pt| pt.into());
            root_builder.nested(project_layout);
        }

        self.apply_layout(root_builder.layout(), animate, scene, font_system);
    }

    pub fn apply_layout(
        &mut self,
        layout: Layout<DesktopTarget, 2>,
        animate: bool,
        scene: &Scene,
        font_system: &mut FontSystem,
    ) {
        layout.place_inline(PointPx::origin(), |id, rect_px: RectPx| match id {
            DesktopTarget::Desktop => {
                self.rect = rect_px.into();
            }
            DesktopTarget::TopBand => {
                self.top_band_rect = rect_px.into();
            }
            DesktopTarget::Instance(instance_id) => {
                self.top_band
                    .set_instance_rect(instance_id, rect_px, animate);
            }
            DesktopTarget::Group(group_id) => {
                self.project.set_rect(
                    ProjectTarget::Group(group_id),
                    rect_px.into(),
                    scene,
                    font_system,
                );
            }
            DesktopTarget::Launcher(launcher_id) => {
                self.project.set_rect(
                    ProjectTarget::Launcher(launcher_id),
                    rect_px.into(),
                    scene,
                    font_system,
                );
            }
            DesktopTarget::View(..) => {
                panic!("View layout isn't supported (instance target defines its size)");
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
                    .map_target(|bt| bt.into())
                    .with_target(DesktopTarget::TopBand),
                // Project navigation.
                self.project.navigation().map_target(|pt| pt.into()),
            ]
            .into()
        })
    }

    // Camera

    pub fn camera_for_focus(&self, focus: &DesktopFocusPath) -> Option<PixelCamera> {
        match focus.last()? {
            // Desktop and TopBand are constrained to their size.
            DesktopTarget::Desktop => Some(self.rect.to_camera()),
            DesktopTarget::TopBand => Some(self.top_band_rect.to_camera()),

            DesktopTarget::Instance(instance_id) => {
                // Architecture: The Band should be responsible for resolving at least the rects, if
                // not the camera?
                match focus[focus.len() - 2] {
                    DesktopTarget::TopBand => {
                        Some(self.top_band.instance_transform(*instance_id)?.to_camera())
                    }
                    DesktopTarget::Launcher(_) => self.camera_for_focus(&focus.parent()?),
                    _ => {
                        error!("Unexpected parent of instance");
                        None
                    }
                }
            }
            DesktopTarget::View(_) => {
                // Forward this to the parent (which must be a ::Instance).
                self.camera_for_focus(&focus.parent()?)
            }

            DesktopTarget::Group(group) => Some(
                self.project
                    .rect_of(ProjectTarget::Group(*group))
                    .to_camera(),
            ),
            DesktopTarget::Launcher(launcher) => Some(
                self.project
                    .rect_of(ProjectTarget::Launcher(*launcher))
                    .center()
                    .to_camera(),
            ),
        }
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
                    DesktopTarget::Instance(..) => {
                        // Shouldn't we forward this to the band here?
                    }
                    DesktopTarget::View(view_id) => {
                        let Some(instance) = focus_path.instance() else {
                            bail!("Internal error: Instance of view {view_id:?} not found");
                        };
                        if let Err(e) = instance_manager
                            .send_view_event((instance, *view_id), view_event.clone())
                        {
                            // This is not an error we want to stop the world for now.
                            error!("Sending view event {view_event:?} failed with {e:?}");
                        }
                    }

                    DesktopTarget::Group(group_id) => {
                        // Forward to project presenter
                        let project_transition = EventTransition::Directed(
                            vec![ProjectTarget::Group(*group_id)].into(),
                            view_event,
                        );
                        user_intent = project_presenter.process_transition(project_transition)?;
                    }
                    DesktopTarget::Launcher(launcher_id) => {
                        // Forward to project presenter
                        let project_transition = EventTransition::Directed(
                            vec![ProjectTarget::Launcher(*launcher_id)].into(),
                            view_event,
                        );
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
    pub fn instance(&self) -> Option<InstanceId> {
        self.iter().rev().find_map(|t| match t {
            DesktopTarget::Instance(id) => Some(*id),
            _ => None,
        })
    }

    /// Is this or a parent something that can be added new instances to?
    pub fn instance_parent(&self) -> Option<DesktopFocusPath> {
        self.iter()
            .enumerate()
            .rev()
            .find_map(|(i, t)| match t {
                DesktopTarget::Desktop => None,
                DesktopTarget::TopBand => Some(i + 1),
                DesktopTarget::Group(..) => None,
                DesktopTarget::Launcher(..) => Some(i + 1),
                DesktopTarget::Instance(..) => Some(i),
                DesktopTarget::View(..) => {
                    assert!(matches!(self[i - 1], DesktopTarget::Instance(..)));
                    Some(i - 1)
                }
            })
            .map(|i| self.iter().take(i).cloned().collect::<Vec<_>>().into())
    }
}
