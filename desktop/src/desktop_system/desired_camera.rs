//! The camera's desired view: it frames the keyboard focus at the zoom level, and keeps an
//! extended selection in view (ADR 0018, ADR 0032).

use derive_more::Debug;

use massive_geometry::PixelCamera;

use super::navigation::ZoomGeometry;
use super::presentation::selection_outline;
use super::selection::Selection;
use super::{DesktopLayoutState, DesktopSystem, FocusableTarget, LayoutContext, ZoomLevel};
use crate::projects::slot_extent;

impl DesktopSystem {
    /// Lends the camera's framing the parts of the desktop system it reads.
    pub fn camera_framing(&self) -> CameraFraming<'_> {
        CameraFraming {
            focused: self.keyboard_focus.focused(),
            zoom_level: self.zoom_level,
            selection: &self.selection,
            layout_state: &self.layout_state,
            layout_context: self.aggregates.layout_context(self.layout_inputs()),
        }
    }
}

/// What the camera's desired view is computed from: the keyboard focus, the zoom level, the
/// selection, and the placed layout. Computing it reaches nothing else of the desktop system.
#[derive(Debug)]
pub struct CameraFraming<'a> {
    /// `None` while nothing has the keyboard focus, so there is nothing to frame.
    focused: Option<FocusableTarget>,
    zoom_level: ZoomLevel,
    selection: &'a Selection,
    #[debug(skip)]
    layout_state: &'a DesktopLayoutState,
    layout_context: LayoutContext<'a>,
}

impl CameraFraming<'_> {
    /// The camera that frames the focused target at the zoom level, widened to an extended
    /// selection. `None` while nothing has the keyboard focus.
    pub fn desired_camera(&self) -> Option<PixelCamera> {
        let focused = self.focused?;
        let zoom_geometry = self.zoom_geometry();
        let camera = zoom_geometry.resolve_camera_for_target(focused, self.zoom_level);
        let Some(selection_camera) = self.selection_camera(focused, &zoom_geometry) else {
            return Some(camera);
        };
        Some(zoom_geometry.joined_camera(camera, selection_camera))
    }

    /// The camera that frames the extended selection around the `focused` target with a margin
    /// around it (ADR 0032). `None` when the selection has no outline.
    fn selection_camera(
        &self,
        focused: FocusableTarget,
        zoom_geometry: &ZoomGeometry,
    ) -> Option<PixelCamera> {
        let outline = selection_outline(
            self.selection,
            self.layout_state,
            focused,
            &self.layout_context,
        )?;
        Some(zoom_geometry.selection_camera(&outline, slot_extent(&self.layout_context)))
    }

    /// Where the camera points for a zoom level, from the placed layout.
    fn zoom_geometry(&self) -> ZoomGeometry<'_> {
        ZoomGeometry::new(
            self.layout_context.hierarchy,
            self.layout_context.configuration,
            self.layout_state,
            self.layout_context.window_size,
        )
    }
}
