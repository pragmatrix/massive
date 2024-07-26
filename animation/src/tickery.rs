use std::{cell::RefCell, collections::HashMap, rc::Weak, time::Instant};

#[derive(Debug)]
pub struct Tickery {
    receivers: RefCell<HashMap<*const dyn ReceivesTicks, Weak<dyn ReceivesTicks>>>,
}

impl Tickery {
    pub fn start_sending(&self, receiver: Weak<dyn ReceivesTicks>) {
        let ptr = receiver.as_ptr();
        assert!(self.receivers.borrow_mut().insert(ptr, receiver).is_none());
    }
}

#[derive(Debug)]
pub enum TickResponse {
    Continue,
    Stop,
}

pub trait ReceivesTicks {
    #[must_use]
    fn tick(&self, instant: Instant) -> TickResponse;
}

pub trait TickProvider {
    fn start_sending(&self);
}

impl ReceivesTicks for Tickery {
    fn tick(&self, instant: Instant) -> TickResponse {
        let mut receivers = self.receivers.borrow_mut();

        let mut removal_queue = Vec::new();

        for (ptr, registration) in receivers.iter() {
            if let Some(registration) = registration.upgrade() {
                match registration.tick(instant) {
                    TickResponse::Stop => {
                        removal_queue.push(*ptr);
                    }
                    TickResponse::Continue => {}
                }
            } else {
                removal_queue.push(*ptr);
            }
        }

        // Cleanup
        removal_queue
            .into_iter()
            .for_each(|ptr| assert!(receivers.remove(&ptr).is_some()));

        if receivers.is_empty() {
            TickResponse::Stop
        } else {
            TickResponse::Continue
        }
    }
}
