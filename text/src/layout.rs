use cgmath::EuclideanSpace;
use granularity::{map, Value};
use granularity_geometry::{Bounds3, Matrix4, Point3, Size3, Vector3};

pub trait Extent {
    /// The size of the object.
    fn size(&self) -> Size3;

    /// The baseline of the object in layout direction, if any.
    ///
    /// Also the `ascent` of the object. Always positive. Describes the distance of the baseline from the top of the extent.
    fn baseline(&self) -> Option<f64> {
        None
    }
}

pub fn center<E: Extent>(container: Value<Bounds3>, inner: Value<E>) -> Value<Matrix4> {
    map!(|*container, inner| {
        let container_size = container.size();
        let inner_size = inner.size();
        let container_center = container.min + container_size / 2.0;
        let inner_center: Point3 = Point3::origin() + Vector3::from(inner_size / 2.0);
        let offset = container_center - inner_center;
        Matrix4::from_translation(offset)
    })
}
