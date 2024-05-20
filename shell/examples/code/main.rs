use std::{
    collections::HashMap,
    env, fs,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use base_db::{SourceDatabase, SourceDatabaseExt};
use chrono::{DateTime, Local};
use cosmic_text::{fontdb, FontSystem};
// use hir::db::DefDatabase;
use ide::{
    AnalysisHost, FilePosition, HighlightConfig, HighlightRelatedConfig, HlMod, HlMods, HlTag,
    SymbolKind,
};
use load_cargo::{LoadCargoConfig, ProcMacroServerChoice};
use project_model::CargoConfig;
use shared::{application, code_viewer};
use syntax::{AstNode, SyntaxKind, WalkEvent};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use vfs::VfsPath;
use winit::event_loop::EventLoop;

use massive_geometry::{Camera, Color, SizeI};
use massive_shapes::TextWeight;
use massive_shell::Shell;

use crate::code_viewer::TextAttribute;

// Simple file for testing less code.
mod test;

#[tokio::main]
async fn main() -> Result<()> {
    let env_filter = EnvFilter::from_default_env();
    let console_formatter = tracing_subscriber::fmt::Layer::default();
    // let (flame_layer, _flame_guard) = FlameLayer::with_file("./tracing.folded").unwrap();

    let now: DateTime<Local> = Local::now();
    #[allow(unused)]
    let time_code = now.format("%Y%m%d%H%M").to_string();

    // let (chrome_layer, _chrome_guard) = tracing_chrome::ChromeLayerBuilder::new()
    //     .file(format!("./{time_code}-massive-trace.json"))
    //     .build();

    Registry::default()
        // Filter seems to be applied globally, which is what we want.
        .with(env_filter)
        // Console formatter currently captures only log::xxx! macros for some reason.
        .with(console_formatter)
        // .with(flame_layer)
        // .with(chrome_layer)
        .init();

    let progress = |p: String| {
        let mut handle = io::stdout().lock();
        let _ = writeln!(handle, "{}", p.as_str());
    };

    // let root_path = env::current_dir().unwrap().join(Path::new("Cargo.toml"));
    let root_path = env::current_dir()
        .unwrap()
        .join(Path::new("/Users/armin/dev/massive/Cargo.toml"));

    println!("Root path: {}", root_path.display());

    let example_dir = root_path
        .parent()
        .unwrap()
        .join(Path::new("shell/examples/code"));

    // FontSystem

    let mut font_system = {
        let mut db = fontdb::Database::new();
        // let font_dir = example_dir.join("JetBrainsMono-2.304/fonts/ttf");
        // db.load_fonts_dir(font_dir);

        db.load_font_data(shared::fonts::jetbrains_mono().to_vec());
        // Use an invariant locale.
        FontSystem::new_with_locale_and_db("en-US".into(), db)
    };

    println!("Loaded {} font faces.", font_system.db().faces().count());

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

    let file_to_show = example_dir.join("main.rs");
    // let file_to_show = example_dir.join("test.rs");

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

    // Get all names

    let names = {
        let mut names = Vec::new();
        let db = analysis_host.raw_database();
        let tree = db.parse(file_id).tree();
        let syntax = tree.syntax().preorder();
        for event in syntax {
            match event {
                WalkEvent::Enter(node) if node.kind() == SyntaxKind::NAME => {
                    names.push(node.text_range());
                }

                _ => {}
            }
        }
        names
    };

    for name in &names {
        // let x : TextRange
        println!("name: {}", &file_text[*name])
    }

    // Item tree

    // let db = analysis_host.raw_database();
    // let item_tree = db.file_item_tree(file_id.into());
    // println!("Item Tree: {:#?}", item_tree);

    // Analysis

    println!("Analysis");
    let analysis = analysis_host.analysis();

    // A table from a name (definition) to its references.
    let mut relation_table = HashMap::new();

    {
        println!("Highlight Related");
        let config = HighlightRelatedConfig {
            references: true,
            ..HighlightRelatedConfig::default()
        };

        for name in names {
            let position = FilePosition {
                file_id,
                offset: name.start(),
            };

            if let Some(related) = analysis
                .highlight_related(config.clone(), position)
                .unwrap()
            {
                relation_table.insert(name, related.iter().map(|hr| hr.range).collect::<Vec<_>>());
                let related: Vec<_> = related.iter().map(|hr| &file_text[hr.range]).collect();

                println!("related: {:?}", related)
            }
        }
    }

    // File Structure

    // let file_structure = analysis.file_structure(file_id);
    // println!("File structure: {:#?}", file_structure);

    // Highlight

    println!("Highlight");
    let highlight_config = HighlightConfig {
        strings: true,
        punctuation: true,
        specialize_punctuation: true,
        operator: true,
        specialize_operator: true,
        inject_doc_comment: true,
        macro_bang: true,
        syntactic_name_ref_highlighting: true,
    };

    let syntax = analysis.highlight(highlight_config, file_id)?;

    // Colorize ranges

    let attributes: Vec<_> = syntax
        .iter()
        .map(|range| {
            let (color, weight) = attribute(range.highlight.tag, range.highlight.mods);
            TextAttribute {
                range: range.range.into(),
                color,
                weight,
            }
        })
        .collect();

    // Store for the web viewer.

    let attributed_code = code_viewer::AttributedCode {
        text: file_text.to_string(),
        attributes: attributes.clone(),
    };

    fs::write(
        "/tmp/code.json",
        serde_json::to_string(&attributed_code).unwrap(),
    )
    .unwrap();

    fs::write(
        "/tmp/code.postcard",
        postcard::to_stdvec(&attributed_code).unwrap(),
    )
    .unwrap();

    // Shape and layout text.

    let font_size = 32.;
    let line_height = 40.;
    // let font_size = 16.;
    // let line_height = 20.;

    let (glyph_runs, height) = code_viewer::shape_text(
        &mut font_system,
        &file_text,
        &attributes,
        font_size,
        line_height,
    );

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

fn attribute(tag: HlTag, mods: HlMods) -> (Color, TextWeight) {
    if mods.contains(HlMod::Unsafe) {
        return (unsafe_red(), TextWeight::NORMAL);
    }

    let color = match tag {
        HlTag::Symbol(symbol) => match symbol {
            SymbolKind::Attribute => black(),
            SymbolKind::BuiltinAttr => black(),
            SymbolKind::Const => const_blue(),
            SymbolKind::ConstParam => type_green(),
            SymbolKind::Derive => keyword_blue(),
            SymbolKind::DeriveHelper => keyword_blue(),
            SymbolKind::Enum => type_green(),
            SymbolKind::Field => default_text(),
            SymbolKind::Function => function_brown(),
            SymbolKind::Method => function_brown(),
            SymbolKind::Impl => black(),
            SymbolKind::Label => keyword_blue(),
            SymbolKind::LifetimeParam => keyword_blue(),
            SymbolKind::Local => default_text(),
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
            SymbolKind::ValueParam => default_text(),
            SymbolKind::Variant => const_blue(),
        },
        HlTag::AttributeBracket => keyword_blue(),
        HlTag::BoolLiteral => keyword_blue(),
        HlTag::BuiltinType => type_light_green(),
        HlTag::ByteLiteral => literal_green(),
        HlTag::CharLiteral => literal_red(),
        HlTag::Comment => comment_green(),
        HlTag::EscapeSequence => keyword_blue(),
        HlTag::FormatSpecifier => default_text(),
        HlTag::InvalidEscapeSequence => error_red(),
        HlTag::Keyword => keyword_blue(),
        HlTag::NumericLiteral => literal_green(),
        HlTag::Operator(_) => black(),
        HlTag::Punctuation(_) => black(),
        HlTag::StringLiteral => literal_red(),
        HlTag::UnresolvedReference => error_red(),
        HlTag::None => default_text(),
    };

    let weight = if mods.contains(HlMod::Mutable) {
        TextWeight::BOLD
    } else {
        TextWeight::NORMAL
    };

    (color, weight)
}

fn black() -> Color {
    rgb(0)
}

#[allow(unused)]
fn marker() -> Color {
    rgb(0xff00ff)
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

fn default_text() -> Color {
    rgb(0x001080)
}

fn function_brown() -> Color {
    rgb(0x795e26)
}

fn comment_green() -> Color {
    rgb(0x008000)
}

fn rgb(rgb: u32) -> Color {
    Color::rgb_u32(rgb)
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
