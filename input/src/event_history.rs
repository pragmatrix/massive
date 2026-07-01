use std::collections::VecDeque;
use std::time::{Duration, Instant};

use winit::event::{DeviceId, ElementState, MouseButton};

use massive_geometry::Point;

use crate::event_aggregator::DeviceStates;
use crate::{AggregationEvent, InputEvent};

#[derive(Debug)]
pub struct EventHistory<E: InputEvent> {
    max_duration: Duration,
    current_id: u64,
    /// The event records, most recent first.
    records: VecDeque<EventRecord<E>>,
}

impl<E: InputEvent> EventHistory<E> {
    pub fn new(max_duration: Duration) -> Self {
        Self {
            max_duration,
            current_id: 0,
            records: Default::default(),
        }
    }

    pub fn push(&mut self, event: E, time: Instant, states: DeviceStates) {
        if let Some(front) = self.records.front()
            && front.time() > time
        {
            panic!("New event arrived with an earlier timestamp.")
        }

        self.gc(time);

        let record = EventRecord {
            id: self.next_id(),
            event,
            time,
            states,
        };

        self.records.push_front(record);
    }

    /// The current record.
    pub fn current(&self) -> Option<&EventRecord<E>> {
        self.records.front()
    }

    /// The previous record.
    pub fn previous(&self) -> Option<&EventRecord<E>> {
        self.records.get(1)
    }
    /// An iterator over historic events. These are all events that came before, starting with the
    /// event that came before the current / most recent one.
    pub fn historic(&self) -> impl HistoryIterator<'_, Event = E> {
        self.iter().skip(1)
    }

    /// An iterator that iterates over all events, including the most recent one.
    pub fn iter(&self) -> impl HistoryIterator<'_, Event = E> {
        self.records.iter()
    }

    /// Returns the Record at the `point` or the one that is newer.
    pub fn at_or_newer(&self, point: impl Into<RecordPoint>) -> Option<&EventRecord<E>> {
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
pub struct EventRecord<E: InputEvent> {
    pub id: u64,
    pub event: E,
    pub time: Instant,
    // Memory: may recycle states if they don't change (e.g. use `Arc`).
    pub states: DeviceStates,
}

impl<E: InputEvent> EventRecord<E> {
    pub fn is_mouse_event(
        &self,
        element_state: ElementState,
        mouse_button: MouseButton,
    ) -> Option<DeviceId> {
        match self.event().to_aggregation_event()? {
            AggregationEvent::MouseInput {
                device_id,
                state,
                button,
            } if state == element_state && button == mouse_button => Some(device_id),
            _ => None,
        }
    }

    pub fn is_cursor_moved_event(&self) -> Option<DeviceId> {
        if let AggregationEvent::CursorMoved { device_id, .. } =
            self.event().to_aggregation_event()?
        {
            Some(device_id)
        } else {
            None
        }
    }

    pub fn event(&self) -> &E {
        &self.event
    }

    pub fn time(&self) -> Instant {
        self.time
    }
}

pub trait HistoryIterator<'a>: Iterator<Item = &'a EventRecord<Self::Event>> {
    type Event: InputEvent;

    /// Skips over events back in time until (but not including) [`Instant`] or an `EntryId`.
    fn from<P>(
        self,
        until: P,
    ) -> impl Iterator<Item = &'a EventRecord<Self::Event>> + use<'a, P, Self>
    where
        P: Into<RecordPoint>;

    /// Iterates over events back in time until (but not including) [`Instant`] or an `EntryId`.
    fn until<P>(
        self,
        until: P,
    ) -> impl Iterator<Item = &'a EventRecord<Self::Event>> + use<'a, P, Self>
    where
        P: Into<RecordPoint>;

    /// Returns the maximum distance a pointer device moved in relation to the given point in the
    /// range of all events.
    fn max_distance_moved(self, device_id: DeviceId, pos: Point) -> f64;
}

impl<'a, T, E> HistoryIterator<'a> for T
where
    T: Iterator<Item = &'a EventRecord<E>> + 'a,
    E: InputEvent,
{
    type Event = E;

    fn from<P>(self, point: P) -> impl Iterator<Item = &'a EventRecord<E>> + use<'a, P, E, T>
    where
        P: Into<RecordPoint>,
    {
        let point = point.into();
        self.skip_while(move |record| record.received_after(point))
    }

    fn until<P>(self, point: P) -> impl Iterator<Item = &'a EventRecord<E>> + use<'a, P, E, T>
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
        // Can't use `max`, because floats do not implement `Ord`.
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

impl<E: InputEvent> From<&EventRecord<E>> for RecordPoint {
    fn from(e: &EventRecord<E>) -> Self {
        RecordPoint::EntryId(e.id)
    }
}

impl<E: InputEvent> EventRecord<E> {
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
