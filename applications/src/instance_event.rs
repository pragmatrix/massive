use std::mem;

use winit::event::DeviceId;

use massive_util::CoalescingKey;

use crate::{
    InstanceId, PresentationId, PresentationTick, ToolCall, ToolCallId, ViewEvent, ViewId,
};

#[derive(Debug, Clone)]
pub enum InstanceEvent<T> {
    View(ViewId, ViewEvent),
    ApplyAnimations(PresentationTick),
    FullscreenRequested,
    Shutdown(InstanceId),
    /// Only instances receive tool calls (ADR 0036). The shell never sends them to its application.
    ToolCall(ToolCall),
    /// The agent cancelled the call, or its tool connection closed. An instance gets at most one
    /// cancel per call, and it may still answer.
    CancelToolCall(ToolCallId),
    Custom(T),
}

#[derive(Debug, Clone)]
pub enum InstanceMessage {
    View(ViewId, ViewEvent),
    ApplyAnimations(PresentationTick),
    FullscreenRequested,
    Shutdown(InstanceId),
    ToolCall(ToolCall),
    CancelToolCall(ToolCallId),
}

impl<T> From<InstanceMessage> for InstanceEvent<T> {
    fn from(value: InstanceMessage) -> Self {
        match value {
            InstanceMessage::View(view_id, event) => Self::View(view_id, event),
            InstanceMessage::ApplyAnimations(tick) => Self::ApplyAnimations(tick),
            InstanceMessage::FullscreenRequested => Self::FullscreenRequested,
            InstanceMessage::Shutdown(instance_id) => Self::Shutdown(instance_id),
            InstanceMessage::ToolCall(call) => Self::ToolCall(call),
            InstanceMessage::CancelToolCall(call_id) => Self::CancelToolCall(call_id),
        }
    }
}

impl CoalescingKey for InstanceMessage {
    type Key = InstanceEventCoalescingKey;

    fn coalescing_key(&self) -> Option<InstanceEventCoalescingKey> {
        match self {
            InstanceMessage::FullscreenRequested => None,
            InstanceMessage::View(view_id, event) => match event {
                ViewEvent::Resized(..) => Some(InstanceEventCoalescingKey::View(
                    *view_id,
                    mem::discriminant(event),
                    None,
                )),
                ViewEvent::CursorMoved { device_id, .. } => Some(InstanceEventCoalescingKey::View(
                    *view_id,
                    mem::discriminant(event),
                    Some(*device_id),
                )),
                _ => None,
            },
            // Coalescing keeps the last tick, so a consumer that fell behind applies the newest
            // vblank time.
            InstanceMessage::ApplyAnimations(tick) => Some(
                InstanceEventCoalescingKey::ApplyAnimations(tick.presentation_id),
            ),
            InstanceMessage::Shutdown(_)
            | InstanceMessage::ToolCall(_)
            | InstanceMessage::CancelToolCall(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum InstanceEventCoalescingKey {
    ApplyAnimations(PresentationId),
    View(ViewId, mem::Discriminant<ViewEvent>, Option<DeviceId>),
}
