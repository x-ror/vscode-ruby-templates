//! The `ruby-templates` command line, and the `--check`/`--format` flags `ruby-templates-ls` keeps.

use crate::checks::{diagnostics_in, Doc};
use crate::config;
use crate::format;
use crate::format::SkipKind;
use crate::lang::TemplateLang;
use crate::options::Options;
use crate::rails::project::{Project, Projects};
use crate::rails::view_roots;
use crate::rules::file::ConfigFile;
use crate::rules::resolve::{resolve, Configs, ResolvedRules};
use crate::util;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use tower_lsp_server::ls_types::{DiagnosticSeverity, Uri};

const OK: i32 = 0;
const FINDINGS: i32 = 1;
const USAGE: i32 = 2;
/// The main thread's usual stack: Prism and the template walkers recurse with the nesting of the code.
const WORKER_STACK: usize = 8 << 20;

const HELP: &str = "\
Usage:
  ruby-templates check <files|dirs>...
  ruby-templates format [--check | --write] [--verbose] [--align-comments] <files|dirs>...
  ruby-templates format [--check] [--verbose] [--align-comments] --stdin-filepath <path> [-]

check   prints the diagnostics of ERB, Haml, Slim, Jbuilder and Builder templates, and with Layout/LineLength (off) the lines
        over its max.
format  re-indents HTML ERB, Haml and Slim templates and fixes trailing whitespace. The rules Layout/EmptyLines,
        Layout/EmptyLinesAroundBlockBody, Layout/EmptyLinesAroundElementBody and Layout/EmptyLinesAroundBranches (all
        off) and Layout/TrailingEmptyLines (on for Haml and Slim, off for ERB) set blank lines and the final newline.
        Layout/SpaceInsideErbTag, Layout/SpaceInHtmlTag, Style/ErbCommentSyntax and Layout/SpaceAfterScriptMarker
        (all off) set spaces inside ERB and HTML tags and after Haml and Slim script markers. Layout/IndentationStyle
        (ERB; spaces unless .editorconfig or its style say tabs) and Layout/TrailingWhitespace (on) set the indent
        character and whether line-end whitespace goes. Turbo/PreferTurboAttributes (off) renames the Rails UJS
        attributes data-confirm, data-method and data-disable-with to Turbo's. Style/AttributeQuotes, Style/BooleanAttributes,
        Style/VoidTagClosing and Style/TagNameCase (all off) set attribute quotes, boolean attributes, the end of void
        tags and the case of tag names. Style/ImplicitDiv, Style/ClassesBeforeIds and Style/HashAttributes (all off) drop
        a div a class or id implies, put Haml classes before ids and write Haml attributes as a hash or in HTML style. Layout/ElementContentOnOwnLine (off) moves the content of the elements in its
        tags option to a line of its own. Layout/AttributesPerLine (ERB, off) puts each attribute of a start tag longer than its
        max_line_length on a line of its own. Nothing else changes;
        a file whose structure is in doubt is left as it is, with the reason on stderr. Without --check or --write
        the result goes to stdout.
        --align-comments moves the lines inside ERB comments with the line the comment opens on.

Both read .ruby-templates.toml files, from the file's folder up. Exit codes: 0 ok, 1 diagnostics (errors or
warnings), a problem in a .ruby-templates.toml, or files that would change, 2 usage or I/O error, a formatter bug,
or (for check) a named file that isn't a template.
";

/// `ruby-templates-ls --check <file>...` and `--format <file.slim>`: the exit code, or `None` to start the server.
pub fn legacy(args: &[String]) -> Option<i32> {
    match args {
        [flag, file] if flag == "--format" => Some(legacy_format(file)),
        [flag, files @ ..] if flag == "--check" => {
            let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
            Some(check(&files, DiagnosticSeverity::ERROR, FINDINGS))
        }
        _ => None,
    }
}

/// Prints the Slim file formatted with 2 spaces, or fails if it can't be.
fn legacy_format(file: &str) -> i32 {
    let formatted = std::fs::read_to_string(file).ok().and_then(|text| {
        let settings = crate::format::Settings::default();
        let indented = crate::format::slim::format(&text, &settings, &[])?;
        Some(crate::format::blank_lines::apply(&indented.text, &indented.lines, &settings, &text))
    });
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
            Ok(files) if files.is_empty() => usage("check needs files or folders"),
            Ok(files) => check(&files, DiagnosticSeverity::WARNING, USAGE),
            Err(message) => usage(&message),
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
    align_comments: bool,
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
                "--align-comments" => parsed.align_comments = true,
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

struct File {
    path: PathBuf,
    /// Named on the command line rather than found in a folder: kept even when its type is unknown, so it can be
    /// reported.
    named: bool,
}

/// Template files under `paths`, following symlinked folders as the server does.
fn template_files(paths: &[PathBuf]) -> Vec<File> {
    let mut out = Vec::new();
    for path in paths {
        if !path.is_dir() {
            out.push(File { path: path.clone(), named: true });
            continue;
        }
        let walk = walkdir::WalkDir::new(path).follow_links(true).sort_by_file_name().into_iter();
        let entries = walk.filter_entry(|e| e.depth() == 0 || !is_skipped_dir(e)).filter_map(Result::ok);
        let templates = entries.filter(|e| e.file_type().is_file() && language(e.path()).is_some());
        out.extend(templates.map(|e| File { path: e.into_path(), named: false }));
    }
    out
}

fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    entry.file_type().is_dir() && (name.starts_with('.') || name == "node_modules")
}

