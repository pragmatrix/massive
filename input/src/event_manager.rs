use std::time::Duration;

use crate::{Event, EventAggregator, ExternalEvent, event_history::EventHistory};

#[derive(Debug)]
pub struct EventManager {
    aggregator: EventAggregator,
    history: EventHistory,
}

/// The maximum time needed for detecting a gesture. This currently equals to the maximum time we
/// store past events.
const DEFAULT_MAXIMUM_DETECTION_DURATION: Duration = Duration::from_secs(10);

impl Default for EventManager {
    fn default() -> Self {
        Self::new(DEFAULT_MAXIMUM_DETECTION_DURATION)
    }
}

impl EventManager {
    pub fn new(max_detection_duration: Duration) -> Self {
        Self {
            aggregator: Default::default(),
            history: EventHistory::new(max_detection_duration),
        }
    }

    /// Add a new event at the current time.
    pub fn event(&mut self, event: ExternalEvent) -> Event<'_> {
        self.aggregator.update(&event);
        self.history.push(event, self.aggregator.to_device_states());
        Event::new(&self.history)
    }
}
