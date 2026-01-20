use massive_geometry::Rect;
use massive_scene::Transform;

pub enum NavigationObject<'a, Target> {
    Leaf {
        target: Target,
        transform: Transform,
        rect: Rect,
    },
    Container {
        id: Target,
        transform: Transform,
        /// This is used for deciding if nested objects are queried on hit testing. None: Query them all.
        rect: Option<Rect>,
        nested: Box<dyn Fn() -> Vec<NavigationObject<'a, Target>> + 'a>,
    },
}

impl<Target> NavigationObject<'_, Target> {
    pub fn with_transform(mut self, tf: Transform) -> Self {
        match &mut self {
            NavigationObject::Leaf { transform, .. } => *transform = tf,
            NavigationObject::Container { transform, .. } => *transform = tf,
        }
        self
    }
}

pub fn leaf<'a, Target>(id: impl Into<Target>, rect: Rect) -> NavigationObject<'a, Target> {
    NavigationObject::Leaf {
        target: id.into(),
        transform: Transform::IDENTITY,
        rect,
    }
}

pub fn container<'a, Target>(
    id: impl Into<Target>,
    f: impl Fn() -> Vec<NavigationObject<'a, Target>> + 'a,
) -> NavigationObject<'a, Target> {
    NavigationObject::Container {
        id: id.into(),
        transform: Transform::IDENTITY,
        rect: None,
        nested: Box::new(f),
    }
}
