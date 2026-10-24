# ERB lint rules

What the three sources check in ERB (`.erb`, `.html.erb`, `.erubi`, …). Reference only: nothing here is implemented beyond the built-in checks. Index, cross-language matrix and helper guidance: [README.md](README.md).

## Built-in (this extension's language server)

| Code                   | What it flags                                                                                                        |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `unclosed-tag`         | `<%` with no `%>`, or a tag left open before the next `<%` (reported at its opener)                                  |
| `malformed-tag`        | A tag tree-sitter-embedded-template can't parse                                                                      |
| `stray-end`            | `<% end %>` with no open block                                                                                       |
| `unmatched-branch`     | `<% else %>` / `elsif` / `when` outside a block                                                                      |
| `unclosed-block`       | `<% if %>` / `do` / `case` … never closed with `<% end %>`                                                           |
| `output-control-flow`  | `<%= if … %>` / `<%= end %>` (an output tag around control flow)                                                     |
| `silent-output-helper` | `<% form_with … do %>` / `link_to … do` in a non-output tag: renders nothing                                         |
| `unterminated-string`  | `%>` inside a Ruby string ends the tag early (`<%= "50%>" %>`)                                                       |
| `ruby-syntax`          | Ruby syntax error inside tags (tree-sitter-ruby over the tags joined like Erubi)                                     |
| `unexpected-close-tag` | `</div>` with no open element (HTML-host views)                                                                      |
| `mismatched-close-tag` | `</span>` closing a `<div>` (HTML-host views)                                                                        |
| `unclosed-element`     | `<div>` never closed (HTML-host views, not partials)                                                                 |
| `img-missing-alt`      | `<img>` with no `alt` (unless ERB adds attributes), `image_tag` with no `alt:` (HTML-host views)                     |
| `duplicate-id`         | The same static `id` twice with no ERB branch between them (HTML-host views)                                         |
| `prefer-image-tag`     | `<img>` whose `src` holds ERB output; info (HTML-host views)                                                         |
| `unsafe-output`        | `<%==`, `raw(…)`, `.html_safe`: output that skips escaping; info (HTML-host views)                                   |
| `unresolved-render`    | `render "dir/name"` with no matching partial/template (also Haml, Slim, Jbuilder)                                    |
| `strict-locals`        | `render` passes locals a partial's `locals: (…)` doesn't accept, or misses required ones (also Haml, Slim, Jbuilder) |

Turned off per code with `rubyTemplates.erb.disabledDiagnostics` (Jbuilder/Builder: `rubyTemplates.jbuilder.disabledDiagnostics`).

## erb_lint

Wired in through `rubyTemplates.erbLint.enabled` (offenses shown as reported).

Version 0.9.0. **On** = enabled by `erb_lint` without a config (`EnableDefaultLinters`, `RunnerConfig.default`); others need `enabled: true` in `.erb_lint.yml`. Autofix = `--autocorrect`.

| Linter                     | On  | Autofix | What it flags                                                                                                         |
| -------------------------- | --- | ------- | --------------------------------------------------------------------------------------------------------------------- |
| `AllowedScriptType`        | yes | yes     | Inline `<script>` without an allowed `type` (`text/javascript` by default); `javascript_tag` is left to a RuboCop cop |
| `ClosingErbTagIndent`      | yes | yes     | `%>` on its own line when `<%` isn't followed by a newline, or `%>` not indented like its `<%`                        |
| `CommentSyntax`            | yes | no      | Comment syntax that isn't valid ERB (`<% # x %>`, `<%= # x %>`)                                                       |
| `DeprecatedClasses`        | no  | no      | Configured deprecated CSS classes in start tags, with a suggestion                                                    |
| `ErbSafety`                | no  | no      | Unsafe Ruby interpolation into JavaScript (better-html rules)                                                         |
| `ExtraNewline`             | yes | yes     | More than one consecutive blank line                                                                                  |
| `FinalNewline`             | yes | yes     | Missing (or, configured, present) final newline                                                                       |
| `HardCodedString`          | no  | yes     | Text that isn't translated through i18n                                                                               |
| `NoJavascriptTagHelper`    | yes | yes     | `javascript_tag do`: "confuses tests"; use `<script>`                                                                 |
| `NoUnusedDisable`          | no  | no      | `erb_lint:disable` comments that disable nothing                                                                      |
| `ParserErrors`             | yes | no      | HTML parser errors (better-html)                                                                                      |
| `PartialInstanceVariable`  | no  | no      | Instance variables in partials                                                                                        |
| `RequireInputAutocomplete` | yes | no      | `<input>` / input field helpers without `autocomplete`                                                                |
| `RequireScriptNonce`       | no  | no      | Inline `<script>` / `javascript_tag` without `nonce`                                                                  |
| `RightTrim`                | yes | yes     | Enforces one trim style, `-%>` or `=%>`                                                                               |
| `Rubocop`                  | no  | yes     | Selected RuboCop cops on the Ruby in tags                                                                             |
| `RubocopText`              | no  | yes     | Selected RuboCop cops on ERB inside text nodes                                                                        |
| `SelfClosingTag`           | yes | yes     | Void elements written `<br/>` (or, configured, not self-closed)                                                       |
| `SpaceAroundErbTag`        | yes | yes     | Not exactly one space after `<%`/`<%=` and before `%>`                                                                |
| `SpaceIndentation`         | yes | yes     | Tab indentation                                                                                                       |
| `SpaceInHtmlTag`           | yes | yes     | Extra or missing whitespace inside HTML tags (`<div  class>`, `< div>`)                                               |
| `StrictLocals`             | no  | yes     | Partials without a `<%# locals: (…) %>` magic comment                                                                 |
| `TrailingWhitespace`       | yes | yes     | Trailing whitespace                                                                                                   |

