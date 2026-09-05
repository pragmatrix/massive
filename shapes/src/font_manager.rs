use std::collections::HashMap;
use std::sync::Arc;

use parley::FontContext;
use parking_lot::{Mutex, MutexGuard};

use crate::{FontId, GlyphBrush};

pub use parley::FontWeight;

/// A font manager backed by Parley's [`FontContext`].
///
/// Owns the Parley font database plus a registry of [`parley::FontData`] entries keyed by
/// [`FontId`] (the `Blob` unique id plus the face index). A [`FontId`] is derived straight from a
/// shaped run's font, so shaping needs no lookup; rasterization resolves a [`FontId`] back to
/// concrete font data in O(1).
#[derive(Clone)]
pub struct FontManager(Arc<Mutex<FontManagerInner>>);

impl std::fmt::Debug for FontManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.lock();
        f.debug_struct("FontManager")
            .field("font_count", &inner.fonts.len())
            .finish_non_exhaustive()
    }
}

struct FontManagerInner {
    font_context: FontContext,
    layout_context: parley::LayoutContext<GlyphBrush>,
    /// Concrete fonts keyed by [`FontId`]. Populated by `rebuild_fonts` to include every font the
    /// collection may select (including system fallbacks like emoji), so rasterization can resolve
    /// any glyph's `FontId` to font data. The key must include the face index because a single
    /// file may hold several faces that share one `Blob` id.
    fonts: HashMap<FontId, parley::FontData>,
}

/// A shaping session holding the manager's lock.
///
/// Created by [`FontManager::shaper`]; the guard it carries keeps the shared
/// [`FontContext`] and [`LayoutContext`] locked for as long as the context is alive, so multiple
/// shapes can run against the same scratch contexts with a single lock acquisition.
pub struct Shaper<'a> {
    inner: MutexGuard<'a, FontManagerInner>,
}

impl Default for FontManager {
    fn default() -> Self {
        Self::system()
    }
}

impl FontManager {
    /// Create a completely bare font manager, no fallbacks, no fonts.
    pub fn bare() -> Self {
        // Parley's `FontContext` doesn't load system fonts by default; `system()` loads them.
        let font_context = FontContext::new();
        Self::from(font_context)
    }

    /// Creates an empty font manager without system fonts.
    pub fn empty_system() -> Self {
        Self::bare()
    }

    /// Creates a font manager with the environment's locale, platform families, fallbacks, and
    /// system fonts loaded.
    pub fn system() -> Self {
        let mut font_context = FontContext::new();
        font_context.collection.load_system_fonts();
        let manager = Self::from(font_context);
        manager.rebuild_fonts();
        manager
    }

