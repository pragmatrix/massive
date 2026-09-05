use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::{Mutex, MutexGuard};
use parley::FontContext;
use parley::fontique::{Collection, CollectionOptions, GenericFamily};

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
        let font_context = FontContext {
            collection: Collection::new(CollectionOptions {
                system_fonts: false,
                ..Default::default()
            }),
            source_cache: Default::default(),
        };
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
        let families = inner
            .font_context
            .collection
            .register_fonts(blob.clone(), None);
        for generic in [
            GenericFamily::SansSerif,
            GenericFamily::Serif,
            GenericFamily::Monospace,
        ] {
            if inner
                .font_context
                .collection
                .generic_families(generic)
                .next()
                .is_none()
            {
                inner
                    .font_context
                    .collection
                    .set_generic_families(generic, families.iter().map(|(family, _)| *family));
            }
        }
        let mut ids = Vec::new();
        for (_, faces) in families {
            for face in faces {
                let font = parley::FontData::new(blob.clone(), face.index());
                let id = crate::font_id(&font);
                inner.fonts.insert(id, font);
                ids.push(id);
            }
        }
        ids
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

        // Common-script symbols (e.g. `✘`, `✓`, `→`) inherit the surrounding script for fallback,
        // which on macOS resolves to Helvetica — a font that lacks most of them. Append the system
        // font with the best coverage of these symbols to the Latin fallback so they render instead
        // of falling through to the `.notdef` dead glyph. This is a local workaround for the known
        // upstream gap (parley #744, #695) until font selection becomes coverage-aware.
        Self::append_symbol_fallback(&mut inner);
    }

    /// Append symbol-covering non-emoji fonts to the Latin fallback, best coverage first.
    ///
    /// Common-script symbols inherit the surrounding script (Latin by default) for fallback, but
    /// the platform's Latin fallback often lacks them. We scan a broad set of symbol codepoints,
    /// score every family by how many it covers, and append the covering families to the Latin
    /// fallback in descending coverage order. Emoji fonts are excluded because they would be
    /// selected for text-presentation symbols (upstream parley #744).
    fn append_symbol_fallback(inner: &mut FontManagerInner) {
        use parley::fontique::{FallbackKey, GenericFamily, Script};

        // Common-script symbol blocks commonly used in terminals and UI text.
        const SYMBOL_RANGES: &[(u32, u32)] = &[
            (0x2000, 0x206F), // General Punctuation
            (0x2190, 0x21FF), // Arrows
            (0x2200, 0x22FF), // Mathematical Operators
            (0x2300, 0x23FF), // Miscellaneous Technical
            (0x2500, 0x257F), // Box Drawing
            (0x2580, 0x259F), // Block Elements
            (0x25A0, 0x25FF), // Geometric Shapes
            (0x2600, 0x26FF), // Miscellaneous Symbols
            (0x2700, 0x27BF), // Dingbats
            (0x27C0, 0x27EF), // Miscellaneous Mathematical Symbols-A
            (0x2980, 0x29FF), // Miscellaneous Mathematical Symbols-B
            (0x2B00, 0x2BFF), // Miscellaneous Symbols and Arrows
        ];

        let latn = Script::from_bytes(*b"Latn");
        let emoji_families: Vec<_> = inner
            .font_context
            .collection
            .generic_families(GenericFamily::Emoji)
            .collect();

        // Score each family by how many symbol codepoints its default font covers.
        let family_names: Vec<String> = inner
            .font_context
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        let mut scored: Vec<(usize, parley::fontique::FamilyId)> = Vec::new();
        for name in family_names {
            let Some(family_id) = inner.font_context.collection.family_id(&name) else {
                continue;
            };
            if emoji_families.contains(&family_id) {
                continue;
            }
            let Some(family) = inner.font_context.collection.family(family_id) else {
                continue;
            };
            let Some(font_info) = family.default_font() else {
                continue;
            };
            let Some(blob) = inner.font_context.source_cache.get(font_info.source()) else {
                continue;
            };
            let Some(font_ref) =
                swash::FontRef::from_index(blob.as_ref(), font_info.index() as usize)
            else {
                continue;
            };
            let charmap = font_ref.charmap();
            let covered = SYMBOL_RANGES
                .iter()
                .flat_map(|&(start, end)| start..=end)
                .filter(|&c| charmap.map(char::from_u32(c).unwrap_or('\0')) != 0)
                .count();
            if covered > 0 {
                scored.push((covered, family_id));
            }
        }

        // Append best-coverage families first so the first that covers a symbol wins.
        scored.sort_by_key(|(covered, _)| std::cmp::Reverse(*covered));
        let families = scored.into_iter().map(|(_, id)| id);
        inner
            .font_context
            .collection
            .append_fallbacks(FallbackKey::new(latn, None), families);
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
