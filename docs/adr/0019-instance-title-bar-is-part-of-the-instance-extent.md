# The instance title bar is part of the instance extent and stays regular-sized in Full Screen Mode

## Status: accepted

The desktop draws an **instance title bar** above every instance that has a primary view. It shows the launcher's name and the view title the application already reports through `View::set_title`, so no new application-facing channel exists and no description is persisted.

- **Extent, not overhang.** The bar is part of the instance's extent: layout, spacing, hit testing and the `Focus` camera frame the bar and the panel together. An overhang drawn outside the layout rectangle was rejected because it would overlap neighbours and make `Focus` framing disagree with what is visible.
- **Outside the content grid.** The view keeps its own size, so the terminal's rows, columns and PTY size are unaffected by the bar.
- **Regular-sized in Full Screen Mode.** Full Screen Mode scales only the content (ADR 0014); the bar keeps its regular presented height, so the working directory stays readable in fullscreen. This is why `Focus` frames the bar too.
- **No opt-in.** Presence follows the primary view. A per-view flag or a desktop-wide toggle was rejected as API surface nobody needed yet.
- **One title.** The same title feeds the bar and the native window title, instead of a second channel.
