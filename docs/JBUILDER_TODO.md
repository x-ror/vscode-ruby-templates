# Jbuilder support: TODO

Goal: in `*.json.jbuilder` views, go to definition, hover, links and completion for `json.partial! "x"` / `partial: "x"`, reusing `server/src/partials.rs`. Highlighting stays with the `ruby` language. A separate grammar would add nothing.

Legend as in `ERB_TODO.md`.

## Decision: keep the `ruby` language id

Ruby LSP (`shopify.ruby-lsp`) already maps `.jbuilder` and `.builder` to `ruby`. A new `jbuilder` id with a grammar embedding `source.ruby` would take these files away from Ruby LSP (completion, RuboCop, formatting). So the file stays `ruby`, and our server attaches to it by path pattern.

## Tasks

### Client / manifest
- [ ] Contribute `{ "id": "ruby", "extensions": [".jbuilder", ".builder"] }` so VS Code without Ruby LSP still opens them as Ruby; VS Code's built-in `ruby` doesn't claim them (`package.json` `contributes.languages`). No grammar entry.
- [ ] `documentSelector` maps *every* contributed language id (`src/client.js` `startClient`). With `ruby` contributed, every `.rb` file would be sent to our server. Exclude `ruby` from that map and add `{ language: 'ruby', pattern: '**/*.{jbuilder,builder}' }`.
- [ ] Activation: VS Code only generates `onLanguage:<id>` for a contributed language with a `configuration`, and the `ruby` entry has none; `activationEvents` lists only `erb|haml|slim`. So opening just a `.jbuilder` never starts the extension. Add an explicit event to `package.json` `activationEvents` (see open question).

### Server
- [ ] New `TemplateLang::Ruby` (shared with Builder, see `BUILDER_TODO.md`), picked by path in `TemplateLang::from_uri_or_id` when the id is `ruby` and the path ends in `.jbuilder`/`.builder` (`server/src/main.rs`). Plain `.rb` must still return `None`.
- [ ] Matches that need a `Ruby` arm (`server/src/main.rs`):
  - `diagnostics`: no template diagnostics (Ruby LSP reports syntax errors). `unresolved_renders`/`strict_locals_mismatches` already run for every language; only the arm is new.
  - `render_in_code`: `(TemplateLang::Ruby, _) => in_ruby().then_some(0)`. It matches every language, so the compiler asks for this arm. `line_roles` falls back to `LineRole::Start` and needs nothing.
  - `in_ruby_context`: `true` unless `ruby_comment_before(&text[line_start..offset])`.
- [ ] Partial refs: widen `RENDER_STR` and the `OPEN` regex in `render_string_prefix` from `render` to `(?:render|json\.partial!)` (`server/src/partials.rs`). That covers `json.partial! "x"`, `json.partial! partial: "x"` and `json.partial! "x", user: u`.
- [ ] Jbuilder option keys: `json.partial! "posts/post", collection: @posts, as: :post` makes `passed_locals` return `collection`, `as` (`server/src/partials.rs` `passed_locals`), so a `# locals: (post:)` partial gets a false strict-locals warning. Drop `collection`/`as`/`cached` from the passed names for `json.partial!` refs.
- [ ] Collection form `json.array! @posts, partial: "posts/post", as: :post` and `json.comments @c, partial: "…"`: the `partial:` key isn't directly after the call. Add one `JSON_PARTIAL_KEY` regex (`json\.\w+!?\b[^#]*?\bpartial\s*(?::|=>)\s*["']…["']`) feeding `render_refs` (`server/src/partials.rs`). It overlaps the widened `RENDER_STR` on `json.partial! partial: 'x'` (same span), and the `regex` crate has no lookahead to exclude it, so `refs.dedup_by_key(|r| r.start)` after the sort in `render_refs`. It feeds only `render_refs`, so `json.array! …, partial: "` gets no completion; accepted.
- [x] Format: `resolve`/`resolve_model` keep only partials in the view's format (`server/src/partials.rs` `renders_format`, test `resolve_by_view_format`). No-op when the view has no format (`x.builder`); format-less partials (`_post.jbuilder`) always match; JS and turbo_stream views also take HTML. Builder reuses it unchanged. Same gap as `ERB_TODO.md` §14 "Jbuilder/other templates in the same dir don't confuse partial resolution".
- [ ] Strict locals: `STRICT_LOCALS` already accepts `# locals: (user:)` at end of line (`(?m)…$`), so `file_signature` should work for `_post.json.jbuilder`. Confirm with a test, don't write code.
- [ ] Hover: `erb::tag_doc` is ERB-gated already. Rails `HELPERS` hover/completion will also show in jbuilder (completion step 4 offers all helpers and locals on every identifier, duplicating Ruby LSP's list); accepted noise, since view helpers are available there.

## Tests

- [ ] `partials.rs` `refs`: `json.partial! "users/user", user: u`, `json.array! @u, partial: "users/user", as: :user` → one ref each; `json.partial! partial: 'x'` → exactly one ref (dedup); `render partial: "x"` in ERB still one ref.
- [ ] `partials.rs`: `json.partial! "posts/post", collection: @p, as: :post` against a `# locals: (post:)` partial → no strict-locals mismatch.
- [x] `partials.rs` format filter on a temp dir (`resolve_by_view_format`).
- [ ] `main.rs`: `from_uri_or_id(index.json.jbuilder, Some("ruby"))` → `Ruby`, `foo.rb` → `None`; `render_in_code(Ruby, …)` true for `json.partial! "x"`; `in_ruby_context` false after `# `.
- [ ] `test/e2e.js` (asserts and exits non-zero on a mismatch): open `users/index.json.jbuilder` with `languageId: 'ruby'`, definition on `json.partial! "users/user"` → only `_user.json.jbuilder`. Checks the client-facing path: language id, selector, format filter.
- No token sample (no grammar); a `test/sample.json.jbuilder` would also need a `test/tokenize.js` `samples` entry.

## Open questions

- Activation: explicit `onLanguage:ruby` (cheap, but one idle server per Ruby workspace) vs. `workspaceContains:**/*.{jbuilder,builder}` (a file search at startup). Default: `workspaceContains`.
- Completion for `json.` DSL methods (`set!`, `array!`, `extract!`, `cache!`): skip; Ruby LSP's job.
- Watched files: `**/app/views/**` already covers jbuilder partials (`src/client.js`); nothing to add.

## Size

S: ~40 lines Rust, ~10 lines JS/JSON, ~6 unit assertions + 1 e2e case (format filter already done).
