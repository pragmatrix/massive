use massive_geometry::Point;

use crate::tracker::Movement;

#[derive(Debug, Clone)]
pub enum MouseGesture {
    /// Single click (this can be also the first click of a double click gesture, meaning that the
    /// double click gesture detector does not suppress single click gesture detection)
    Click(Point),
    /// Mouse double click.
    DoubleClick(Point),
    /// A movement while the sensor was pressed got detected.
    Movement(Movement),
    Clicked(Point),
}
