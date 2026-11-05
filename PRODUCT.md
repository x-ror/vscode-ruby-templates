# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Astro + Starlight in `site/`, deployed to GitHub Pages from this repository. Code examples are highlighted with the extension's own TextMate grammars (`syntaxes/`) through Expressive Code.

## Users

Primary: a Rails developer deciding whether to install the extension. They work in `app/views` daily, know ERB and usually Haml or Slim, and compare this extension with Ruby LSP and other ERB/Haml/Slim extensions. They need to see quickly what it does in their own templates and that it is trustworthy.

Secondary (served by the same pages, not designed around): existing users looking up a setting, a diagnostic code or a troubleshooting step, and teams wiring the `ruby-templates` CLI into CI.

## Product Purpose

Ruby Templates is one VS Code extension for Rails views in ERB, Haml and Slim: highlighting, errors as you type, whitespace-only formatting, and navigation and completion that understand the Rails app (partials, components, helpers, routes, instance variables, translations, Stimulus). The site is the canonical documentation; success is a Rails developer understanding what it does for their templates and installing it.

## Positioning

- No gems to install: the checks and formatter are a Rust language server bundled with the extension.
- Errors follow Erubi, Rails, Haml 7 and Slim 5 and use those projects' own messages, at the same line and column.
- Formatting changes only whitespace and verifies the template still compiles to the same thing; when unsure it leaves the file alone.
- Works alongside Ruby LSP (Ruby LSP for `.rb`, this extension for templates).
- The same checks and formatter run in CI through the `@x-ror/ruby-templates` npm CLI.

## Operating Context

Evaluated in a browser next to VS Code, often after landing from the Marketplace or a search. Installed from the VS Code Marketplace or Open VSX; works immediately on opening a Rails view. Requires VS Code 1.100+ on Linux (x64, arm64), macOS (Apple silicon, Intel) or Windows (x64).

## Capabilities and Constraints

- Languages: ERB (with host variants such as `.js.erb`, `.yml.erb`), Haml, Slim, Skim; Liquid for highlighting and editing only.
- Every diagnostic has a stable code listed in `package.json` (`rubyTemplates.<lang>.disabledDiagnostics`); the site documents each one.
- `README.md` stays the Marketplace page but becomes a short overview linking to the site; the site holds the full documentation.
- `CHANGELOG.md` is rendered on the site from the file itself.
- English only.

## Brand Commitments

- Name: "Ruby Templates".
- Icon: `images/icon.png`. Everything else (colour, type, voice beyond the README's plain, factual tone) is open.

## Evidence on Hand

- Screenshots: `images/screenshots/{diagnostics,completion,hover,slim}.png`.
- Demo Rails apps: `demo/rails_app`, `demo/shop_app`, `demo/no_rails`.
- Grammar samples: `test/sample.*`.
- Marketplace, Open VSX and npm listings (badges in README).
- No testimonials, user counts, benchmarks or customer logos exist; do not invent them.

## Product Principles

- Show it in a real template: claims are backed by highlighted code or a screenshot from the extension itself.
- Say exactly what is and isn't supported; limits are documented, not hidden.
- Rails conventions first: examples use real Rails idioms (`render`, strict locals, `form_with`, Turbo, Stimulus).
- Nothing to install beyond the extension is a feature to make visible.

## Accessibility & Inclusion

WCAG 2.2 AA is required: sufficient contrast in light and dark, visible focus, full keyboard use, reduced-motion support, alt text for every screenshot, semantic landmarks and correct HTML elements. Custom CSS follows BEM naming.
