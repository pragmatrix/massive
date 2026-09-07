//! Font assets bundled into binaries and tests via `include_bytes!`.
//!
//! Files live in the top-level `assets/fonts/` directory; statics here are the single
//! access point for code that needs the bytes.

pub static JETBRAINS_MONO: &[u8] = include_bytes!(
    "../../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
);

pub static MONTSERRAT_REGULAR: &[u8] =
    include_bytes!("../../../../assets/fonts/Montserrat/Montserrat-Regular.ttf");