## Herb (`herb-lint`)

`@herb-tools/linter` 0.11.1, `marcoroth/herb` @ `fc3ca234a2b568cfc731bcf26b81cf4d47504df7` (2026-10-20): **170 rules, 148 on by default, 37 with an autofix**. Taken from `javascript/packages/linter/src/rules/*.ts` (id, default, severity, autofix, version) and `docs/rules/README.md` (category, description); https://herb-tools.dev/linter/rules/ lists the same 170.

Legend:

- **Autofix**: `yes` = `--fix`; `unsafe only` = needs `--fix-unsafely`; `yes (some offenses unsafe-only)` = safe fix, but some offenses mark their fix unsafe; `partial` = `autofixRequiresContext`, only some offenses carry a fix.
- **Severity**: default from `defaultConfig`; `cli: …, editor: …` = different severity per environment.
- _only with `framework: actionview`_ = rule declares `frameworks: ["actionview"]` and only reports when `.herb.yml` sets `framework: actionview`.

### Accessibility (`a11y-*`, 10 rules, 0 on by default)

| Rule                                           | Default on | Autofix | Severity | What it flags                                                                           |
| ---------------------------------------------- | ---------- | ------- | -------- | --------------------------------------------------------------------------------------- |
| `a11y-avoid-generic-link-text`                 | no         | no      | warning  | Avoid generic link text like "Click here" or "Read more" _(added 0.10.2)_               |
| `a11y-disabled-attribute`                      | no         | no      | warning  | Prevent usage of the `disabled` attribute on unsupported elements _(added 0.10.2)_      |
| `a11y-nested-interactive-elements`             | no         | no      | error    | Disallow nesting interactive elements _(added 0.10.2)_                                  |
| `a11y-no-accesskey-attribute`                  | no         | no      | warning  | Prevent usage of the `accesskey` attribute _(added 0.9.3)_                              |
| `a11y-no-aria-label-misuse`                    | no         | no      | warning  | Prevent `aria-label` and `aria-labelledby` on non-interactive elements _(added 0.10.2)_ |
| `a11y-no-aria-unsupported-elements`            | no         | no      | warning  | Prevent usage of ARIA on unsupported elements _(added 0.9.3)_                           |
| `a11y-no-autofocus-attribute`                  | no         | no      | warning  | Prevent usage of the `autofocus` attribute _(added 0.9.3)_                              |
| `a11y-no-redundant-image-alt`                  | no         | no      | warning  | Prevent redundant words in `<img>` `alt` attributes _(added 0.10.2)_                    |
| `a11y-no-visually-hidden-interactive-elements` | no         | no      | warning  | Prevent visually hidden interactive elements _(added 0.11.0)_                           |
| `a11y-svg-has-accessible-text`                 | no         | no      | warning  | Require accessible text on `<svg>` elements _(added 0.10.2)_                            |

### Action View (`actionview-*`, 23 rules, 21 on by default)