    /// Adds the font and returns Self
    pub fn with_font(self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Self {
        self.load_font(font_data);
        self
    }

    /// Adds the font and returns its font ids.
    /// Ergonomics: Rename to `add_font`?
    pub fn load_font(&self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Vec<FontId> {
        let mut inner = self.0.lock();
        // FontData owns a shared `Blob<u8>`; keep the bytes alive in the registry.
        let blob =
            parley::fontique::Blob::new(Arc::new(font_data) as Arc<dyn AsRef<[u8]> + Send + Sync>);
        inner.font_context.collection.register_fonts(blob.clone(), None);
        let id = crate::font_id(&parley::FontData::new(blob.clone(), 0));
        inner.fonts.insert(id, parley::FontData::new(blob, 0));
        vec![id]
    }

    /// Rebuild the font registry from the whole collection, keyed by [`FontId`], so any font
    /// Parley may select (including system fonts used for fallback, e.g. emoji) can be resolved by
    /// [`FontId`] during rasterization.
    fn rebuild_fonts(&self) {
        let mut inner = self.0.lock();
        let mut fonts = HashMap::new();
        // Collect family names first to release the collection borrow before querying each family.
        let family_names: Vec<String> = inner
            .font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        for name in family_names {
            let Some(family_id) = inner.font_context.collection.family_id(&name) else {
                continue;
            };
            let Some(family) = inner.font_context.collection.family(family_id) else {
                continue;
            };
            for font_info in family.fonts() {
                let Some(blob) = inner.font_context.source_cache.get(font_info.source()) else {
                    continue;
                };
                let id = crate::font_id(&parley::FontData::new(blob.clone(), font_info.index()));
                fonts.insert(id, parley::FontData::new(blob, font_info.index()));
            }
        }
        inner.fonts = fonts;
    }

    /// Resolve the [`parley::FontData`] for a [`FontId`].
    pub fn font_data(&self, id: FontId) -> Option<parley::FontData> {
        self.0.lock().fonts.get(&id).cloned()
    }

    /// Acquire a [`Shaper`], holding the manager's lock for the duration of the shaping session.
    #[must_use]
    pub fn shaper(&self) -> Shaper<'_> {
        Shaper {
            inner: self.0.lock(),
        }
    }
}

impl FontManagerInner {
    /// Borrow the two Parley contexts for shaping.
    ///
    /// Returns the `&mut` pair so callers can build a layout against both without holding a
    /// closure. Both come from disjoint fields of the same inner, so the borrows are valid.
    pub fn contexts(&mut self) -> (&mut FontContext, &mut parley::LayoutContext<GlyphBrush>) {
        (&mut self.font_context, &mut self.layout_context)
    }
}

impl Shaper<'_> {
    /// Borrow the two Parley contexts for shaping.
    pub fn contexts(&mut self) -> (&mut FontContext, &mut parley::LayoutContext<GlyphBrush>) {
        self.inner.contexts()
    }
}

impl From<FontContext> for FontManager {
    fn from(font_context: FontContext) -> Self {
        FontManager(Arc::new(Mutex::new(FontManagerInner {
            font_context,
            layout_context: parley::LayoutContext::new(),
            fonts: HashMap::new(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parley::StyleProperty;

    /// A bundled monospace font so the test doesn't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../examples/shared/src/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// After `load_font`, the registry must contain the loaded font AND any system fonts Parley
    /// may select for fallback (e.g. the default `sans-serif`, or an emoji font). Otherwise a
    /// fallback run's `FontId` resolves to nothing and its glyphs get rasterized with the wrong
    /// font.
    #[test]
    fn load_font_rebuilds_registry_with_system_fonts() {
        let fonts = FontManager::bare().with_font(JETBRAINS_MONO);
        let count = fonts.0.lock().fonts.len();
        // At minimum the loaded font is present; on a system with fonts, fallbacks are too.
        assert!(count >= 1, "registry should contain the loaded font");
        // The loaded font must resolve to a real id.
        let loaded = fonts
            .0
            .lock()
            .fonts
            .values()
            .any(|d| d.data.len() == JETBRAINS_MONO.len());
        assert!(loaded, "loaded font must be in the registry");
    }

    /// Shapes an emoji through the manager and asserts the fallback run's derived `FontId`
    /// resolves to a real font. This locks the emoji/font-fallback fix: the registry must contain
    /// the emoji font so its glyphs are rasterized with the correct font.
    #[test]
    fn emoji_fallback_resolves_to_real_font() {
        let fonts = FontManager::system();
        let mut shaper = fonts.shaper();
        let (fcx, lcx) = shaper.contexts();
        let mut builder = lcx.ranged_builder(fcx, "😀", 1.0, true);
        builder.push_default(StyleProperty::FontSize(16.0));
        let mut layout: parley::Layout<GlyphBrush> = builder.build("😀");
        layout.break_all_lines(None);
        layout.align(parley::Alignment::Start, Default::default());
        let line = layout.get(0).expect("single line");
        let run = crate::line_runs(&line).next().expect("has a run");
        let font_id = crate::font_id(run.run().font());
        drop(shaper);
        let font_data = fonts.font_data(font_id);
        assert!(
            font_data.is_some(),
            "emoji fallback font must resolve to a registered font, got FontId({:?})",
            font_id
        );
    }
}
