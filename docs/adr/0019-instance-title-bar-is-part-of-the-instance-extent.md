# The instance title bar is part of the instance extent and stays regular-sized in Full Screen Mode

## Status: accepted

The desktop draws an **instance title bar** above every instance that has a primary view. It shows the launcher's name and the view title the application already reports through `View::set_title`, so no new application-facing channel exists and no description is persisted.

- **Extent, not overhang.** The bar is part of the instance's extent: layout, spacing, hit testing and the `Focus` camera frame the bar and the panel together. An overhang drawn outside the layout rectangle was rejected because it would overlap neighbours and make `Focus` framing disagree with what is visible.
- **Launchers share the extent.** A visor launcher and an empty launcher measure with the instance extent, so a launcher lines up with the instances it starts. An empty launcher additionally stretches to its matrix row's height; once it holds instances it is hidden and takes their intrinsic size at the top of the row, where the first instance appears. **Amended by [ADR 0023](./0023-matrix-slots-share-one-slot-extent-and-center-their-content.md):** the matrix centers that intrinsic size in its slot instead of placing it at the top of the row.
- **Outside the content grid.** The view keeps its own size, so the terminal's rows, columns and PTY size are unaffected by the bar.
- **Regular-sized in Full Screen Mode.** Full Screen Mode scales only the content (ADR 0014); the bar keeps its regular presented height, so the working directory stays readable in fullscreen. This is why `Focus` frames the bar too.
- **Selects its instance.** A click on the bar selects its instance, as a click on a project's header or empty matrix selects the project. Such parts never hold the keyboard focus and never handle keys: their parent is on the same focus path and takes both, so focus-driven behaviour such as navigation and zoom only ever sees whole targets.
- **No opt-in.** Presence follows the primary view. A per-view flag or a desktop-wide toggle was rejected as API surface nobody needed yet.
- **One title.** The same title feeds the bar and the native window title, instead of a second channel.