| Rule                                        | Default on | Autofix              | Severity                 | What it flags                                                                                                                                  |
| ------------------------------------------- | ---------- | -------------------- | ------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `actionview-no-content-argument-with-block` | yes        | no                   | error                    | Disallow passing a content argument to a helper that is given a block _(only with `framework: actionview`; added 0.11.0)_                      |
| `actionview-no-dynamic-partial-path`        | yes        | no                   | info                     | Disallow partial paths that are built at runtime _(only with `framework: actionview`; added 0.11.0)_                                           |
| `actionview-no-helper-shadowing`            | yes        | no                   | warning                  | Disallow shadowing Action View helpers with block variables _(only with `framework: actionview`; added 0.11.0)_                                |
| `actionview-no-implicit-partial`            | yes        | no                   | info                     | Disallow `render` calls that infer the partial from an object _(only with `framework: actionview`; added 0.11.0)_                              |
| `actionview-no-implicit-polymorphic-url`    | yes        | no                   | info                     | Prefer explicit route helpers over implicit polymorphic URLs _(only with `framework: actionview`; added 0.11.0)_                               |
| `actionview-no-local-assigns`               | yes        | no                   | info                     | Prefer a strict locals declaration over reading locals through `local_assigns` _(only with `framework: actionview`; added 0.11.1)_             |
| `actionview-no-mistyped-locals`             | yes        | no                   | warning                  | Disallow literal `render` arguments that contradict a strict local's declared default type _(only with `framework: actionview`; added 0.11.0)_ |
| `actionview-no-redundant-local-assigns`     | yes        | no                   | cli: error, editor: info | Disallow `local_assigns` reads that the strict locals declaration already answers _(only with `framework: actionview`; added 0.11.0)_          |
| `actionview-no-render-option-shadowing`     | yes        | no                   | info                     | Disallow locals that shadow a `render` option name _(only with `framework: actionview`; added 0.11.0)_                                         |
| `actionview-no-silent-helper`               | yes        | unsafe only, partial | error                    | Disallow silent ERB tags for Action View helpers _(only with `framework: actionview`; added 0.9.0)_                                            |
| `actionview-no-silent-render`               | yes        | no                   | error                    | Disallow calling `render` without outputting the result _(only with `framework: actionview`; added 0.9.1)_                                     |
| `actionview-no-strict-locals-error`         | yes        | no                   | error                    | Disallow `render` calls that would raise an `ActionView::StrictLocalsError` _(only with `framework: actionview`; added 0.11.0)_                |
| `actionview-no-unnecessary-html-safe`       | yes        | yes                  | error                    | Disallow calling `.html_safe` on String literals _(only with `framework: actionview`; added 0.11.0)_                                           |
| `actionview-no-unnecessary-tag-attributes`  | yes        | yes                  | warning                  | Disallow unnecessary attributes on Action View tag helpers _(only with `framework: actionview`; added 0.9.3)_                                  |
| `actionview-no-unused-strict-locals`        | yes        | no                   | cli: error, editor: info | Disallow strict locals that are never used in the partial _(only with `framework: actionview`; added 0.11.0)_                                  |
| `actionview-no-void-element-content`        | yes        | no                   | error                    | Disallow content arguments for void Action View elements _(only with `framework: actionview`; added 0.9.3)_                                    |
| `actionview-prefer-collection-render`       | yes        | no                   | error                    | Prefer collection rendering over rendering a partial in a loop _(only with `framework: actionview`; added 0.11.0)_                             |
| `actionview-prefer-form-with`               | yes        | no                   | info                     | Prefer the `form_with` helper over the legacy `form_for` helper _(only with `framework: actionview`; added 0.11.0)_                            |
| `actionview-prefer-link-to-helper`          | yes        | yes, partial         | info                     | Prefer `link_to` over a manual `<a>` tag with an ERB `href` _(only with `framework: actionview`; added 0.11.0)_                                |
| `actionview-prefer-pluralize-helper`        | yes        | no                   | warning                  | Prefer the `pluralize` helper over a separate count and `String#pluralize` _(only with `framework: actionview`; added 0.11.0)_                 |
| `actionview-prefer-qualified-partial-path`  | yes        | unsafe only, partial | info                     | Prefer partial paths qualified from the view root _(only with `framework: actionview`; added 0.11.0)_                                          |
| `actionview-strict-locals-first-line`       | no         | yes                  | error                    | Require strict locals on the first line of a template with a blank line after. _(only with `framework: actionview`; added 0.9.3)_              |
| `actionview-strict-locals-partial-only`     | no         | no                   | warning                  | Only allow strict local definitions in partial files. _(only with `framework: actionview`; added 0.9.3)_                                       |

### ERB (`erb-*`, 51 rules, 48 on by default)

