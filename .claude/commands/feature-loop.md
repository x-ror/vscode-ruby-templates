---
description: One step of a staged feature pipeline (implement, tests, review, fix, checks); run as /loop /feature-loop <feature>
argument-hint: "<feature description> | restart <feature description>"
---

Feature: $ARGUMENTS (drop a leading `restart`).

Progress lives in `.claude/reviews/feature.md` (git-ignored). If it is missing, its `Feature:` line differs from the feature above, or `$ARGUMENTS` starts with `restart`, create it with the feature on a `Feature:` line, this checklist, and an empty `## Log` section below it:

```markdown
- [ ] 1. Implement: feature-dev agent
- [ ] 2. Tests: test-writer agent
- [ ] 3. Review: code-reviewer agent
- [ ] 4. Fix: feature-dev agent on confirmed findings
- [ ] 5. Checks: run the CLAUDE.md checks that cover the changed files
```

Then do exactly one stage:

1. If a line is marked `- [~]`, a stage is already running: do nothing this tick.
2. Take the first `- [ ]` line and mark it `- [~]`. If none is left, write a `## Summary` (what changed, tests added, findings fixed or left, check results), tell me the feature is done, and stop the loop.
3. Run the stage:
   - Implement: feature-dev agent on the feature. Tell it to follow CLAUDE.md and not to commit.
   - Tests: test-writer agent on the uncommitted diff, with the stage 1 log entry as context.
   - Review: code-reviewer agent on the uncommitted diff. Verify each finding against the code yourself and mark it confirmed or rejected.
   - Fix: if stage 3 has no confirmed findings, note "nothing to fix". Otherwise give feature-dev only the confirmed findings. If the fix is more than trivial, reset stage 3 and 4 to `- [ ]` so the diff is reviewed again, at most twice in total; after that leave remaining findings for me.
   - Checks: run the checks yourself. On a failure, have feature-dev fix it and rerun; if it still fails after two attempts, record the output and move on.
4. Append the stage's result under `## Log` as `### <stage title>`, concisely, and mark the line `- [x]`.
5. Reply with one line: the stage done and its outcome.

Never commit, push or open a PR. I review the diff and commit myself.
