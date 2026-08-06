use massive_applications::InstanceId;

use super::change::FullScreenAction;
use super::{FocusDepth, NativeFullScreen};

#[derive(Debug, Default)]
pub(super) struct FullScreenDecision {
    pub focus_depth: Option<FocusDepth>,
    pub actions: Vec<FullScreenAction>,
}

pub(super) fn enter(
    native_fullscreen: NativeFullScreen,
    focused_instance: Option<InstanceId>,
) -> FullScreenDecision {
    let mut decision = FullScreenDecision {
        focus_depth: Some(FocusDepth::InstanceFullScreen(native_fullscreen)),
        actions: Vec::new(),
    };

    match native_fullscreen {
        NativeFullScreen::Existing => {
            if let Some(instance) = focused_instance {
                decision
                    .actions
                    .push(FullScreenAction::SetInstanceFullScreen(instance));
            }
        }
        NativeFullScreen::Requested => decision
            .actions
            .push(FullScreenAction::ToggleNativeFullScreen),
        NativeFullScreen::Entered => {
            panic!("Internal error: cannot enter fullscreen in an already-entered state")
        }
    }

    decision
}

pub(super) fn exit(
    focus_depth: FocusDepth,
    focused_instance: Option<InstanceId>,
) -> FullScreenDecision {
    let FocusDepth::InstanceFullScreen(native_fullscreen) = focus_depth else {
        panic!("Internal error: cannot exit fullscreen when it is inactive")
    };

    let mut actions = Vec::new();
    if let Some(instance) = focused_instance {
        actions.push(FullScreenAction::SetInstanceRegular(instance));
    }
    if native_fullscreen != NativeFullScreen::Existing {
        actions.push(FullScreenAction::ToggleNativeFullScreen);
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
        (FocusDepth::InstanceFullScreen(NativeFullScreen::Requested), true) => {
            decision.focus_depth = Some(FocusDepth::InstanceFullScreen(NativeFullScreen::Entered));
            add_full_screen_action(&mut decision.actions, focused_instance);
        }
        (
            FocusDepth::InstanceFullScreen(NativeFullScreen::Existing | NativeFullScreen::Entered),
            true,
        ) => add_full_screen_action(&mut decision.actions, focused_instance),
        (
            FocusDepth::InstanceFullScreen(NativeFullScreen::Existing | NativeFullScreen::Entered),
            false,
        ) => {
            decision.focus_depth = Some(FocusDepth::Instance);
            add_regular_action(&mut decision.actions, focused_instance);
        }
        (FocusDepth::InstanceFullScreen(NativeFullScreen::Requested), false)
        | (FocusDepth::Instance, _)
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
        || !matches!(
            focus_depth,
            FocusDepth::InstanceFullScreen(NativeFullScreen::Existing | NativeFullScreen::Entered)
        )
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
