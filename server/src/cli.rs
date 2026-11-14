//! The `ruby-templates` command line.

use crate::checks::{diagnostics_in, Doc};
use crate::config;
use crate::format;
use crate::format::SkipKind;
use crate::lang::TemplateLang;
use crate::options::Options;
use crate::rails::paths;
use crate::rails::project::{Project, Projects};
use crate::rails::view_roots;
use crate::report::{self, Finding, Format, Severity};
use crate::rules::file::ConfigFile;
use crate::rules::resolve::{resolve, Configs, ResolvedRules};
use crate::util;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::io::{BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use tower_lsp_server::ls_types::{DiagnosticSeverity, Range, Uri};

const OK: i32 = 0;
const FINDINGS: i32 = 1;
const USAGE: i32 = 2;
/// The main thread's usual stack: Prism and the template walkers recurse with the nesting of the code.
const WORKER_STACK: usize = 8 << 20;

const HELP: &str = "\
Usage:
  ruby-templates check [--format text|github|sarif|json] <files|dirs>...
  ruby-templates format [--check | --write] [--format text|github] [--verbose] [--align-comments] <files|dirs>...
  ruby-templates format [--check] [--format text|github] [--verbose] [--align-comments] --stdin-filepath <path> [-]

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

--format picks how check prints: text (the default), github (workflow commands that annotate a pull request), sarif
(SARIF 2.1.0 for code scanning) or json (one object per line). With format --check, github prints a warning per file that
would change. In github, sarif and json the columns count characters and start at 1.

Both read .ruby-templates.toml files, from the file's folder up. Exit codes: 0 ok, 1 diagnostics (errors or
warnings), a problem in a .ruby-templates.toml, or files that would change, 2 usage or I/O error, a formatter bug,
or (for check) a named file that isn't a template.
";

pub fn run(args: &[String]) -> i32 {
    match args.split_first() {
        Some((command, rest)) if command == "check" => match CheckArgs::parse(rest) {
            Ok(parsed) => check(&parsed.paths, parsed.format),
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

#[derive(Debug, PartialEq)]
struct CheckArgs {
    format: Format,
    paths: Vec<PathBuf>,
}

impl CheckArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let (mut format, mut paths) = (Format::Text, Vec::new());
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--format" => format = Format::parse(args.next().ok_or("--format needs a name")?)?,
                flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
                path => paths.push(path.into()),
            }
        }
        if paths.is_empty() {
            return Err("check needs files or folders".into());
        }
        Ok(CheckArgs { format, paths })
    }
}

#[derive(Debug, Default, PartialEq)]
struct FormatArgs {
    check: bool,
    write: bool,
    verbose: bool,
    align_comments: bool,
    format: Format,
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
                "--format" => parsed.format = Format::parse(args.next().ok_or("--format needs a name")?)?,
                "--stdin-filepath" => parsed.stdin_path = Some(args.next().ok_or("--stdin-filepath needs a path")?.into()),
                "-" => stdin = true,
                flag if flag.starts_with('-') => return Err(format!("unknown option {flag}")),
                path => parsed.paths.push(path.into()),
            }
        }
        if parsed.format != Format::Text && (!parsed.check || parsed.format != Format::Github) {
            return Err("format takes --format github only with --check".into());
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

/// Folder names a walk skips outside `app/views`.
const SKIPPED_DIRS: &[&str] = &["node_modules", "vendor", "tmp", "public", "log"];

/// Template files under `paths`, following symlinked folders as the server does.
fn template_files(paths: &[PathBuf]) -> Vec<File> {
    let mut out = Vec::new();
    for path in paths {
        if !path.is_dir() {
            out.push(File { path: path.clone(), named: true });
            continue;
        }
        let walk = walkdir::WalkDir::new(path).follow_links(true).sort_by_file_name().into_iter();
        let absolute = util::canonicalize(path).unwrap_or_else(|_| path.clone());
        let entries = walk.filter_entry(|e| e.depth() == 0 || !is_skipped_dir(e, path, &absolute)).filter_map(Result::ok);
        let templates = entries.filter(|e| e.file_type().is_file() && language(e.path()).is_some());
        out.extend(templates.map(|e| File { path: e.into_path(), named: false }));
    }
    out
}

/// Folders a walk skips: hidden ones, dependencies and build output, and Rails generator templates (`lib/templates`,
/// ERB that produces templates). Inside `app/views` a folder may have any of these names (`public/`).
fn is_skipped_dir(entry: &walkdir::DirEntry, root: &Path, absolute_root: &Path) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    if name.starts_with('.') {
        return true;
    }
    let path = entry.path();
    if name == "templates" && path.parent().and_then(Path::file_name).is_some_and(|parent| parent == "lib") {
        return true;
    }
    let inside_views = path.strip_prefix(root).is_ok_and(|below| absolute_root.join(below).ancestors().any(paths::is_app_views));
    !inside_views && SKIPPED_DIRS.contains(&name.as_ref())
}

