mod app_model;
mod db;
mod embedder;
mod gemma;
#[cfg(feature = "gui")]
mod gui;
mod indexer;
mod lookup;
mod parser;
mod preview;
mod sources;
mod vfs;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nfsserve::tcp::{NFSTcp, NFSTcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::info;

use crate::db::CatalogDb;
use crate::embedder::MultimodalEmbedder;
use crate::gemma::GemmaEmbedder;
use crate::indexer::write_config;
use crate::lookup::{print_results, search_catalogs};
use crate::parser::ParsedQuery;
use crate::sources::find_catalog;
use crate::vfs::PhotoSearchVfs;

#[derive(Parser)]
#[command(name = "photo-engine")]
#[command(about = "Index and search photos on this Mac and on disks you plug in")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init {
        #[arg(default_value = ".")]
        drive_root: PathBuf,
    },
    Index {
        #[arg(default_value = ".")]
        drive_root: PathBuf,
    },
    /// Search this Mac's indexes and every connected external disk that has `.photo_index`.
    Search {
        /// Query words. Camera terms such as `35mm`, `f1.8`, and `iso400` filter too.
        #[arg(required = true)]
        words: Vec<String>,
        /// Extra folder or disk to include this once. It must already contain an index.
        #[arg(short, long)]
        source: Vec<PathBuf>,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    /// How many photos each connected index holds.
    Status,
    /// Load EmbeddingGemma and leave it running. Prints `ready <url>` when it answers.
    Serve {
        /// TCP port. 0 picks a free port.
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Show the local folders and connected disks search will read.
    Sources {
        #[command(subcommand)]
        action: Option<SourceAction>,
    },
    Query {
        #[arg(short, long, default_value = ".")]
        drive_root: PathBuf,
        query: String,
        #[arg(short, long, default_value_t = 20)]
        limit: usize,
    },
    Mount {
        #[arg(default_value = ".")]
        drive_root: PathBuf,
        #[arg(short, long, default_value_t = 11111)]
        port: u16,
    },
    /// Open the search window. `photo-engine gui --self-check` prints the folder list and exits.
    Gui {
        /// Print folders, the shortcut, and the update check, then exit. No window.
        #[arg(long)]
        self_check: bool,
    },
}

#[derive(Subcommand)]
enum SourceAction {
    /// Remember a folder on this Mac. A connected external disk is picked up without this.
    Add { path: PathBuf },
    /// Stop searching a remembered folder. The index on disk is left in place.
    Remove { path: PathBuf },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    match cli.command {
        Commands::Gui { self_check } => {
            if self_check {
                app_model::print_self_check()
            } else {
                open_gui()
            }
        }
        command => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(run_command(command))
        }
    }
}

fn open_gui() -> Result<()> {
    #[cfg(feature = "gui")]
    {
        gui::run()
    }
    #[cfg(not(feature = "gui"))]
    {
        anyhow::bail!(
            "This build has no window. Rebuild with `cargo build --features gui`, then run `photo-engine gui`."
        );
    }
}

