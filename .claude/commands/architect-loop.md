---
description: One step of a staged rust-architect review of server/src; run as /loop /architect-loop
argument-hint: "[restart]"
---

Report and progress live in `.claude/reviews/architect.md` (git-ignored). If it is missing, or `$ARGUMENTS` is `restart`, create it with this checklist and an empty `## Findings` section below it:

```markdown
# Architect review of server/src

- [ ] 1. Request flow and state: lsp.rs, project.rs, lib.rs, config.rs, util.rs, main.rs
- [ ] 2. Diagnostics: checks.rs, html_lint.rs, tags.rs, html.rs, strict_locals.rs, code_context.rs
- [ ] 3. Haml parsing: haml_parser.rs, haml.rs, haml_ruby.rs
- [ ] 4. Slim parsing: slim_parser.rs, slim_ruby.rs
- [ ] 5. ERB and Ruby: erb.rs, erb_ruby.rs, ruby.rs, prism.rs, generated_ruby.rs
- [ ] 6. Completion and hover: completion.rs, erb_complete.rs, slim_complete.rs, slim_hover.rs, helpers.rs, helper_defs.rs, ivars.rs, i18n.rs, routes.rs, stimulus.rs, assets.rs, schema.rs, controllers.rs, slots.rs
- [ ] 7. Partials, references, rename: partials.rs, partial_rename.rs, rails_paths.rs
- [ ] 8. Formatting and indentation: format.rs, erb_format.rs, haml_format.rs, slim_format.rs, haml_indent.rs, slim_indent.rs
- [ ] 9. Outline: outline.rs, slim_outline.rs, lang.rs
- [ ] 10. Command line: cli.rs, bin/ruby-templates.rs
```

Then do exactly one stage:

1. If a line is marked `- [~]`, a stage is already running: do nothing this tick.
2. Take the first `- [ ]` line and mark it `- [~]`. If none is left, write a `## Summary` at the end of the report (the top 5 recommendations across all stages, ranked by impact / effort, with `file:line`), tell me the review is done, and stop the loop.
3. Run the rust-architect agent on that stage's files (paths under `server/src/`). Tell it to read the `## Findings` already in the report first, so it doesn't repeat them, and to note cross-stage issues there instead of re-reviewing other modules.
4. Append its output under `## Findings` as `### <stage title>`, as-is, and mark the line `- [x]`.
5. Reply with one line: the stage done, its number of findings, and the highest-impact one.

Never edit code under `server/`. This loop only reviews.
