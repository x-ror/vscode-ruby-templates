//! The machine-readable output of `ruby-templates check`: GitHub workflow commands, JSON lines and SARIF.

use crate::rules;
use crate::util;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use tower_lsp_server::ls_types::{DiagnosticSeverity, Position, Range};

/// The rule id of a problem in a `.ruby-templates.toml`, which no diagnostic code covers.
pub const CONFIG_PROBLEM: &str = "Config/Invalid";
const BYTE_ORDER_MARK: char = '\u{feff}';
const SITE: &str = "https://x-ror.github.io/vscode-ruby-templates/";
const SARIF_SCHEMA: &str = "https://json.schemastore.org/sarif-2.1.0.json";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    #[default]
    Text,
    Github,
    Sarif,
    Json,
}

impl Format {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "text" => Ok(Self::Text),
            "github" => Ok(Self::Github),
            "sarif" => Ok(Self::Sarif),
            "json" => Ok(Self::Json),
            _ => Err(format!("unknown format {name}; expected text, github, sarif or json")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
}

impl Severity {
    pub fn of(severity: DiagnosticSeverity) -> Self {
        match severity {
            DiagnosticSeverity::ERROR => Self::Error,
            DiagnosticSeverity::WARNING => Self::Warning,
            _ => Self::Note,
        }
    }

    /// The SARIF `level`, also used by the JSON lines.
    fn level(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }

    fn workflow_command(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "notice",
        }
    }
}

/// A diagnostic as these formats show it: the path from the working folder with forward slashes (`uri` is the absolute
/// `file:` URI for SARIF), and 1-based positions whose columns count Unicode characters, not the UTF-16 units of the LSP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: String,
    pub uri: String,
    pub severity: Severity,
    pub rule: Option<String>,
    pub message: String,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

impl Finding {
    pub fn new(path: &Path, uri: &str, text: &str, severity: Severity, rule: Option<&str>, message: &str, range: Range) -> Self {
        let ((line, column), (end_line, end_column)) = (character_position(text, range.start), character_position(text, range.end));
        Finding {
            path: util::slash_path(path),
            uri: uri.to_string(),
            severity,
            rule: rule.map(str::to_string),
            message: message.to_string(),
            line,
            column,
            end_line,
            end_column,
        }
    }
}

/// The 1-based line and character column of an LSP position in `text`; a leading byte order mark takes no column.
fn character_position(text: &str, position: Position) -> (u32, u32) {
    let (_, line, byte) = util::line_at(text, position);
    let column = line[..byte].chars().count() as u32 + 1;
    let marked = position.line == 0 && text.starts_with(BYTE_ORDER_MARK);
    (position.line + 1, if marked { column.saturating_sub(1).max(1) } else { column })
}

