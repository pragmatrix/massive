use massive_layout::{LayoutAxis, Thickness};

#[derive(Debug)]
pub struct ContainerBuilder {
    axis: LayoutAxis,
    padding: Thickness<2>,
    spacing: u32,
}

impl ContainerBuilder {
    pub fn new(axis: LayoutAxis) -> Self {
        Self {
            axis,
            padding: Default::default(),
            spacing: 0,
        }
    }

    pub(crate) fn into_parts(self) -> (LayoutAxis, Thickness<2>, u32) {
        (self.axis, self.padding, self.spacing)
    }
}

// We seem to benefit from .into() and to_container() invocations. to_container is useful for
// chaining follow ups to the builder.

impl From<LayoutAxis> for ContainerBuilder {
    fn from(axis: LayoutAxis) -> Self {
        ContainerBuilder::new(axis)
    }
}

pub trait ToContainer {
    fn to_container(self) -> ContainerBuilder;
}

impl ToContainer for LayoutAxis {
    fn to_container(self) -> ContainerBuilder {
        ContainerBuilder::new(self)
    }
}
