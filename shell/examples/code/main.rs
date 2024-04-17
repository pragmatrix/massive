use std::{
    env,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use base_db::SourceDatabaseExt;
use cosmic_text::{
    fontdb::{self},
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping,
};
use ide::{AnalysisHost, HighlightConfig, HlMod, HlMods, HlTag, SymbolKind};
use load_cargo::{LoadCargoConfig, ProcMacroServerChoice};
use massive_geometry::{Camera, Color, Point, SizeI};
use massive_shapes::{GlyphRun, PositionedGlyph};
use massive_shell::Shell;
use project_model::CargoConfig;
use vfs::VfsPath;
use winit::event_loop::EventLoop;

#[path = "../shared/application.rs"]
mod application;
#[path = "../shared/positioning.rs"]
mod positioning;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let progress = |p: String| {
        let mut handle = io::stdout().lock();
        let _ = writeln!(handle, "{}", p.as_str());
    };

    // let root_path = env::current_dir().unwrap().join(Path::new("Cargo.toml"));
    let root_path = env::current_dir()
        .unwrap()
        .join(Path::new("/Users/armin/dev/massive/Cargo.toml"));

    println!("Root path: {}", root_path.display());

    let cargo_config = CargoConfig {
        // need to be able to look up examples.
        all_targets: true,
        ..CargoConfig::default()
    };

    let load_config = LoadCargoConfig {
        load_out_dirs_from_check: false,
        with_proc_macro_server: ProcMacroServerChoice::None,
        prefill_caches: false,
    };

    let example_dir = root_path
        .parent()
        .unwrap()
        .join(Path::new("shell/examples/code"));

    let file_to_show = example_dir.join("main.rs");

    println!("Looking for {}", file_to_show.display());

    let (db, vfs, _proc_macro_server) =
        load_cargo::load_workspace_at(&root_path, &cargo_config, &load_config, &progress)?;

    println!("db: {db:?}");
    println!("vfs: {vfs:?}");

    let path = VfsPath::new_real_path(file_to_show.to_string_lossy().into_owned());

    // for (id, path) in vfs.iter() {
    //     println!("file: {}", path)
    // }

    let file_id = vfs.file_id(&path).expect("File not found");
    let file_text = db.file_text(file_id);

    let analysis_host = AnalysisHost::with_database(db);

    println!("analysis");
    let analysis = analysis_host.analysis();

    println!("highlight");

    let highlight_config = HighlightConfig {
        strings: true,
        punctuation: true,
        specialize_punctuation: true,
        operator: true,
        specialize_operator: true,
        inject_doc_comment: true,
        macro_bang: true,
        syntactic_name_ref_highlighting: false,
    };

    // let _syntax = analysis.highlight(highlight_config, file_id);

    // font_system

    let mut font_system = {
        let mut db = fontdb::Database::new();
        // let font_dir = example_dir.join("JetBrainsMono-2.304/fonts/ttf");
        // db.load_fonts_dir(font_dir);
        let font_file =
            example_dir.join("JetBrainsMono-2.304/fonts/variable/JetBrainsMono[wght].ttf");
        db.load_font_file(font_file)?;
        // Use an invariant locale.
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    };

    // layout

    // let font_size = 32.;
    // let line_height = 40.;
    let font_size = 16.;
    let line_height = 20.;

    let (glyph_runs, height) = shape_text(&mut font_system, &file_text, font_size, line_height);

    // Window

    let event_loop = EventLoop::new()?;
    let window = application::create_window(&event_loop, None)?;
    let initial_size = window.inner_size();

    // Camera

    let camera = {
        let fovy: f64 = 45.0;
        let camera_distance = 1.0 / (fovy / 2.0).to_radians().tan();
        Camera::new((0.0, 0.0, camera_distance), (0.0, 0.0, 0.0))
    };

    // Application

    let application =
        application::Application::new(camera, glyph_runs, SizeI::new(1280, height as u64));

    // Shell

    let font_system = Arc::new(Mutex::new(font_system));
    let mut shell = Shell::new(&window, initial_size, font_system.clone()).await?;
    shell.run(event_loop, &window, application).await
}

fn shape_text(
    font_system: &mut FontSystem,
    text: &str,
    font_size: f32,
    line_height: f32,
) -> (Vec<(Point, GlyphRun)>, f64) {
    let attrs = Attrs::new().family(Family::Monospace);
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, line_height));
    buffer.set_size(font_system, f32::INFINITY, f32::INFINITY);
    buffer.set_text(font_system, text, attrs, Shaping::Advanced);
    buffer.shape_until_scroll(font_system, true);

    let mut runs = Vec::new();
    let mut height: f64 = 0.;

    for run in buffer.layout_runs() {
        let offset = Point::new(0., run.line_top as f64);
        runs.push((offset, positioning::to_glyph_run(&run, line_height)));
        height = height.max(offset.y + line_height as f64);
    }

    (runs, height)
}

