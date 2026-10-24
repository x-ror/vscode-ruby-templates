# Jbuilder support: TODO

Goal: in `*.json.jbuilder` views, go to definition, hover, links and completion for `json.partial! "x"` / `partial: "x"`, reusing `server/src/partials.rs`. Highlighting stays with the `ruby` language. A separate grammar would add nothing.

Legend as in `ERB_TODO.md`.

## Decision: keep the `ruby` language id

Ruby LSP (`shopify.ruby-lsp`) already maps `.jbuilder` and `.builder` to `ruby`. A new `jbuilder` id with a grammar embedding `source.ruby` would take these files away from Ruby LSP (completion, RuboCop, formatting). So the file stays `ruby`, and our server attaches to it by path pattern.

## Tasks

### Client / manifest

- [x] Contribute `{ "id": "ruby", "extensions": [".jbuilder", ".builder"] }` so VS Code without Ruby LSP still opens them as Ruby; VS Code's built-in `ruby` doesn't claim them (`package.json` `contributes.languages`). No grammar entry.
- [x] `documentSelector` maps _every_ contributed language id (`src/client.js` `startClient`). With `ruby` contributed, every `.rb` file would be sent to our server. Exclude `ruby` from that map and add `{ language: 'ruby', pattern: '**/*.{jbuilder,builder}' }`.
- [x] Activation: VS Code only generates `onLanguage:<id>` for a contributed language with a `configuration`, and the `ruby` entry has none; the template languages get theirs generated. So opening just a `.jbuilder` never starts the extension. Add an explicit event to `package.json` `activationEvents` (see open question); it is now the only entry there.

### Server

- [x] New `TemplateLang::Ruby` (shared with Builder, see `BUILDER_TODO.md`), picked by path in `TemplateLang::from_uri_or_id` when the id is `ruby` and the path ends in `.jbuilder`/`.builder` (`server/src/lang.rs`). Plain `.rb` must still return `None`.
- [x] Matches that need a `Ruby` arm (`server/src/lsp.rs`):
  - `diagnostics`: no template diagnostics (Ruby LSP reports syntax errors). `unresolved_renders`/`strict_locals_mismatches` already run for every language; only the arm is new.
  - `render_in_code`: `(TemplateLang::Ruby, _) => in_ruby().then_some(0)`. It matches every language, so the compiler asks for this arm. `line_roles` falls back to `LineRole::Start` and needs nothing.
  - `in_ruby_context`: `true` unless `ruby_comment_before(&text[line_start..offset])`.
- [x] Partial refs (`server/src/partials.rs` `RENDER_STR`, `render_string_prefix`): widen `RENDER_STR` and the `OPEN` regex in `render_string_prefix` from `render` to `(?:render|json\.partial!)` (`server/src/partials.rs`). That covers `json.partial! "x"`, `json.partial! partial: "x"` and `json.partial! "x", user: u`.
- [x] Jbuilder option keys: `json.partial! "posts/post", collection: @posts, as: :post` makes `passed_locals` return `collection`, `as` (`server/src/partials.rs` `passed_locals`), so a `# locals: (post:)` partial gets a false strict-locals warning. Done as: `passed_locals` returns `None` (unknown) when a Jbuilder call passes `collection`/`as`/`cached`, since `as:` names the local; dropping the names would report the `as:` local as missing.
- [x] Collection form `json.array! @posts, partial: "posts/post", as: :post` and `json.comments @c, partial: "…"`: the `partial:` key isn't directly after the call. Added `JSON_PARTIAL_KEY` (`json\.\w+!?[\s(][^#]*?\bpartial\s*(?::|=>)\s*["']…["']`; a `\b` after `!?` would never match after `!`) feeding `render_refs` (`server/src/partials.rs`). It overlaps the widened `RENDER_STR` on `json.partial! partial: 'x'` (same span), and the `regex` crate has no lookahead to exclude it, so `refs.dedup_by_key(|r| r.start)` after the sort in `render_refs`. It feeds only `render_refs`, so `json.array! …, partial: "` gets no completion; accepted.
- [x] Format: `resolve`/`resolve_model` keep only partials in the view's format (`server/src/partials.rs` `renders_format`, test `resolve_by_view_format`). No-op when the view has no format (`x.builder`); format-less partials (`_post.jbuilder`) always match; JS and turbo_stream views also take HTML. Builder reuses it unchanged. Same gap as `ERB_TODO.md` §14 "Jbuilder/other templates in the same dir don't confuse partial resolution".
- [x] Strict locals: `STRICT_LOCALS` already accepts `# locals: (user:)` at end of line (`(?m)…$`), so `file_signature` should work for `_post.json.jbuilder`. Confirm with a test, don't write code.
- [x] Hover: `erb::tag_doc` is ERB-gated already; Rails `HELPERS` hover shows in jbuilder. Helper completion is off for `TemplateLang::Ruby` (`server/src/lsp.rs` `completion`): it would duplicate Ruby LSP's list on every identifier.

## Tests

- [x] `partials.rs` `refs`: `json.partial! "users/user", user: u`, `json.array! @u, partial: "users/user", as: :user` → one ref each; `json.partial! partial: 'x'` → exactly one ref (dedup); `render partial: "x"` in ERB still one ref.
- [x] `partials.rs`: `json.partial! "posts/post", collection: @p, as: :post` against a `# locals: (post:)` partial → no strict-locals mismatch.
- [x] `partials.rs` format filter on a temp dir (`resolve_by_view_format`).
- [x] `main.rs`: `from_uri_or_id(index.json.jbuilder, Some("ruby"))` → `Ruby`, `foo.rb` → `None`; `render_in_code(Ruby, …)` true for `json.partial! "x"`; `in_ruby_context` false after `# `.
- [x] `test/e2e.js` (asserts and exits non-zero on a mismatch): open `users/index.json.jbuilder` with `languageId: 'ruby'`, definition on `json.partial! "users/user"` → only `_user.json.jbuilder`. Checks the client-facing path: language id, selector, format filter.
- No token sample (no grammar); a `test/sample.json.jbuilder` would also need a `test/tokenize.js` `samples` entry.

## Open questions

- Heredoc bodies count as code in `TemplateLang::Ruby` (only a same-line `#` comment is excluded), so `json.partial! "x"` inside `<<~T … T` gets warnings and rename edits. Rare in Jbuilder; Builder's `xml.cdata! <<~X` holds markup, not `render` calls. Skip until reported.
- `JSON_PARTIAL_KEY` stops at the first `#`, so `partial:` after `"#x"` or an interpolation is missed: no ref, which fails safe.

- Activation: explicit `onLanguage:ruby` (cheap, but one idle server per Ruby workspace) vs. `workspaceContains:**/*.{jbuilder,builder}` (a file search at startup). Default: `workspaceContains`.
- Completion for `json.` DSL methods (`set!`, `array!`, `extract!`, `cache!`): skip; Ruby LSP's job.
- Watched files: `**/app/views/**` already covers jbuilder partials (`src/client.js`); nothing to add.

## Size

S: ~40 lines Rust, ~10 lines JS/JSON, ~6 unit assertions + 1 e2e case (format filter already done).
