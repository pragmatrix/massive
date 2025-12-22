use derive_more::From;
use massive_applications::{InstanceId, ViewId};

use crate::instance_manager::ViewPath;

/// The focus manager organizes Instances and views in a focus hierarchy.
///
/// A view can only be focused if the containing instance is focused. But an instance can be focused
/// without a focused view. E.g. when the whole window gets unfocused.
///
/// Initially no instance is focused.
#[derive(Debug, Default)]
pub struct FocusManager {
    current: Option<FocusPath>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, From)]
pub struct FocusPath {
    pub instance: InstanceId,
    pub view: Option<ViewId>,
}

impl From<ViewPath> for FocusPath {
    fn from(value: ViewPath) -> Self {
        Self {
            instance: value.instance,
            view: Some(value.view),
        }
    }
}

#[derive(Debug)]
pub enum FocusTransition {
    // Architecture: InstanceId is redundant here, but it makes for simpler processing later.
    UnfocusView(ViewPath),
    FocusView(ViewPath),
    UnfocusInstance(InstanceId),
    FocusInstance(InstanceId),
}

impl FocusManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused_instance(&self) -> Option<InstanceId> {
        self.focused().map(|p| p.instance)
    }

    pub fn focused_view(&self) -> Option<ViewPath> {
        self.focused()
            .and_then(|path| path.view.map(|view| (path.instance, view).into()))
    }

    pub fn focused(&self) -> Option<FocusPath> {
        self.current
    }

    #[must_use]
    /// Focus the instance, and optionally a view.
    pub fn focus(&mut self, focus_path: impl Into<FocusPath>) -> Vec<FocusTransition> {
        let focus_path = focus_path.into();
        if let Some(instance) = &mut self.current
            && instance.instance == focus_path.instance
        {
            return instance.focus_view(focus_path.view);
        }

        let mut transitions = self.unfocus();

        let mut new_path = FocusPath {
            instance: focus_path.instance,
            view: None,
        };
        transitions.push(FocusTransition::FocusInstance(focus_path.instance));
        transitions.extend(new_path.focus_view(focus_path.view));
        self.current = Some(new_path);
        transitions
    }

    /// Clear focus of the views, meaning that the focus can stay on the current instance.
    ///
    /// This is useful when the whole desktop application gets unfocused.
    #[must_use]
    pub fn unfocus_view(&mut self) -> Vec<FocusTransition> {
        let Some(instance) = &mut self.current else {
            return Vec::new();
        };

        if let Some(view) = instance.view.take() {
            return vec![FocusTransition::UnfocusView(
                (instance.instance, view).into(),
            )];
        }

        Vec::new()
    }

    #[must_use]
    pub fn unfocus(&mut self) -> Vec<FocusTransition> {
        let mut transitions = Vec::new();

        let Some(instance) = self.current.take() else {
            return transitions;
        };

        if let Some(view) = instance.view {
            transitions.push(FocusTransition::UnfocusView(
                (instance.instance, view).into(),
            ));
        }
        transitions.push(FocusTransition::UnfocusInstance(instance.instance));
        transitions
    }
}

impl FocusPath {
    #[must_use]
    fn focus_view(&mut self, new_view: Option<ViewId>) -> Vec<FocusTransition> {
        if self.view == new_view {
            return [].into();
        }

        let mut transitions = self.unfocus_view();
        if let Some(new_view) = new_view {
            self.view = Some(new_view);
            transitions.push(FocusTransition::FocusView((self.instance, new_view).into()));
        }
        transitions
    }

    #[must_use]
    fn unfocus_view(&mut self) -> Vec<FocusTransition> {
        self.view
            .take()
            .map(|view| vec![FocusTransition::UnfocusView((self.instance, view).into())])
            .unwrap_or_default()
    }
}
