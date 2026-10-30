use crate::util::LineIndex;
use tower_lsp::lsp_types::Range;

pub(crate) struct RubyError {
    /// Prism's text, verbatim.
    pub message: String,
    pub range: Range,
    /// Byte offsets into the parsed source.
    pub start: usize,
    pub end: usize,
}

pub(crate) fn syntax_errors(source: &str, version: ruby_prism::Version) -> Vec<RubyError> {
    errors_and_def_parameters(source, version).0
}

/// The errors of one parse of `source` and its first `def`'s parameters.
pub(crate) fn errors_and_def_parameters(source: &str, version: ruby_prism::Version) -> (Vec<RubyError>, DefParameters) {
    let options = ruby_prism::Options::default().version(version).build();
    let result = ruby_prism::parse_with_options(source.as_bytes(), &options);
    let index = LineIndex::new(source);
    let errors = result
        .errors()
        .map(|error| {
            let location = error.location();
            let range = Range::new(index.position(location.start() as usize), index.position(location.end() as usize));
            RubyError {
                message: String::from_utf8_lossy(error.message()).into_owned(),
                range,
                start: location.start() as usize,
                end: location.end() as usize,
            }
        })
        .collect();
    (errors, def_positional_parameters(&result))
}

/// The positional parameters of a `def`, in Ruby's order, and its block parameter apart.
pub(crate) type DefParameters = (Vec<String>, Option<String>);

/// The non-keyword parameters of the first `def`, named as Ruby's `Method#parameters` does (`*` and `&` when
/// anonymous, empty for a destructured one).
fn def_positional_parameters(result: &ruby_prism::ParseResult) -> DefParameters {
    let parameters = result
        .node()
        .as_program_node()
        .and_then(|program| program.statements().body().iter().next())
        .and_then(|statement| statement.as_def_node())
        .and_then(|def| def.parameters());
    let Some(parameters) = parameters else { return (vec![], None) };
    let positional = parameters
        .requireds()
        .iter()
        .chain(parameters.optionals().iter())
        .chain(parameters.rest())
        .chain(parameters.posts().iter())
        .map(|node| parameter_name(&node, "*"))
        .collect();
    (positional, parameters.block().map(|node| parameter_name(&node, "&")))
}

fn parameter_name(node: &ruby_prism::Node, anonymous: &str) -> String {
    let name = node
        .as_required_parameter_node()
        .map(|n| n.name())
        .or_else(|| node.as_optional_parameter_node().map(|n| n.name()))
        .or_else(|| node.as_rest_parameter_node().and_then(|n| n.name()))
        .or_else(|| node.as_block_parameter_node().and_then(|n| n.name()));
    match name {
        Some(name) => String::from_utf8_lossy(name.as_slice()).into_owned(),
        None if node.as_multi_target_node().is_some() => String::new(),
        None => anonymous.to_string(),
    }
}

/// The local variables of the blocks and lambdas around `offset`, outermost first. Names starting with `_` are left out:
/// numbered parameters, unused ones, and the variables Slim and Haml generate.
pub(crate) fn block_locals(source: &str, offset: usize) -> Vec<String> {
    let result = ruby_prism::parse(source.as_bytes());
    let mut scopes = Scopes { offset, names: Vec::new() };
    ruby_prism::Visit::visit(&mut scopes, &result.node());
    scopes.names
}

struct Scopes {
    offset: usize,
    names: Vec<String>,
}

impl Scopes {
    fn holds(&self, location: &ruby_prism::Location) -> bool {
        (location.start() as usize) < self.offset && self.offset <= location.end() as usize
    }

    fn add(&mut self, locals: &ruby_prism::ConstantList) {
        for local in locals {
            let name = String::from_utf8_lossy(local.as_slice()).into_owned();
            if !name.starts_with('_') && !self.names.contains(&name) {
                self.names.push(name);
            }
        }
    }
}

impl<'pr> ruby_prism::Visit<'pr> for Scopes {
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        if self.holds(&node.location()) {
            self.add(&node.locals());
            ruby_prism::visit_block_node(self, node);
        }
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        if self.holds(&node.location()) {
            self.add(&node.locals());
            ruby_prism::visit_lambda_node(self, node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(source: &str) -> Vec<(String, u32, u32, u32, u32)> {
        syntax_errors(source, ruby_prism::Version::Latest)
            .into_iter()
            .map(|e| (e.message, e.range.start.line, e.range.start.character, e.range.end.line, e.range.end.character))
            .collect()
    }

    #[test]
    fn valid_ruby_has_no_errors() {
        assert!(summary("x = [1, 2].map { |n| n * 2 }\n").is_empty());
    }

    // Messages and positions as the prism 2.0.0 gem reports them (0-based line here, 1-based there).
    #[test]
    fn erb_conditional_without_end() {
        let errors = summary("@output_buffer << ( if x ).to_s; @output_buffer << 'y'.to_s; end");
        assert_eq!(errors[0], ("expected `then` or `;` or '\\n'".to_string(), 0, 25, 0, 26));
        assert!(errors.contains(&("expected an `end` to close the conditional clause".to_string(), 0, 20, 0, 22)));
    }

    #[test]
    fn column_is_utf16_and_line_is_zero_based() {
        // "é" is 2 bytes and 1 unit, the emoji 4 bytes and 2 units.
        let errors = summary("a = \"é😀\"; foo(\nx = 1 +");
        let (message, line, column, end_line, end_column) = &errors[0];
        assert_eq!(message, "unexpected end-of-input; expected an expression after the operator");
        assert_eq!((*line, *column, *end_line, *end_column), (1, 7, 1, 7));

        let errors = summary("a = \"é😀\" +");
        assert_eq!((errors[0].1, errors[0].2), (0, 11));
    }

    #[test]
    fn block_locals_are_those_of_the_blocks_around_the_offset() {
        let source = "users.each do |user, (a, b)|\n  x = 1\n  user.posts.map { |post| post }\n  HERE\nend\nlater { |gone| }\n";
        let here = source.find("HERE").unwrap();
        assert_eq!(block_locals(source, here), ["user", "a", "b", "x"]);
        let inner = source.find("post }").unwrap();
        assert_eq!(block_locals(source, inner), ["user", "a", "b", "x", "post"]);
        assert!(block_locals(source, 0).is_empty());
        assert_eq!(block_locals("f = ->(n) { n }", 12), ["n"]);
        assert!(block_locals("[1].map { _1 }", 11).is_empty());
        assert!(block_locals("x.each { |_unused, _slim_controls1| }", 20).is_empty());
    }

    #[test]
    fn block_locals_without_the_end_typed_yet() {
        assert_eq!(block_locals("users.each do |user|\n  us", 25), ["user"]);
    }
}
