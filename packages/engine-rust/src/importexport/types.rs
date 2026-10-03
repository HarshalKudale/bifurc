//! The shapes shared by every importer and exporter.
//!
//! A failure is the text the Node engine put in `{ ok: false, error }`: `String(err)` of whatever it threw.

use std::{future::Future, path::Path, pin::Pin};

use serde_json::{Value, json};

/// What an exporter renders: the file's text and the name to offer for it.
pub struct ExportFile {
    pub content: String,
    pub suggested_name: &'static str,
}

/// An uploaded file as the importers read it.
pub struct ImportSource {
    pub content: String,
    pub filename: String,
}

/// What to do with an imported item whose id already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    Keep,
    Override,
    New,
}

impl Strategy {
    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("override") => Strategy::Override,
            Some("new") => Strategy::New,
            _ => Strategy::Keep,
        }
    }
}

/// A preflight that found a readable file.
pub struct Preflight {
    pub item_count: usize,
    pub collision_ids: Vec<String>,
}

impl Preflight {
    pub fn counted(item_count: usize) -> Self {
        Self { item_count, collision_ids: Vec::new() }
    }

    pub fn to_value(&self) -> Value {
        json!({ "ok": true, "itemCount": self.item_count, "collisionIds": self.collision_ids })
    }
}

/// A finished import. `skipped` is reported only by the formats that can skip.
pub struct Imported {
    pub imported: usize,
    pub skipped: Option<usize>,
}

impl Imported {
    pub fn all(imported: usize) -> Self {
        Self { imported, skipped: None }
    }

    pub fn with_skipped(imported: usize, skipped: usize) -> Self {
        Self { imported, skipped: Some(skipped) }
    }

    pub fn to_value(&self) -> Value {
        match self.skipped {
            Some(skipped) => json!({ "ok": true, "imported": self.imported, "skipped": skipped }),
            None => json!({ "ok": true, "imported": self.imported }),
        }
    }
}

pub type ExportOutcome = Result<ExportFile, String>;
pub type PreflightOutcome = Result<Preflight, String>;
pub type ImportOutcome = Result<Imported, String>;
pub type ImportFuture = Pin<Box<dyn Future<Output = ImportOutcome> + Send>>;

/// An importer that reads the uploaded text.
pub struct TextImporter {
    pub preflight: fn(&str, &ImportSource) -> PreflightOutcome,
    pub run: fn(String, ImportSource, Strategy) -> ImportFuture,
}

/// An importer that needs the uploaded file itself.
pub struct PathImporter {
    pub preflight: fn(&Path) -> PreflightOutcome,
    pub run: fn(String, std::path::PathBuf, Strategy) -> ImportFuture,
}

pub enum Importer {
    Text(TextImporter),
    Path(PathImporter),
}

pub enum Exporter {
    /// Renders text in memory.
    Text(fn(&str) -> ExportOutcome),
    /// Writes a file (a zip archive) to the given path.
    Path(fn(&str, &Path) -> Result<(), String>),
}

/// One row of the format registry.
pub struct FormatEntry {
    pub kind: &'static str,
    pub id: &'static str,
    pub label: &'static str,
    pub extensions: &'static [&'static str],
    pub exporter: Exporter,
    pub importer: Importer,
}

impl FormatEntry {
    /// The format as `export.formats` lists it.
    pub fn definition(&self) -> Value {
        json!({
            "id": self.id,
            "label": self.label,
            "extensions": self.extensions,
            "supportsExport": true,
            "supportsImport": true,
        })
    }
}
