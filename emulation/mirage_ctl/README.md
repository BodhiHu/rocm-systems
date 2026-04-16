# mirage-ctl

`mirage-ctl` is the operator CLI for the Mirage control plane.

This crate is still in the early scaffold stage. The command reference below is
the intended operator contract for `mirage-ctl` and should be treated as the
source of truth for CLI shape, validation rules, and expected output while the
implementation is being built.


## Core model

- `simulator`: a registered backend such as `rocjitsu`.
- `profile`: a named device and cluster topology definition.
- `session`: a booted cluster instance created from a profile and container
	image.
- `run`: a workload executed inside an existing session.
- `snapshot`: persisted boot or runtime state used for replay and debugging.
- `replay`: deterministic restoration of a prior run from traces and snapshots.

The schema crate already models most of these concepts:

- `ProfileDef` describes a named simulator, GPU, mode, and cluster shape.
- `SessionDef` describes a bootable session.
- `RunDef` and `ExecDef` describe workload execution inside a session.
- `AttachRequest` and `AttachReply` provide streaming stdout, stderr, stdin, and
	exit events for attached runs.

## Conventions

- Commands that execute a workload inside a session use `--` to separate
	`mirage-ctl` flags from the program being executed.
- Commands that support `--json` should emit stable machine-readable output with
	no extra human text on stdout.
- Human-readable output should optimize for operator workflows: concise summary
	by default, more detail only when explicitly requested.
- Session names, profile names, simulator names, and run IDs should be treated
	as stable identifiers suitable for logs, scripting, and artifact naming.
- Failures should return a non-zero exit code and a single actionable error on
	stderr.

## Exit behavior

- `0`: the requested operation succeeded.
- `1`: the command was valid but the requested resource or runtime action
	failed.
- `2`: the CLI invocation was invalid, such as missing required flags,
	conflicting options, or malformed values.

## Command reference

### `mirage-ctl simulators list`

List registered simulator backends that the control plane can boot.

```text
mirage-ctl simulators list [--json]
```

Expected behavior:

- Enumerate every simulator currently registered with the daemon.
- Surface enough metadata for an operator to decide whether a simulator can be
	used to create a profile.
- Sort output by simulator name for stable scripting and predictable diffs.

Options:

- `--json`: emit an array of simulator objects instead of a table.

Human-readable output should include at least:

- simulator name
- version
- supported GPUs
- supported execution modes
- whether custom GPU design is allowed
- daemon or backend health if discovery depends on live registration

JSON objects should include at least:

- `name`
- `version`
- `gpus`
- `modes`
- `supports_custom_gpu`
- `health`

Examples:

```text
mirage-ctl simulators list
mirage-ctl simulators list --json
```

### `mirage-ctl simulators show`

Show the capabilities and compatibility details for a single simulator.

```text
mirage-ctl simulators show <simulator> [--json]
```

Arguments:

- `<simulator>`: registered simulator name such as `rocjitsu`.

Expected behavior:

- Fail if the simulator does not exist.
- Show the simulator's advertised GPUs, supported modes, version, and feature
	flags.
- Include any compatibility notes that would block or constrain profile
	creation.

Output should include at least:

- simulator name and version
- supported GPU identifiers
- supported execution modes
- default mode, if one exists
- whether custom GPU descriptions are accepted
- backend endpoint or registration source, when relevant
- compatibility warnings or constraints

Example:

```text
mirage-ctl simulators show rocjitsu
```

### `mirage-ctl profile create`

Create a named topology profile that can later be used to boot sessions.

```text
mirage-ctl profile create \
	--name <profile-name> \
	--simulator <simulator> \
	--gpu <gpu> \
	--mode <mode> \
	--gpus-per-node <count> \
	--nodes <count>
```

Required options:

- `--name`: unique profile name.
- `--simulator`: simulator backend to target.
- `--gpu`: GPU family or SKU known to that simulator.
- `--mode`: execution mode such as `functional`, `clocked`, or
	`cycle-accurate`.
- `--gpus-per-node`: positive integer GPU count per node.
- `--nodes`: positive integer node count for the cluster.

Validation rules:

- The simulator must exist.
- The requested GPU must be supported by that simulator.
- The requested mode must be supported by that simulator.
- The profile name must be unique unless an explicit future replace flag is
	introduced.
- `--gpus-per-node` and `--nodes` must both be greater than zero.
- Schema-level compatibility checks must pass before the profile is persisted.

Expected behavior:

- Persist a `ProfileDef`-compatible record.
- Return a summary of the created topology.
- Avoid partially-written state if validation fails.

Example:

```text
mirage-ctl profile create \
	--name mi300x-func-2x8 \
	--simulator rocjitsu \
	--gpu MI300X \
	--mode functional \
	--gpus-per-node 8 \
	--nodes 2
```

### `mirage-ctl profile list`

List all saved profiles.

```text
mirage-ctl profile list [--json]
```

Expected behavior:

- Show all locally known or daemon-managed profiles.
- Present enough information to identify the topology without opening each
	profile individually.