fn absolute_file_uri(path: &Path) -> Option<Uri> {
    let absolute = std::fs::canonicalize(path).or_else(|_| std::path::absolute(path)).ok()?;
    crate::util::file_uri(absolute)
}

fn language(path: &Path) -> Option<TemplateLang> {
    TemplateLang::from_file_name(&path.file_name()?.to_string_lossy())
}

/// Prints diagnostics as `file:line:col: severity: message`; 1 when one is at least as severe as `fail_at`, `bad_file`
/// for a file that can't be read or isn't a template.
fn check(paths: &[PathBuf], fail_at: DiagnosticSeverity, bad_file: i32) -> i32 {
    let files = template_files(paths);
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).min(files.len()).max(1);
    let next = AtomicUsize::new(0);
    let view_roots = Arc::new(view_roots::Cache::default());
    let reported = Mutex::new(HashSet::new());
    let (sender, results) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (sender, next, files, view_roots, reported) = (sender.clone(), &next, &files, &view_roots, &reported);
            let work = move || {
                let (mut projects, mut configs) = (Projects::new(), Configs::new());
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(index) else { break };
                    // The printer hung up (stdout closed): nothing left to do.
                    let checked = check_file(&file.path, fail_at, bad_file, (&mut projects, &mut configs), view_roots, reported);
                    if sender.send((index, checked)).is_err() {
                        break;
                    }
                }
            };
            // A thread the system refuses still gets its share of the files, here.
            if std::thread::Builder::new().stack_size(WORKER_STACK).spawn_scoped(scope, work.clone()).is_err() {
                work();
            }
        }
        drop(sender);
        print_in_order(results)
    })
}

struct Checked {
    stdout: String,
    stderr: String,
    code: i32,
}

fn check_file(
    path: &Path,
    fail_at: DiagnosticSeverity,
    bad_file: i32,
    (projects, configs): (&mut Projects, &mut Configs),
    view_roots: &Arc<view_roots::Cache>,
    reported: &Mutex<HashSet<PathBuf>>,
) -> Checked {
    let shown = path.display();
    let failed = |stderr| Checked { stdout: String::new(), stderr, code: bad_file };
    let Ok(text) = std::fs::read_to_string(path) else { return failed(format!("cannot read {shown}\n")) };
    let Some((uri, lang)) = absolute_file_uri(path).zip(language(path)) else { return failed(format!("{shown}: unknown template type\n")) };
    // Every worker's projects of one app share its view roots.
    Project::memoized(projects, path, Some(view_roots));
    let mut checked = Checked { stdout: String::new(), stderr: String::new(), code: OK };
    let rules = resolve(&uri, lang, &Options::default(), configs);
    for file in rules.files() {
        // Each file's problems print once, with the first template it applies to.
        if !file.problems.is_empty() && reported.lock().is_ok_and(|mut seen| seen.insert(file.path.clone())) {
            print_problems(file, &mut checked.stdout);
            if DiagnosticSeverity::WARNING <= fail_at {
                checked.code = FINDINGS;
            }
        }
    }
    for d in diagnostics_in(&uri, &Doc { lang, text, plain_erb: false, percent_lines: false, version: 0 }, &rules, projects, &[]) {
        let severity = d.severity.unwrap_or(DiagnosticSeverity::ERROR);
        let name = match severity {
            DiagnosticSeverity::ERROR => "error",
            DiagnosticSeverity::WARNING => "warning",
            _ => "info",
        };
        let (line, col) = (d.range.start.line + 1, d.range.start.character + 1);
        let _ = writeln!(checked.stdout, "{shown}:{line}:{col}: {name}: {}", d.message);
        // Lower numbers are more severe.
        if severity <= fail_at {
            checked.code = FINDINGS;
        }
    }
    checked
}

/// `path` from the current folder when it is below it, as the template paths are given.
fn shown_path(path: &Path) -> &Path {
    let here = std::fs::canonicalize(".").ok();
    here.and_then(|here| path.strip_prefix(here).ok()).unwrap_or(path)
}

fn print_problems(file: &ConfigFile, out: &mut String) {
    for problem in &file.problems {
        let (line, col) = (problem.range.start.line + 1, problem.range.start.character + 1);
        let _ = writeln!(out, "{}:{line}:{col}: warning: {}", shown_path(&file.path).display(), problem.message);
    }
}

