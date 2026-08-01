use std::mem;

use winit::event::DeviceId;

use massive_util::CoalescingKey;

use crate::{InstanceId, PresentationId, ViewEvent, ViewId};

#[derive(Debug, Clone)]
pub enum ApplicationEvent<T> {
    View(ViewId, ViewEvent),
    ApplyAnimations(PresentationId),
    Shutdown(InstanceId),
    Custom(T),
}

#[derive(Debug, Clone)]
pub enum ApplicationMessage {
    View(ViewId, ViewEvent),
    ApplyAnimations(PresentationId),
    Shutdown(InstanceId),
}

impl<T> From<ApplicationMessage> for ApplicationEvent<T> {
    fn from(value: ApplicationMessage) -> Self {
        match value {
            ApplicationMessage::View(view_id, event) => Self::View(view_id, event),
            ApplicationMessage::ApplyAnimations(presentation_id) => {
                Self::ApplyAnimations(presentation_id)
            }
            ApplicationMessage::Shutdown(instance_id) => Self::Shutdown(instance_id),
        }
    }
}

impl CoalescingKey for ApplicationMessage {
    type Key = ApplicationEventCoalescingKey;

    fn coalescing_key(&self) -> Option<ApplicationEventCoalescingKey> {
        match self {
            ApplicationMessage::View(view_id, event) => match event {
                ViewEvent::Resized(..) => Some(ApplicationEventCoalescingKey::View(
                    *view_id,
                    mem::discriminant(event),
                    None,
                )),
                ViewEvent::CursorMoved { device_id, .. } => {
                    Some(ApplicationEventCoalescingKey::View(
                        *view_id,
                        mem::discriminant(event),
                        Some(*device_id),
                    ))
                }
                _ => None,
            },
            ApplicationMessage::ApplyAnimations(presentation_id) => Some(
                ApplicationEventCoalescingKey::ApplyAnimations(*presentation_id),
            ),
            ApplicationMessage::Shutdown(_) => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum ApplicationEventCoalescingKey {
    ApplyAnimations(PresentationId),
    View(ViewId, mem::Discriminant<ViewEvent>, Option<DeviceId>),
}
