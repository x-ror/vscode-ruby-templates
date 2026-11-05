//! Semantic tokens for the Ruby locals the TextMate grammars can't tell from other identifiers. Nothing else gets a token,
//! so the grammar's colours stay.

use crate::code_context;
use crate::lang::TemplateLang;
use crate::prism::{LocalKind, LocalToken};
use crate::strict_locals::Close;
use crate::util::LineIndex;
use tower_lsp_server::ls_types::{SemanticToken, SemanticTokenType, SemanticTokensLegend};

const TOKEN_TYPES: [SemanticTokenType; 2] = [SemanticTokenType::PARAMETER, SemanticTokenType::VARIABLE];

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend { token_types: TOKEN_TYPES.to_vec(), token_modifiers: Vec::new() }
}

fn type_index(kind: LocalKind) -> u32 {
    let token_type = match kind {
        LocalKind::Parameter => SemanticTokenType::PARAMETER,
        LocalKind::Variable => SemanticTokenType::VARIABLE,
    };
    TOKEN_TYPES.iter().position(|t| *t == token_type).expect("TOKEN_TYPES holds every local's type") as u32
}

/// Whether the language compiles to Ruby this module reads; others are left to other servers.
pub fn supports(lang: TemplateLang) -> bool {
    !matches!(lang, TemplateLang::Skim | TemplateLang::Ruby | TemplateLang::Liquid)
}

/// The tokens of the template `text`, delta-encoded in document order.
pub fn tokens(lang: TemplateLang, text: &str, close: Close, extra_engines: &[String]) -> Vec<SemanticToken> {
    let mut found = code_context::local_tokens(lang, text, close, extra_engines);
    found.sort_by_key(|t| t.start);
    found.dedup_by_key(|t| t.start);
    encode(text, &found)
}

fn encode(text: &str, tokens: &[LocalToken]) -> Vec<SemanticToken> {
    let index = LineIndex::new(text);
    let (mut line, mut column) = (0, 0);
    let mut out = Vec::with_capacity(tokens.len());
    for token in tokens {
        let Some(name) = text.get(token.start..token.end).filter(|name| !name.contains('\n')) else { continue };
        let at = index.position(token.start);
        let delta_line = at.line - line;
        let delta_start = if delta_line == 0 { at.character - column } else { at.character };
        let length = name.encode_utf16().count() as u32;
        out.push(SemanticToken { delta_line, delta_start, length, token_type: type_index(token.kind), token_modifiers_bitset: 0 });
        (line, column) = (at.line, at.character);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(line, column, length, type)` of each token, as a client decodes them.
    fn decoded(lang: TemplateLang, text: &str) -> Vec<(u32, u32, u32, &'static str)> {
        let (mut line, mut column) = (0, 0);
        tokens(lang, text, Close::default(), &[])
            .into_iter()
            .map(|t| {
                line += t.delta_line;
                column = if t.delta_line == 0 { column + t.delta_start } else { t.delta_start };
                (line, column, t.length, if t.token_type == 0 { "parameter" } else { "variable" })
            })
            .collect()
    }

    #[test]
    fn ruby_and_skim_are_left_alone() {
        assert!(!supports(TemplateLang::Ruby) && !supports(TemplateLang::Skim));
        assert!(supports(TemplateLang::Erb) && supports(TemplateLang::Haml) && supports(TemplateLang::Slim));
    }

    #[test]
    fn erb_wrapper_parameters_get_no_token() {
        let text = "<%= local_assigns[:a] %><%= output_buffer %>\n";
        assert_eq!(decoded(TemplateLang::Erb, text), []);
    }

    #[test]
    fn erb_strict_local_block_variable_and_utf16() {
        let text =
            "<%# locals: (user:) %>\n<p>😀é <%= user.name %></p>\n<% items.each do |item| %>\n  <%= item.title if user %>\n<% end %>\n";
        assert_eq!(
            decoded(TemplateLang::Erb, text),
            [(1, 11, 4, "parameter"), (2, 18, 4, "variable"), (3, 6, 4, "variable"), (3, 20, 4, "parameter"),]
        );
    }

    #[test]
    fn haml_strict_local_and_block_variable() {
        let text = "-# locals: (user:)\n%p= user.name\n- items.each do |item|\n  %li= item.title\n";
        assert_eq!(decoded(TemplateLang::Haml, text), [(1, 4, 4, "parameter"), (2, 17, 4, "variable"), (3, 7, 4, "variable")]);
    }

    #[test]
    fn slim_block_variable_and_no_token_for_calls() {
        let text = "p = helper_call\n- items.each do |item|\n  li = \"é\" + item.title\n";
        assert_eq!(decoded(TemplateLang::Slim, text), [(1, 17, 4, "variable"), (2, 13, 4, "variable")]);
    }
}