async fn run_command(command: Commands) -> Result<()> {
    let embedder = Arc::new(GemmaEmbedder::new());

    match command {
        Commands::Init { drive_root } => {
            let root = std::fs::canonicalize(&drive_root).unwrap_or(drive_root);
            std::fs::create_dir_all(root.join("Photos"))?;
            std::fs::create_dir_all(root.join("Search"))?;
            std::fs::create_dir_all(root.join(".photo_index/models"))?;
            std::fs::create_dir_all(root.join(".photo_index/cache"))?;
            write_config(&root)?;
            let db_path = root.join(".photo_index/db/catalog.db");
            let _db = CatalogDb::open(&db_path)?;
            println!("initialized {}", root.display());
            info!("Initialized portable .photo_index workspace at {:?}", root);
        }

        Commands::Index { drive_root } => {
            let outcome = app_model::index_folder(&drive_root, embedder.as_ref())?;
            let report = &outcome.report;
            println!(
                "indexed {}\n  unchanged {}\n  moved {}\n  embedded {}\n  removed {}",
                outcome.root.display(),
                report.unchanged,
                report.moved,
                report.embedded,
                report.removed
            );
            if let Some(err) = &outcome.remember_error {
                eprintln!(
                    "indexed {}, but this Mac will not remember it: {err}",
                    outcome.root.display()
                );
            }
        }

        Commands::Search {
            words,
            source,
            limit,
        } => {
            let query = words.join(" ");
            let set = sources::collect(&source)?;
            let (parsed, hits, errors) =
                search_catalogs(&set, embedder.as_ref(), &query, limit)?;
            print_results(
                embedder.model_version(),
                &parsed,
                &set,
                &hits,
                &errors,
            );
            if set.catalogs.is_empty() {
                anyhow::bail!("nothing to search");
            }
        }

        Commands::Status => print_status()?,

        Commands::Serve { port } => embedder.serve(port)?,

        Commands::Sources { action } => match action {
            None => print_source_report()?,
            Some(SourceAction::Add { path }) => {
                let root = std::fs::canonicalize(&path)
                    .with_context(|| format!("{} is not available", path.display()))?;
                sources::remember(&root)?;
                match find_catalog(&root) {
                    Some(catalog) => println!(
                        "remembered {}  ({})",
                        root.display(),
                        catalog.db_path.display()
                    ),
                    None => println!(
                        "remembered {}  (no index yet — search skips it until this folder is indexed)",
                        root.display()
                    ),
                }
            }
            Some(SourceAction::Remove { path }) => {
                if sources::forget(&path)? {
                    println!("forgot {}", path.display());
                } else {
                    anyhow::bail!("{} is not a remembered folder", path.display());
                }
            }
        },

        Commands::Query {
            drive_root,
            query,
            limit,
        } => {
            let root = std::fs::canonicalize(&drive_root)?;
            let db_path = root.join(".photo_index/db/catalog.db");
            let db = CatalogDb::open_readonly(&db_path)?;

            let parsed = ParsedQuery::parse(&query);
            info!("Parsed Query -> Semantic: '{}' | Focal: {:?}mm | Max Aperture: f/{:?} | Max ISO: {:?} | Year: {:?}",
                parsed.semantic_prompt, parsed.focal_length_mm, parsed.max_aperture_f, parsed.max_iso, parsed.year_range);

            let q_vec = embedder.embed_text(&parsed.semantic_prompt)?;
            let hits = db.search_hybrid(&q_vec, &parsed, limit)?;

            println!("\nFound {} matches:", hits.len());
            for (i, h) in hits.iter().enumerate() {
                println!(
                    "  {:02}. [sim: {:.3}] {} (Camera: {:?}, Focal: {:?}mm, f/{:?})",
                    i + 1,
                    1.0 - h.distance,
                    h.rel_path,
                    h.camera_model.as_deref().unwrap_or("-"),
                    h.focal_length_mm.unwrap_or(0.0),
                    h.aperture_f.unwrap_or(0.0)
                );
            }
        }

        Commands::Mount { drive_root, port } => {
            let root = std::fs::canonicalize(&drive_root)?;
            let db_path = root.join(".photo_index/db/catalog.db");
            let search_mount = root.join("Search");
            std::fs::create_dir_all(&search_mount)?;

            let vfs = PhotoSearchVfs::new(root.clone(), db_path, embedder);
            let listener = NFSTcpListener::bind(&format!("127.0.0.1:{}", port), vfs).await?;

            println!("==================================================================");
            println!("Live Virtual Photo Search Server listening on 127.0.0.1:{}", port);
            println!("To mount natively in macOS Finder (Zero Kexts Required), run:");
            println!(
                "  mount_nfs -o nolocks,vers=3,tcp,port={},mountport={} 127.0.0.1:/ {:?}",
                port, port, search_mount
            );
            println!("Then in Finder or Terminal, open any query as a folder:");
            println!("  ls -la {:?}/\"sunset mountain 35mm f1.8\"", search_mount);
            println!("==================================================================");

            listener.handle_forever().await?;
        }
        Commands::Gui { .. } => {
            anyhow::bail!("the window is opened before the runtime starts");
        }
    }

    Ok(())
}

fn print_status() -> Result<()> {
    let set = sources::collect(&[])?;
    if set.catalogs.is_empty() {
        println!("no indexes yet");
        return Ok(());
    }
    for catalog in &set.catalogs {
        match CatalogDb::open_readonly(&catalog.db_path).and_then(|db| db.stats()) {
            Ok(stats) => println!(
                "{}  {} photos  {} paths  model {}",
                catalog.label,
                stats.assets,
                stats.paths,
                stats.model.as_deref().unwrap_or("—")
            ),
            Err(err) => println!("{}  unreadable  {err}", catalog.label),
        }
    }
    for note in &set.notes {
        println!("{}  {}", note.status, note.path.display());
    }
    Ok(())
}

fn print_source_report() -> Result<()> {
    let set = sources::collect(&[])?;
    println!("This Mac and connected disks\n");
    if set.catalogs.is_empty() && set.notes.is_empty() {
        println!("no indexes yet");
        println!("index a folder on this Mac, or connect a disk that already has .photo_index");
        return Ok(());
    }
    for catalog in &set.catalogs {
        println!("  ready          {}", catalog.label);
        println!("                 {}", catalog.drive_root.display());
        println!("                 {}", catalog.db_path.display());
    }
    for note in &set.notes {
        println!("  {:14} {}", note.status, note.path.display());
    }
    Ok(())
}