Output should include at least:

- profile name
- simulator
- GPU
- mode
- node count
- GPUs per node

Examples:

```text
mirage-ctl profile list
mirage-ctl profile list --json
```

### `mirage-ctl profile show`

Show one saved profile in detail.

```text
mirage-ctl profile show <profile> [--json]
```

Arguments:

- `<profile>`: previously created profile name.

Expected behavior:

- Fail if the profile does not exist.
- Render the stored profile exactly enough for a user to reason about future
	boot behavior.

Output should include at least:

- profile name
- simulator name
- GPU identifier
- mode
- node count
- GPUs per node
- derived total GPU count
- creation metadata if tracked

Example:

```text
mirage-ctl profile show mi300x-func-2x8
```

### `mirage-ctl profile delete`

Delete a saved profile.

```text
mirage-ctl profile delete <profile>
```

Arguments:

- `<profile>`: profile name to remove.

Expected behavior:

- Refuse deletion if the profile is still referenced by an active session.
- Remove the persisted definition and confirm the deletion.
- Return a clear error if the profile does not exist.

Example:

```text
mirage-ctl profile delete mi300x-func-2x8
```

### `mirage-ctl boot`

Create and boot a named session from a profile and container image.

```text
mirage-ctl boot \
	--name <session-name> \
	--profile <profile> \
	--image <container-image>
```

Required options:

- `--name`: unique session name.
- `--profile`: previously created profile to boot.
- `--image`: OCI image containing the runtime environment.

Expected behavior:

- Resolve the profile and allocate a new `SessionDef`.
- Start the required node containers.
- Inject the synthetic topology into the session environment.
- Drive the daemon through create, boot, and ready states.
- Return once the session becomes ready or terminally failed.

Boot should record or expose at least:

- session name
- profile name
- simulator name
- lifecycle state
- per-node readiness
- health summary
- artifact locations for logs, traces, or snapshots when available

Failure cases should distinguish between:

- invalid profile or image input
- container creation failure
- simulator initialization failure
- topology injection failure
- timeout waiting for readiness

Example:

```text
mirage-ctl boot \
	--name smoke-a \
	--profile mi300x-func-2x8 \
	--image ghcr.io/therock/mirage-runtime:rocm6.4
```

### `mirage-ctl status`

Inspect the current lifecycle and health of a booted session.

```text
mirage-ctl status --name <session-name> [--watch] [--json]
```

Required options:

- `--name`: session to inspect.

Optional flags:

- `--watch`: keep refreshing until interrupted.
- `--json`: emit machine-readable status instead of a live table or summary.

Expected behavior:

- Show the current lifecycle state for the session.
- Include node-level readiness and last known health information.
- Surface the last error if the session is degraded or failed.
- In watch mode, refresh in place and exit cleanly on interrupt.

Status output must include at least:

- session name
- lifecycle state
- profile name
- simulator name
- per-node readiness or failed state
- health status
- last error
- trace and snapshot locations when present

Examples:

```text
mirage-ctl status --name smoke-a
mirage-ctl status --name smoke-a --watch
```

### `mirage-ctl logs`

Read buffered logs for a session or one node within that session.

```text
mirage-ctl logs --name <session-name> [--node <node-name>] [--follow]
```

Required options:

- `--name`: session whose logs should be read.

Optional options:

- `--node`: restrict output to a specific node such as `node0`.
- `--follow`: stream new log lines until interrupted.

Expected behavior:

- Show daemon or container logs relevant to session bring-up and workload
	execution.
- Default to aggregated session logs when `--node` is omitted.
- Maintain line ordering that is good enough for operator debugging.

Examples:

```text
mirage-ctl logs --name smoke-a
mirage-ctl logs --name smoke-a --node node0 --follow
```

### `mirage-ctl shutdown`

Terminate a running session and release its associated resources.

```text
mirage-ctl shutdown --name <session-name>
```

Required options:

- `--name`: session to stop.

Expected behavior:

- Stop workloads still associated with the session.
- Tear down node containers and networking state.
- Persist final metadata needed for later inspection or replay.
- Mark the session as stopped or deleted, depending on the final lifecycle
	model.

The command should be idempotent when practical. Shutting down an already
stopped session should either succeed as a no-op or return a specific
already-stopped state without being treated as a generic error.

Example:

```text
mirage-ctl shutdown --name smoke-a
```

### `mirage-ctl run`

Execute a workload inside a booted session, or optionally create a temporary
session for a one-shot smoke test.

Persistent-session form:

```text
mirage-ctl run --name <session-name> -- <program> [args...]
```

One-shot form:

```text
mirage-ctl run \
	--profile <profile> \
	--image <container-image> \
	-- <program> [args...]
```

Required options for persistent-session form:

- `--name`: existing ready session.

Required options for one-shot form:

- `--profile`: profile used to create the temporary session.
- `--image`: container image for that temporary session.

Required trailing command:

- `<program> [args...]`: workload to execute inside the simulated environment.