/// Prints each file's result in walk order as it arrives; the worst exit code, `USAGE` once stdout is closed.
fn print_in_order(results: mpsc::Receiver<(usize, Checked)>) -> i32 {
    let mut stdout = BufWriter::new(std::io::stdout().lock());
    let mut waiting = BTreeMap::new();
    let mut next = 0;
    let mut code = OK;
    for (index, checked) in results {
        waiting.insert(index, checked);
        while let Some(checked) = waiting.remove(&next) {
            next += 1;
            code = code.max(checked.code);
            let printed = stdout.write_all(checked.stdout.as_bytes()).and_then(|()| {
                if checked.stderr.is_empty() {
                    return Ok(());
                }
                // Keep stderr lines in place among the diagnostics.
                stdout.flush()?;
                std::io::stderr().write_all(checked.stderr.as_bytes())
            });
            if printed.is_err() {
                return USAGE;
            }
        }
    }
    if stdout.flush().is_err() {
        return USAGE;
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
        return format_text(path, true, &text, args, (&mut Projects::new(), &mut Configs::new()));
    }
    let files = template_files(&args.paths);
    // Several files on stdout would run together.
    if !args.check && !args.write && files.len() != 1 {
        return usage(&format!("printing to stdout takes one file, not {}; use --check or --write", files.len()));
    }
    let mut code = OK;
    let (mut projects, mut configs) = (Projects::new(), Configs::new());
    for File { path, named } in files {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("cannot read {}: {e}", path.display());
                code = USAGE;
                continue;
            }
        };
        code = code.max(format_text(&path, named, &text, args, (&mut projects, &mut configs)));
    }
    code
}

/// The settings in force for `shown`, what was read to get them, and the config files' problems.
fn print_verbose(shown: &str, settings: &format::Settings, notes: &[String], rules: &ResolvedRules) {
    eprintln!("{shown}: {settings:?}");
    notes.iter().for_each(|note| eprintln!("{shown}: {note}"));
    for file in rules.files() {
        eprintln!("{shown}: read {}", shown_path(&file.path).display());
        let mut problems = String::new();
        print_problems(file, &mut problems);
        eprint!("{problems}");
    }
}

fn format_text(path: &Path, named: bool, text: &str, args: &FormatArgs, (projects, configs): (&mut Projects, &mut Configs)) -> i32 {
    let shown = path.display();
    let result = match (absolute_file_uri(path), language(path)) {
        (Some(uri), Some(lang)) => {
            let rules = resolve(&uri, lang, &Options::default(), configs);
            let resolved = config::settings_for(&util::file_path(&uri).unwrap_or_else(|| path.to_path_buf()));
            let mut settings = rules.format_settings(resolved.settings);
            settings.align_comments |= args.align_comments;
            if args.verbose {
                print_verbose(&shown.to_string(), &settings, &resolved.notes, &rules);
            }
            if let Some(by) = rules.excluded_by() {
                format::skip(SkipKind::NotHandled, format!("excluded by {}", by.display()))
            } else if !rules.format_enabled() {
                format::skip(SkipKind::NotHandled, "formatting is turned off in .ruby-templates.toml")
            } else {
                format::format_in(&uri, lang, false, text, &settings, rules.extra_engines(), projects)
            }
        }
        _ => format::skip(SkipKind::NotHandled, "not an ERB, Haml or Slim template"),
    };
    let mut code = OK;
    let formatted = match result {
        Ok(formatted) => {
            if args.verbose {
                formatted.notes.iter().for_each(|note| eprintln!("{shown}: {note}"));
            }
            formatted.text
        }
        Err(skipped) => {
            // Folders hold plenty of views the formatter never handles; only doubts and bugs are news there.
            if named || args.verbose || skipped.kind != SkipKind::NotHandled {
                eprintln!("{shown}: left unchanged: {skipped}");
            }
            if skipped.kind == SkipKind::Internal {
                code = USAGE;
            }
            text.to_string()
        }
    };
    let changed = formatted != text;
    if args.check {
        if changed {
            println!("{shown}");
        }
        return code.max(if changed { FINDINGS } else { OK });
    }
    if !args.write {
        return match std::io::stdout().write_all(formatted.as_bytes()) {
            Ok(()) => code,
            Err(e) => {
                eprintln!("cannot write stdout: {e}");
                USAGE
            }
        };
    }
    if !changed {
        return code;
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
        assert!(FormatArgs::parse(&args(&["--align-comments", "a.html.erb"])).unwrap().align_comments);
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
        let found: Vec<PathBuf> = template_files(std::slice::from_ref(&root)).into_iter().map(|f| f.path).collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, [root.join("app/views/users/show.html.erb")]);
    }
}
