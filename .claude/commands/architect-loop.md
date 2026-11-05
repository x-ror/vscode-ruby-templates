---
description: One step of a staged rust-architect review of server/src; run as /loop /architect-loop
argument-hint: "[restart]"
---

Report and progress live in `.claude/reviews/architect.md` (git-ignored). If it is missing, or `$ARGUMENTS` is `restart`, create it with this checklist and an empty `## Findings` section below it:

```markdown
# Architect review of server/src

- [ ] 1. Request flow and state: lsp.rs, lsp/, rails/project.rs, options.rs, lib.rs, config.rs, util.rs, main.rs
- [ ] 2. Diagnostics: checks.rs, erb/html_lint.rs, erb/tags.rs, html.rs, ruby/strict_locals.rs, code_context.rs
- [ ] 3. Haml parsing: haml/parser.rs, haml.rs, haml/ruby.rs
- [ ] 4. Slim parsing: slim/parser.rs, slim/ruby.rs
- [ ] 5. ERB and Ruby: erb.rs, erb/ruby.rs, ruby.rs, ruby/prism.rs, ruby/generated.rs
- [ ] 6. Completion and hover: completion.rs, lsp/complete.rs, erb/complete.rs, slim/complete.rs, slim/hover.rs, helpers.rs, html.rs, rails/helper_defs.rs, rails/ivars.rs, rails/controller_classes.rs, rails/i18n.rs, rails/routes.rs, rails/stimulus.rs, rails/assets.rs, rails/schema.rs, rails/model_columns.rs, rails/controllers.rs, rails/slots.rs
- [ ] 7. Partials, references, rename: rails/render_refs.rs, rails/render_call.rs, rails/partials.rs, partial_rename.rs, rails/paths.rs, inflect.rs
- [ ] 8. Formatting and indentation: format.rs, format/erb.rs, format/haml.rs, format/slim.rs, haml/indent.rs, slim/indent.rs
- [ ] 9. Outline: outline.rs, outline/slim.rs, lang.rs
- [ ] 10. Command line: cli.rs, bin/ruby-templates.rs
```

Then do exactly one stage:

1. If a line is marked `- [~]`, a stage is already running: do nothing this tick.
2. Take the first `- [ ]` line and mark it `- [~]`. If none is left, write a `## Summary` at the end of the report (the top 5 recommendations across all stages, ranked by impact / effort, with `file:line`), tell me the review is done, and stop the loop.
3. Run the rust-architect agent on that stage's files (paths under `server/src/`). Tell it to read the `## Findings` already in the report first, so it doesn't repeat them, and to note cross-stage issues there instead of re-reviewing other modules.
4. Append its output under `## Findings` as `### <stage title>`, as-is, and mark the line `- [x]`.
5. Reply with one line: the stage done, its number of findings, and the highest-impact one.

Never edit code under `server/`. This loop only reviews.
