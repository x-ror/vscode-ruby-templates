# Builder support: TODO

Goal: `*.xml.builder` / `*.builder` views (`xml.tag!`, `xml.instruct!`) get partial navigation, hover and completion for `xml << render(partial: "x")`. Do this after `JBUILDER_TODO.md`, which adds the plumbing (`ruby` id passthrough, `TemplateLang::Ruby`, client selector).

Legend as in `ERB_TODO.md`.

## Tasks

- [ ] Nothing extra in `package.json` / `src/client.js`: the Jbuilder entries already list `.builder` (`contributes.languages[id=ruby]`, `documentSelector` pattern `**/*.{jbuilder,builder}`).
- [ ] Server detection: `.builder` maps to `TemplateLang::Ruby` in `TemplateLang::from_uri_or_id` (`server/src/main.rs`), done in the Jbuilder change.
- [ ] `xml << render(partial: "items/item", locals: { i: i })` and `xml << render "items/item"` already match `RENDER_STR` (`server/src/partials.rs`; checked). Once `in_ruby_context`/`ruby_line` return true for `TemplateLang::Ruby`, definition, hover, links, unresolved-render warnings and completion work with no new code.
- [ ] Legacy `render(:partial => "items/item")` does **not** match `RENDER_STR` (it requires `partial` without the leading `:`; checked). It's common in old Builder views and also affects ERB/Haml/Slim. Fix once: `:?(partial|template|layout)` in `RENDER_STR` and `render_string_prefix`'s `OPEN`, and `:?` in both places of `RENDER_LAYOUT_KEY` (`:?(partial|template)` and `:?layout`), otherwise `render(:partial => "x", :layout => "y")` misses the layout (`server/src/partials.rs`). Cross-cutting; can ship before Builder.
- [ ] Format filter: the Jbuilder one, unchanged (format segment of the current file name; bare `x.builder` doesn't filter).

## Tests

- [ ] `partials.rs` `refs`: `render(:partial => "a/b")` and `render :template => "a/b"` → refs with the right `RenderKind`; `render(:partial => "x", :layout => "y")` → also the layout ref. This is the only new logic.
- [ ] `main.rs`: `from_uri_or_id(feed.atom.builder, Some("ruby"))` → `Ruby`.
- No e2e case (`test/e2e.js` has no assertions). No token sample (no grammar).

## Open questions

- Builder partials receive the parent's `xml` builder only when rendered via `xml << render(...)`; a bare `render` inside a block outputs nothing. Diagnostic for it? Default: skip until someone reports it.
- Bare `.builder` outside `app/views` (gem scripts, Rakefile helpers) gets nothing, because `partials::views_root` is `None`. Fine.

## Size

XS on top of Jbuilder: ~3 regex edits, 2 test assertions.