| Rule                                    | Default on | Autofix                                  | Severity                 | What it flags                                                                                                  |
| --------------------------------------- | ---------- | ---------------------------------------- | ------------------------ | -------------------------------------------------------------------------------------------------------------- |
| `erb-closing-tag-indent`                | yes        | yes                                      | cli: error, editor: info | Enforce consistent closing ERB tag indentation _(added 0.11.0)_                                                |
| `erb-comment-syntax`                    | yes        | yes                                      | error                    | Disallow Ruby comments immediately after ERB tags _(added 0.7.5)_                                              |
| `erb-no-byte-order-mark`                | yes        | yes                                      | cli: error, editor: info | Disallow a byte order mark at the start of a template _(added 0.11.0)_                                         |
| `erb-no-case-node-children`             | yes        | no                                       | error                    | Don't use `children` for `case/when` and `case/in` nodes _(added 0.8.0)_                                       |
| `erb-no-class-definitions`              | yes        | no                                       | cli: error, editor: info | Disallow class definitions in ERB templates _(added 0.11.0)_                                                   |
| `erb-no-commented-out-output-tags`      | yes        | no                                       | info                     | Disallow commented-out ERB output tags (`<%#=`, `<%# =`) _(added 0.10.3)_                                      |
| `erb-no-conditional-html-element`       | yes        | no                                       | error                    | Disallow conditional HTML elements _(added 0.9.0)_                                                             |
| `erb-no-conditional-open-tag`           | yes        | no                                       | error                    | Disallow conditional HTML open tags _(added 0.9.0)_                                                            |
| `erb-no-debug-output`                   | yes        | no                                       | cli: error, editor: info | Disallow debug output methods (`p`, `pp`, `puts`, `print`, `debug`) in ERB templates _(added 0.9.3)_           |
| `erb-no-duplicate-branch-elements`      | yes        | yes (some offenses unsafe-only), partial | warning                  | Disallow duplicate elements across conditional branches _(added 0.9.0)_                                        |
| `erb-no-empty-control-flow`             | yes        | no                                       | hint                     | Disallow empty ERB control flow blocks _(added 0.9.1)_                                                         |
| `erb-no-empty-tags`                     | yes        | no                                       | cli: error, editor: info | Disallow empty ERB tags _(added 0.4.0)_                                                                        |
| `erb-no-extra-newline`                  | yes        | yes                                      | cli: error, editor: info | Disallow extra newlines. _(added 0.8.0)_                                                                       |
| `erb-no-extra-whitespace-inside-tags`   | yes        | yes                                      | error                    | Disallow multiple consecutive spaces inside ERB tags _(added 0.8.0)_                                           |
| `erb-no-inline-case-conditions`         | yes        | no                                       | warning                  | Disallow inline `case`/`when` and `case`/`in` conditions in a single ERB tag _(added 0.9.0)_                   |
| `erb-no-instance-variables-in-partials` | yes        | no                                       | error                    | Disallow instance variables in partials _(added 0.9.0)_                                                        |
| `erb-no-interpolated-class-names`       | yes        | no                                       | warning                  | Disallow ERB interpolation inside CSS class names _(added 0.9.0)_                                              |
| `erb-no-javascript-tag-helper`          | yes        | no                                       | warning                  | Disallow `javascript_tag` helper _(only with `framework: actionview`; added 0.9.0)_                            |
| `erb-no-method-definitions`             | yes        | no                                       | cli: error, editor: info | Disallow method definitions in ERB templates _(added 0.11.0)_                                                  |
| `erb-no-module-definitions`             | yes        | no                                       | cli: error, editor: info | Disallow module definitions in ERB templates _(added 0.11.0)_                                                  |
| `erb-no-multiple-statements`            | yes        | yes, partial                             | warning                  | Disallow multiple Ruby statements in a single ERB tag _(added 0.11.0)_                                         |
| `erb-no-output-control-flow`            | yes        | yes                                      | error                    | Prevents outputting control flow blocks _(added 0.4.0)_                                                        |
| `erb-no-output-in-attribute-name`       | yes        | no                                       | error                    | Disallow ERB output in attribute names _(added 0.9.0)_                                                         |
| `erb-no-output-in-attribute-position`   | yes        | no                                       | error                    | Disallow ERB output in attribute position _(added 0.9.0)_                                                      |
| `erb-no-raw-output-in-attribute-value`  | yes        | no                                       | error                    | Disallow `<%==` in attribute values _(added 0.9.0)_                                                            |
| `erb-no-return`                         | yes        | no                                       | cli: error, editor: info | Disallow return statements in ERB templates _(added 0.11.0)_                                                   |
| `erb-no-shadowed-block-argument`        | yes        | no                                       | error                    | Disallow block arguments that shadow an enclosing binding _(added 0.11.0)_                                     |
| `erb-no-silent-statement`               | no         | no                                       | warning                  | Disallow silent ERB statements _(added 0.9.1)_                                                                 |
| `erb-no-silent-tag-in-attribute-name`   | yes        | no                                       | error                    | Disallow ERB silent tags in HTML attribute names _(added 0.6.0)_                                               |
| `erb-no-sleep`                          | yes        | no                                       | cli: error, editor: info | Disallow `sleep` in ERB templates _(added 0.11.0)_                                                             |
| `erb-no-statement-in-script`            | yes        | no                                       | warning                  | Disallow ERB statements inside `<script>` tags _(added 0.9.0)_                                                 |
| `erb-no-then-in-control-flow`           | yes        | no                                       | warning                  | Disallow `then` in ERB control flow expressions _(added 0.9.0)_                                                |
| `erb-no-trailing-whitespace`            | yes        | yes                                      | cli: error, editor: info | Disallow trailing whitespace at end of lines. _(added 0.9.0)_                                                  |
| `erb-no-unsafe-js-attribute`            | yes        | no                                       | error                    | Disallow unsafe ERB output in JavaScript attributes _(added 0.9.0)_                                            |
| `erb-no-unsafe-raw`                     | yes        | no                                       | error                    | Disallow `raw()` and `.html_safe` in ERB output _(added 0.9.0)_                                                |
| `erb-no-unsafe-script-interpolation`    | yes        | no                                       | error                    | Disallow unsafe ERB output inside `<script>` tags _(added 0.9.0)_                                              |
| `erb-no-unused-block-argument`          | yes        | no                                       | cli: error, editor: info | Disallow unused block arguments in ERB blocks _(added 0.11.0)_                                                 |
| `erb-no-unused-expressions`             | yes        | no                                       | cli: error, editor: info | Disallow unused expressions in silent ERB tags _(added 0.9.3)_                                                 |
| `erb-no-unused-literals`                | yes        | no                                       | cli: error, editor: info | Disallow Ruby literals in ERB without output _(added 0.9.3)_                                                   |
| `erb-no-unused-local-variable`          | yes        | no                                       | cli: error, editor: info | Disallow unused local variables in ERB templates _(added 0.11.0)_                                              |
| `erb-prefer-direct-output`              | yes        | yes                                      | error                    | Prefer direct ERB output over string interpolation _(added 0.9.4)_                                             |
| `erb-prefer-do-end-blocks`              | yes        | yes                                      | error                    | Prefer `do ... end` over `{ ... }` for blocks that span multiple ERB tags _(added 0.11.0)_                     |
| `erb-prefer-each-over-map`              | yes        | no                                       | error                    | Prefer `each` over `map` when the result is discarded _(added 0.11.0)_                                         |
| `erb-prefer-explicit-conditionals`      | no         | yes                                      | error                    | Prefer explicit `if`/`unless` blocks over inline conditions in ERB output tags _(added 0.11.0)_                |
| `erb-prefer-image-tag-helper`           | yes        | no                                       | warning                  | Prefer `image_tag` helper over `<img>` with ERB expressions _(only with `framework: actionview`; added 0.4.3)_ |
| `erb-require-template-format`           | yes        | no                                       | error                    | Require a format in template file names _(added 0.11.1)_                                                       |
| `erb-require-trailing-newline`          | yes        | yes                                      | cli: error, editor: info | Enforces that all HTML+ERB template files end with exactly one trailing newline character. _(added 0.8.0)_     |
| `erb-require-whitespace-inside-tags`    | yes        | yes (some offenses unsafe-only)          | error                    | Requires whitespace around ERB tags _(added 0.4.0)_                                                            |
| `erb-right-trim`                        | yes        | yes                                      | error                    | Enforce consistent right-trimming syntax. _(added 0.7.5)_                                                      |
| `erb-strict-locals-comment-syntax`      | yes        | yes, partial                             | error                    | Enforce strict locals comment syntax. _(only with `framework: actionview`; added 0.8.8)_                       |
| `erb-strict-locals-required`            | no         | unsafe only                              | error                    | Require strict locals in Rails partials. _(only with `framework: actionview`; added 0.8.8)_                    |

