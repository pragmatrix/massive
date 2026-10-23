use massive_applications::{InstanceId, ViewId};

use crate::projects::{LaunchProfileId, ProjectId};

/// A unique target inside the navigation and layout history: a presenter and the part of it the
/// hierarchy refers to (ADR 0022).
///
/// `Desktop` is the hierarchy's virtual root: it is never inserted explicitly and only appears as
/// the parent key under which the root project's target is added.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DesktopTarget {
    Desktop,
    Project(ProjectId, ProjectPart),
    Launcher(LaunchProfileId),
    Instance(InstanceId, InstancePart),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectPart {
    Project,
    Header,
    Matrix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstancePart {
    Instance,
    /// The bar above the instance's view (ADR 0019).
    TitleBar,
    View(ViewId),
}

/// The targets that hold the keyboard focus, so decisions about the focus match exhaustively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusableTarget {
    Desktop,
    Project(ProjectId),
    Launcher(LaunchProfileId),
    Instance(InstanceId),
    /// Carries its instance so it converts back into a target without a hierarchy lookup.
    View(InstanceId, ViewId),
}

impl DesktopTarget {
    /// The matrix of `project`. Its children in the hierarchy are the project's slots.
    pub fn matrix(project: ProjectId) -> Self {
        Self::Project(project, ProjectPart::Matrix)
    }

    /// The header of `project`, which sits beside its matrix under the project in the hierarchy.
    pub fn header(project: ProjectId) -> Self {
        Self::Project(project, ProjectPart::Header)
    }

    /// The bar above the instance's view (ADR 0019).
    pub fn title_bar(instance: InstanceId) -> Self {
        Self::Instance(instance, InstancePart::TitleBar)
    }

    pub fn view(instance: InstanceId, view: ViewId) -> Self {
        Self::Instance(instance, InstancePart::View(view))
    }

    /// Only a target that selects itself holds the keyboard focus and handles keys on a focus path;
    /// a part that selects another leaves both to its parent on the same path (ADR 0019).
    pub fn as_focusable(&self) -> Option<FocusableTarget> {
        let selected = self.selected_target();
        (DesktopTarget::from(selected) == *self).then_some(selected)
    }

    /// What a click on the target selects: a part selects the target it belongs to, a view
    /// selects itself (ADR 0019).
    pub fn selected_target(&self) -> FocusableTarget {
        match *self {
            Self::Desktop => FocusableTarget::Desktop,
            Self::Project(project, _) => FocusableTarget::Project(project),
            Self::Launcher(launcher) => FocusableTarget::Launcher(launcher),
            Self::Instance(instance, InstancePart::View(view)) => {
                FocusableTarget::View(instance, view)
            }
            Self::Instance(instance, InstancePart::Instance | InstancePart::TitleBar) => {
                FocusableTarget::Instance(instance)
            }
        }
    }

    /// Whether content below the target may extend beyond its bounds: slots can hold visors, whose
    /// instances overflow their launcher, so the desktop, projects and matrices never clip hits.
    pub fn children_may_overflow(&self) -> bool {
        matches!(
            self,
            Self::Desktop | Self::Project(_, ProjectPart::Project | ProjectPart::Matrix)
        )
    }
}

impl FocusableTarget {
    /// Whether the target is an instance or its view.
    pub fn is_instance_target(&self) -> bool {
        matches!(self, Self::Instance(_) | Self::View(..))
    }

    /// The project the target stands for; `Desktop` stands for the root project.
    pub fn stands_for_project(&self) -> Option<ProjectId> {
        match *self {
            Self::Project(project) => Some(project),
            Self::Desktop => Some(ProjectId::ROOT),
            Self::Launcher(_) | Self::Instance(_) | Self::View(..) => None,
        }
    }
}

impl From<ProjectId> for FocusableTarget {
    fn from(project: ProjectId) -> Self {
        Self::Project(project)
    }
}

impl From<FocusableTarget> for DesktopTarget {
    fn from(target: FocusableTarget) -> Self {
        match target {
            FocusableTarget::Desktop => Self::Desktop,
            FocusableTarget::Project(project) => project.into(),
            FocusableTarget::Launcher(launcher) => Self::Launcher(launcher),
            FocusableTarget::Instance(instance) => instance.into(),
            FocusableTarget::View(instance, view) => Self::view(instance, view),
        }
    }
}

impl From<ProjectId> for DesktopTarget {
    fn from(value: ProjectId) -> Self {
        Self::Project(value, ProjectPart::Project)
    }
}

impl From<LaunchProfileId> for DesktopTarget {
    fn from(value: LaunchProfileId) -> Self {
        Self::Launcher(value)
    }
}

impl From<InstanceId> for DesktopTarget {
    fn from(value: InstanceId) -> Self {
        Self::Instance(value, InstancePart::Instance)
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;

    /// A title bar leaves selection and focus to its instance, while a view, though also a part of
    /// the instance, is the application level and selects itself (ADR 0019).
    #[test]
    fn a_view_selects_itself_while_a_title_bar_selects_its_instance() {
        let instance: InstanceId = Uuid::new_v4().into();
        let view = ViewId::new();

        let title_bar = DesktopTarget::title_bar(instance);
        assert_eq!(
            title_bar.selected_target(),
            FocusableTarget::Instance(instance)
        );
        assert_eq!(title_bar.as_focusable(), None);

        let view_target = DesktopTarget::view(instance, view);
        assert_eq!(
            view_target.selected_target(),
            FocusableTarget::View(instance, view)
        );
        assert_eq!(
            view_target.as_focusable(),
            Some(FocusableTarget::View(instance, view))
        );
    }
}
