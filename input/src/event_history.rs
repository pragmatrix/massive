use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

use super::ExternalEvent;
use crate::event_aggregator::DeviceStates;
use massive_geometry::Point;

#[derive(Debug)]
pub struct EventHistory {
    max_duration: Duration,
    current_id: u64,
    /// The event records, most recent first.
    records: VecDeque<EventRecord>,
}

impl EventHistory {
    pub fn new(max_duration: Duration) -> Self {
        Self {
            max_duration,
            current_id: 0,
            records: Default::default(),
        }
    }

    pub fn push(&mut self, event: ExternalEvent, states: DeviceStates) {
        let time = event.time();
        if let Some(front) = self.records.front()
            && front.time() > time
        {
            panic!("New event arrived with an earlier timestamp.")
        }

        self.gc(time);

        let record = EventRecord {
            id: self.next_id(),
            event,
            states,
        };

        self.records.push_front(record);
    }

    /// The current record.
    pub fn current(&self) -> Option<&EventRecord> {
        self.records.front()
    }

    /// The previous record.
    pub fn previous(&self) -> Option<&EventRecord> {
        self.records.get(1)
    }

    /// An iterator over all events, including the most recent one including.
    pub fn iter(&self) -> impl Iterator<Item = &EventRecord> {
        // Can't use HistoryIterator as return type:
        // <https://github.com/rust-lang/rust-analyzer/issues/9881>
        self.records.iter()
    }

    /// An iterator over historic events. These are all events that came before, starting with the
    /// event that came before the current / most recent one.
    pub fn historic(&self) -> impl Iterator<Item = &EventRecord> {
        // Can't use HistoryIterator as return type:
        // <https://github.com/rust-lang/rust-analyzer/issues/9881>
        self.iter().skip(1)
    }

    /// Returns the Record at the `point` or the one that is newer.
    pub fn at_or_newer(&self, point: impl Into<RecordPoint>) -> Option<&EventRecord> {
        // TODO: optimize this by using binary searches.
        self.iter().from(point).next()
    }

    pub fn max_duration(&self) -> Duration {
        self.max_duration
    }

    fn next_id(&mut self) -> u64 {
        self.current_id += 1;
        self.current_id
    }

    fn gc(&mut self, now: Instant) {
        let oldest = now - self.max_duration;
        while let Some(back) = self.records.back() {
            if back.time() > oldest {
                break;
            }
            self.records.pop_back();
        }
    }
}

/// A record of an [`Event`] and all device states at that time.
#[derive(Debug)]
pub struct EventRecord {
    pub id: u64,
    pub event: ExternalEvent,
    // TODO: may recycle states if they don't change (use `Rc`).
    pub states: DeviceStates,
}

impl EventRecord {
    pub fn is_mouse_event(
        &self,
        element_state: ElementState,
        mouse_button: MouseButton,
    ) -> Option<DeviceId> {
        match &self.event {
            ExternalEvent::Window {
                event:
                    WindowEvent::MouseInput {
                        device_id,
                        state,
                        button,
                        ..
                    },
                ..
            } if *state == element_state && *button == mouse_button => Some(*device_id),
            _ => None,
        }
    }

    pub fn is_cursor_moved_event(&self) -> Option<DeviceId> {
        if let WindowEvent::CursorMoved { device_id, .. } = self.window_event()? {
            Some(*device_id)
        } else {
            None
        }
    }

    pub fn window_event(&self) -> Option<&WindowEvent> {
        let ExternalEvent::Window { event, .. } = &self.event;
        Some(event)
    }

    pub fn time(&self) -> Instant {
        self.event.time()
    }
}

pub trait HistoryIterator<'a>: Iterator<Item = &'a EventRecord> {
    /// Skips over events back in time until (but not including) [`Instant`] or an `EntryId`.
    fn from<P>(self, until: P) -> impl Iterator<Item = &'a EventRecord> + use<'a, P, Self>
    where
        P: Into<RecordPoint>;

    /// Iterates over events back in time until (but not including) [`Instant`] or an `EntryId`.
    fn until<P>(self, until: P) -> impl Iterator<Item = &'a EventRecord> + use<'a, P, Self>
    where
        P: Into<RecordPoint>;

    /// Returns the maximum distance a pointer device moved in relation to the given point in the
    /// range of all events.
    fn max_distance_moved(self, device_id: DeviceId, pos: Point) -> f64;
}

impl<'a, T> HistoryIterator<'a> for T
where
    T: Iterator<Item = &'a EventRecord> + 'a,
{
    fn from<P>(self, point: P) -> impl Iterator<Item = &'a EventRecord> + use<'a, P, T>
    where
        P: Into<RecordPoint>,
    {
        let point = point.into();
        self.skip_while(move |record| record.received_after(point))
    }

    fn until<P>(self, point: P) -> impl Iterator<Item = &'a EventRecord> + use<'a, P, T>
    where
        P: Into<RecordPoint>,
    {
        let point = point.into();
        self.take_while(move |record| record.received_after(point))
    }

    fn max_distance_moved(self, device_id: DeviceId, pos: Point) -> f64 {
        self.map(|e| {
            e.states
                .pos(device_id)
                .map(|p| (pos - p).length())
                .unwrap_or_default()
        })
        // can't use `max`, because floats do not implement `Ord`.
        .reduce(|a, b| if a > b { a } else { b })
        .unwrap_or(0.0)
    }
}

/// Represents a point in time in relation to a the event history.
#[derive(Copy, Clone, Debug)]
pub enum RecordPoint {
    Instant(Instant),
    EntryId(u64),
}

impl From<Instant> for RecordPoint {
    fn from(i: Instant) -> Self {
        RecordPoint::Instant(i)
    }
}

impl From<&EventRecord> for RecordPoint {
    fn from(e: &EventRecord) -> Self {
        RecordPoint::EntryId(e.id)
    }
}

impl EventRecord {
    /// Was the record received after the given `p` point.
    fn received_after(&self, p: RecordPoint) -> bool {
        use RecordPoint::*;
        match p {
            Instant(i) => self.time() > i,
            EntryId(id) => self.id > id,
        }
    }

    #[allow(unused)]
    /// Was the record received before the given `p` point.
    fn received_before(&self, p: RecordPoint) -> bool {
        use RecordPoint::*;
        match p {
            Instant(i) => self.time() < i,
            EntryId(id) => self.id < id,
        }
    }
}
