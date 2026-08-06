use massive_applications::InstanceId;

use super::FocusDepth;
use super::change::FullScreenAction;

#[derive(Debug, Default)]
pub(super) struct FullScreenDecision {
    pub focus_depth: Option<FocusDepth>,
    pub actions: Vec<FullScreenAction>,
}

pub(super) fn enter(focused_instance: Option<InstanceId>) -> FullScreenDecision {
    let mut actions = Vec::new();
    add_full_screen_action(&mut actions, focused_instance);

    FullScreenDecision {
        focus_depth: Some(FocusDepth::InstanceFullScreen),
        actions,
    }
}

pub(super) fn exit(
    focus_depth: FocusDepth,
    focused_instance: Option<InstanceId>,
) -> FullScreenDecision {
    let FocusDepth::InstanceFullScreen = focus_depth else {
        panic!("Internal error: cannot exit fullscreen when it is inactive")
    };

    let mut actions = Vec::new();
    if let Some(instance) = focused_instance {
        actions.push(FullScreenAction::SetInstanceRegular(instance));
    }

    FullScreenDecision {
        focus_depth: Some(FocusDepth::Instance),
        actions,
    }
}

pub(super) fn native_fullscreen_changed(
    focus_depth: FocusDepth,
    is_native_fullscreen: bool,
    focused_instance: Option<InstanceId>,
) -> FullScreenDecision {
    let mut decision = FullScreenDecision::default();

    match (focus_depth, is_native_fullscreen) {
        (FocusDepth::InstanceFullScreen, true) => {
            add_full_screen_action(&mut decision.actions, focused_instance)
        }
        (FocusDepth::InstanceFullScreen, false) => {
            decision.focus_depth = Some(FocusDepth::Instance);
            add_regular_action(&mut decision.actions, focused_instance);
        }
        (FocusDepth::Instance, _)
        | (FocusDepth::Launcher, _)
        | (FocusDepth::Row, _)
        | (FocusDepth::Project, _)
        | (FocusDepth::Desktop, _) => {}
    }

    decision
}

pub(super) fn focus_changed(
    focus_depth: FocusDepth,
    previous_instance: Option<InstanceId>,
    focused_instance: Option<InstanceId>,
) -> FullScreenDecision {
    if previous_instance == focused_instance
        || !matches!(focus_depth, FocusDepth::InstanceFullScreen)
    {
        return FullScreenDecision::default();
    }

    let mut actions = Vec::new();
    add_regular_action(&mut actions, previous_instance);
    add_full_screen_action(&mut actions, focused_instance);
    FullScreenDecision {
        focus_depth: None,
        actions,
    }
}

fn add_full_screen_action(actions: &mut Vec<FullScreenAction>, instance: Option<InstanceId>) {
    if let Some(instance) = instance {
        actions.push(FullScreenAction::SetInstanceFullScreen(instance));
    }
}

fn add_regular_action(actions: &mut Vec<FullScreenAction>, instance: Option<InstanceId>) {
    if let Some(instance) = instance {
        actions.push(FullScreenAction::SetInstanceRegular(instance));
    }
}