Expected behavior:

- Fail fast if no command is provided after `--`.
- Attach the run to the target session and return the workload exit status.
- Capture stdout, stderr, timing, and run metadata.
- In one-shot mode, boot a temporary session, execute the workload, and shut it
	down automatically unless a future keep-alive flag is added.

Examples:

```text
mirage-ctl run --name smoke-a -- rocminfo
mirage-ctl run --name smoke-a -- python -c "import torch; print(torch.cuda.is_available())"

mirage-ctl run \
	--profile mi300x-func-2x8 \
	--image ghcr.io/therock/mirage-runtime:rocm6.4 \
	-- rocminfo
```

### `mirage-ctl attach`

Attach to a running workload stream or open an interactive shell within a
session.

Attach to a specific run:

```text
mirage-ctl attach --run <run-id>
```

Open a shell in a session:

```text
mirage-ctl attach --name <session-name> --shell
```

Supported modes:

- `--run <run-id>`: stream stdout and stderr for a known running workload and
	optionally forward stdin.
- `--name <session-name> --shell`: create or attach to an interactive shell in
	the target session.

Expected behavior:

- Reuse the `AttachRequest` and `AttachReply` streaming model already present in
	the schema.
- Preserve stream identity for stdout, stderr, stdin, and exit events.
- Exit with the remote process exit code when attached to a single run.

This command is the live-streaming path. Historical, non-interactive output
remains the responsibility of `logs`.

Examples:

```text
mirage-ctl attach --run <run-id>
mirage-ctl attach --name smoke-a --shell
```

### `mirage-ctl snapshot`

Persist boot or runtime state for later replay and debugging.

```text
mirage-ctl snapshot --name <session-name> --output <path>
```

Required options:

- `--name`: source session.
- `--output`: destination artifact path, typically ending in `.mirsnap`.

Expected behavior:

- Capture enough state to reproduce the selected point in time.
- Include topology identity, daemon state, and references to logs or trace
	artifacts.
- Write a single artifact or manifest rooted at the requested output path.

Snapshot contents should preserve at least:

- session identity
- profile identity
- topology description
- simulator mode
- log references
- trace references
- deterministic checkpoint information when available

Example:

```text
mirage-ctl snapshot --name smoke-a --output artifacts/smoke-a.boot.mirsnap
```

### `mirage-ctl replay`

Restore a prior session or run from a snapshot artifact.

```text
mirage-ctl replay --snapshot <path> [--until <checkpoint>]
```

Required options:

- `--snapshot`: previously generated snapshot artifact.

Optional options:

- `--until`: stop replay at a named checkpoint instead of running to the end.

Expected behavior:

- Validate that the snapshot is compatible with the local runtime.
- Reconstruct the recorded topology and daemon state.
- Resume deterministic replay from stored checkpoints and traces.
- Surface any compatibility mismatch before replay starts.

Examples:

```text
mirage-ctl replay --snapshot artifacts/smoke-a.boot.mirsnap
mirage-ctl replay --snapshot artifacts/hip-smoke.mirsnap --until checkpoint-17
```

## Operator workflows

### Discover a simulator and create a profile

```text
mirage-ctl simulators list
mirage-ctl simulators list --json
mirage-ctl simulators show rocjitsu
```

This is the discovery step before profile creation. It should list simulator
name, version, supported GPUs, supported modes, and whether custom GPU design is
allowed.

### Profile management

```text
mirage-ctl profile create \
	--name mi300x-func-2x8 \
	--simulator rocjitsu \
	--gpu MI300X \
	--mode functional \
	--gpus-per-node 8 \
	--nodes 2

mirage-ctl profile list
mirage-ctl profile show mi300x-func-2x8
mirage-ctl profile delete mi300x-func-2x8
```

### Boot and inspect a session

```text
mirage-ctl boot \
	--name smoke-a \
	--profile mi300x-func-2x8 \
	--image ghcr.io/therock/mirage-runtime:rocm6.4

mirage-ctl status --name smoke-a
mirage-ctl status --name smoke-a --watch
mirage-ctl logs --name smoke-a
mirage-ctl logs --name smoke-a --node node0 --follow
mirage-ctl shutdown --name smoke-a
```

### Run a workload

```text
mirage-ctl run --name smoke-a -- rocminfo
mirage-ctl run --name smoke-a -- python -c "import torch; print(torch.cuda.is_available())"
```

```bash
mirage-ctl run \
	--profile mi300x-func-2x8 \
	--image ghcr.io/therock/mirage-runtime:rocm6.4 \
	-- rocminfo
```

### Attach, snapshot, and replay

```text
mirage-ctl attach --run <run-id>
mirage-ctl attach --name smoke-a --shell
mirage-ctl snapshot --name smoke-a --output artifacts/smoke-a.boot.mirsnap
mirage-ctl replay --snapshot artifacts/smoke-a.boot.mirsnap
mirage-ctl replay --snapshot artifacts/hip-smoke.mirsnap --until checkpoint-17
```