/// Escapes a workflow command's message, or its property value when `property` (which also holds `:` and `,`).
fn escape(value: &str, property: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\r' => out.push_str("%0D"),
            '\n' => out.push_str("%0A"),
            ':' if property => out.push_str("%3A"),
            ',' if property => out.push_str("%2C"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn github_line(finding: &Finding) -> String {
    let Finding { path, severity, rule, message, line, column, end_line, end_column, .. } = finding;
    let mut properties = format!("file={},line={line},col={column},endLine={end_line},endColumn={end_column}", escape(path, true));
    if let Some(rule) = rule {
        properties.push_str(&format!(",title={}", escape(rule, true)));
    }
    format!("::{} {properties}::{}", severity.workflow_command(), escape(message, false))
}

/// The annotation for a file `format --check` would change.
pub fn github_unformatted(path: &Path) -> String {
    let message = "needs formatting; run `ruby-templates format --write`";
    format!("::warning file={},title=Format::{}", escape(&util::slash_path(path), true), escape(message, false))
}

pub fn json_line(finding: &Finding) -> String {
    json!({
        "file": finding.path,
        "line": finding.line,
        "column": finding.column,
        "endLine": finding.end_line,
        "endColumn": finding.end_column,
        "severity": finding.severity.level(),
        "rule": finding.rule,
        "message": finding.message,
    })
    .to_string()
}

/// A SARIF 2.1.0 log with one run; its rules are the ones the findings name.
pub fn sarif(findings: &[Finding]) -> String {
    let rule_ids: BTreeSet<&str> = findings.iter().filter_map(|f| f.rule.as_deref()).collect();
    let descriptors: Vec<Value> = rule_ids.into_iter().map(rule_descriptor).collect();
    let results: Vec<Value> = findings.iter().map(sarif_result).collect();
    let log = json!({
        "$schema": SARIF_SCHEMA,
        "version": "2.1.0",
        "runs": [{
            "columnKind": "unicodeCodePoints",
            "tool": { "driver": {
                "name": "ruby-templates",
                "version": env!("CARGO_PKG_VERSION"),
                "informationUri": SITE,
                "rules": descriptors,
            } },
            "results": results,
        }],
    });
    serde_json::to_string_pretty(&log).unwrap_or_default()
}

fn rule_descriptor(id: &str) -> Value {
    let mut descriptor = json!({ "id": id });
    if id == CONFIG_PROBLEM {
        descriptor["shortDescription"] = json!({ "text": "A problem in a .ruby-templates.toml." });
        descriptor["helpUri"] = json!(format!("{SITE}reference/rules/#configuration"));
    } else if let Some(rule) = rules::by_name(id) {
        let anchor = rule.name.to_lowercase().replace('/', "-");
        descriptor["shortDescription"] = json!({ "text": rule.summary });
        descriptor["fullDescription"] = json!({ "text": rule.summary });
        descriptor["helpUri"] = json!(format!("{SITE}reference/diagnostics/#{anchor}"));
    }
    descriptor
}

fn sarif_result(finding: &Finding) -> Value {
    let mut result = json!({
        "level": finding.severity.level(),
        "message": { "text": finding.message },
        "locations": [{ "physicalLocation": {
            "artifactLocation": { "uri": finding.uri },
            "region": {
                "startLine": finding.line,
                "startColumn": finding.column,
                "endLine": finding.end_line,
                "endColumn": finding.end_column,
            },
        } }],
    });
    if let Some(rule) = &finding.rule {
        result["ruleId"] = json!(rule);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(rule: Option<&str>, message: &str) -> Finding {
        Finding {
            path: "app/views/a b.erb".into(),
            uri: "file:///work/app/views/a%20b.erb".into(),
            severity: Severity::Warning,
            rule: rule.map(str::to_string),
            message: message.into(),
            line: 12,
            column: 5,
            end_line: 12,
            end_column: 18,
        }
    }

    #[test]
    fn escapes_data_and_properties_as_github_does() {
        assert_eq!(escape("50%\r\nnext: `a`, b", false), "50%25%0D%0Anext: `a`, b");
        assert_eq!(escape("a:b,c%\n", true), "a%3Ab%2Cc%25%0A");
    }

    #[test]
    fn github_line_escapes_path_title_and_message() {
        let line = github_line(&finding(Some("Rails/SilentRender"), "`render` needs: a, b"));
        assert_eq!(
            line,
            "::warning file=app/views/a b.erb,line=12,col=5,endLine=12,endColumn=18,title=Rails/SilentRender::`render` needs: a, b"
        );
        let colon = Finding { path: "a:b,c.erb".into(), ..finding(None, "x") };
        assert_eq!(github_line(&colon), "::warning file=a%3Ab%2Cc.erb,line=12,col=5,endLine=12,endColumn=18::x");
        assert!(github_line(&Finding { severity: Severity::Note, ..finding(None, "x") }).starts_with("::notice "));
    }

    #[test]
    fn columns_count_characters_not_utf16_units() {
        // The emoji is two UTF-16 units and one character; `д` is one of each.
        let text = "<p>😀д<img></p>\n";
        let start = Position::new(0, 3 + 2 + 1);
        let (line, column) = character_position(text, start);
        assert_eq!((line, column), (1, 3 + 1 + 1 + 1));
        let range = Range::new(start, Position::new(0, 3 + 2 + 1 + 5));
        let found = Finding::new(Path::new("a.erb"), "file:///a.erb", text, Severity::Error, None, "m", range);
        assert_eq!((found.column, found.end_column), (6, 11));
    }

    #[test]
    fn a_byte_order_mark_takes_no_column() {
        let text = "\u{feff}<img>\n<b>";
        assert_eq!(character_position(text, Position::new(0, 1)), (1, 1));
        assert_eq!(character_position(text, Position::new(0, 2)), (1, 2));
        assert_eq!(character_position(text, Position::new(1, 1)), (2, 2));
    }

    #[test]
    fn column_past_the_line_end_clamps() {
        assert_eq!(character_position("ab\n", Position::new(0, 9)), (1, 3));
        assert_eq!(character_position("ab\n", Position::new(5, 0)), (6, 1));
    }

    #[test]
    fn sarif_describes_the_rules_found() {
        let found = [finding(Some("Rails/SilentRender"), "m"), finding(Some("Syntax/Ruby"), "s"), finding(Some(CONFIG_PROBLEM), "c")];
        let log: Value = serde_json::from_str(&sarif(&found)).unwrap();
        let run = &log["runs"][0];
        assert_eq!(log["version"], "2.1.0");
        assert_eq!(run["columnKind"], "unicodeCodePoints");
        let rules = run["tool"]["driver"]["rules"].as_array().unwrap();
        let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["Config/Invalid", "Rails/SilentRender", "Syntax/Ruby"]);
        assert_eq!(rules[0]["helpUri"], format!("{SITE}reference/rules/#configuration"));
        assert_eq!(rules[1]["helpUri"], format!("{SITE}reference/diagnostics/#rails-silentrender"));
        assert_eq!(rules[2]["helpUri"], format!("{SITE}reference/diagnostics/#syntax-ruby"));
        let summary = rules::by_name("Syntax/Ruby").unwrap().summary;
        assert_eq!(rules[2]["shortDescription"]["text"], summary);
        assert_eq!(rules[2]["fullDescription"]["text"], summary);
        assert_eq!(run["results"][0]["ruleId"], "Rails/SilentRender");
        let location = &run["results"][0]["locations"][0]["physicalLocation"];
        assert_eq!(location["artifactLocation"]["uri"], "file:///work/app/views/a%20b.erb");
    }

    #[test]
    fn json_line_has_every_field() {
        let value: Value = serde_json::from_str(&json_line(&finding(None, "m"))).unwrap();
        assert_eq!(value["rule"], Value::Null);
        assert_eq!((value["line"].as_u64(), value["endColumn"].as_u64()), (Some(12), Some(18)));
        assert_eq!(value["severity"], "warning");
    }
}
