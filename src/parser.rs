use chrono::{TimeZone, Utc};
use regex::Regex;

#[derive(Debug, Clone, Default)]
pub struct ParsedQuery {
    pub semantic_prompt: String,
    pub focal_length_mm: Option<f64>,
    pub max_aperture_f: Option<f64>,
    pub max_iso: Option<u32>,
    pub year_range: Option<(i64, i64)>,
}

impl ParsedQuery {
    pub fn parse(raw_input: &str) -> Self {
        let mut remaining = raw_input.to_string();
        let mut parsed = ParsedQuery::default();

        let re_fl = Regex::new(r"(?i)\b(\d{1,3}(?:\.\d+)?)\s*mm\b").unwrap();
        if let Some(caps) = re_fl.captures(&remaining) {
            if let Ok(val) = caps[1].parse::<f64>() {
                parsed.focal_length_mm = Some(val);
            }
            remaining = re_fl.replace_all(&remaining, "").to_string();
        }

        let re_ap = Regex::new(r"(?i)\bf/?(\d{1,2}(?:\.\d+)?)\b").unwrap();
        if let Some(caps) = re_ap.captures(&remaining) {
            if let Ok(val) = caps[1].parse::<f64>() {
                parsed.max_aperture_f = Some(val);
            }
            remaining = re_ap.replace_all(&remaining, "").to_string();
        }

        let re_iso = Regex::new(r"(?i)\biso\s*(\d{2,6})\b").unwrap();
        if let Some(caps) = re_iso.captures(&remaining) {
            if let Ok(val) = caps[1].parse::<u32>() {
                parsed.max_iso = Some(val);
            }
            remaining = re_iso.replace_all(&remaining, "").to_string();
        }

        let re_year = Regex::new(r"\b(20\d{2})\b").unwrap();
        if let Some(caps) = re_year.captures(&remaining) {
            if let Ok(year) = caps[1].parse::<i32>() {
                let start = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).unwrap().timestamp();
                let end = Utc.with_ymd_and_hms(year, 12, 31, 23, 59, 59).unwrap().timestamp();
                parsed.year_range = Some((start, end));
            }
            remaining = re_year.replace_all(&remaining, "").to_string();
        }

        let clean = remaining
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        parsed.semantic_prompt = if clean.is_empty() {
            "photo".to_string()
        } else {
            clean
        };

        parsed
    }

    pub fn has_exif_filter(&self) -> bool {
        self.focal_length_mm.is_some()
            || self.max_aperture_f.is_some()
            || self.max_iso.is_some()
            || self.year_range.is_some()
    }
}
