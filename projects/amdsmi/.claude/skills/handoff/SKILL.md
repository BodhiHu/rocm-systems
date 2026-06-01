---
name: handoff
description: "Use when handing work from one amd-smi agent to another (router→planning/dev/review, planning→dev, dev→review) or compacting a long session into a fresh one. Produces a compact handoff doc referencing artifacts by path instead of duplicating them."
---

# Handoff — amd-smi

The single hand-off contract between amd-smi agents and across session boundaries.
One shape for every hop, so no agent restates its own dispatch/return format.

**Core principle:** Reference artifacts by path. Never paste file contents, diffs,
or plans into the handoff — point to them. Redact secrets.

## When to Use

- Router agent routes a request to Planning, Development, or Review
- Planning dispatches a task to the Development agent
- Development returns results to Planning, or hands a branch to Review
- A session is getting long and work must continue in a fresh session

**Don't use when:** answering a trivial inline request the router can handle itself.

## Where the Doc Goes

Write to the OS temp dir, **not** the workspace:

```bash
HANDOFF="${TMPDIR:-/tmp}/amdsmi-handoff-$(date +%Y%m%d-%H%M%S).md"
```

Pass the path to the receiver. Workspace stays clean (no stray handoff files in git).

## Handoff Template

```markdown
# Handoff: <one-line goal>

**From:** <router | planning | development>  **To:** <planning | development | review>
**Date:** <YYYY-MM-DD>

## Goal
<one or two sentences — what the receiver must accomplish>

## Scope
- In: <files/dirs the receiver may change>
- Out: <files/dirs that are OFF LIMITS>

## Constraints
- <e.g., do not regenerate the wrapper unless adding a C API function>
- <project rules: TDD first, verification-before-completion, no push w/o approval>

## Artifacts (by path — do NOT inline)
- Spec: ${TMPDIR:-/tmp}/amdsmi-agent-specs/<file>.md
- Plan: .claude/context/plans/<file>.md (task N, lines X–Y)
- Worktree: <abs path>
- Related: <PR URL, issue, prior handoff path>

## Suggested Skills
- <skill the receiver should load — e.g., test-driven-development, systematic-debugging>

## Expected Return
- <what the sender needs back — STATUS, files changed, tests run, blockers>
```

## Expected Return (receiver → sender)

Receivers report back in this shape (replaces per-agent return formats):

```markdown
STATUS: DONE | BLOCKED | NEEDS_CONTEXT
FILES CHANGED:
- <path>:<lines> — <summary>
TESTS RUN:
- <command> → <result>
VERIFICATION:
- <what was run from verification-before-completion>
BLOCKERS (if any):
- <description + which skill/agent resolves it>
```

## Common Mistakes

| Mistake | Fix |
|---------|-----|
| Pasting the plan/diff into the handoff | Reference it by path |
| Writing the handoff into the workspace | Write to `${TMPDIR:-/tmp}` |
| Omitting the Out-of-scope list | Always state what NOT to touch |
| Including tokens/passwords in Artifacts | Redact; reference the secret's location instead |
| Restating a skill's steps | List it under Suggested Skills — the receiver loads it |
