//! Shared by the language server (`ruby-templates-ls`) and the command line (`ruby-templates`).

mod checks;
pub mod cli;
mod code_context;
mod completion;
mod config;
mod erb;
mod format;
mod haml;
mod helpers;
mod html;
mod inflect;
mod lang;
mod lsp;
mod options;
mod outline;
mod partial_rename;
mod rails;
mod ruby;
mod semantic_tokens;
mod slim;
mod util;

pub use lsp::serve;
