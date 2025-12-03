use massive_applications::{InstanceId, ViewId};

/// The focus manager organizes Instances an views in a focus hierarchy.
///
/// A view can only be focused if the containing instance is focused. But an instance can be focused
/// without a focused view. E.g. when the whole window gets unfocused.
///
/// Initially no instance is focused.
///
/// Conceptual: Isn't there some generic system. This looks like something resembling an hierarchical
/// switch.
/// Architecture: This type applies changes while it generates their effects. We could just
#[derive(Debug, Default)]
pub struct FocusManager {
    instance: Option<FocusedInstance>,
}

#[derive(Debug)]
struct FocusedInstance {
    id: InstanceId,
    focused_view: Option<ViewId>,
}

#[derive(Debug)]
pub enum FocusTransition {
    // Architecture: InstanceId is redundant here, but it makes for simpler processing later.
    UnfocusView(InstanceId, ViewId),
    FocusView(InstanceId, ViewId),
    UnfocusInstance(InstanceId),
    FocusInstance(InstanceId),
}

impl FocusManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused_instance(&self) -> Option<InstanceId> {
        self.instance.as_ref().map(|instance| instance.id)
    }

    pub fn focused_view(&self) -> Option<(InstanceId, ViewId)> {
        self.instance
            .as_ref()
            .and_then(|instance| instance.focused_view.map(|view| (instance.id, view)))
    }

    #[must_use]
    /// Focus the instance, and optionally a view.
    pub fn focus(&mut self, instance_id: InstanceId, view: Option<ViewId>) -> Vec<FocusTransition> {
        if let Some(instance) = &mut self.instance
            && instance.id == instance_id
        {
            return instance.focus_view(view);
        }

        let mut transitions = self.unfocus();

        let mut instance = FocusedInstance {
            id: instance_id,
            focused_view: None,
        };
        transitions.push(FocusTransition::FocusInstance(instance_id));
        transitions.extend(instance.focus_view(view));
        self.instance = Some(instance);
        transitions
    }

    /// Clear focus of the views, meaning that the focus can stay on the current instance.
    ///
    /// This is useful when the whole desktop application gets unfocused.
    #[must_use]
    pub fn unfocus_view(&mut self) -> Vec<FocusTransition> {
        let Some(instance) = &mut self.instance else {
            return Vec::new();
        };

        if let Some(view) = instance.focused_view.take() {
            return vec![FocusTransition::UnfocusView(instance.id, view)];
        }

        Vec::new()
    }

    #[must_use]
    pub fn unfocus(&mut self) -> Vec<FocusTransition> {
        let mut transitions = Vec::new();

        let Some(instance) = self.instance.take() else {
            return transitions;
        };

        if let Some(view) = instance.focused_view {
            transitions.push(FocusTransition::UnfocusView(instance.id, view));
        }
        transitions.push(FocusTransition::UnfocusInstance(instance.id));
        transitions
    }
}

impl FocusedInstance {
    pub fn focus_view(&mut self, new_view: Option<ViewId>) -> Vec<FocusTransition> {
        if self.focused_view == new_view {
            return Vec::new();
        }

        let mut transitions = self.unfocus_view();
        if let Some(new_view) = new_view {
            self.focused_view = Some(new_view);
            transitions.push(FocusTransition::FocusView(self.id, new_view));
        }
        transitions
    }

    pub fn unfocus_view(&mut self) -> Vec<FocusTransition> {
        self.focused_view
            .take()
            .map(|view| vec![FocusTransition::UnfocusView(self.id, view)])
            .unwrap_or_default()
    }
}
