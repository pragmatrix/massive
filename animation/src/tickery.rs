use std::sync::{self, Arc, Mutex};

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

use crate::{Interpolatable, Timeline};

#[derive(Debug, Default)]
pub struct Tickery {
    receivers: Mutex<Vec<sync::Weak<dyn ReceivesTicks>>>,
}

impl Tickery {
    pub fn timeline<T: Interpolatable + Send>(self: &Arc<Self>, value: T) -> Timeline<T> {
        Timeline::new(self.clone(), value)
    }

    pub fn tick(&self, instant: Instant) {
        self.receivers
            .lock()
            .expect("poisoned")
            .retain_mut(|registration| {
                if let Some(registration) = registration.upgrade() {
                    match registration.tick(instant) {
                        TickResponse::Stop => false,
                        TickResponse::Continue => true,
                    }
                } else {
                    false
                }
            });
    }

    pub fn wants_ticks(&self) -> bool {
        !self.receivers.lock().unwrap().is_empty()
    }

    pub(crate) fn start_sending(&self, receiver: sync::Weak<dyn ReceivesTicks>) {
        self.receivers.lock().unwrap().push(receiver);
    }
}

#[derive(Debug)]
pub enum TickResponse {
    Continue,
    Stop,
}

pub trait ReceivesTicks: Send + Sync {
    #[must_use]
    fn tick(&self, instant: Instant) -> TickResponse;
}

pub trait TickProvider {
    fn start_sending(&self);
}
