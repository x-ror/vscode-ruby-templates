# Liquid support: TODO

Goal: highlight `.liquid` / `.html.liquid` templates (HTML host with `{{ }}`, `{% %}`, filters, comment/raw blocks), and resolve `{% render 'x' %}` / `{% include 'x' %}` to files. Target Rails apps using the `liquid` gem and Jekyll. Shopify themes are covered by Shopify's official extension, so no Shopify-specific roots or tags. Phase 1 is grammar + gate; the resolver (phase 2) and diagnostics (phase 3) wait until someone asks.

Legend as in `ERB_TODO.md`.

## Tasks

### Grammar and language
- [ ] `syntaxes/liquid.tmLanguage.json`, scope `text.html.liquid`, modelled on `syntaxes/erb.tmLanguage.json`: `patterns: [#liquid, text.html.basic]` plus an `L:` injection so tags work inside attributes, `<script>` and `<style>`. Repository:
  - `output` `{{ … }}`, `{{- … -}}`
  - `tag` `{% … %}`, `{%- … -%}`
  - filters `| name: arg, arg`
  - strings, numbers, ranges `(1..5)`, operators (`==`, `contains`, `and`, `or`)
  - `{% comment %}…{% endcomment %}` and inline `{% # note %}` as `comment.block.liquid`
  - `{% raw %}…{% endraw %}` as unhighlighted text
  - `{% liquid … %}` multi-line tag (one statement per line)
- [ ] Register language `liquid` (extensions `.liquid`, `.html.liquid`) and grammar (`package.json` `contributes.languages`, `contributes.grammars` with `embeddedLanguages` for `source.js`/`source.css`). The client picks it up automatically (`src/client.js` `startClient` maps all language ids).
- [ ] `language-configuration/liquid.json`, based on `language-configuration/erb.json`:
  - block comment `{% comment %}` / `{% endcomment %}`
  - brackets and auto-close `{{`→` }}`, `{%`→` %}`
  - `indentationRules` for `if|unless|case|for|capture|tablerow|form|paginate` … `end*`
- [ ] `configurationDefaults["[liquid]"]` tab size, and `emmet.includeLanguages.liquid = "html"` (`package.json`).
- [ ] Commands stay ERB/Haml/Slim-only: `TEMPLATE_LANGS` (`src/extension.js`) and the `when` clauses don't match `liquid`, so preview/toggle stay hidden. No change.

### Server
- [ ] `TemplateLang::Liquid` from id `liquid` or `.liquid` path (`server/src/main.rs` `TemplateLang::from_uri_or_id`). Arms for the exhaustive matches: `diagnostics` → `vec![]` at first, `render_in_code` → `None` (also keeps Liquid renders out of unresolved-render and strict-locals warnings), `in_ruby_context` → `false`.
- [ ] Gate the Rails-only paths on `lang != Liquid` (`server/src/main.rs`). `goto_definition`, `hover`, `document_link` and `completion` call `partials::render_ref_at`/`render_refs`/`render_string_prefix` without checking `in_ruby_context`, and `RENDER_STR` matches `{% render "x" %}` (checked). Without the gate, Liquid renders would resolve as Rails partials by accident. The Rails `HELPERS` hover via `word_at` is ungated too, and so is the form-builder branch of `completion` (step 3, before any `in_ruby_context` check), so `{{ f.` would offer `FormBuilder` methods. One `lang == Liquid` early return at the top of each handler, not per call.
- [ ] Phase 2: new `server/src/liquid.rs`:
  - `refs(line)` for `{%-? (render|include|include_relative) 'name' … %}`. Jekyll also allows unquoted `{% include footer.html %}`. Skip `{{ var }}` dynamic names.
  - `resolve(current, ref)`: Jekyll (walk up to `_config.yml`) → `_includes/<name>`, or relative to the file for `include_relative`; otherwise `partials::resolve` filtered to `.liquid` (the liquid gem's `_<name>.liquid` default is the same lookup).
  - `candidates` for completion: reuse `partials::files_under`.
- [ ] Phase 2: wire `liquid::refs`/`resolve` into `goto_definition`, `document_link`, `hover` (resolved file list, like `render_hover`) and `completion` (inside `{% render '`) for `TemplateLang::Liquid` (`server/src/main.rs`).
- [ ] Diagnostics (phase 3): unclosed or stray block tags (`if`/`for`/`case`/`capture`/`comment`/`raw` without `end…`), unclosed `{{`/`{%`. One stack pass in `liquid.rs`; reuse `util::error`/`util::warning`/`util::offset_to_position`.

## Tests

- [ ] `test/sample.html.liquid` covering every construct above, plus `'sample.html.liquid': 'text.html.liquid'` in `test/tokenize.js` `samples` and a baseline from `UPDATE=1 npm run test:tokens`. `test:tokens` fails on any unlisted `test/sample.*`.
- [ ] `test:server` and CI (`.github/workflows/build.yml`) run `--check test/sample.*`, so the server's `.liquid` detection must land with the sample or the check exits 1 ("unknown template type").
- [ ] Gate (phase 1), Rust unit test on a temp dir like `partials.rs` `model_render_partials`: `{% render "card" %}` in a `.liquid` file next to `_card.html.slim` → no definition, no link. Plus one `test/e2e.js` case (it asserts): definition on that render returns nothing.
- [ ] `liquid.rs` unit tests (phase 2): refs with quotes / no quotes / `{%-`, `{{ x }}` ignored, `include_relative` path; resolve for Jekyll `_config.yml` and the `_x.liquid` fallback on a temp dir.
- [ ] Block-balance diagnostics (phase 3): unit tests next to them. A `test/broken/bad.html.liquid` fixture only counts if a unit test reads it via `include_str!`, as `haml_indent.rs`/`slim_indent.rs` `broken_fixture` do.

## Open questions

- **Jekyll files are mostly `.html`/`.md`, not `.liquid`.** Claiming `.html` would take HTML language features away. Default: don't claim them. Document `files.associations` (`"**/_includes/**/*.html": "liquid"`) in `README.md`. Without this the Jekyll value is small.
- Language id clash: Shopify's extension (and `sissel.shopify-liquid`) also contribute `liquid`. Two grammars for one id conflict. Not checked locally, since neither is installed here. Options: reuse `liquid` (interop with Prettier, settings) or use our own id. Default: `liquid`, and document "disable one of them".
- Rails + `liquid` gem: templates are often stored in the DB, and the file-system root is app-defined (`Liquid::Template.file_system`). Default: same-dir `_name.liquid` only, with no setting until asked.
- Shopify `{{ 'key' | t }}` → `locales/*.json`, filter/tag completion, object hover: out of scope (official extension).
- Caching: `liquid::candidates` walks the tree per request. Add a cache + `src/client.js` watchers (`**/_includes/**`) only if it's slow.

## Size

L:
- grammar ~200 lines JSON + language config ~60
- phase 1: grammar ~200 lines JSON + language config ~60, gate ~10 lines Rust, 1 sample + 1 unit test
- phase 2: `liquid.rs` ~100 lines, `main.rs` wiring ~40, ~5 unit tests
- phase 3: diagnostics ~80 lines + tests
