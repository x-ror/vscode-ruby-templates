---
description: Advance docs/LINT_PLAN.md by one verifiable step through the agent cycle; run it under /loop
argument-hint: "[stage, e.g. 1a; default: the first one not done]"
---

Work on `docs/LINT_PLAN.md`, one step per run. Read it and `docs/DECISIONS.md` first; they are the spec. Stage: $ARGUMENTS (if empty: the first row of "Order and status" that isn't `done`).

## Pick the step

1. Stage `in review` (its PR is open): `gh pr view` it. Merged → mark it `done` on `main` and pick the next stage. Not merged → stop: "waiting for the merge of <PR>". Don't start the next stage on top of an unmerged one.
2. Otherwise check out the stage's branch, `lint/<stage>-<slug>` (e.g. `lint/1a-slim-errors`), from an up-to-date `main` if it doesn't exist.
3. Choose the next unit of the stage: one error message, one syntax item, one formatter rule, one diagnostic code to fold or remove. It must end in a state the oracle harness or a test can measure. The stage's status row tracks progress: `in progress: <what's done>`.

Stop and ask me, without writing code, when the step needs something the docs leave open: an item in "Not from official sources" or "Other items" that isn't decided (the 3b ERB whitespace question), a behaviour the plan says is "checked against the oracle, not guessed" while the oracle can't run here, or a new dependency the plan doesn't already name.

## Run the cycle

1. **feature-dev**: implement the unit. Give it the stage text, the official source it must cite, and the oracle's output for the case.
2. **code-reviewer** on the diff. Send confirmed bugs back to feature-dev; repeat until none are left.
3. **refactorer** on the touched code.
4. **test-writer**: tests where `CLAUDE.md` "Where tests go" says; each seen failing first.
5. Run the `CLAUDE.md` "Checks" for what changed, plus `bun test/oracle.js <language>` once stage 0 exists. A failing check goes back to step 1; never commit over it.

## Finish the run

- Commit per `CLAUDE.md` (Conventional Commits, one logical change, `style:` separately), update the stage's status row and, when a doc in "Docs that change with the code" is affected, that doc in the same commit. Push the branch.
- Stage complete (its "Done when" or "Compare" target reached): open a PR to `main` with the harness report in the description, set the row to `in review`, stop.
- Report in a few lines: the unit done, the checks and oracle numbers, what's next or what blocks.
