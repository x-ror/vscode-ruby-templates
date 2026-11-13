//! Signature help: the calls the cursor is inside, and the parameter of a signature it is at.

use crate::ruby;
use tower_lsp_server::ls_types::*;

/// A call the cursor is in: its name, how many arguments precede the cursor's, and that argument's `key:` if it has one.
#[derive(Debug, PartialEq)]
pub struct Call {
    pub name: String,
    pub index: usize,
    pub keyword: Option<String>,
}

enum Kind {
    Call(String),
    /// Parentheses, brackets and hash braces that aren't a call's arguments.
    Group,
    /// A `do` or `{ |x| }` block, which starts new statements: the calls around it don't take what is inside.
    Block,
}

struct Frame {
    kind: Kind,
    /// The bracket that ends it; `None` for a call without parentheses and for a `do` block.
    close: Option<char>,
    commas: usize,
    /// Where the current argument starts.
    arg_start: usize,
}

impl Frame {
    fn is_bare_call(&self) -> bool {
        matches!(self.kind, Kind::Call(_)) && self.close.is_none()
    }
}

/// The calls `code`, the Ruby before the cursor, is inside, innermost first. Partial input is fine: the code is
/// scanned for brackets, strings and blocks, not parsed.
pub fn calls_before(code: &str) -> Vec<Call> {
    if ruby::code_end(code) != ruby::CodeEnd::Code {
        return Vec::new();
    }
    let bytes = code.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut last = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let start = i;
        i += 1;
        match byte {
            b'"' | b'\'' | b'`' => i = string_end(bytes, start),
            b'#' => i = code[start..].find('\n').map_or(bytes.len(), |end| start + end),
            b'(' | b'[' => push(&mut stack, Kind::Group, Some(closer(byte)), i),
            b'{' if matches!(last, b')' | b'_' | b'?' | b'!') || last.is_ascii_alphanumeric() => {
                push(&mut stack, Kind::Block, Some('}'), i);
                i = after_block_parameters(bytes, i);
            }
            b'{' => push(&mut stack, Kind::Group, Some('}'), i),
            b')' | b']' | b'}' => close(&mut stack, byte as char),
            b',' => {
                if let Some(frame) = stack.last_mut() {
                    frame.commas += 1;
                    frame.arg_start = i;
                }
            }
            b'\n' | b';' if last != b',' && last != b'\\' => {
                while stack.last().is_some_and(Frame::is_bare_call) {
                    stack.pop();
                }
            }
            b if b.is_ascii_alphabetic() || b == b'_' => {
                i = word_end(code, start);
                match &code[start..i] {
                    "do" => {
                        while stack.last().is_some_and(Frame::is_bare_call) {
                            stack.pop();
                        }
                        push(&mut stack, Kind::Block, None, i);
                        i = after_block_parameters(bytes, i);
                    }
                    "end" => {
                        if let Some(block) = stack.iter().rposition(|frame| matches!(frame.kind, Kind::Block)) {
                            stack.truncate(block);
                        }
                    }
                    word => {
                        let receiver = start > 0 && matches!(bytes[start - 1], b'.' | b'@' | b':' | b'$');
                        if !receiver && word.starts_with(|c: char| c.is_ascii_lowercase() || c == '_') && !ruby::KEYWORDS.contains(&word) {
                            if code[i..].starts_with('(') {
                                push(&mut stack, Kind::Call(word.to_string()), Some(')'), i + 1);
                                i += 1;
                            } else if starts_arguments(&code[i..]) {
                                push(&mut stack, Kind::Call(word.to_string()), None, i);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        if let Some(&seen) = bytes.get(i.wrapping_sub(1)).filter(|b| !b.is_ascii_whitespace()) {
            last = seen;
        }
    }
    let mut calls = Vec::new();
    for frame in stack.iter().rev() {
        match &frame.kind {
            Kind::Block => break,
            Kind::Group => {}
            Kind::Call(name) => calls.push(Call { name: name.clone(), index: frame.commas, keyword: keyword_of(&code[frame.arg_start..]) }),
        }
    }
    calls
}

fn push(stack: &mut Vec<Frame>, kind: Kind, close: Option<char>, arg_start: usize) {
    stack.push(Frame { kind, close, commas: 0, arg_start });
}

fn closer(open: u8) -> char {
    if open == b'(' {
        ')'
    } else {
        ']'
    }
}

/// A closing bracket ends the calls without parentheses inside it, then the group it closes.
fn close(stack: &mut Vec<Frame>, bracket: char) {
    while stack.last().is_some_and(Frame::is_bare_call) {
        stack.pop();
    }
    if stack.last().is_some_and(|frame| frame.close == Some(bracket)) {
        stack.pop();
    }
}

/// Just past the string that starts at `open`, interpolations included; the end of `bytes` when it isn't closed.
fn string_end(bytes: &[u8], open: usize) -> usize {
    let quote = bytes[open];
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'#' if quote == b'"' && bytes.get(i + 1) == Some(&b'{') => i = interpolation_end(bytes, i + 2),
            b if b == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Just past the `}` that closes the interpolation whose code starts at `start`.
fn interpolation_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = string_end(bytes, i) - 1,
            b'{' => depth += 1,
            b'}' if depth == 0 => return i + 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// Just past the identifier at `start`, with its `?` or `!`.
fn word_end(code: &str, start: usize) -> usize {
    let rest = &code[start..];
    let len = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(rest.len());
    let after = &rest[len..];
    let suffix = after.starts_with(['?', '!']) && !after[1..].starts_with(['=', '~']);
    start + len + usize::from(suffix)
}

/// Just past the `|a, b|` that a block opens with, from `i` right after `do` or `{`.
fn after_block_parameters(bytes: &[u8], i: usize) -> usize {
    let open = i + bytes[i..].iter().take_while(|b| b.is_ascii_whitespace()).count();
    if bytes.get(open) != Some(&b'|') {
        return i;
    }
    bytes[open + 1..].iter().position(|&b| b == b'|').map_or(bytes.len(), |end| open + end + 2)
}

/// Whether `after`, the code after a name, is what a call without parentheses takes: a space, then an argument or nothing yet.
fn starts_arguments(after: &str) -> bool {
    let argument = after.trim_start_matches([' ', '\t']);
    if argument.len() == after.len() {
        return false;
    }
    let Some(first) = argument.chars().next() else { return true };
    if first.is_ascii_alphabetic() || first == '_' {
        let word = &argument[..argument.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(argument.len())];
        return !ruby::KEYWORDS.contains(&word) || matches!(word, "nil" | "true" | "false" | "self");
    }
    first.is_ascii_digit() || matches!(first, '"' | '\'' | ':' | '@' | '[' | '-' | '~')
}

/// The `key` of an argument that starts with `key:`.
fn keyword_of(argument: &str) -> Option<String> {
    let argument = argument.trim_start();
    let end = argument.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    let after = &argument[end..];
    (end > 0 && after.starts_with(':') && !after.starts_with("::")).then(|| argument[..end].to_string())
}

#[derive(Debug, PartialEq)]
enum ParameterKind {
    Positional,
    Splat,
    Keyword(String),
    KeywordSplat,
    Block,
}

#[derive(Debug, PartialEq)]
struct Parameter {
    kind: ParameterKind,
    /// Byte range in the label.
    span: (usize, usize),
}

/// The parameters in the parentheses that follow the name in `label` (`link_to(name, options = nil)`,
/// `def price(amount, currency: "USD")`); none when the label has no parameter list.
fn parameters(label: &str) -> Vec<Parameter> {
    let Some(open) = label.find('(') else { return Vec::new() };
    if label[..open].trim_start_matches("def ").contains(char::is_whitespace) {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut depth = 0;
    let mut start = open + 1;
    let mut i = open + 1;
    let bytes = label.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = string_end(bytes, i) - 1,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth > 0 => depth -= 1,
            b',' if depth == 0 => {
                found.extend(parameter(label, start, i));
                start = i + 1;
            }
            b')' => {
                found.extend(parameter(label, start, i));
                return found;
            }
            _ => {}
        }
        i += 1;
    }
    found
}

fn parameter(label: &str, start: usize, end: usize) -> Option<Parameter> {
    let text = label[start..end].trim();
    if text.is_empty() {
        return None;
    }
    let kind = match text {
        t if t.starts_with("**") => ParameterKind::KeywordSplat,
        t if t.starts_with('*') => ParameterKind::Splat,
        t if t.starts_with('&') => ParameterKind::Block,
        t => match keyword_of(t) {
            Some(key) => ParameterKind::Keyword(key),
            None => ParameterKind::Positional,
        },
    };
    let begin = start + label[start..end].len() - label[start..end].trim_start().len();
    Some(Parameter { kind, span: (begin, begin + text.len()) })
}

/// The parameter an argument of `call` is: its keyword's, else the positional one, else the `**options` that takes the rest.
fn active_parameter(parameters: &[Parameter], call: &Call) -> Option<usize> {
    let position_of = |wanted: fn(&ParameterKind) -> bool| parameters.iter().position(|p| wanted(&p.kind));
    let by_keyword = call.keyword.as_ref().and_then(|key| parameters.iter().position(|p| p.kind == ParameterKind::Keyword(key.clone())));
    let positional: Vec<usize> =
        (0..parameters.len()).filter(|&i| matches!(parameters[i].kind, ParameterKind::Positional | ParameterKind::Splat)).collect();
    let by_position =
        positional.get(call.index).copied().or_else(|| positional.last().copied().filter(|&i| parameters[i].kind == ParameterKind::Splat));
    by_keyword
        .or_else(|| call.keyword.is_some().then(|| position_of(|k| *k == ParameterKind::KeywordSplat)).flatten())
        .or(by_position)
        .or_else(|| position_of(|k| *k == ParameterKind::KeywordSplat))
        .or_else(|| position_of(|k| matches!(k, ParameterKind::Keyword(_))))
}

/// The signature help for `call` of the method written as `label`; none for a method without parameters.
pub fn signature_help(label: &str, documentation: Option<String>, call: &Call) -> Option<SignatureHelp> {
    let parameters = parameters(label);
    if parameters.is_empty() {
        return None;
    }
    let utf16 = |byte: usize| label[..byte].encode_utf16().count() as u32;
    let active = active_parameter(&parameters, call).unwrap_or(parameters.len()) as u32;
    let infos = parameters
        .iter()
        .map(|p| ParameterInformation { label: ParameterLabel::LabelOffsets([utf16(p.span.0), utf16(p.span.1)]), documentation: None })
        .collect();
    let documentation = documentation.map(|value| Documentation::MarkupContent(MarkupContent { kind: MarkupKind::Markdown, value }));
    let signature =
        SignatureInformation { label: label.to_string(), documentation, parameters: Some(infos), active_parameter: Some(active) };
    Some(SignatureHelp { signatures: vec![signature], active_signature: Some(0), active_parameter: Some(active) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn innermost(code: &str) -> Option<(String, usize, Option<String>)> {
        calls_before(code).into_iter().next().map(|c| (c.name, c.index, c.keyword))
    }

    fn at(name: &str, index: usize) -> Option<(String, usize, Option<String>)> {
        Some((name.to_string(), index, None))
    }

    #[test]
    fn counts_top_level_commas() {
        assert_eq!(innermost("link_to("), at("link_to", 0));
        assert_eq!(innermost("link_to \"Users\", "), at("link_to", 1));
        assert_eq!(innermost("link_to(\"a, b\", 'c, d', "), at("link_to", 2));
        assert_eq!(innermost("link_to \"a #{x(1, 2)}, b\", "), at("link_to", 1));
        assert_eq!(innermost("link_to \"x\", users_path, "), at("link_to", 2));
        assert_eq!(innermost("link_to \"x\", ["), at("link_to", 1), "an array is an argument");
        assert_eq!(innermost("link_to(\"x\", { a: 1, b: [1, 2], "), at("link_to", 1), "commas in a hash");
        assert_eq!(innermost("link_to(\"x\", [1, 2], "), at("link_to", 2));
    }

    #[test]
    fn nested_calls_come_innermost_first() {
        let calls = calls_before("link_to t(\".x\", count: 1), user_path(@user, ");
        assert_eq!(calls.iter().map(|c| (c.name.as_str(), c.index)).collect::<Vec<_>>(), [("user_path", 1), ("link_to", 1)]);
        let calls = calls_before("link_to t(\".x\", ");
        assert_eq!(calls.iter().map(|c| (c.name.as_str(), c.index)).collect::<Vec<_>>(), [("t", 1), ("link_to", 0)]);
        assert_eq!(innermost("link_to t(\".x\", count: 1), "), at("link_to", 1), "the nested call is closed");
        assert_eq!(innermost("link_to user.name, "), at("link_to", 1), "a receiver's method is no call");
    }

    #[test]
    fn blocks_hide_the_calls_around_them() {
        assert_eq!(innermost("form_with model: @user do |f, g|\n  link_to \"x\", "), at("link_to", 1));
        assert!(calls_before("form_with model: @user do |f, ").is_empty());
        assert!(calls_before("form_with model: @user do |f|\n  f.text_field :a, ").is_empty());
        assert!(calls_before("items.each { |a, b| link_to(a, ").iter().any(|c| c.name == "link_to"));
        assert_eq!(innermost("link_to(\"x\") { |a, b| b, "), None);
        assert_eq!(innermost("content_tag(:div) do\n  x\nend\nlink_to "), at("link_to", 0));
        assert_eq!(innermost("link_to(\"x\", {a: 1}, "), at("link_to", 2));
    }

    #[test]
    fn calls_without_parentheses_end_at_the_line() {
        assert_eq!(innermost("link_to \"Users\", "), at("link_to", 1));
        assert_eq!(innermost("link_to "), at("link_to", 0));
        assert_eq!(innermost("link_to \"x\",\n  path, "), at("link_to", 2));
        assert_eq!(innermost("link_to \"x\", path\nrender "), at("render", 0));
        assert_eq!(innermost("x = link_to \"x\", y if a, "), at("link_to", 2));
        assert_eq!(innermost("foo(link_to \"x\", y, "), at("link_to", 2));
        assert_eq!(innermost("foo(link_to \"x\", y), "), None, "foo is closed");
        assert_eq!(innermost("link_to"), None);
        assert_eq!(innermost("link_to = 1"), None);
    }

    #[test]
    fn unfinished_and_unreachable_code_has_no_call() {
        assert!(calls_before("link_to \"Users").is_empty(), "inside a string");
        assert!(calls_before("link_to \"x\", # a, b").is_empty(), "inside a comment");
        assert!(calls_before("").is_empty());
    }

    #[test]
    fn keyword_of_the_current_argument() {
        let key = |code: &str| calls_before(code).pop().and_then(|call| call.keyword);
        assert_eq!(key("turbo_frame_tag \"x\", src: "), Some("src".into()));
        assert_eq!(key("turbo_frame_tag(\"x\", target: foo(1, "), Some("target".into()));
        assert_eq!(key("turbo_frame_tag(\"x\", "), None);
        assert_eq!(key("link_to \"x\", User::"), None);
    }

    fn highlighted(label: &str, code: &str) -> Option<String> {
        let call = calls_before(code).into_iter().next()?;
        let help = signature_help(label, None, &call)?;
        let info = &help.signatures[0];
        let ParameterLabel::LabelOffsets([start, end]) = info.parameters.as_ref()?.get(info.active_parameter? as usize)?.label.clone()
        else {
            return None;
        };
        Some(label[start as usize..end as usize].to_string())
    }

    #[test]
    fn highlights_the_active_parameter() {
        let link_to = "link_to(name = nil, options = nil, html_options = nil, &block)";
        assert_eq!(highlighted(link_to, "link_to("), Some("name = nil".into()));
        assert_eq!(highlighted(link_to, "link_to \"x\", "), Some("options = nil".into()));
        assert_eq!(highlighted(link_to, "link_to \"x\", y, class: "), Some("html_options = nil".into()));
        assert_eq!(highlighted(link_to, "link_to \"x\", y, z, "), None, "past the parameters");
        let frame = "turbo_frame_tag(*ids, src: nil, target: nil, **attributes, &block)";
        assert_eq!(highlighted(frame, "turbo_frame_tag \"a\", "), Some("*ids".into()));
        assert_eq!(highlighted(frame, "turbo_frame_tag \"a\", src: "), Some("src: nil".into()));
        assert_eq!(highlighted(frame, "turbo_frame_tag \"a\", data: "), Some("**attributes".into()));
        let form = "form_with(model: nil, scope: nil, url: nil, format: nil, **options, &block)";
        assert_eq!(highlighted(form, "form_with url: x, "), Some("**options".into()));
        assert_eq!(highlighted(form, "form_with scope: "), Some("scope: nil".into()));
    }

    #[test]
    fn parameters_of_app_helper_definitions() {
        let label = "def label_for(a = \"(,\", b, *rest, key: [1, 2], **opts, &block)";
        let found: Vec<&str> = parameters(label).iter().map(|p| &label[p.span.0..p.span.1]).collect();
        assert_eq!(found, ["a = \"(,\"", "b", "*rest", "key: [1, 2]", "**opts", "&block"]);
        assert!(parameters("def current_user").is_empty());
        assert!(parameters("def total = sum(1)").is_empty());
        assert!(parameters("local_assigns — partial locals hash").is_empty());
        assert!(signature_help("def current_user", None, &Call { name: "x".into(), index: 0, keyword: None }).is_none());
    }

    #[test]
    fn parameter_offsets_count_utf16() {
        let label = "price(note = \"€💶\", amount)";
        let call = Call { name: "price".into(), index: 1, keyword: None };
        let help = signature_help(label, None, &call).unwrap();
        let ParameterLabel::LabelOffsets(offsets) = help.signatures[0].parameters.as_ref().unwrap()[1].label.clone() else { panic!() };
        let units: Vec<u16> = label.encode_utf16().collect();
        assert_eq!(String::from_utf16(&units[offsets[0] as usize..offsets[1] as usize]).unwrap(), "amount");
    }
}
