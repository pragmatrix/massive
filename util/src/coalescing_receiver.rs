use std::{collections::VecDeque, hash::Hash};

use anyhow::{Result, bail};
use tokio::sync::mpsc::{UnboundedReceiver, error::TryRecvError};

use crate::message_filter;

#[derive(Debug)]
pub struct CoalescingReceiver<T: CoalescingKey> {
    receiver: UnboundedReceiver<T>,
    pending: VecDeque<T>,
}

pub trait CoalescingKey {
    type Key: Eq + Hash;

    fn coalescing_key(&self) -> Option<Self::Key>;
}

impl<T: CoalescingKey> From<UnboundedReceiver<T>> for CoalescingReceiver<T> {
    fn from(receiver: UnboundedReceiver<T>) -> Self {
        Self::new(receiver)
    }
}

impl<T: CoalescingKey> CoalescingReceiver<T> {
    pub fn new(receiver: UnboundedReceiver<T>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
        }
    }

    /// Receives an event and returns an error when the sender disconnects.
    pub async fn recv(&mut self) -> Result<T> {
        loop {
            // Pull in every event we can get.
            loop {
                match self.receiver.try_recv() {
                    Ok(event) => self.pending.push_back(event),
                    Err(TryRecvError::Disconnected) => {
                        bail!("Sender disconnected");
                    }
                    Err(TryRecvError::Empty) => {
                        break;
                    }
                }
            }

            // Skip Window events by key.
            //
            // This is to remove the lagging of resizes, redraws and
            // other events that are considered safe to skip without causing side effects.
            //
            // Robustness: Going from VecDequeue to Vec and back is a mess.
            //
            // Performance: Reuse capacity?
            {
                let events: Vec<T> =
                    message_filter::keep_last_per_key(self.pending.drain(..).collect(), |ev| {
                        ev.coalescing_key()
                    });
                self.pending = events.into();
            }

            // Any events?

            if let Some(pending) = self.pending.pop_front() {
                return Ok(pending);
            }

            // No events yet?, now we wait.
            if let Some(event) = self.receiver.recv().await {
                self.pending.push_back(event);
            } else {
                bail!("Sender disconnected");
            }
        }
    }
}
