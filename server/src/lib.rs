//! Shared by the language server (`ruby-templates-ls`) and the command line (`ruby-templates`).

mod attributes;
mod checks;
pub mod cli;
mod code_context;
mod completion;
mod config;
mod element_lints;
mod erb;
mod format;
mod glob;
mod haml;
mod helpers;
mod html;
mod lang;
mod line_length;
mod lsp;
mod nested_interactive;
mod options;
mod outline;
mod partial_rename;
mod rails;
mod ruby;
mod rules;
mod semantic_tokens;
mod slim;
mod util;

pub use lsp::serve;
