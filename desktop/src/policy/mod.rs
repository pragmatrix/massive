//! The desktop policy holds the choices that shape how the desktop behaves and feels.
//!
//! The `when` rules decide the desktop's exceptional behaviors (ADR 0029). The `constants` and
//! `colors` modules hold the values that were picked by taste and usability experience rather than
//! derived from a structural reason.

pub mod colors;
pub mod constants;
pub mod when;
