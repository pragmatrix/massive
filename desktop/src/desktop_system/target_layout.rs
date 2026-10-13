use anyhow::Result;

use massive_applications::{InstanceId, ViewId};
use massive_geometry::{SizePx, SizedTransform, Transform};
use massive_layout::{MeasuredLayout, Placement, Size};

use super::DesktopTarget;
use crate::OrderedHierarchy;
use crate::instance_presenter::InstanceTitleBarMetrics;
use crate::projects::RuntimeConfiguration;

/// How a presenter's parts are measured, placed and applied (ADR 0022).
pub trait TargetLayout {
    /// `()` for presenters that are a single part.
    type Part: Copy;

    fn measure(
        &self,
        part: Self::Part,
        context: &LayoutContext,
        children: &[MeasuredLayout<2>],
    ) -> MeasuredLayout<2>;

    /// Writes one placement per child into `placements`.
    fn place_children(
        &self,
        part: Self::Part,
        context: &LayoutContext,
        parent_size: Size<2>,
        children: &[MeasuredLayout<2>],
        placements: &mut [Placement<Transform, 2>],
    );

    fn apply_layout(
        &mut self,
        part: Self::Part,
        layout: SizedTransform,
        visible: bool,
        animate: bool,
    ) -> Result<AppliedLayout>;
}

/// What measuring and placing read beyond the presenter itself.
#[derive(Debug)]
pub struct LayoutContext<'a> {
    pub hierarchy: &'a OrderedHierarchy<DesktopTarget>,
    pub configuration: &'a RuntimeConfiguration,
    pub default_panel_size: SizePx,
    pub window_size: SizePx,
    /// The visor collapses only while one of its instances holds keyboard focus.
    pub focused_instance: Option<InstanceId>,
    pub title_bar_metrics: InstanceTitleBarMetrics,
}

/// What applying a layout asks of the system beyond the scene update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedLayout {
    Applied,
    /// The view presents at a new size, which its application must learn.
    ViewResized {
        view: ViewId,
        size: SizePx,
    },
}
