use std::sync::Arc;

use cosmic_text::{
    Fallback, Font, FontSystem,
    fontdb::{self, Source},
};
use derive_more::Deref;
use parking_lot::Mutex;

pub use cosmic_text::Weight as FontWeight;
pub use fontdb::ID as FontId;

#[derive(Debug, Clone, Deref)]
pub struct FontManager(Arc<Mutex<FontSystem>>);

impl From<FontSystem> for FontManager {
    fn from(font_system: FontSystem) -> Self {
        FontManager(Mutex::new(font_system).into())
    }
}

impl Default for FontManager {
    fn default() -> Self {
        Self::system()
    }
}

impl FontManager {
    /// Create a completely bare font manager, no fallbacks, no fonts.
    ///
    /// Detail: Cosmic text sets platform dependent font families here.
    pub fn bare(locale: impl Into<String>) -> Self {
        let db = fontdb::Database::new();
        FontSystem::new_with_locale_and_db_and_fallback(locale.into(), db, NoFallback).into()
    }

    /// Creates an empty font manager (but with platform locale, font families and fallback rules
    /// suitable for the platform / system)
    pub fn empty_system() -> Self {
        FontSystem::new_with_locale_and_db(Self::platform_locale(), fontdb::Database::new()).into()
    }

    /// Creates a font manager with the environment's locale, platform families, fallbacks, and
    /// system fonts loaded.
    pub fn system() -> Self {
        FontSystem::new().into()
    }

    /// Adds the font and returns Self
    pub fn with_font(self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Self {
        self.load_font(font_data);
        self
    }

    /// Adds the font and return its font ids.
    /// Ergonomics: May rename to `add_font`?
    pub fn load_font(&self, font_data: impl AsRef<[u8]> + Sync + Send + 'static) -> Vec<FontId> {
        self.lock()
            .db_mut()
            .load_font_source(Source::Binary(Arc::new(font_data)))
            .to_vec()
    }

    // Feature: Encapsulate font and create platform independent metrics.
    pub fn get_font(&self, id: FontId, weight: FontWeight) -> Option<Arc<Font>> {
        self.lock().get_font(id, weight)
    }

    pub fn load_system_fonts(&self) {
        self.lock().db_mut().load_system_fonts();
    }

    /// Get the platform locale. Falls back to `en-US` if not available.
    pub fn platform_locale() -> String {
        sys_locale::get_locale().unwrap_or_else(|| {
            log::warn!("failed to get system locale, falling back to en-US");
            String::from("en-US")
        })
    }
}

struct NoFallback;
impl Fallback for NoFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _script: unicode_script::Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}
