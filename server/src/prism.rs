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

/// Where a template local is read or assigned, and whether a block or lambda has a variable of that name too.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct LocalUses {
    pub spans: Vec<(usize, usize)>,
    pub shadowed: bool,
}

/// Where `source` reads or assigns the template local `name`: a variable of the outermost scope (ERB and Haml compile
/// into a method whose parameters are the strict locals), or a bare call, which is what a local is to Ruby where nothing
/// declares it (Slim compiles into no method). A block variable of the same name hides it.
pub(crate) fn local_uses(source: &str, name: &str) -> LocalUses {
    let result = ruby_prism::parse(source.as_bytes());
    let mut uses = Uses { name: name.as_bytes(), blocks: 0, found: LocalUses::default() };
    ruby_prism::Visit::visit(&mut uses, &result.node());
    uses.found
}

/// Where the keyword parameter `name` starts in `parameters`, a method's parameter list.
pub(crate) fn keyword_parameter_at(parameters: &str, name: &str) -> Option<usize> {
    let source = format!("{PARAMETERS_DEF}{parameters}\n); end");
    let result = ruby_prism::parse(source.as_bytes());
    let def = result.node().as_program_node()?.statements().body().iter().next()?.as_def_node()?;
    let keywords = def.parameters()?.keywords();
    let at = keywords.iter().find_map(|node| {
        let (found, at) = match (node.as_required_keyword_parameter_node(), node.as_optional_keyword_parameter_node()) {
            (Some(required), _) => (required.name(), required.name_loc()),
            (_, Some(optional)) => (optional.name(), optional.name_loc()),
            _ => return None,
        };
        (found.as_slice() == name.as_bytes()).then(|| at.start() as usize)
    })?;
    Some(at - PARAMETERS_DEF.len())
}

/// Where `parameters`, a method's parameter list, reads the parameter `name` in the defaults of the others.
pub(crate) fn parameter_reads(parameters: &str, name: &str) -> Vec<(usize, usize)> {
    let source = format!("{PARAMETERS_DEF}{parameters}\n); end");
    let end = PARAMETERS_DEF.len() + parameters.len();
    local_uses(&source, name)
        .spans
        .into_iter()
        .filter(|&(start, _)| (PARAMETERS_DEF.len()..end).contains(&start))
        .map(|(start, finish)| (start - PARAMETERS_DEF.len(), finish - PARAMETERS_DEF.len()))
        .collect()
}

const PARAMETERS_DEF: &str = "def _(";

struct Uses<'a> {
    name: &'a [u8],
    /// How many blocks and lambdas the visitor is in, which a read of the outermost scope's variable reaches up through.
    blocks: u32,
    found: LocalUses,
}

impl Uses<'_> {
    fn local(&mut self, name: ruby_prism::ConstantId, depth: u32, at: ruby_prism::Location) {
        if name.as_slice() == self.name && depth == self.blocks {
            self.found.spans.push((at.start() as usize, at.end() as usize));
        }
    }

    fn declares(&mut self, locals: &ruby_prism::ConstantList) {
        self.found.shadowed |= locals.iter().any(|local| local.as_slice() == self.name);
    }
}

impl<'pr> ruby_prism::Visit<'pr> for Uses<'_> {
    fn visit_block_node(&mut self, node: &ruby_prism::BlockNode<'pr>) {
        self.declares(&node.locals());
        self.blocks += 1;
        ruby_prism::visit_block_node(self, node);
        self.blocks -= 1;
    }

    fn visit_lambda_node(&mut self, node: &ruby_prism::LambdaNode<'pr>) {
        self.declares(&node.locals());
        self.blocks += 1;
        ruby_prism::visit_lambda_node(self, node);
        self.blocks -= 1;
    }

    fn visit_local_variable_read_node(&mut self, node: &ruby_prism::LocalVariableReadNode<'pr>) {
        self.local(node.name(), node.depth(), node.location());
    }

    fn visit_local_variable_target_node(&mut self, node: &ruby_prism::LocalVariableTargetNode<'pr>) {
        self.local(node.name(), node.depth(), node.location());
    }

    fn visit_local_variable_write_node(&mut self, node: &ruby_prism::LocalVariableWriteNode<'pr>) {
        self.local(node.name(), node.depth(), node.name_loc());
        ruby_prism::visit_local_variable_write_node(self, node);
    }

    fn visit_local_variable_or_write_node(&mut self, node: &ruby_prism::LocalVariableOrWriteNode<'pr>) {
        self.local(node.name(), node.depth(), node.name_loc());
        ruby_prism::visit_local_variable_or_write_node(self, node);
    }

    fn visit_local_variable_and_write_node(&mut self, node: &ruby_prism::LocalVariableAndWriteNode<'pr>) {
        self.local(node.name(), node.depth(), node.name_loc());
        ruby_prism::visit_local_variable_and_write_node(self, node);
    }

    fn visit_local_variable_operator_write_node(&mut self, node: &ruby_prism::LocalVariableOperatorWriteNode<'pr>) {
        self.local(node.name(), node.depth(), node.name_loc());
        ruby_prism::visit_local_variable_operator_write_node(self, node);
    }

    fn visit_call_node(&mut self, node: &ruby_prism::CallNode<'pr>) {
        if node.is_variable_call() && node.name().as_slice() == self.name {
            let at = node.location();
            self.found.spans.push((at.start() as usize, at.end() as usize));
        }
        ruby_prism::visit_call_node(self, node);
    }
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
    fn local_uses_skip_shadowing_block_variables() {
        let source = "def t(user:)\n  user.name; @user; x.user; :user\n  [1].each { |user| user }\n  [2].each { user }\nend\n";
        let uses = local_uses(source, "user");
        let found: Vec<&str> = uses.spans.iter().map(|&(s, e)| &source[s..e]).collect();
        assert_eq!(found, ["user", "user"]);
        assert!(uses.shadowed);
        let slim = "user.name\nitems.each { |item| item }\n";
        assert_eq!(local_uses(slim, "user"), LocalUses { spans: vec![(0, 4)], shadowed: false }, "a bare call where nothing declares it");
        assert!(local_uses(slim, "item").shadowed);
    }

    #[test]
    fn local_uses_include_assignments() {
        let source = "def t(user:)\n  user ||= 1; user += 1; user = 2; a, user = 3, 4; user &&= 5\nend\n";
        assert_eq!(local_uses(source, "user").spans.len(), 5);
    }

    #[test]
    fn parameters_of_a_list() {
        let list = r#"title: "user: guest", user:, label: user.name"#;
        assert_eq!(keyword_parameter_at(list, "user"), list.find("user:,"));
        assert_eq!(keyword_parameter_at(list, "label"), list.find("label"));
        assert_eq!(keyword_parameter_at(list, "guest"), None);
        let at = list.find("user.name").unwrap();
        assert_eq!(parameter_reads(list, "user"), [(at, at + 4)]);
    }

    #[test]
    fn block_locals_without_the_end_typed_yet() {
        assert_eq!(block_locals("users.each do |user|\n  us", 25), ["user"]);
    }
}
