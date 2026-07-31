use std::collections::VecDeque;
use std::fmt;
use std::hash::Hash;

use anyhow::Result;
use anyhow::bail;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::error::TryRecvError;

use crate::message_filter;

#[derive(Debug)]
pub struct CoalescingReceiver<T: CoalescingKey + fmt::Debug> {
    receiver: UnboundedReceiver<T>,
    pending: VecDeque<T>,
}

pub trait CoalescingKey {
    type Key: Eq + Hash;

    fn coalescing_key(&self) -> Option<Self::Key>;
}

impl<T: CoalescingKey + fmt::Debug> From<UnboundedReceiver<T>> for CoalescingReceiver<T> {
    fn from(receiver: UnboundedReceiver<T>) -> Self {
        Self::new(receiver)
    }
}

impl<T: CoalescingKey + fmt::Debug> CoalescingReceiver<T> {
    pub fn new(receiver: UnboundedReceiver<T>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
        }
    }

    /// Receives all currently available events, waiting for at least one if none are pending. Returns an error if the sender disconnects before an event is received.
    pub async fn recv_all(&mut self) -> Result<Vec<T>> {
        if self.pending.is_empty() {
            let Some(event) = self.receiver.recv().await else {
                bail!("Sender disconnected");
            };
            self.pending.push_back(event);
        }

        while let Ok(event) = self.receiver.try_recv() {
            self.pending.push_back(event);
        }

        Ok(self.drain_and_coalesce_pending())
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

            self.pending = self.drain_and_coalesce_pending().into();

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

    fn drain_and_coalesce_pending(&mut self) -> Vec<T> {
        message_filter::keep_last_per_key(self.pending.drain(..).collect(), |event| {
            event.coalescing_key()
        })
    }
}
