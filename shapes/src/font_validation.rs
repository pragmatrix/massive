//! Swash readability checks for font files admitted to shaping engines.

use std::error::Error;
use std::fmt;

use swash::{FontDataRef, FontRef};

/// Why a font file cannot be admitted to a shaping engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FontValidationError {
    /// Swash cannot recognize the bytes as a font or collection.
    InvalidFile,
    /// Swash recognizes no faces in the file.
    NoFaces,
    /// Swash cannot parse one face at the reported collection index.
    InvalidFace { index: usize },
}

impl fmt::Display for FontValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFile => f.write_str("data is not a readable font file"),
            Self::NoFaces => f.write_str("font file contains no faces"),
            Self::InvalidFace { index } => {
                write!(f, "font face at index {index} is not readable by Swash")
            }
        }
    }
}

impl Error for FontValidationError {}

/// Validate every face before registering a font file as a unit: a face the registry cannot
/// fully serve must not become selectable.
pub(crate) fn validate_font_file(data: &[u8]) -> Result<(), FontValidationError> {
    let faces = FontDataRef::new(data).ok_or(FontValidationError::InvalidFile)?;
    if faces.is_empty() {
        return Err(FontValidationError::NoFaces);
    }

    for index in 0..faces.len() {
        FontRef::from_index(data, index).ok_or(FontValidationError::InvalidFace { index })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FontValidationError, validate_font_file};

    /// A bundled monospace font so the tests don't depend on system fonts.
    const JETBRAINS_MONO: &[u8] = include_bytes!(
        "../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    #[test]
    fn validates_every_face_before_registration() {
        validate_font_file(JETBRAINS_MONO).expect("bundled font is valid");
    }

    #[test]
    fn rejects_unreadable_font_bytes() {
        assert!(matches!(
            validate_font_file(b"not a font"),
            Err(FontValidationError::InvalidFile)
        ));
    }
}