fn absolute_file_uri(path: &Path) -> Option<Uri> {
    let absolute = crate::util::canonicalize(path).or_else(|_| std::path::absolute(path)).ok()?;
    crate::util::file_uri(absolute)
}

fn language(path: &Path) -> Option<TemplateLang> {
    TemplateLang::from_file_name(&path.file_name()?.to_string_lossy())
}

/// How `check` prints: the format, and the working folder paths are shown from.
#[derive(Clone, Copy)]
struct Output<'a> {
    format: Format,
    cwd: Option<&'a Path>,
}

/// The file a diagnostic is in: `shown` is the path the text format prints, `path` the real one, `text` its content.
struct Source<'a> {
    shown: &'a Path,
    path: &'a Path,
    text: &'a str,
}

/// Prints diagnostics as `file:line:col: severity: message`; 1 when one is a warning or an error, 2
/// for a file that can't be read or isn't a template.
fn check(paths: &[PathBuf], format: Format) -> i32 {
    let files = template_files(paths);
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get()).min(files.len()).max(1);
    let next = AtomicUsize::new(0);
    let view_roots = Arc::new(view_roots::Cache::default());
    let cwd = crate::util::canonicalize(Path::new(".")).ok();
    let output = Output { format, cwd: cwd.as_deref() };
    let (sender, results) = mpsc::channel();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (sender, next, files, view_roots) = (sender.clone(), &next, &files, &view_roots);
            let work = move || {
                let (mut projects, mut configs) = (Projects::new(), Configs::new());
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(index) else { break };
                    // The printer hung up (stdout closed): nothing left to do.
                    let checked = check_file(&file.path, output, (&mut projects, &mut configs), view_roots);
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
        print_in_order(results, format)
    })
}

/// What was printed for some diagnostics, and for a format that prints them all at the end, the diagnostics.
#[derive(Default)]
struct Rendered {
    stdout: String,
    findings: Vec<Finding>,
}

impl Rendered {
    fn add(&mut self, output: Output, source: &Source, (severity, rule, message): (Severity, Option<&str>, &str), range: Range) {
        if output.format == Format::Text {
            let name = match severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Note => "info",
            };
            let (line, col) = (range.start.line + 1, range.start.character + 1);
            let _ = writeln!(self.stdout, "{}:{line}:{col}: {name}: {message}", source.shown.display());
            return;
        }
        let relative = relative_path(source.path, output.cwd);
        let uri = absolute_file_uri(source.path).map_or_else(|| util::slash_path(source.path), |uri| uri.as_str().to_string());
        let finding = Finding::new(&relative, &uri, source.text, severity, rule, message, range);
        match output.format {
            Format::Github => self.stdout.push_str(&report::github_line(&finding)),
            Format::Json => self.stdout.push_str(&report::json_line(&finding)),
            _ => return self.findings.push(finding),
        }
        self.stdout.push('\n');
    }
}

struct Checked {
    rendered: Rendered,
    /// The problems of each `.ruby-templates.toml` the file uses; the printer shows a file's once, with its first template.
    config: Vec<(PathBuf, Rendered)>,
    stderr: String,
    code: i32,
}

impl Checked {
    fn new(code: i32) -> Self {
        Checked { rendered: Rendered::default(), config: Vec::new(), stderr: String::new(), code }
    }
}

fn render_config_problems(file: &ConfigFile, output: Output) -> Rendered {
    // Only the formats that show columns need the file's text.
    let text = if output.format == Format::Text { String::new() } else { std::fs::read_to_string(&file.path).unwrap_or_default() };
    let shown = shown_path(&file.path, output.cwd);
    let source = Source { shown, path: &file.path, text: &text };
    let mut rendered = Rendered::default();
    for problem in &file.problems {
        rendered.add(output, &source, (Severity::Warning, Some(report::CONFIG_PROBLEM), &problem.message), problem.range);
    }
    rendered
}

