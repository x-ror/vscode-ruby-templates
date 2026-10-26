//! Shared by the language server (`ruby-templates-ls`) and the command line (`ruby-templates`).

mod assets;
pub mod cli;
mod config;
mod controllers;
mod erb;
mod erb_complete;
mod erb_format;
mod format;
mod haml;
mod haml_format;
mod haml_indent;
mod helper_defs;
mod helpers;
mod html;
mod html_lint;
mod i18n;
mod ivars;
mod lang;
mod lsp;
mod outline;
mod partial_rename;
mod partials;
// Used by the Slim and ERB Ruby checks (docs/LINT_PLAN.md 1a, 3a).
#[cfg_attr(not(test), allow(dead_code))]
mod prism;
mod project;
mod routes;
mod ruby;
mod ruby_syntax;
mod schema;
mod slim_complete;
mod slim_format;
mod slim_hover;
mod slim_indent;
mod slim_outline;
mod slim_parser;
mod slots;
mod stimulus;
mod tags;
mod util;

pub use lsp::serve;
