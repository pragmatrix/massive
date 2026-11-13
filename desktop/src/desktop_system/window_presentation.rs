//! What the host's window shows of the desktop: its title, the cursor, and where the IME composes.

use anyhow::Result;

use massive_applications::ViewId;
use massive_geometry::{BoxPx, Point, Rect, SizePx};
use massive_renderer::RenderGeometry;

use super::{DesktopSystem, DesktopTarget};
use crate::coordinate_mapper::CoordinateMapper;
use crate::instance_manager::ViewPath;
use crate::instance_presenter::TITLE_SEPARATOR;
use crate::window_state::WindowPresentationState;

impl DesktopSystem {
    pub fn window_presentation_state(
        &self,
        surface_size: SizePx,
    ) -> Result<WindowPresentationState> {
        let (view_path, view_window_state) = self.focused_view_window_state()?.unzip();
        let view_window_state = view_window_state.unwrap_or_default();
        let (ime_view, input_area) =
            self.window_ime(view_path.zip(view_window_state.ime_area), surface_size);
        let title = self.window_title(view_window_state.title);
        let cursor = view_window_state.cursor;
        // Pointer-feedback state drives cursor visibility (hidden during keyboard navigation).
        let cursor_visible = self.event_router.pointer_focus().is_some();
        Ok(WindowPresentationState {
            title,
            cursor_visible,
            cursor,
            ime_view,
            input_area,
        })
    }

    /// The view the IME composes for, and where macOS places its text input UI. Only a focused
    /// view that declares an IME area gets the IME, so a composition never reaches a view that
    /// can't take text.
    fn window_ime(
        &self,
        focused_ime: Option<(ViewPath, Rect)>,
        surface_size: SizePx,
    ) -> (Option<ViewId>, Option<BoxPx>) {
        let ime_view = focused_ime.map(|(view_path, _)| view_path.view);
        let ime_target = focused_ime.map(|(view_path, area)| {
            (
                DesktopTarget::view(view_path.instance, view_path.view),
                area,
            )
        });
        (ime_view, self.input_area(ime_target, surface_size))
    }

    /// The focused view's IME area, or else the whole focused target, so the caps lock indicator
    /// follows the focus. `None` while nothing visible is focused.
    fn input_area(
        &self,
        ime: Option<(DesktopTarget, Rect)>,
        surface_size: SizePx,
    ) -> Option<BoxPx> {
        // Projected through the camera's target, so the area moves once instead of following an
        // animation; placements are layout targets already.
        let geometry = RenderGeometry::new(surface_size, *self.camera.target());
        let mapper =
            CoordinateMapper::new(&self.aggregates.hierarchy, &self.layout_state, &geometry);
        let (target, ime_area) = match ime {
            Some((target, area)) => (target, Some(area)),
            None => (self.focused_target()?, None),
        };
        let surface = mapper.surface(&target)?;
        let area = ime_area.unwrap_or_else(|| Rect::from_size(surface.size));
        let corners = area
            .to_quad()
            .into_iter()
            .map(|corner| mapper.to_surface(corner, &surface))
            .collect();
        Some(surface_bounds(corners))
    }

    fn window_title(&self, terminal_title: String) -> String {
        // Window title suffixes are configuration names, read from the aggregate, not
        // from presenters.
        let launcher = self
            .focused_target()
            .and_then(|target| self.aggregates.hierarchy.launcher_of_target(&target))
            .map(|id| {
                self.aggregates
                    .configuration
                    .launcher(id)
                    .expect("Focused launcher has no presenter")
                    .name
                    .as_str()
            });
        let project = self
            .focused_target()
            .map(|target| self.aggregates.hierarchy.project_of_target(&target))
            .map(|id| {
                self.aggregates
                    .configuration
                    .project(id)
                    .expect("Focused project has no presenter")
                    .name
                    .as_str()
            });

        let mut title = if terminal_title.is_empty() {
            self.env.primary_application.clone()
        } else {
            terminal_title
        };
        for name in launcher.into_iter().chain(project) {
            title.push_str(TITLE_SEPARATOR);
            title.push_str(name);
        }
        title
    }
}

/// The pixel box macOS places its text input UI next to, from a target's projected corners: a
/// rotated or perspective target doesn't project to an axis-aligned rect, so its bounding box is
/// the area. Not built with `Rect::joined`, which skips empty rects and so ignores every point.
fn surface_bounds(points: Vec<Point>) -> BoxPx {
    assert!(!points.is_empty());
    let (min, max) = points.iter().fold(
        (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        ),
        |((min_x, min_y), (max_x, max_y)), p| {
            (
                (min_x.min(p.x), min_y.min(p.y)),
                (max_x.max(p.x), max_y.max(p.y)),
            )
        },
    );
    BoxPx::new(
        (min.0.floor() as i32, min.1.floor() as i32).into(),
        (max.0.ceil() as i32, max.1.ceil() as i32).into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_bounds_cover_every_point() {
        let corners = [(34.5, 148.5), (48.5, 148.5), (48.5, 177.5), (34.5, 177.5)]
            .map(|(x, y)| Point::new(x, y))
            .to_vec();

        assert_eq!(
            surface_bounds(corners),
            BoxPx::new((34, 148).into(), (49, 178).into())
        );
    }
}
