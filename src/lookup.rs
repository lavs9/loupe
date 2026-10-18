use anyhow::Result;
use std::path::PathBuf;

use crate::db::{CatalogDb, SearchHit};
use crate::embedder::MultimodalEmbedder;
use crate::parser::ParsedQuery;
use crate::sources::{Catalog, SearchSet};

#[derive(Debug, Clone)]
pub struct RankedHit {
    pub similarity: f64,
    pub path: PathBuf,
    pub label: String,
    pub camera_model: Option<String>,
    pub focal_length_mm: Option<f64>,
    pub aperture_f: Option<f64>,
    /// 32×32 JPEG written beside the catalog. Missing until that photo is indexed.
    #[cfg_attr(not(feature = "gui"), allow(dead_code))]
    pub preview_path: PathBuf,
}

/// Search every catalog in `set`. A disk that fails to open is skipped; the others still answer.
pub fn search_catalogs(
    set: &SearchSet,
    embedder: &dyn MultimodalEmbedder,
    query: &str,
    limit: usize,
) -> Result<(ParsedQuery, Vec<RankedHit>, Vec<String>)> {
    let parsed = ParsedQuery::parse(query);
    if set.catalogs.is_empty() {
        return Ok((parsed, Vec::new(), Vec::new()));
    }
    let query_vec = embedder.embed_text(&parsed.semantic_prompt)?;
    let mut ranked = Vec::new();
    let mut errors = Vec::new();

    for catalog in &set.catalogs {
        match search_one(catalog, &query_vec, &parsed, limit) {
            Ok(hits) => ranked.extend(hits),
            Err(err) => errors.push(catalog_search_error(&catalog.label, &err)),
        }
    }

    ranked.sort_by(|left, right| {
        right
            .similarity
            .partial_cmp(&left.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(limit);
    Ok((parsed, ranked, errors))
}

fn search_one(
    catalog: &Catalog,
    query_vec: &[f32; crate::db::EMBED_DIM],
    parsed: &ParsedQuery,
    limit: usize,
) -> Result<Vec<RankedHit>> {
    let db = CatalogDb::open_readonly(&catalog.db_path)?;
    if db.model_mismatch(crate::embedder::MODEL_ID)? {
        anyhow::bail!("index is still updating");
    }
    let hits = db.search_hybrid(query_vec, parsed, limit)?;
    Ok(hits
        .into_iter()
        .map(|hit| ranked_hit(catalog, hit))
        .collect())
}

fn catalog_search_error(label: &str, err: &anyhow::Error) -> String {
    let text = err.to_string();
    if text.contains("still updating")
        || text.contains("vec_media")
        || text.contains("no such table")
        || text.contains("no such module")
    {
        format!("{label} is still updating. Search the other folders, or press Update.")
    } else {
        format!("{label}: {err}")
    }
}

fn ranked_hit(catalog: &Catalog, hit: SearchHit) -> RankedHit {
    RankedHit {
        similarity: 1.0 - hit.distance,
        path: catalog.drive_root.join(hit.rel_path),
        label: catalog.label.clone(),
        camera_model: hit.camera_model,
        focal_length_mm: hit.focal_length_mm,
        aperture_f: hit.aperture_f,
        preview_path: crate::indexer::preview_path(&catalog.drive_root, &hit.blake3_hash),
    }
}

pub fn print_results(
    model: &str,
    parsed: &ParsedQuery,
    set: &SearchSet,
    hits: &[RankedHit],
    errors: &[String],
) {
    println!("model {model}");
    let mut summary = parsed.semantic_prompt.clone();
    if let Some(mm) = parsed.focal_length_mm {
        summary.push_str(&format!(" · {mm} mm"));
    }
    if let Some(aperture) = parsed.max_aperture_f {
        summary.push_str(&format!(" · f/{aperture}"));
    }
    if let Some(iso) = parsed.max_iso {
        summary.push_str(&format!(" · ISO {iso}"));
    }
    println!("{summary}");

    if set.catalogs.is_empty() {
        let unplugged = set.notes.iter().any(|note| note.status == "not connected");
        if unplugged {
            println!("a disk is unplugged; folders on this Mac are still searchable");
        } else {
            println!("no indexes to search");
            println!("index a folder on this Mac, or connect a disk that already has .photo_index");
        }
        for note in &set.notes {
            eprintln!("{}  {}", note.status, note.path.display());
        }
        return;
    }

    let indexes = set
        .catalogs
        .iter()
        .map(|catalog| catalog.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "{} {} : {indexes}",
        set.catalogs.len(),
        if set.catalogs.len() == 1 { "index" } else { "indexes" }
    );

    for note in &set.notes {
        eprintln!("{}  {}", note.status, note.path.display());
    }
    for err in errors {
        eprintln!("skipped  {err}");
    }

    if hits.is_empty() {
        println!("\n0 matches");
        return;
    }

    println!("\n{} matches", hits.len());
    for (index, hit) in hits.iter().enumerate() {
        let camera = hit.camera_model.as_deref().unwrap_or("—");
        let focal = hit
            .focal_length_mm
            .map(|mm| format!("{mm} mm"))
            .unwrap_or_else(|| "—".to_string());
        let aperture = hit
            .aperture_f
            .map(|value| format!("f/{value}"))
            .unwrap_or_else(|| "—".to_string());
        println!(
            "\n  {:2}  {:.3}  {}",
            index + 1,
            hit.similarity,
            hit.label
        );
        println!("      {}", hit.path.display());
        println!("      {camera} · {focal} · {aperture}");
    }
}
