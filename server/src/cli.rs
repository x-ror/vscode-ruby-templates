//! The `--check` and `--format` flags of `ruby-templates-ls`.

use crate::lang::TemplateLang;
use crate::lsp::{diagnostics, Doc, Options};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

const OK: i32 = 0;
const FINDINGS: i32 = 1;

/// `ruby-templates-ls --check <file>...` and `--format <file.slim>`: the exit code, or `None` to start the server.
pub fn legacy(args: &[String]) -> Option<i32> {
    match args {
        [flag, file] if flag == "--format" => Some(legacy_format(file)),
        [flag, files @ ..] if flag == "--check" => {
            let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
            Some(check(&files, DiagnosticSeverity::ERROR))
        }
        _ => None,
    }
}

/// Prints the Slim file formatted with 2 spaces, or fails if it can't be.
fn legacy_format(file: &str) -> i32 {
    let formatted = std::fs::read_to_string(file).ok().and_then(|text| crate::slim_format::format(&text, 2, &[]));
    let Some(formatted) = formatted else {
        eprintln!("{file}: cannot be formatted");
        return FINDINGS;
    };
    print!("{formatted}");
    OK
}

/// Template files under `paths`; named files are kept even when their type is unknown, so they can be reported.
fn template_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if !path.is_dir() {
            out.push(path.clone());
            continue;
        }
        let walk = walkdir::WalkDir::new(path).sort_by_file_name().into_iter();
        let entries = walk.filter_entry(|e| e.depth() == 0 || !is_skipped_dir(e)).filter_map(Result::ok);
        out.extend(entries.filter(|e| e.file_type().is_file() && language(e.path()).is_some()).map(|e| e.into_path()));
    }
    out
}

fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    entry.file_type().is_dir() && (name.starts_with('.') || name == "node_modules")
}

fn file_uri(path: &Path) -> Option<Url> {
    let absolute = std::fs::canonicalize(path).or_else(|_| std::path::absolute(path)).ok()?;
    Url::from_file_path(absolute).ok()
}

fn language(path: &Path) -> Option<TemplateLang> {
    TemplateLang::from_uri_or_id(&file_uri(path)?, None)
}

/// Prints diagnostics as `file:line:col: severity: message`; 1 when one is at least as severe as `fail_at`.
fn check(paths: &[PathBuf], fail_at: DiagnosticSeverity) -> i32 {
    let mut code = OK;
    for file in template_files(paths) {
        let shown = file.display();
        let Ok(text) = std::fs::read_to_string(&file) else {
            eprintln!("cannot read {shown}");
            code = code.max(FINDINGS);
            continue;
        };
        let (Some(uri), Some(lang)) = (file_uri(&file), language(&file)) else {
            eprintln!("{shown}: unknown template type");
            code = code.max(FINDINGS);
            continue;
        };
        for d in diagnostics(&uri, &Doc { lang, text, version: 0 }, &Options::default()) {
            let severity = d.severity.unwrap_or(DiagnosticSeverity::ERROR);
            let name = match severity {
                DiagnosticSeverity::ERROR => "error",
                DiagnosticSeverity::WARNING => "warning",
                _ => "info",
            };
            println!("{shown}:{}:{}: {name}: {}", d.range.start.line + 1, d.range.start.character + 1, d.message);
            // Lower numbers are more severe.
            if severity <= fail_at {
                code = code.max(FINDINGS);
            }
        }
    }
    code
}
