# Agent Router + Handoff Contract — Design Spec

**Date:** 2026-05-31
**Branch:** `users/marifamd/skills_rework`
**Status:** Approved (build)

## Problem

The amd-smi agent system is a triumvirate — Planning, Development, Review — each
invokable directly by the user. Two gaps:

1. **No single front door.** The user must know which of the three to pick. A
   trivial question still requires choosing an agent built for multi-step work.
2. **Divergent, duplicated handoff formats.** Three different hand-off shapes
   exist as prose inside the agent files:
   - Planning → Development: "Subagent Dispatch Template"
   - Development → Planning: "Mode B structured return format"
   - Development/Planning → Review: implicit (the review agent's input contract)

   Each is restated in the agent files, so the parallel-dispatch, verification,
   and hand-off *descriptions* are duplicated across 2–3 files (agent-files-vs-skills
   duplication). Changing one role means editing it in several places.

## Goals

- Add one thin **router** agent as the single entry point that triages intent and
  hands off to the right specialist (or answers trivial requests inline).
- Add one **handoff** skill that is the single hand-off contract used at every hop
  (router→specialist, planning→dev, dev→review, any agent→fresh session).
- Refactor the three existing agents to **link** the handoff / parallel-dispatch /
  verification skills instead of **restating** them.

## Non-Goals

- No change to the 8 review subagents.
- No change to the skill taxonomy (unprefixed generic vs `amdsmi-` domain) — that
  split is intentional per `writing-skills`.
- The router does NOT plan, implement, or review. It only triages + routes + passes
  context. Any duplication of specialist logic is a design failure.

## Design

### A. Router agent — `.github/agents/amdsmi.agent.md`

Single front door. Intent triage:

| User intent | Action |
|-------------|--------|
| Trivial question / one-line lookup | Answer inline — do NOT spin up the triumvirate |
| Feature / defect / refactor (multi-step) | Hand off to **Planning** |
| "Implement this specific task" | Hand off to **Development** |
| "Review this branch / PR" | Hand off to **Review** |

Hands off using the `handoff` skill. Never duplicates planning/dev/review logic.

### B. Handoff skill — `.claude/skills/handoff/SKILL.md`

One contract for all hops:

- Writes a compact handoff doc to the OS temp dir (NOT the workspace).
- Includes a "suggested skills" section so the receiver loads the right skills.
- References artifacts by path/URL — never duplicates file contents.
- Redacts secrets.
- Fields: GOAL, SCOPE (files in / out), CONSTRAINTS, ARTIFACTS (by path),
  SUGGESTED SKILLS, EXPECTED RETURN.

Replaces the Planning "Subagent Dispatch Template", the Development "Mode B return
format", and the implicit review input contract — those become references to this
skill.

### C. Agent refactor (link-not-restate)

| Agent | Change |
|-------|--------|
| Planning | Replace inline "Subagent Dispatch Template" with a link to `handoff`. Keep workflow + iteration loop. |
| Development | Replace inline "Mode B return format" with a link to `handoff`. Keep modes + skill map. |
| Review | Reference `handoff` for the input contract it expects. |

## Files

| Action | File |
|--------|------|
| Create | `.github/agents/amdsmi.agent.md` (router) |
| Create | `.claude/skills/handoff/SKILL.md` |
| Modify | `.github/agents/amdsmi-planning.agent.md` (link handoff) |
| Modify | `.github/agents/amdsmi-development.agent.md` (link handoff) |
| Modify | `.github/agents/amdsmi-review.agent.md` (reference handoff input) |
| Modify | `.claude/context/specs/` (this file) |

## Verification

- Router file parses (valid frontmatter, `agents:` lists the three specialists).
- `handoff` skill passes `writing-skills` checklist (description starts "Use when",
  kebab name, under token budget, single example).
- No agent file still contains a full inline copy of a hand-off template — each
  references the `handoff` skill instead.
