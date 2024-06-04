use id_table::IdTable;
use massive_geometry::Matrix4;
use massive_scene::{PositionedShape, SceneChange};

mod id_table;

#[derive(Debug, Default)]
struct Scene {
    matrices: IdTable<Matrix4>,
    shapes: IdTable<PositionedShape>,
}

impl Scene {
    pub fn apply(&mut self, change: SceneChange) {
        match change {
            SceneChange::Matrix(change) => self.matrices.apply(change),
            SceneChange::PositionedShape(change) => self.shapes.apply(change),
        }
    }



}
