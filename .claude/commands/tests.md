---
description: Write only the tests worth having via the test-writer agent
argument-hint: "[file, function or area; default: current diff]"
---

Use the test-writer agent to write tests for: $ARGUMENTS (if empty: the code changed in the uncommitted diff and the current branch against main)

Relay which tests it added, what bug each one guards against, and which candidates it rejected.
