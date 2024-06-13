use std::collections::HashMap;

use id_table::IdTable;
use massive_geometry::Matrix4;
use massive_scene::{Id, PositionedRenderShape, SceneChange, Shape};

mod id_table;

#[derive(Debug, Default)]
pub struct Scene {
    matrices: IdTable<Matrix4>,
    shapes: IdTable<PositionedRenderShape>,
}

impl Scene {
    pub fn apply(&mut self, change: SceneChange) {
        match change {
            SceneChange::Matrix(change) => self.matrices.apply(change),
            SceneChange::PositionedShape(change) => self.shapes.apply(change),
        }
    }

    pub fn grouped_shapes(&self) -> impl Iterator<Item = (&Matrix4, Vec<&Shape>)> {
        let mut map: HashMap<Id, Vec<&Shape>> = HashMap::new();

        for positioned in self.shapes.iter() {
            let matrix_id = positioned.matrix;
            map.entry(matrix_id).or_default().push(&positioned.shape);
        }

        map.into_iter().map(|(matrix_id, shapes)| {
            let matrix = &self.matrices[matrix_id];
            (matrix, shapes)
        })
    }

    pub fn reset(&mut self) {
        self.matrices.reset();
        self.shapes.reset();
    }
}