### Herb (`herb-*`, 28 rules, 28 on by default)

| Rule                                      | Default on | Autofix      | Severity                 | What it flags                                                                                             |
| ----------------------------------------- | ---------- | ------------ | ------------------------ | --------------------------------------------------------------------------------------------------------- |
| `herb-component-requires-slots`           | yes        | no           | error                    | Require a `herb:slots` directive on templates using component tags. _(added 0.11.0)_                      |
| `herb-config-framework-option`            | yes        | no           | info                     | Require the `framework` option to be set in `.herb.yml`. _(added 0.11.0)_                                 |
| `herb-disable-comment-malformed`          | yes        | no           | error                    | Detect malformed `herb:disable` comments. _(added 0.8.0)_                                                 |
| `herb-disable-comment-missing-rules`      | yes        | no           | error                    | Require rule names in `herb:disable` comments. _(added 0.8.0)_                                            |
| `herb-disable-comment-no-duplicate-rules` | yes        | no           | warning                  | Disallow duplicate rule names in `herb:disable` comments. _(added 0.8.0)_                                 |
| `herb-disable-comment-no-redundant-all`   | yes        | no           | warning                  | Disallow redundant use of `all` in `herb:disable` comments. _(added 0.8.0)_                               |
| `herb-disable-comment-out-of-date`        | yes        | yes          | warning                  | Detect file-scoped `herb:disable rule N` entries whose count no longer matches the file. _(added 0.11.0)_ |
| `herb-disable-comment-unnecessary`        | yes        | no           | warning                  | Detect unnecessary `herb:disable` comments. _(added 0.8.0)_                                               |
| `herb-disable-comment-valid-rule-name`    | yes        | no           | warning                  | Validate rule names in `herb:disable` comments. _(added 0.8.0)_                                           |
| `herb-into-requires-collection`           | yes        | no           | error                    | Require `data-herb-into` to name a keyed collection. _(added 0.11.0)_                                     |
| `herb-scoped-style-no-unused-selector`    | yes        | no           | cli: error, editor: info | No unused selector in a `<style scoped>` block. _(added 0.11.0)_                                          |
| `herb-scoped-style-require-top-level`     | yes        | no           | warning                  | Require a `<style scoped>` block to be a top-level element. _(added 0.11.0)_                              |
| `herb-scoped-style-single-declaration`    | yes        | no           | warning                  | One `<style scoped>` block per file. _(added 0.11.0)_                                                     |
| `herb-slots-single-directive`             | yes        | no           | error                    | Allow one `herb:slots` directive per template. _(added 0.11.0)_                                           |
| `herb-slots-valid-components`             | yes        | no           | error                    | Validate component structure and attributes in slots templates. _(added 0.11.0)_                          |
| `herb-slots-valid-mode`                   | yes        | no           | error                    | Validate the `herb:slots` mode. _(added 0.11.0)_                                                          |
| `herb-state-directive-syntax`             | yes        | yes, partial | error                    | Spell the `herb:state` directive in its canonical form. _(added 0.11.0)_                                  |
| `herb-state-no-server-writes`             | yes        | no           | error                    | Disallow server-side writes to declared states. _(added 0.11.0)_                                          |
| `herb-state-no-shadowed-states`           | yes        | no           | error                    | Disallow block arguments that shadow a declared `herb:state` name. _(added 0.11.0)_                       |
| `herb-state-no-silent-reads`              | yes        | no           | cli: error, editor: info | Disallow bare state reads in silent tags. _(added 0.11.0)_                                                |
| `herb-state-no-unused-states`             | yes        | no           | warning                  | Disallow unused `herb:state` declarations. _(added 0.11.0)_                                               |
| `herb-state-requires-slots`               | yes        | no           | error                    | Require a `herb:slots` directive for `herb:state`. _(added 0.11.0)_                                       |
| `herb-state-single-declaration`           | yes        | no           | warning                  | One `herb:state` declaration per scope. _(added 0.11.0)_                                                  |
| `herb-state-valid-actions`                | yes        | no           | error                    | Validate `data-herb-*` action attributes. _(added 0.11.0)_                                                |
| `herb-state-valid-bindings`               | yes        | no           | error                    | Validate the kind a bound form control holds. _(added 0.11.0)_                                            |
| `herb-state-valid-declaration`            | yes        | no           | error                    | Validate `herb:state` declarations. _(added 0.11.0)_                                                      |
| `herb-state-valid-reads`                  | yes        | no           | error                    | Validate how declared states are read. _(added 0.11.0)_                                                   |
| `herb-valid-slot-names`                   | yes        | no           | error                    | Validate `data-herb-name` slot names. _(added 0.11.0)_                                                    |

