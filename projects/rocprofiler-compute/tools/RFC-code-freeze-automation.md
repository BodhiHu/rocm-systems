# RFC: Automated Code Freeze Management for rocprofiler-compute

**Status**: Draft
**Author**: Xuan Chen
**Created**: 2026-03-11
**Target Release**: TBD

---

## Table of Contents

1. [Summary](#summary)
2. [Motivation](#motivation)
3. [Goals](#goals)
4. [Non-Goals](#non-goals)
5. [Design](#design)
6. [Implementation Phases](#implementation-phases)
7. [Questions for Repo Admins](#questions-for-repo-admins)
8. [Alternatives Considered](#alternatives-considered)
9. [Security Considerations](#security-considerations)
10. [Success Metrics](#success-metrics)
11. [Timeline](#timeline)
12. [References](#references)

---

## Summary

This RFC proposes a GitHub Actions-based automation system to manage code freeze periods for the rocprofiler-compute project within the rocm-systems monorepo. The system will automate branch management, PR routing, and enforcement during release preparation periods.

---

## Motivation

### Current State (Manual Process)

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         CURRENT MANUAL PROCESS                          │
├─────────────────────────────────────────────────────────────────────────┤
│ 1. Developer manually creates temp branch                               │
│ 2. Developer communicates code freeze via Teams                         │
│ 3. PR reviewers must manually check if PR should go to temp or develop  │
│ 4. Risk of PRs accidentally merged to develop during freeze             │
│ 5. Developer manually merges temp → develop after branching             │
│ 6. No audit trail of freeze periods                                     │
└─────────────────────────────────────────────────────────────────────────┘
```

Example of existing manual temp branch: `users/vedithal/rocprofiler-compute-temp-develop`

### Problems

| Problem | Impact |
|---------|--------|
| **Human error** | PRs can accidentally be merged to develop during freeze |
| **Communication overhead** | Manual announcements, repeated reminders |
| **Inconsistent enforcement** | Depends on reviewer awareness |
| **No audit trail** | Difficult to track freeze history and metrics |
| **Coordination burden** | Manual branch creation/merging |

---

## Goals

1. **Automate branch lifecycle**: Create/delete temp branches automatically
2. **Enforce code freeze**: Prevent or redirect PRs during freeze periods
3. **Allow authorized bypass**: Critical fixes can still target develop with tech lead approval
4. **Minimize disruption**: Integrate with existing review workflow
5. **Scope to rocprofiler-compute**: Only affect this project within the monorepo
6. **Provide audit trail**: Track freeze periods and PR routing decisions

---

## Non-Goals

- Automating the actual release branch creation (owned by DevOps)
- Enforcing freeze on other projects in rocm-systems
- Replacing existing CI/CD pipelines

---

## Design

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    AUTOMATED RELEASE SCHEDULE MANAGEMENT                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  ┌──────────────────┐    ┌───────────────────┐    ┌──────────────────────┐  │
│  │  TRIGGER LAYER   │    │   STATE LAYER     │    │   ENFORCEMENT LAYER  │  │
│  ├──────────────────┤    ├───────────────────┤    ├──────────────────────┤  │
│  │ • Manual dispatch│    │ .code-freeze.json │    │ • PR Check workflow  │  │
│  │ • Webhook (Jira) │───>│  - active: bool   │───>│ • Auto-retarget PRs  │  │
│  │ • Scheduled      │    │  - release: "7.13"│    │ • CODEOWNERS bypass  │  │
│  │                  │    │  - temp_branch    │    │ • Merge gate         │  │
│  └──────────────────┘    │  - start_date     │    └──────────────────────┘  │
│                          │  - end_date       │                              │
│                          └───────────────────┘                              │
│                                   │                                         │
│                                   ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                         LIFECYCLE ACTIONS                            │   │
│  ├──────────────────────────────────────────────────────────────────────┤   │
│  │ START FREEZE:           │ END FREEZE:                                │   │
│  │ 1. Create temp branch   │ 1. Create PR: temp → develop               │   │
│  │ 2. Update state file    │ 2. Update state file (active: false)       │   │
│  │ 3. Post notification    │ 3. Post notification                       │   │
│  │ 4. Retarget open PRs    │ 4. Retarget PRs back to develop            │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

### File Structure

```
.github/
├── CODEOWNERS                                  # Repo-level CODEOWNERS (required location)
└── workflows/                                  # Repo root (required by GitHub)
    ├── rocprofiler-compute-freeze-manage.yml   # Start/end code freeze
    └── rocprofiler-compute-freeze-enforce.yml  # PR check and auto-retarget

projects/rocprofiler-compute/
├── .code-freeze.json                           # State file (version controlled)
└── docs/
    └── examples/                               # Reference implementations
        ├── rocprofiler-compute-freeze-manage.yml
        ├── rocprofiler-compute-freeze-enforce.yml
        └── code-freeze.json.template
```

> **Note**: GitHub only reads CODEOWNERS from `.github/CODEOWNERS`, repo root `CODEOWNERS`, or `docs/CODEOWNERS`. Project-level CODEOWNERS files (e.g., `projects/*/CODEOWNERS`) are **not processed** by GitHub.

### Components

#### 1. State File (`.code-freeze.json`)

Located at `projects/rocprofiler-compute/.code-freeze.json`, this file tracks the current freeze state:

```json
{
  "active": true,
  "release": "7.13",
  "temp_branch": "rocprofiler-compute-temp-develop-7.13",
  "start_date": "2026-03-15T00:00:00Z",
  "started_by": "<user>",
  "reason": "Release preparation",
  "end_date": null,
  "ended_by": null,
  "bypass_approvers": ["@ROCm/rocprofiler-compute-leads"],
  "monitored_paths": [
    "projects/rocprofiler-compute/**"
  ],
  "history": [
    {
      "release": "7.12",
      "start_date": "2026-01-15T00:00:00Z",
      "end_date": "2026-02-01T00:00:00Z",
      "temp_branch": "users/vedithal/rocprofiler-compute-temp-develop",
      "prs_retargeted": 12
    }
  ]
}
```

#### 2. CODEOWNERS Configuration

Add to `.github/CODEOWNERS` (at repo root level):

```
# =============================================================================
# rocprofiler-compute
# =============================================================================
# Code freeze bypass approvers - during code freeze, PRs targeting develop
# require approval from this team to bypass the freeze and merge directly.
projects/rocprofiler-compute/ @ROCm/rocprofiler-compute-leads
```

> **Important**: CODEOWNERS must be placed in `.github/CODEOWNERS`, not in the project subdirectory. GitHub ignores CODEOWNERS files in other locations. The path pattern `projects/rocprofiler-compute/` ensures the rule only applies to files within our project, not the entire repository.

#### 3. GitHub Workflows

**Location constraint**: GitHub only processes workflows from `.github/workflows/` at the repository root. Workflows in subdirectories are ignored.

**Proposed location**: `.github/workflows/` with path filters and naming convention:
- `rocprofiler-compute-freeze-manage.yml`
- `rocprofiler-compute-freeze-enforce.yml`

### Workflow Specifications

#### A. `rocprofiler-compute-freeze-manage.yml`

**Purpose**: Start or end code freeze periods

**Triggers**:
- `workflow_dispatch` (manual) - POC phase
- `repository_dispatch` (webhook) - Future Jira integration
- `schedule` (cron) - Future scheduled freezes

**Inputs**:

| Input | Type | Required | Description |
|-------|------|----------|-------------|
| `action` | choice | Yes | `start` or `end` |
| `release_version` | string | Yes for start | e.g., `7.13` |
| `reason` | string | No | Context for the freeze |
| `dry_run` | boolean | No | Preview changes without applying |

**Actions on `start`**:
1. Validate no active freeze exists
2. Create branch `rocprofiler-compute-temp-develop-{version}` from `develop`
3. Update `.code-freeze.json` with active state
4. Find open PRs targeting `develop` that touch `projects/rocprofiler-compute/**`
5. Retarget those PRs to the temp branch
6. Comment on each retargeted PR explaining the situation
7. Commit state file changes

**Actions on `end`**:
1. Validate freeze is active
2. Create PR from temp branch to `develop`
3. Update `.code-freeze.json` with inactive state
4. Comment on the merge PR with summary
5. Optionally retarget any PRs on temp branch back to develop

#### B. `rocprofiler-compute-freeze-enforce.yml`

**Purpose**: Enforce freeze rules on incoming PRs

**Triggers**:
```yaml
on:
  pull_request:
    types: [opened, synchronize, reopened, ready_for_review]
    paths:
      - 'projects/rocprofiler-compute/**'
    branches:
      - develop

  # Re-check when reviews are submitted (for bypass approval)
  pull_request_review:
    types: [submitted]
```

> **Note on `pull_request_review` trigger**: GitHub does not support `paths:` filters on `pull_request_review` events. To prevent this workflow from running on unrelated PRs (and potentially blocking other projects), we implement an early `path-filter` job that:
> 1. Checks if the PR touches `projects/rocprofiler-compute/**` files
> 2. Sets `should_run=false` for `pull_request_review` events on unrelated PRs
> 3. All downstream jobs skip when `should_run=false`
>
> This ensures the workflow only affects rocprofiler-compute PRs, even when triggered by review events.

**Enforcement Logic**:

| Check | Behavior |
|-------|----------|
| Freeze inactive | Pass - normal workflow |
| Freeze active + no bypass | Fail - auto-retarget to temp branch |
| Freeze active + tech lead approved | Pass - allow merge to develop |

**Workflow Job Flow**:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    ENFORCEMENT WORKFLOW JOB FLOW                            │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│  Event: pull_request OR pull_request_review                                 │
│                    │                                                        │
│                    ▼                                                        │
│  ┌─────────────────────────────────────────────────────────────┐            │
│  │ path-filter job                                             │            │
│  │ • Check if PR targets 'develop'                             │            │
│  │ • Check if PR touches projects/rocprofiler-compute/**       │            │
│  │ • For pull_request_review: skip if doesn't touch project    │            │
│  └─────────────────────────────────────────────────────────────┘            │
│                    │                                                        │
│         ┌─────────┴─────────┐                                               │
│         │                   │                                               │
│   should_run=true    should_run=false                                       │
│         │                   │                                               │
│         ▼                   ▼                                               │
│  ┌──────────────┐    (all jobs skip - no impact on other projects)          │
│  │check-freeze- │                                                           │
│  │status job    │                                                           │
│  └──────────────┘                                                           │
│         │                                                                   │
│    ┌────┴────┐                                                              │
│    │         │                                                              │
│ active    not active                                                        │
│    │         │                                                              │
│    ▼         ▼                                                              │
│ enforce   no-freeze                                                         │
│ -freeze   job (pass)                                                        │
│ job                                                                         │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

**Pseudocode**:
```
IF PR targets 'develop' branch
  AND PR touches 'projects/rocprofiler-compute/**' files
  AND freeze is active (from .code-freeze.json)
THEN
  IF PR has approval from @ROCm/rocprofiler-compute-leads
    THEN allow (check passes)
  ELSE
    Add comment explaining freeze and how to retarget or request bypass
    Check fails with informative message
    (Developer must manually retarget PR to temp branch)
```

### Sequence Diagrams

#### Starting Code Freeze

```
Tech Lead                GitHub Action              Repository
    │                          │                        │
    │ dispatch(start, 7.13)    │                        │
    │─────────────────────────>│                        │
    │                          │ create branch          │
    │                          │───────────────────────>│
    │                          │ update .code-freeze.json
    │                          │───────────────────────>│
    │                          │ find open PRs          │
    │                          │───────────────────────>│
    │                          │ retarget PRs to temp   │
    │                          │───────────────────────>│
    │                          │ post comments          │
    │<─────────────────────────│                        │
    │     summary report       │                        │
```

#### PR During Code Freeze

```
Developer               PR Check Action            Tech Lead
    │                          │                        │
    │ open PR → develop        │                        │
    │─────────────────────────>│                        │
    │                          │ check freeze state     │
    │                          │ (active: true)         │
    │                          │                        │
    │ <── check FAILS          │                        │
    │     + comment with       │                        │
    │     retarget instructions│                        │
    │                          │                        │
    │ ── retarget to temp ───> │                        │
    │    (manual action)       │                        │
    │                          │                        │
    │ OR (critical fix needed):│                        │
    │ request bypass  ─────────────────────────────────>│
    │                          │                        │
    │ <───────────────────────────── approve PR (review)
    │                          │                        │
    │                          │ check: tech lead approved
    │ <── check passes         │                        │
```

### PR Workflow During Freeze

```
┌─────────────────────────────────────────────────────────────────────────┐
│                     DEVELOPER PR WORKFLOW (FREEZE ACTIVE)               │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  Developer opens PR targeting 'develop'                                 │
│           │                                                             │
│           ▼                                                             │
│  ┌─────────────────────────────────────────┐                            │
│  │ Workflow checks .code-freeze.json       │                            │
│  │ Result: freeze is ACTIVE                │                            │
│  └─────────────────────────────────────────┘                            │
│           │                                                             │
│           ▼                                                             │
│  ┌─────────────────────────────────────────┐                            │
│  │ Check FAILS with informative message    │                            │
│  │ Comment added with retarget instructions│                            │
│  └─────────────────────────────────────────┘                            │
│           │                                                             │
│           ├──────────────── Normal case ───────────────────┐            │
│           │                                                ▼            │
│           │                               ┌──────────────────────────┐  │
│           │                               │ PR merged to temp branch │  │
│           │                               │ (standard review process)│  │
│           │                               └──────────────────────────┘  │
│           │                                                             │
│           └──── Critical fix needed ────┐                               │
│                                         ▼                               │
│                       ┌──────────────────────────────────────────┐      │
│                       │ Developer requests bypass:               │      │
│                       │ 1. Re-target PR to 'develop'             │      │
│                       │ 2. Request review from tech lead         │      │
│                       │ 3. Tech lead approves (CODEOWNERS rule)  │      │
│                       └──────────────────────────────────────────┘      │
│                                         │                               │
│                                         ▼                               │
│                       ┌──────────────────────────────────────────┐      │
│                       │ Workflow re-checks:                      │      │
│                       │ - Freeze active: YES                     │      │
│                       │ - Tech lead approved: YES                │      │
│                       │ - Result: CHECK PASSES                   │      │
│                       └──────────────────────────────────────────┘      │
│                                         │                               │
│                                         ▼                               │
│                       ┌──────────────────────────────────────────┐      │
│                       │ PR merged to develop (critical fix)      │      │
│                       └──────────────────────────────────────────┘      │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Branch Naming Convention

| Branch Type | Pattern | Example |
|-------------|---------|---------|
| Temp develop | `rocprofiler-compute-temp-develop-{version}` | `rocprofiler-compute-temp-develop-7.13` |
| Release | `release/therock-{version}` | `release/therock-7.13` (existing) |

### Permissions Required

| Permission | Scope | Purpose |
|------------|-------|---------|
| `contents: write` | Workflow | Create branches, update state file |
| `pull-requests: write` | Workflow | Retarget PRs, add comments |
| `actions: read` | Workflow | Check workflow run status |

### Monorepo Considerations

Since rocprofiler-compute is part of the rocm-systems monorepo:

1. **Path filtering**: All workflows use `paths:` filter to only trigger on rocprofiler-compute changes
2. **Naming convention**: Workflow files prefixed with `rocprofiler-compute-` to avoid conflicts
3. **State isolation**: State file stored within project directory
4. **No impact on other projects**: Freeze enforcement only checks PRs touching our paths

### Extensibility for Jira Integration (Future)

```yaml
# Webhook endpoint for Jira automation
on:
  repository_dispatch:
    types: [rocprofiler-compute-freeze]

# Expected payload from Jira:
# {
#   "event_type": "code_freeze_start" | "code_freeze_end",
#   "release_version": "7.13",
#   "branching_date": "2026-03-20"
# }
```

### Constraints & Assumptions

1. **Monorepo limitation**: GitHub Actions at project level (`projects/rocprofiler-compute/.github/workflows/`) won't auto-trigger. Workflows must be at repo root (`.github/workflows/`) with path filters.

2. **Path-based scoping**: All workflows will use:
   ```yaml
   paths:
     - 'projects/rocprofiler-compute/**'
   ```

3. **CODEOWNERS location constraint**: GitHub only reads CODEOWNERS from three locations:
   - `.github/CODEOWNERS` (recommended)
   - `CODEOWNERS` (repo root)
   - `docs/CODEOWNERS`

   Project-level CODEOWNERS files are **ignored**. We use path patterns (e.g., `projects/rocprofiler-compute/`) within the repo-level CODEOWNERS to scope rules to our project.

4. **`pull_request_review` path limitation**: GitHub does not support `paths:` filters on `pull_request_review` events. The workflow includes an early `path-filter` job to skip processing for PRs that don't touch rocprofiler-compute files, preventing interference with other projects.

5. **Branch permissions**: Requires write access to create branches and PRs. Branch protection rules would need admin coordination.

---

## Implementation Phases

| Phase | Scope | Deliverables |
|-------|-------|--------------|
| **1. POC** | Manual dispatch only | 2 workflow files, state file, CODEOWNERS |
| **2. Hardening** | Add comprehensive comments, error handling | Rich PR comments, dry-run mode |
| **3. Notifications** | Slack/Teams integration (optional) | Webhook integration for announcements |
| **4. Integration** | Jira webhook trigger | repository_dispatch handler |
| **5. Analytics** | Dashboard & metrics | Freeze history, PR stats |

### Phase 1: Proof of Concept (Minimal)
- [x] Create `.code-freeze.json` schema and initial file
- [x] Implement `rocprofiler-compute-freeze-manage.yml` with manual dispatch
- [x] Implement `rocprofiler-compute-freeze-enforce.yml` with basic checks
- [ ] Test on feature branch before merging to develop

### Phase 2: Hardening
- [ ] Add comprehensive PR comments explaining freeze status
- [ ] Add history tracking in state file
- [ ] Improve error handling and edge cases
- [ ] Add dry-run mode for testing

### Phase 3: Notifications (Optional)
- [ ] Slack webhook integration for freeze announcements
- [ ] GitHub Discussion/Issue for tracking freeze periods

### Phase 4: External Integration
- [ ] Jira webhook handler via `repository_dispatch`
- [ ] Scheduled triggers based on release calendar

---

## Questions for Repo Admins

Before implementation, we need to clarify with rocm-systems maintainers:

1. **Workflow placement**: Can we add workflows to `.github/workflows/` with `rocprofiler-compute-` prefix?
2. **Branch protection**: Are there existing rules on `develop` that might conflict?
3. **Required checks**: Can we add a required status check for our enforcement workflow?
4. **CODEOWNERS**: Can we add path-scoped rules to `.github/CODEOWNERS` for `projects/rocprofiler-compute/`? (Note: project-level CODEOWNERS files are not supported by GitHub)
5. **GitHub team**: Create `@ROCm/rocprofiler-compute-leads`

---

## Alternatives Considered

### 1. Branch Protection Rules Only
- **Pros**: Native GitHub feature, no custom code
- **Cons**: Cannot be toggled dynamically, applies to all files (not path-scoped)

### 2. External Bot (Probot/GitHub App)
- **Pros**: More flexibility, can run on separate infrastructure
- **Cons**: Additional infrastructure to maintain, authentication complexity

### 3. Label-based Enforcement Only
- **Pros**: Simple, no state file needed
- **Cons**: Labels can be removed accidentally, no auto-retargeting

### 4. Separate Repository for rocprofiler-compute
- **Pros**: Full control over branch protection and CI
- **Cons**: Major restructuring, impacts entire team workflow

---

## Security Considerations

1. **State file tampering**: Malicious PR could modify `.code-freeze.json` to bypass freeze
   - **Mitigation (implemented)**: The enforcement workflow:
     1. Always reads state from `develop` branch, not from the PR
     2. Detects if the PR modifies `.code-freeze.json`
     3. If tampering detected: bypass approval is denied and check fails with security alert

2. **Workflow modification**: PR could modify workflow to skip checks
   - Mitigation: Use `pull_request_target` trigger with proper isolation

3. **Token permissions**: Workflows run with limited permissions
   - Mitigation: Only request necessary permissions, use GITHUB_TOKEN

---

## Success Metrics

| Metric | Target | Current State |
|--------|--------|---------------|
| Accidental merges during freeze | 0 | Unknown (no tracking) |
| Time to start/end freeze | < 5 minutes | ~30 minutes manual |
| Audit trail completeness | 100% | 0% |
| Developer friction | Minimal | N/A |

---

## Timeline

| Phase | Duration | Dependencies |
|-------|----------|--------------|
| Design review | 1 week | Team feedback |
| Admin coordination | 1-2 weeks | Repo admin availability |
| POC implementation | 1 week | Admin approval |
| Testing on feature branch | 1 week | POC complete |
| Production rollout | 1 sprint | Testing complete |

---

## References

- [GitHub Actions documentation](https://docs.github.com/en/actions)
- [CODEOWNERS syntax](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners)
- [Existing temp branch](https://github.com/ROCm/rocm-systems/tree/users/vedithal/rocprofiler-compute-temp-develop)
- [Release branch example](https://github.com/ROCm/rocm-systems/tree/release/therock-7.12)
- Example workflow files: `projects/rocprofiler-compute/docs/examples/`
