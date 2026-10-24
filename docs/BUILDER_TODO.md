# Builder support: TODO

Goal: `*.xml.builder` / `*.builder` views (`xml.tag!`, `xml.instruct!`) get partial navigation, hover and completion for `xml << render(partial: "x")`. Do this after `JBUILDER_TODO.md`, which adds the plumbing (`ruby` id passthrough, `TemplateLang::Ruby`, client selector).

Legend as in `ERB_TODO.md`.

## Tasks

- [x] Nothing extra in `package.json` / `src/client.js`: the Jbuilder entries already list `.builder` (`contributes.languages[id=ruby]`, `documentSelector` pattern `**/*.{jbuilder,builder}`).
- [x] Server detection: `.builder` maps to `TemplateLang::Ruby` in `TemplateLang::from_uri_or_id` (`server/src/lang.rs`), done in the Jbuilder change.
- [x] `xml << render(partial: "items/item", locals: { i: i })` and `xml << render "items/item"` already match `RENDER_STR` (`server/src/partials.rs`; checked). Once `in_ruby_context`/`render_in_code` accept `TemplateLang::Ruby`, definition, hover, links, unresolved-render warnings and completion work with no new code.
- [x] Legacy `render(:partial => "items/item")`, `:template =>`, `:layout =>` in every language (`server/src/partials.rs` `RENDER_STR`, `RENDER_LAYOUT_KEY`, `render_string_prefix`).
- [x] Format filter: `partials::renders_format`, unchanged (format segment of the current file name; bare `x.builder` doesn't filter).

## Tests

- [x] `partials.rs` `refs`: hash-rocket `:partial =>`/`:template =>`/`:layout =>` refs and completion.
- [x] `main.rs`: `from_uri_or_id(feed.atom.builder, Some("ruby"))` → `Ruby`.
- No e2e case beyond Jbuilder's: same code path. No token sample (no grammar).

## Open questions

- Builder partials receive the parent's `xml` builder only when rendered via `xml << render(...)`; a bare `render` inside a block outputs nothing. Diagnostic for it? Default: skip until someone reports it.
- Bare `.builder` outside `app/views` (gem scripts, Rakefile helpers) gets nothing, because `partials::views_root` is `None`. Fine.

## Size

XS on top of Jbuilder: 1 test assertion; the regex and format work is done.
