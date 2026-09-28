//! Regression coverage for the cosmic engine's lazy face resolution.
//!
//! `CosmicTextEngine::resolve_face_index` returns `None` when a shaper-selected face's data
//! cannot be read out of the font database; `shape()` propagates that with `?` and silently
//! drops the whole line. Lock the invariant that the seam can actually deliver data, so a
//! future fontdb/cosmic-text upgrade weakening either side fails here instead of in the field.

use fontdb::Source;

use super::*;

/// A bundled font (SIL OFL 1.1) used as the data behind a synthetic face entry.
const JETBRAINS_MONO: &[u8] = include_bytes!(
    "../../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
);

/// A rejected resolution registers the file in the engine's own registry but issues no `FaceId`.
///
/// `resolve_face_index` registers before it looks the selected face index up, so the engine's
/// `faces` grows even when it then reports failure — and it reports that failure as `None`, which
/// is the *only* thing that can reach the manager's published snapshot (the store sits behind the
/// `?`). Callers must therefore key on the returned id, never on the registry's contents.
#[test]
fn a_rejected_resolution_registers_without_issuing_a_face_id() {
    let mut engine = CosmicTextEngine::new(false).expect("bare engine is valid");
    let bytes: FontBytes = std::sync::Arc::new(JETBRAINS_MONO.to_vec());
    assert_eq!(
        engine.font_registry().face_count(),
        0,
        "nothing published yet"
    );

    // A data index the file's faces do not have: registration succeeds, the selected-index
    // lookup then finds nothing.
    let rejected = engine.resolve_face(FontData::new(bytes, 99));
    assert!(rejected.is_none(), "an absent face index must fail");
    assert_eq!(
        engine.font_registry().face_count(),
        1,
        "the file is registered into the engine's own registry either way"
    );
}

/// Shaping must be able to resolve faces the shaper selected lazily, including file-backed
/// faces that fontdb stores as `Source::SharedFile` (the state `FontSystem::get_font` leaves
/// the database in after its `make_shared_face_data` upgrade on first use).
///
/// The reviewed bug was the `self.resolve_face_index(...)?` in the glyph loop aborting the whole
/// `shape()` call when resolution fails; at the pinned dependency versions, cosmic-text
/// ensures every face is `SharedFile`/`Binary` (never an unreadable `Source::File`) before
/// it can shape a glyph through it, so resolution cannot fail on that path. This test seeds
/// the engine's database directly with `Source::SharedFile` — the source kind file-backed
/// system fonts end up as — and asserts the full run shapes with every cluster present and
/// the resolved face resolves font data.
///
/// If that premise ever breaks (upgraded fontdb, changed cosmic-text fallback logic), the
/// glyph loop needs a graceful per-cluster fallback instead of the whole-line `?`; this
/// test then documents the seam that regressed.
#[test]
fn cosmic_engine_shapes_through_shared_file_faces() {
    let jetbrains: &'static [u8] = include_bytes!(
        "../../../../assets/fonts/JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf"
    );

    // Build an engine whose database is seeded exactly like FontSystem's file-backed
    // system fonts after cosmic-text's `make_shared_face_data` upgrade: the face's source
    // is `Source::SharedFile`, not `Binary`, so `load_font` never registered it and
    // shaping it must go through `resolve_face_index`'s lazy path.
    let mut engine = CosmicTextEngine::new(false).expect("bare engine is valid");
    let shared = Source::SharedFile(
        std::path::PathBuf::from("JetBrainsMono[wght].ttf"),
        std::sync::Arc::new(Vec::from(jetbrains)) as std::sync::Arc<dyn AsRef<[u8]> + Send + Sync>,
    );
    let ids = engine.font_system.db_mut().load_font_source(shared);
    assert_eq!(ids.len(), 1, "fixture must load exactly one face");

    // Match the request's weight to the face so the shaper selects it directly.
    let request_weight = {
        let face = engine
            .font_system
            .db()
            .face(ids[0])
            .expect("just-loaded face");
        crate::TextWeight(face.weight.0)
    };

    let text = "jjj";
    let request = ShapingRequest::new(
        text,
        crate::engine::TextAttributes::named_family("JetBrains Mono").with_weight(request_weight),
    );
    let run = engine
        .shape(&request, 16.0)
        .expect("shaping must produce a run, dropping whole lines is the regression");

    assert_eq!(
        run.clusters.len(),
        text.chars().count(),
        "every cluster must be present: a dropped cluster means the glyph loop aborted the run"
    );
    assert_eq!(run.glyphs.len(), run.clusters.len());
    assert!(run.width > 0.0, "the run must be measured as nonempty");

    // The resolved face must be resolvable for rasterization, the whole point of resolution.
    for glyph in &run.glyphs {
        assert!(
            engine.font_data(glyph.face_id).is_some(),
            "resolved face {:?} must resolve font data",
            glyph.face_id
        );
    }
}
