use std::time::{Duration, Instant};

use winit::event::{DeviceId, ElementState, MouseButton};

use crate::{
    DeviceIdExtensions, Sensor,
    event_history::{EventHistory, HistoryIterator},
    tracker::Movement,
};
use massive_geometry::Point;

impl EventHistory {
    pub fn detect_double_click(
        &self,
        button: MouseButton,
        max_duration: Duration,
        max_distance: f64,
    ) -> Option<Point> {
        debug_assert!(max_duration < self.max_duration());
        let record = self.current()?;

        let device = record.is_mouse_event(ElementState::Pressed, button)?;
        let pos: Point = record.states.pos(device)?;

        let previous_press = self
            .historic()
            .until(record.time() - max_duration)
            .find(|e| e.is_mouse_event(ElementState::Pressed, button) == Some(device))?;

        (self
            .historic()
            .until(previous_press)
            .max_distance_moved(device, pos)
            <= max_distance)
            .then_some(pos)
    }

    /// Returns the duration since the mouse button get pressed most recently at the time of the current
    /// event.  
    /// This detects pressing for all events. This make this work as expected, the frame tick events
    /// must be subscribed after an initial click event.
    pub fn detect_pressing(&self, button: MouseButton) -> Option<(DeviceId, (Point, Duration))> {
        let current = self.current()?;
        // Only considering the most recent one.
        let (device_id, (when_pressed, from)) = current.states.all_pressed(button).next()?;
        let duration = current.time() - when_pressed;
        // The gesture "pressing" is not defined at the moment the sensor gets clicked.
        // TODO: Not so sure about this anymore.
        (!duration.is_zero()).then_some((device_id, (from, duration)))
    }

    /// Detects significant movement since the time a mouse button was pressed.
    pub fn detect_movement(&self, button: MouseButton, minimum_distance: f64) -> Option<Movement> {
        let current_event = self.current()?;
        let device = current_event.is_cursor_moved_event()?;
        let sensor = device.sensor(button);
        let (when_pressed, from) = current_event.states.is_pressed(sensor)?;
        // Deduplication: Has the cursor been moved before over the threshold since pressed without
        // taking the current event into account?
        // This check is needed after a cancelling event was detected so that we don't start the
        // movement again.
        // TODO: Find a better way.
        if self
            .historic()
            .until(when_pressed)
            .max_distance_moved(device, from)
            >= minimum_distance
        {
            return None;
        }
        let pos_now = current_event.states.pos(device)?;
        let movement = pos_now - from;
        (movement.length() >= minimum_distance).then(|| Movement {
            sensor,
            began: when_pressed,
            detected_after: current_event.time() - when_pressed,
            from,
            delta: movement,
            minimum_distance,
        })
    }

    /// Detect if there is recent movement activity. Returns the [`Duration`] since when inactivity
    /// began.
    ///
    /// - Returns `None` if movement activity can not be determined.
    pub fn detect_movement_inactivity(
        &self,
        device: DeviceId,
        // How far should we go back to detect inactivity at max?
        check_range: Duration,
        minimum_distance: f64,
    ) -> Option<Duration> {
        // We take the current position as the basis for comparison (Should be symmetric).
        let current = self.current()?;
        let current_pos = current.states.pos(device)?;

        if let Some(r) = self
            .historic()
            .until(current.time() - check_range)
            .find(|r| {
                if let Some(pos) = r.states.pos(device) {
                    (current_pos - pos).length() >= minimum_distance
                } else {
                    false
                }
            })
        {
            // Actually this is the time we consider movement, so shouldn't we return the time of the
            // next event?
            return Some(current.time() - r.time());
        }

        // No Activity found in the range, we return the full range.

        Some(check_range)
    }

    /// Returns the instant when the holding period ended when the button was held for `min_duration` or
    /// longer.  
    /// A movement over `distance_considered_a_movement` breaks the holding period.
    pub fn held(
        &self,
        sensor: Sensor,
        distance_considered_a_movement: f64,
        duration_considered_a_hold: Duration,
    ) -> Option<Instant> {
        // Type of current event does not matter, just the timestamp is important.
        let current = self.current()?;
        let (when_pressed, from) = current.states.is_pressed(sensor)?;
        let holding_period_end = when_pressed + duration_considered_a_hold;
        if holding_period_end > current.time() {
            // Holding period hasn't passed yet.
            return None;
        }
        // Holding period passed

        // Check first if there was a movement while the holding.
        // TODO: Isn't this something of a requirement that the caller should decide?
        if self
            .iter()
            .from(holding_period_end)
            .until(when_pressed)
            .max_distance_moved(sensor.device, from)
            >= distance_considered_a_movement
        {
            return None;
        }

        Some(holding_period_end)
    }

    /// Returns [`Movement`] when it happened after a holding period.
    pub fn movement_after_hold(
        &self,
        button: MouseButton,
        min_hold: Duration,
        distance_considered_movement: f64,
    ) -> Option<Movement> {
        let current_event = self.current()?;
        let device = current_event.is_cursor_moved_event()?;
        let sensor = device.sensor(button);

        let holding_period_end = self.held(sensor, distance_considered_movement, min_hold)?;
        let holding_period_end_record = self.at_or_newer(holding_period_end)?;

        let holding_period_end_pos = holding_period_end_record.states.pos(device)?;
        let pos_now = current_event.states.pos(device)?;
        let movement = pos_now - holding_period_end_pos;

        (movement.length() >= distance_considered_movement).then(|| Movement {
            sensor,
            began: holding_period_end,
            detected_after: current_event.time() - holding_period_end_record.time(),
            from: holding_period_end_pos,
            delta: movement,
            minimum_distance: distance_considered_movement,
        })
    }
}