fn check_file(
    path: &Path,
    output: Output,
    (projects, configs): (&mut Projects, &mut Configs),
    view_roots: &Arc<view_roots::Cache>,
) -> Checked {
    let shown = path.display();
    let failed = |stderr| Checked { stderr, ..Checked::new(USAGE) };
    let Ok(text) = std::fs::read_to_string(path) else { return failed(format!("cannot read {shown}\n")) };
    let Some((uri, lang)) = absolute_file_uri(path).zip(language(path)) else { return failed(format!("{shown}: unknown template type\n")) };
    // Every worker's projects of one app share its view roots.
    Project::memoized(projects, path, Some(view_roots));
    let mut checked = Checked::new(OK);
    let rules = resolve(&uri, lang, &Options::default(), configs);
    for file in rules.files().iter().filter(|file| !file.problems.is_empty()) {
        let rendered = render_config_problems(file, output);
        checked.config.push((file.path.clone(), rendered));
        checked.code = FINDINGS;
    }
    let doc = Doc { lang, text, plain_erb: false, percent_lines: false, version: 0 };
    let source = Source { shown: path, path, text: &doc.text };
    for d in diagnostics_in(&uri, &doc, &rules, projects, &[]) {
        let severity = d.severity.unwrap_or(DiagnosticSeverity::ERROR);
        checked.rendered.add(output, &source, (Severity::of(severity), util::code(&d), &d.message), d.range);
        // Lower numbers are more severe.
        if severity <= DiagnosticSeverity::WARNING {
            checked.code = FINDINGS;
        }
    }
    checked
}

/// `path` from the working folder `cwd` when it is below it, as the template paths are given.
fn shown_path<'a>(path: &'a Path, cwd: Option<&Path>) -> &'a Path {
    cwd.and_then(|cwd| path.strip_prefix(cwd).ok()).unwrap_or(path)
}

/// `path` from the working folder `cwd` however either is spelled (`./`, symlinks); outside it, the path as it is.
fn relative_path(path: &Path, cwd: Option<&Path>) -> PathBuf {
    let real = crate::util::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let from_cwd = cwd.and_then(|cwd| real.strip_prefix(cwd).ok()).unwrap_or(&real);
    from_cwd.components().filter(|part| *part != Component::CurDir).collect()
}

fn print_problems(file: &ConfigFile, cwd: Option<&Path>, out: &mut String) {
    for problem in &file.problems {
        let (line, col) = (problem.range.start.line + 1, problem.range.start.character + 1);
        let _ = writeln!(out, "{}:{line}:{col}: warning: {}", shown_path(&file.path, cwd).display(), problem.message);
    }
}

