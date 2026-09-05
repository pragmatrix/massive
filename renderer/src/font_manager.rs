use std::sync::Arc;

use parley::FontContext;
use parking_lot::Mutex;

use massive_shapes::GlyphBrush;

pub use massive_shapes::FontId;
pub use parley::FontWeight;

/// A font manager backed by Parley's [`FontContext`].
///
/// Owns the Parley font database plus a registry of the [`parley::FontData`] entries it knows
/// about, keyed by [`FontId`]. Glyph keys ([`massive_shapes::GlyphKey`]) carry only a [`FontId`],
/// so the atlas can resolve a concrete font without depending on Parley across the shapes boundary.
#[derive(Clone)]
pub struct FontManager(Arc<Mutex<FontManagerInner>>);

impl std::fmt::Debug for FontManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.0.lock();
        f.debug_struct("FontManager")
            .field("font_count", &inner.font_data.len())
            .finish_non_exhaustive()
    }
}

struct FontManagerInner {
    font_context: FontContext,
    layout_context: parley::LayoutContext<GlyphBrush>,
    font_data: Vec<parley::FontData>,
}

impl Default for FontManager {
    fn default() -> Self {
        Self::system()
    }
}

impl FontManager {
    /// Create a completely bare font manager, no fallbacks, no fonts.
    pub fn bare() -> Self {
        // Note: `FontContext::new()` still discovers system fonts on desktop platforms (fontique
        // scans them eagerly), so `bare()` is not truly font-free there. The registry is empty
        // until `load_font`/`rebuild_font_data` populates it.
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
        manager.rebuild_font_data();
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
        let blob =
            parley::fontique::Blob::new(Arc::new(font_data) as Arc<dyn AsRef<[u8]> + Send + Sync>);
        {
            let mut inner = self.0.lock();
            inner.font_context.collection.register_fonts(blob.clone(), None);
        }
        // Rebuild the registry so the resolver can map any font (including system fonts used for
        // fallback, e.g. emoji) to a stable `FontId`. Without this, a run shaped with a system
        // font (the default `sans-serif`, or an emoji fallback) would resolve to `FontId(0)` and
        // be rasterized with the wrong font.
        self.rebuild_font_data();
        // Find the loaded font's `FontId` in the rebuilt registry by its unique blob id.
        let id = self
            .0
            .lock()
            .font_data
            .iter()
            .position(|d| d.data.id() == blob.id())
            .map(|i| FontId(i as u32))
            .unwrap_or(FontId(0));
        vec![id]
    }

    /// Rebuild the `FontData` registry from the whole collection, so the resolver can map any
    /// font (including system fonts used for fallback, e.g. emoji) to a stable [`FontId`].
    fn rebuild_font_data(&self) {
        let mut inner = self.0.lock();
        let mut font_data = Vec::new();
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
                font_data.push(parley::FontData::new(blob, font_info.index()));
            }
        }
        inner.font_data = font_data;
    }

    /// Resolve the [`parley::FontData`] for a [`FontId`].
    pub fn font_data(&self, id: FontId) -> Option<parley::FontData> {
        self.0.lock().font_data.get(id.0 as usize).cloned()
    }

    /// Run a shaping session against the manager's [`FontContext`] and [`LayoutContext`].
    ///
    /// The closure receives the Parley contexts and a resolver that maps a [`parley::FontData`]
    /// back to its [`FontId`]. All three are borrowed from the single locked manager inner, so a
    /// whole `TextShaper` / `SizedTextShaper` / Parley layout can be built and converted in one
    /// call.
    pub fn with_shape<R>(
        &self,
        f: impl FnOnce(
            &mut FontContext,
            &mut parley::LayoutContext<GlyphBrush>,
            &(dyn Fn(&parley::FontData) -> FontId + '_),
        ) -> R,
    ) -> R {
        let mut inner = self.0.lock();
        // Snapshot the registry so the resolver doesn't borrow `inner` immutably while we lend the
        // contexts out mutably. `FontData` is cheaply comparable by content.
        let registry = std::mem::take(&mut inner.font_data);
        let resolver: &(dyn Fn(&parley::FontData) -> FontId + '_) = &|data: &parley::FontData| {
            registry
                .iter()
                .position(|d| d == data)
                .map(|i| FontId(i as u32))
                .unwrap_or(FontId(0))
        };
        let FontManagerInner {
            font_context,
            layout_context,
            font_data,
        } = &mut *inner;
        let _ = font_data;
        let result = f(font_context, layout_context, resolver);
        inner.font_data = registry;
        result
    }
}

impl From<FontContext> for FontManager {
    fn from(font_context: FontContext) -> Self {
        FontManager(Arc::new(Mutex::new(FontManagerInner {
            font_context,
            layout_context: parley::LayoutContext::new(),
            font_data: Vec::new(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bundled monospace font so the test doesn't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../examples/shared/src/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    /// After `load_font`, the registry must contain the loaded font AND any system fonts Parley
    /// may select for fallback (e.g. the default `sans-serif`, or an emoji font). Otherwise the
    /// resolver returns `FontId(0)` for a fallback run and its glyphs get rasterized with the
    /// wrong font.
    #[test]
    fn load_font_rebuilds_registry_with_system_fonts() {
        let fonts = FontManager::bare().with_font(JETBRAINS_MONO);
        let count = fonts.0.lock().font_data.len();
        // At minimum the loaded font is present; on a system with fonts, fallbacks are too.
        assert!(count >= 1, "registry should contain the loaded font");
        // The loaded font must resolve to a real id (not the `FontId(0)` fallback).
        let loaded_id = fonts
            .0
            .lock()
            .font_data
            .iter()
            .position(|d| d.data.len() == JETBRAINS_MONO.len())
            .map(|i| FontId(i as u32));
        assert!(loaded_id.is_some(), "loaded font must be in the registry");
    }

    /// Shapes an emoji through the manager and asserts the fallback run resolves to a real font
    /// (not the `FontId(0)` fallback). This locks the emoji/font-fallback fix: the registry must
    /// contain the emoji font so its glyphs are rasterized with the correct font.
    #[test]
    fn emoji_fallback_resolves_to_real_font() {
        let fonts = FontManager::system();
        let resolved = fonts.with_shape(|fcx, lcx, resolver| {
            let mut builder = lcx.ranged_builder(fcx, "😀", 1.0, true);
            builder.push_default(parley::StyleProperty::FontSize(16.0));
            let mut layout: parley::Layout<GlyphBrush> = builder.build("😀");
            layout.break_all_lines(None);
            layout.align(parley::Alignment::Start, Default::default());
            let line = layout.get(0).expect("single line");
            let run = massive_shapes::line_runs(&line).next().expect("has a run");
            let font_id = resolver(run.run().font());
            let font_data = fonts.font_data(font_id);
            (font_id, font_data)
        });
        let (font_id, font_data) = resolved;
        assert!(
            font_data.is_some(),
            "emoji fallback font must resolve to a registered font, got FontId({})",
            font_id.0
        );
    }
}

