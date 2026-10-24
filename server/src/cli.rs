//! The `ruby-templates` command line, and the `--check`/`--format` flags `ruby-templates-ls` keeps.

use crate::format::{self, Settings};
use crate::lang::TemplateLang;
use crate::lsp::{diagnostics, Doc, Options};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tower_lsp::lsp_types::{DiagnosticSeverity, Url};

const OK: i32 = 0;
const FINDINGS: i32 = 1;
const USAGE: i32 = 2;

const HELP: &str = "\
Usage:
  ruby-templates check <files|dirs>...
  ruby-templates format [--check | --write] [--verbose] <files|dirs>...
  ruby-templates format [--check] [--verbose] --stdin-filepath <path> [-]

check   prints the diagnostics of ERB, Haml, Slim, Jbuilder and Builder templates.
format  re-indents HTML ERB, Haml and Slim templates and fixes trailing whitespace, blank lines, the final newline
        and line endings. Nothing else changes; a file whose structure is in doubt is left as it is, with the reason
        on stderr. Without --check or --write the result goes to stdout.

Exit codes: 0 ok, 1 diagnostics (errors or warnings) or files that would change, 2 usage or I/O error.
";

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

pub fn run(args: &[String]) -> i32 {
    match args.split_first() {
        Some((command, rest)) if command == "check" => match paths(rest) {
            Ok(files) if !files.is_empty() => check(&files, DiagnosticSeverity::WARNING),
            _ => usage("check needs files or folders"),
        },
        Some((command, rest)) if command == "format" => match FormatArgs::parse(rest) {
            Ok(parsed) => format_command(&parsed),
            Err(message) => usage(&message),
        },
        Some((flag, _)) if flag == "--help" || flag == "-h" || flag == "help" => {
            print!("{HELP}");
            OK
        }
        _ => usage("expected a command"),
    }
}

fn usage(message: &str) -> i32 {
    eprintln!("ruby-templates: {message}\n\n{HELP}");
    USAGE
}

fn paths(args: &[String]) -> Result<Vec<PathBuf>, String> {
    args.iter().map(|a| if a.starts_with('-') { Err(format!("unknown option {a}")) } else { Ok(PathBuf::from(a)) }).collect()
}

#[derive(Debug, Default, PartialEq)]
struct FormatArgs {
    check: bool,
    write: bool,
    verbose: bool,
    /// Read the text from stdin; the path picks the language and the config.
    stdin_path: Option<PathBuf>,
    paths: Vec<PathBuf>,
}

impl FormatArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut stdin = false;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--check" => parsed.check = true,
                "--write" => parsed.write = true,
                "--verbose" => parsed.verbose = true,
                "--stdin-filepath" => parsed.stdin_path = Some(args.next().ok_or("--stdin-filepath needs a path")?.into()),
                "-" => stdin = true,
                flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
                path => parsed.paths.push(path.into()),
            }
        }
        if parsed.check && parsed.write {
            return Err("--check and --write exclude each other".into());
        }
        if stdin && parsed.stdin_path.is_none() {
            return Err("reading stdin needs --stdin-filepath".into());
        }
        if parsed.stdin_path.is_some() && (parsed.write || !parsed.paths.is_empty()) {
            return Err("--stdin-filepath formats stdin only, without --write or other paths".into());
        }
        if parsed.stdin_path.is_none() && parsed.paths.is_empty() {
            return Err("format needs files, folders or --stdin-filepath".into());
        }
        Ok(parsed)
    }
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

fn format_command(args: &FormatArgs) -> i32 {
    if let Some(path) = &args.stdin_path {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("cannot read stdin: {e}");
            return USAGE;
        }
        return format_text(path, &text, args);
    }
    let mut code = OK;
    for file in template_files(&args.paths) {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("cannot read {}: {e}", file.display());
                code = USAGE;
                continue;
            }
        };
        code = code.max(format_text(&file, &text, args));
    }
    code
}

fn format_text(path: &Path, text: &str, args: &FormatArgs) -> i32 {
    let shown = path.display();
    let settings = Settings::default();
    let result = match (file_uri(path), language(path)) {
        (Some(uri), Some(lang)) => format::format(&uri, lang, text, &settings, &[]),
        _ => Err(format::Skipped("not an ERB, Haml or Slim template".into())),
    };
    let formatted = match result {
        Ok(formatted) => formatted,
        Err(reason) => {
            eprintln!("{shown}: left unchanged: {reason}");
            text.to_string()
        }
    };
    let changed = formatted != text;
    if args.check {
        if changed {
            println!("{shown}");
        }
        return if changed { FINDINGS } else { OK };
    }
    if !args.write {
        return match std::io::stdout().write_all(formatted.as_bytes()) {
            Ok(()) => OK,
            Err(e) => {
                eprintln!("cannot write stdout: {e}");
                USAGE
            }
        };
    }
    if !changed {
        return OK;
    }
    match std::fs::write(path, formatted) {
        Ok(()) => {
            println!("{shown}");
            OK
        }
        Err(e) => {
            eprintln!("cannot write {shown}: {e}");
            USAGE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn format_arguments() {
        let parsed = FormatArgs::parse(&args(&["--check", "app/views", "a.html.erb"])).unwrap();
        assert_eq!(parsed, FormatArgs { check: true, paths: vec!["app/views".into(), "a.html.erb".into()], ..Default::default() });
        let stdin = FormatArgs::parse(&args(&["--stdin-filepath", "x.html.erb", "-"])).unwrap();
        assert_eq!(stdin.stdin_path.as_deref(), Some(Path::new("x.html.erb")));
        for bad in [&["--check", "--write", "a"][..], &["-"], &["--stdin-filepath"], &["--stdin-filepath", "a", "b"], &["--wat", "a"], &[]]
        {
            assert!(FormatArgs::parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn walks_folders_for_templates_only() {
        let root = std::env::temp_dir().join(format!("rt-cli-walk-{}", std::process::id()));
        let views = root.join("app/views/users");
        std::fs::create_dir_all(&views).unwrap();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        for file in ["app/views/users/show.html.erb", "app/views/users/a.rb", "node_modules/x/b.html.erb", ".git/c.slim"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let found = template_files(std::slice::from_ref(&root));
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, [root.join("app/views/users/show.html.erb")]);
    }
}
