pub mod application;
pub mod code_viewer;
pub mod positioning;

pub mod fonts {
    pub fn jetbrains_mono() -> &'static [u8] {
        let bytes = include_bytes!("JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf");
        bytes
    }
}
