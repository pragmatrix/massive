use massive_renderer::FontManager;
use tokio::sync::mpsc::UnboundedSender;

use crate::{InstanceCommand, InstanceId};

#[derive(Debug, Clone)]
pub struct InstanceEnvironment {
    pub(crate) command_sender: UnboundedSender<(InstanceId, InstanceCommand)>,
    // Robustness: This might change on runtime.
    pub(crate) primary_monitor_scale_factor: f64,
    pub(crate) font_manager: FontManager,
}

impl InstanceEnvironment {
    pub fn new(
        requests_tx: UnboundedSender<(InstanceId, InstanceCommand)>,
        primary_monitor_scale_factor: f64,
        font_manager: FontManager,
    ) -> Self {
        Self {
            command_sender: requests_tx,
            primary_monitor_scale_factor,
            font_manager,
        }
    }
}