### HTML (`html-*`, 48 rules, 41 on by default)

| Rule                                         | Default on | Autofix     | Severity                 | What it flags                                                                |
| -------------------------------------------- | ---------- | ----------- | ------------------------ | ---------------------------------------------------------------------------- |
| `html-allowed-script-type`                   | yes        | no          | error                    | Restrict allowed `type` attributes for `<script>` tags _(added 0.9.0)_       |
| `html-anchor-require-href`                   | yes        | no          | error                    | Requires an href attribute on anchor tags _(added 0.4.0)_                    |
| `html-aria-attribute-must-be-valid`          | yes        | no          | warning                  | Disallow invalid or unknown `aria-*` attributes. _(added 0.4.1)_             |
| `html-aria-label-is-well-formatted`          | yes        | no          | warning                  | `aria-label` must be well-formatted _(added 0.6.0)_                          |
| `html-aria-level-must-be-valid`              | yes        | no          | warning                  | `aria-level` must be between 1 and 6 _(added 0.4.3)_                         |
| `html-aria-role-heading-requires-level`      | yes        | no          | warning                  | Requires `aria-level` when supplying a `role` _(added 0.4.0)_                |
| `html-aria-role-must-be-valid`               | yes        | no          | warning                  | The `role` attribute must have a valid WAI-ARIA Role. _(added 0.4.1)_        |
| `html-attribute-double-quotes`               | yes        | yes         | warning                  | Enforces double quotes for attribute values _(added 0.4.0)_                  |
| `html-attribute-equals-spacing`              | yes        | yes         | error                    | No whitespace around `=` in HTML attributes _(added 0.6.0)_                  |
| `html-attribute-name-valid-characters`       | yes        | no          | warning                  | Requires letters, digits, and hyphens in attribute names _(added 0.11.0)_    |
| `html-attribute-values-require-quotes`       | yes        | yes         | error                    | Requires quotes around attribute values _(added 0.4.0)_                      |
| `html-avoid-both-disabled-and-aria-disabled` | yes        | no          | warning                  | Avoid using both `disabled` and `aria-disabled` attributes _(added 0.6.0)_   |
| `html-body-only-elements`                    | yes        | no          | error                    | Require content elements inside `<body>`. _(added 0.8.0)_                    |
| `html-boolean-attributes-no-value`           | yes        | yes         | error                    | Prevents values on boolean attributes _(added 0.4.0)_                        |
| `html-details-has-summary`                   | yes        | no          | warning                  | Require `<summary>` in `<details>` elements _(added 0.9.0)_                  |
| `html-head-only-elements`                    | yes        | no          | error                    | Require head-scoped elements inside `<head>`. _(added 0.8.0)_                |
| `html-iframe-has-title`                      | yes        | no          | warning                  | `iframe` elements must have a `title` attribute _(added 0.6.0)_              |
| `html-img-require-alt`                       | yes        | no          | warning                  | Requires `alt` attributes on `<img>` tags _(added 0.4.0)_                    |
| `html-input-require-autocomplete`            | yes        | no          | warning                  | Require `autocomplete` attributes on `<input>` tags. _(added 0.8.0)_         |
| `html-navigation-has-label`                  | no         | no          | warning                  | Navigation landmarks must have accessible labels _(added 0.6.0)_             |
| `html-no-abstract-roles`                     | yes        | no          | warning                  | No abstract ARIA roles _(added 0.9.0)_                                       |
| `html-no-aria-hidden-on-body`                | yes        | no          | warning                  | No `aria-hidden` on `<body>` _(added 0.9.0)_                                 |
| `html-no-aria-hidden-on-focusable`           | yes        | no          | warning                  | Focusable elements should not have `aria-hidden="true"` _(added 0.6.0)_      |
| `html-no-block-inside-inline`                | no         | no          | error                    | Prevents block-level elements inside inline elements _(added 0.4.0)_         |
| `html-no-duplicate-attributes`               | yes        | no          | error                    | Prevents duplicate attributes on HTML elements _(added 0.4.0)_               |
| `html-no-duplicate-ids`                      | yes        | no          | error                    | Prevents duplicate IDs within a document _(added 0.4.1)_                     |
| `html-no-duplicate-meta-names`               | yes        | no          | error                    | Duplicate `<meta>` name attributes are not allowed. _(added 0.8.0)_          |
| `html-no-empty-attributes`                   | yes        | no          | warning                  | Attributes must not have empty values _(added 0.7.0)_                        |
| `html-no-empty-css-rule`                     | yes        | no          | cli: error, editor: info | No empty CSS rule in a `<style>` block. _(added 0.11.0)_                     |
| `html-no-empty-headings`                     | yes        | no          | warning                  | Disallow empty heading elements _(added 0.4.0)_                              |
| `html-no-event-handler-attributes`           | no         | no          | error                    | Disallow inline event handler attributes _(added 0.11.0)_                    |
| `html-no-inline-script-elements`             | no         | no          | error                    | Disallow inline script elements _(added 0.11.0)_                             |
| `html-no-literal-nbsp`                       | yes        | yes         | cli: error, editor: info | Disallow literal non-breaking spaces, prefer `&nbsp;` _(added 0.11.0)_       |
| `html-no-nested-forms`                       | yes        | no          | error                    | Prevents nested form elements, including Rails form helpers _(added 0.11.0)_ |
| `html-no-nested-links`                       | yes        | no          | error                    | Prevents nested anchor tags _(added 0.4.0)_                                  |
| `html-no-obsolete-tags`                      | yes        | no          | warning                  | Disallow obsolete HTML tags _(added 0.11.0)_                                 |
| `html-no-positive-tab-index`                 | yes        | no          | warning                  | Avoid positive `tabindex` values _(added 0.6.0)_                             |
| `html-no-self-closing`                       | yes        | yes         | error                    | Disallow self closing tags _(added 0.6.0)_                                   |
| `html-no-space-in-tag`                       | yes        | yes         | error                    | Disallow spaces in HTML tags _(on by default since 0.10.3; added 0.8.0)_     |
| `html-no-style-attributes`                   | no         | no          | error                    | Disallow inline `style` attributes _(added 0.11.0)_                          |
| `html-no-style-elements`                     | no         | no          | error                    | Disallow inline `<style>` tags _(added 0.11.0)_                              |
| `html-no-title-attribute`                    | no         | no          | warning                  | Avoid using the `title` attribute _(added 0.6.0)_                            |
| `html-no-underscores-in-attribute-names`     | yes        | no          | warning                  | Disallow underscores in HTML attribute names _(added 0.7.0)_                 |
| `html-no-unescaped-entities`                 | yes        | unsafe only | warning                  | Disallow unescaped HTML entities _(added 0.9.3)_                             |
| `html-no-unknown-tag`                        | yes        | no          | warning                  | Disallow unknown HTML tags _(added 0.9.3)_                                   |
| `html-require-closing-tags`                  | yes        | no          | error                    | Require closing tags for non-void HTML elements _(added 0.9.0)_              |
| `html-require-script-nonce`                  | yes        | no          | error                    | Require a `nonce` attribute on inline scripts _(added 0.9.3)_                |
| `html-tag-name-lowercase`                    | yes        | yes         | error                    | Enforces lowercase tag names in HTML _(added 0.4.0)_                         |

