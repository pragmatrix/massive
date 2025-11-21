use std::collections::HashMap;

use derive_more::Deref;
use massive_applications::{InstanceId, ViewCreationInfo, ViewId};

#[derive(Debug, Deref)]
pub struct ViewInfo {
    #[deref]
    pub creation_info: ViewCreationInfo,
}

#[derive(Debug, Default)]
pub struct ViewManager {
    views: HashMap<ViewId, ViewInfo>,
    instance_views: HashMap<InstanceId, Vec<ViewId>>,
}

impl ViewManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_view(&mut self, instance_id: InstanceId, creation_info: ViewCreationInfo) {
        let id = creation_info.id;
        let info = ViewInfo {
            creation_info,
        };
        self.views.insert(id, info);
        self.instance_views.entry(instance_id).or_default().push(id);
    }

    pub fn remove_view(&mut self, instance_id: InstanceId, id: ViewId) {
        if let Some(views) = self.instance_views.get_mut(&instance_id) {
            if let Some(pos) = views.iter().position(|v| *v == id) {
                views.remove(pos);
                self.views.remove(&id);
            }
        }
    }

    pub fn remove_instance_views(&mut self, instance_id: InstanceId) {
        if let Some(views) = self.instance_views.remove(&instance_id) {
            for view_id in views {
                self.views.remove(&view_id);
            }
        }
    }

    #[allow(dead_code)]
    pub fn get(&self, id: ViewId) -> Option<&ViewInfo> {
        self.views.get(&id)
    }
}