/// Prints each file's result in walk order as it arrives; the worst exit code, `USAGE` once stdout is closed.
fn print_in_order(results: mpsc::Receiver<(usize, Checked)>, format: Format) -> i32 {
    let mut stdout = BufWriter::new(std::io::stdout().lock());
    let mut waiting = BTreeMap::new();
    let mut next = 0;
    let mut code = OK;
    let mut findings = Vec::new();
    let mut reported = HashSet::new();
    for (index, checked) in results {
        waiting.insert(index, checked);
        while let Some(checked) = waiting.remove(&next) {
            next += 1;
            code = code.max(checked.code);
            let new_config = checked.config.into_iter().filter(|(path, _)| reported.insert(path.clone()));
            let shown: Vec<Rendered> = new_config.map(|(_, rendered)| rendered).chain([checked.rendered]).collect();
            let text: String = shown.iter().map(|rendered| rendered.stdout.as_str()).collect();
            findings.extend(shown.into_iter().flat_map(|rendered| rendered.findings));
            let printed = stdout.write_all(text.as_bytes()).and_then(|()| {
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
    if format == Format::Sarif && writeln!(stdout, "{}", report::sarif(&findings)).is_err() {
        return USAGE;
    }
    if stdout.flush().is_err() {
        return USAGE;
    }
    code
}

fn format_command(args: &FormatArgs) -> i32 {
    let cwd = crate::util::canonicalize(Path::new(".")).ok();
    let mut reported = HashSet::new();
    if let Some(path) = &args.stdin_path {
        let mut text = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut text) {
            eprintln!("cannot read stdin: {e}");
            return USAGE;
        }
        return format_text(path, true, &text, args, cwd.as_deref(), (&mut Projects::new(), &mut Configs::new(), &mut reported));
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
        code = code.max(format_text(&path, named, &text, args, cwd.as_deref(), (&mut projects, &mut configs, &mut reported)));
    }
    code
}

/// The settings in force for `shown`, what was read to get them, and the config files' problems.
fn print_verbose(shown: &str, settings: &format::Settings, notes: &[String], (rules, cwd): (&ResolvedRules, Option<&Path>)) {
    eprintln!("{shown}: {settings:?}");
    notes.iter().for_each(|note| eprintln!("{shown}: {note}"));
    for file in rules.files() {
        eprintln!("{shown}: read {}", shown_path(&file.path, cwd).display());
        let mut problems = String::new();
        print_problems(file, cwd, &mut problems);
        eprint!("{problems}");
    }
}

/// Config problems go to stderr, as stdout carries the formatted text; `--check --format github` annotates them.
fn report_config_problems(file: &ConfigFile, args: &FormatArgs, cwd: Option<&Path>) {
    if args.check && args.format == Format::Github {
        print!("{}", render_config_problems(file, Output { format: Format::Github, cwd }).stdout);
        return;
    }
    let mut problems = String::new();
    print_problems(file, cwd, &mut problems);
    eprint!("{problems}");
}

fn format_text(
    path: &Path,
    named: bool,
    text: &str,
    args: &FormatArgs,
    cwd: Option<&Path>,
    (projects, configs, reported): (&mut Projects, &mut Configs, &mut HashSet<PathBuf>),
) -> i32 {
    let shown = path.display();
    let mut has_config_problems = false;
    let result = match (absolute_file_uri(path), language(path)) {
        (Some(uri), Some(lang)) => {
            let rules = resolve(&uri, lang, &Options::default(), configs);
            let resolved = config::settings_for(&util::file_path(&uri).unwrap_or_else(|| path.to_path_buf()));
            let mut settings = rules.format_settings(resolved.settings);
            settings.align_comments |= args.align_comments;
            if args.verbose {
                print_verbose(&shown.to_string(), &settings, &resolved.notes, (&rules, cwd));
            }
            for file in rules.files().iter().filter(|file| !file.problems.is_empty()) {
                has_config_problems = true;
                if !args.verbose && reported.insert(file.path.clone()) {
                    report_config_problems(file, args, cwd);
                }
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
        if changed && args.format == Format::Github {
            println!("{}", report::github_unformatted(&relative_path(path, cwd)));
        } else if changed {
            println!("{shown}");
        }
        let invalid_config = if has_config_problems { FINDINGS } else { OK };
        return code.max(if changed { FINDINGS } else { invalid_config });
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
    fn output_format_arguments() {
        let parsed = CheckArgs::parse(&args(&["--format", "sarif", "app/views"])).unwrap();
        assert_eq!(parsed, CheckArgs { format: Format::Sarif, paths: vec!["app/views".into()] });
        assert_eq!(CheckArgs::parse(&args(&["a.erb"])).unwrap().format, Format::Text);
        for bad in [&["--format", "xml", "a"][..], &["--format"], &["--format", "json"], &["--wat", "a"]] {
            assert!(CheckArgs::parse(&args(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(FormatArgs::parse(&args(&["--check", "--format", "github", "a"])).unwrap().format, Format::Github);
        for bad in [&["--format", "github", "a"][..], &["--check", "--format", "json", "a"], &["--write", "--format", "github", "a"]] {
            assert!(FormatArgs::parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn paths_are_shown_from_the_real_working_folder() {
        let root = crate::util::canonicalize(&std::env::temp_dir()).unwrap().join(format!("rt-cli-relative-{}", std::process::id()));
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(root.join("app/a.erb"), "").unwrap();
        let cwd = Some(root.as_path());
        assert_eq!(relative_path(&root.join("app/a.erb"), cwd), Path::new("app/a.erb"));
        assert_eq!(relative_path(&root.join("app/./a.erb"), cwd), Path::new("app/a.erb"));
        assert_eq!(relative_path(Path::new("./app/gone.erb"), cwd), Path::new("app/gone.erb"));
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(root.join("app"), &link).unwrap();
            assert_eq!(relative_path(&link.join("a.erb"), cwd), Path::new("app/a.erb"));
        }
        let above = root.parent().unwrap().join("elsewhere.toml");
        assert_eq!(relative_path(&above, cwd), above);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn walks_folders_for_templates_only() {
        let root = std::env::temp_dir().join(format!("rt-cli-walk-{}", std::process::id()));
        let files = [
            "app/views/users/show.html.erb",
            "app/views/users/a.rb",
            "app/views/public/home.html.erb",
            "lib/other/templates/y.html.erb",
            "node_modules/x/b.html.erb",
            ".git/c.slim",
            "lib/templates/slim/scaffold/_form.html.slim",
            "vendor/bundle/gems/x/app/views/d.html.erb",
            "tmp/cache/e.html.erb",
            "public/f.html.erb",
            "log/g.html.erb",
        ];
        for file in files {
            std::fs::create_dir_all(root.join(file).parent().unwrap()).unwrap();
            std::fs::write(root.join(file), "").unwrap();
        }
        let found: Vec<PathBuf> = template_files(std::slice::from_ref(&root)).into_iter().map(|f| f.path).collect();
        std::fs::remove_dir_all(&root).unwrap();
        let kept = ["app/views/public/home.html.erb", "app/views/users/show.html.erb", "lib/other/templates/y.html.erb"];
        assert_eq!(found, kept.map(|file| root.join(file)));
    }
}