### Parser (`parser-*`, 1 rules, 1 on by default)

| Rule               | Default on | Autofix | Severity | What it flags                                                |
| ------------------ | ---------- | ------- | -------- | ------------------------------------------------------------ |
| `parser-no-errors` | yes        | no      | error    | Disallow parser errors in HTML+ERB documents _(added 0.5.0)_ |

### Source (`source-*`, 1 rules, 1 on by default)

| Rule                 | Default on | Autofix | Severity | What it flags                                       |
| -------------------- | ---------- | ------- | -------- | --------------------------------------------------- |
| `source-indentation` | yes        | yes     | error    | Indent with spaces instead of tabs. _(added 0.9.3)_ |

### SVG (`svg-*`, 2 rules, 2 on by default)

| Rule                          | Default on | Autofix | Severity | What it flags                                                             |
| ----------------------------- | ---------- | ------- | -------- | ------------------------------------------------------------------------- |
| `svg-no-deprecated-tags`      | yes        | no      | error    | Disallow SVG elements removed from SVG 2 _(added 0.11.0)_                 |
| `svg-tag-name-capitalization` | yes        | yes     | error    | Enforces proper camelCase capitalization for SVG elements _(added 0.4.2)_ |

### Turbo (`turbo-*`, 2 rules, 2 on by default)