fn colorize(tag: HlTag, mods: HlMods) -> Color {
    const XX: u32 = 100;

    if mods.contains(HlMod::Unsafe) {
        return unsafe_red();
    }
    match tag {
        HlTag::Symbol(symbol) => match symbol {
            SymbolKind::Attribute => black(),
            SymbolKind::BuiltinAttr => black(),
            SymbolKind::Const => const_blue(),
            SymbolKind::ConstParam => type_green(),
            SymbolKind::Derive => keyword_blue(),
            SymbolKind::DeriveHelper => keyword_blue(),
            SymbolKind::Enum => type_green(),
            SymbolKind::Field => field_black(),
            SymbolKind::Function => function_brown(),
            SymbolKind::Method => function_brown(),
            SymbolKind::Impl => black(),
            SymbolKind::Label => keyword_blue(),
            SymbolKind::LifetimeParam => keyword_blue(),
            SymbolKind::Local => black(),
            SymbolKind::Macro => keyword_blue(),
            SymbolKind::ProcMacro => keyword_blue(),
            SymbolKind::Module => type_green(),
            SymbolKind::SelfParam => keyword_blue(),
            SymbolKind::SelfType => keyword_blue(),
            SymbolKind::Static => black(),
            SymbolKind::Struct => type_green(),
            SymbolKind::ToolModule => black(),
            SymbolKind::Trait => type_green(),
            SymbolKind::TraitAlias => type_green(),
            SymbolKind::TypeAlias => type_green(),
            SymbolKind::TypeParam => type_light_green(),
            SymbolKind::Union => type_green(),
            SymbolKind::ValueParam => keyword_blue(),
            SymbolKind::Variant => const_blue(),
        },
        HlTag::AttributeBracket => keyword_blue(),
        HlTag::BoolLiteral => keyword_blue(),
        HlTag::BuiltinType => type_light_green(),
        HlTag::ByteLiteral => literal_green(),
        HlTag::CharLiteral => literal_red(),
        HlTag::Comment => comment_green(),
        HlTag::EscapeSequence => keyword_blue(),
        HlTag::FormatSpecifier => field_black(),
        HlTag::InvalidEscapeSequence => error_red(),
        HlTag::Keyword => keyword_blue(),
        HlTag::NumericLiteral => literal_green(),
        HlTag::Operator(_) => black(),
        HlTag::Punctuation(_) => black(),
        HlTag::StringLiteral => literal_red(),
        HlTag::UnresolvedReference => error_red(),
        HlTag::None => none(),
    }
}

fn none() -> Color {
    rgb(0xfffffff)
}

fn black() -> Color {
    rgb(0)
}

fn keyword_blue() -> Color {
    rgb(0x0000ff)
}

fn const_blue() -> Color {
    rgb(0x0070c1)
}

fn error_red() -> Color {
    rgb(0xff0000)
}

fn unsafe_red() -> Color {
    rgb(0xdd0000)
}

fn literal_green() -> Color {
    rgb(0x098658)
}

fn literal_red() -> Color {
    rgb(0xA31515)
}

fn type_green() -> Color {
    rgb(0x267f99)
}

fn type_light_green() -> Color {
    rgb(0x2B91AF)
}

fn field_black() -> Color {
    rgb(0x001080)
}

fn function_brown() -> Color {
    rgb(0x795e26)
}

fn comment_green() -> Color {
    rgb(0x008000)
}

fn rgb(rgb: u32) -> Color {
    Color::from_rgb_u32(rgb)
}

// Proposed VSCode Style:

// <style>
// body                { margin: 0; }
// pre                 { color: #000000; background: #FFFFFF; font-size: 22px; padding: 0.4em; }

// .lifetime           { color: #0000FF; }
// .label              { color: #0000FF; }
// .comment            { color: #008000; }
// .documentation      { color: #008000; }
// .intra_doc_link     { color: #0000FF; }
// .injected           { color: #0000FF; }
// .struct, .enum      { color: #267f99; }
// .enum_variant       { color: #0000FF; }
// .string_literal     { color: #A31515; }
// .field              { color: #001080; }
// .function           { color: #795e26; }
// .function.unsafe    { color: #d00; }
// .trait              { color: #267f99; }
// .trait.unsafe       { color: #d00; }
// .type_alias         { color: #267f99; }
// .operator.unsafe    { color: #d00; }
// .mutable.unsafe     { color: #d00; }
// .keyword.unsafe     { color: #d00; }
// .macro.unsafe       { color: #d00; }
// .parameter          { color: #0000FF; }
// .text               { color: #000000; }
// .type               { color: #2B91AF; }
// .builtin_type       { color: #2B91AF; }
// .type_param         { color: #2B91AF; }
// .attribute          { color: #000000; }
// .numeric_literal    { color: #098658; }
// .bool_literal       { color: #0000ff; }
// .macro              { color: #0000FF; }
// .proc_macro         { color: #0000FF; }
// .derive             { color: #0000FF; }
// .module             { color: #267f99; }
// .value_param        { color: #0000FF; }
// .variable           { color: #001080; }
// .format_specifier   { color: #001080; }
// .mutable            { color: #001080; font-weight: bold; }
// .escape_sequence    { color: #0000FF; }
// .keyword            { color: #0000FF; }
// .control            { color: #0000FF; }
// .reference          { color: #795e26; }
// .const              { color: #0000FF; }

// .invalid_escape_sequence { color: #FF0000; text-decoration: wavy underline; }
// .unresolved_reference    { color: #FF0000; text-decoration: wavy underline; }
// </style>
