use std::time::Duration;

use crate::{
    AggregationReport, Event, EventAggregator, ExternalEvent, event_history::EventHistory,
};

// Naming: GestureDetector?
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
    ///
    /// `None`: The event is redundant in terms of the state update. Like a CursorMoved event that
    /// moves the same device to the same point as before. This happens on winit when a mouse state
    /// is changed, for example.
    ///
    /// Architecture: Even aggregation and event queries should be part of the massive shell.
    pub fn add_event(&mut self, event: ExternalEvent) -> Option<Event<'_>> {
        if self.aggregator.update(&event) == AggregationReport::Redundant {
            return None;
        }

        self.history.push(event, self.aggregator.to_device_states());
        Some(Event::new(&self.history))
    }
}