| Rule                                  | Default on | Autofix | Severity | What it flags                                                                  |
| ------------------------------------- | ---------- | ------- | -------- | ------------------------------------------------------------------------------ |
| `turbo-permanent-no-misleading-value` | yes        | yes     | error    | Disallow misleading values on `data-turbo-permanent` _(added 0.11.0)_          |
| `turbo-permanent-require-id`          | yes        | no      | error    | Require `id` attribute on elements with `data-turbo-permanent` _(added 0.9.0)_ |

### UJS (`ujs-*`, 4 rules, 4 on by default)

| Rule                            | Default on | Autofix | Severity | What it flags                                                                             |
| ------------------------------- | ---------- | ------- | -------- | ----------------------------------------------------------------------------------------- |
| `ujs-no-remote-attribute`       | yes        | no      | warning  | Disallow the deprecated `data-remote` attribute and helper option _(added 0.11.0)_        |
| `ujs-prefer-turbo-confirm`      | yes        | no      | warning  | Prefer `data-turbo-confirm` over the deprecated `data-confirm` _(added 0.11.0)_           |
| `ujs-prefer-turbo-method`       | yes        | no      | warning  | Prefer `data-turbo-method` over the deprecated `data-method` _(added 0.11.0)_             |
| `ujs-prefer-turbo-submits-with` | yes        | no      | warning  | Prefer `data-turbo-submits-with` over the deprecated `data-disable-with` _(added 0.11.0)_ |
