use std::time::{Duration, Instant};

use winit::event::{DeviceId, ElementState, MouseButton};

use crate::{
    Sensor,
    event_history::{EventHistory, HistoryIterator},
    tracker::Movement,
};
use massive_geometry::Point;

pub fn double_click(
    history: &EventHistory,
    button: MouseButton,
    max_duration: Duration,
    max_distance: f64,
) -> Option<Point> {
    debug_assert!(max_duration < history.max_duration());
    let record = history.current()?;

    let device = record.is_mouse_event(ElementState::Pressed, button)?;
    let pos: Point = record.states.pos(device)?;

    let previous_press = history
        .historic()
        .until(record.time() - max_duration)
        .find(|e| e.is_mouse_event(ElementState::Pressed, button) == Some(device))?;

    (history
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
pub fn pressing(
    history: &EventHistory,
    button: MouseButton,
) -> Option<(DeviceId, (Point, Duration))> {
    let current = history.current()?;
    // Only considering the most recent one.
    let (device_id, (when_pressed, from)) = current.states.all_pressed(button).next()?;
    let duration = current.time() - when_pressed;
    // The gesture "pressing" is not defined at the moment the sensor gets clicked.
    // TODO: Not so sure about this anymore.
    (!duration.is_zero()).then_some((device_id, (from, duration)))
}

/// Detects significant movement since the time a mouse button was pressed.
pub fn movement(
    history: &EventHistory,
    button: MouseButton,
    minimum_distance: f64,
) -> Option<Movement> {
    let current_event = history.current()?;
    let device_id = current_event.is_cursor_moved_event()?;
    let sensor = Sensor::new(device_id, button);
    let (when_pressed, from) = current_event.states.is_pressed(device_id, button)?;
    // Deduplication: Has the cursor been moved before over the threshold since pressed without
    // taking the current event into account?
    // This check is needed after a cancelling event was detected so that we don't start the
    // movement again.
    // TODO: Find a better way.
    if history
        .historic()
        .until(when_pressed)
        .max_distance_moved(device_id, from)
        >= minimum_distance
    {
        return None;
    }
    let pos_now = current_event.states.pos(device_id)?;
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
pub fn movement_inactivity(
    history: &EventHistory,
    device: DeviceId,
    // How far should we go back to detect inactivity at max?
    check_range: Duration,
    minimum_distance: f64,
) -> Option<Duration> {
    // We take the current position as the basis for comparison (Should be symmetric).
    let current = history.current()?;
    let current_pos = current.states.pos(device)?;

    if let Some(r) = history
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
#[allow(unused)]
fn held(
    history: &EventHistory,
    (device_id, button): (DeviceId, MouseButton),
    distance_considered_a_movement: f64,
    duration_considered_a_hold: Duration,
) -> Option<Instant> {
    // Type of current event does not matter, just the timestamp is important.
    let current = history.current()?;
    let (when_pressed, from) = current.states.is_pressed(device_id, button)?;
    let holding_period_end = when_pressed + duration_considered_a_hold;
    if holding_period_end > current.time() {
        // Holding period hasn't passed yet.
        return None;
    }
    // Holding period passed

    // Check first if there was a movement while the holding.
    // TODO: Isn't this something of a requirement that the caller should decide?
    if history
        .iter()
        .from(holding_period_end)
        .until(when_pressed)
        .max_distance_moved(device_id, from)
        >= distance_considered_a_movement
    {
        return None;
    }

    Some(holding_period_end)
}

/*
/// Returns [`Movement`] when it happened after a holding period.
pub fn movement_after_hold(
    history: &EventHistory,
    button: MouseButton,
    min_hold: Duration,
    distance_considered_movement: scalar,
) -> Option<Movement> {
    let current_event = history.current()?;
    let device_id = current_event.is_cursor_moved_event()?;
    let sensor = (device_id, button).into();

    let holding_period_end = held(
        history,
        (device_id, button),
        distance_considered_movement,
        min_hold,
    )?;

    let pos_holding_period_end = history
        .at_or_newer(holding_period_end)?
        .states
        .pos(device_id)?
        .into_point();
    let pos_now = current_event.states.pos(device_id)?.into_point();
    let movement = pos_now - pos_holding_period_end;

    (movement.length() >= distance_considered_movement).then(|| Movement {
        sensor,
        began: holding_period_end,
        from: pos_holding_period_end,
        movement,
    })
}

*/
