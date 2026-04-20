/**
 * Mock API server mirroring InMemoryMirageDaemon behavior.
 *
 * Provides the JSON REST endpoints consumed by the dashboard.
 * Run with: npx tsx server/index.ts
 */

import express from "express";

import type {
  OverviewData,
  SimulatorSummary,
  ProfileDef,
  SessionSummary,
  SessionDetail,
  ServiceResult,
  RunRecord,
  TerminalInfo,
  GpuFamily,
  SimulatorMode,
  HealthStatus,
} from "../src/api/types";

// ── In-memory state ────────────────────────────────────────────────────────

interface GpuDef {
  name: string;
  arch: string;
  family: GpuFamily;
  description: string;
}

interface SimulatorInfo {
  name: string;
  version: string;
  description: string;
  supported_gpus: GpuDef[];
  supports_custom_gpus: boolean;
  supported_modes: SimulatorMode[];
}

interface SessionRecord {
  name: string;
  profile: string;
  simulator: string;
  image: string;
  health_status: HealthStatus;
  created_at: number;
}

const simulators: Map<string, SimulatorInfo> = new Map([
  [
    "rocjitsu",
    {
      name: "rocjitsu",
      version: "0.5.0",
      description: "AMD CDNA functional simulator",
      supported_gpus: [
        { name: "MI300X", arch: "gfx942", family: "AmdCdna", description: "AMD Instinct MI300X" },
        { name: "MI325X", arch: "gfx942", family: "AmdCdna", description: "AMD Instinct MI325X" },
        { name: "MI350X", arch: "gfx950", family: "AmdCdna", description: "AMD Instinct MI350X" },
      ],
      supports_custom_gpus: false,
      supported_modes: ["Functional"],
    },
  ],
]);

const profiles: Map<string, ProfileDef> = new Map();
const sessions: Map<string, SessionRecord> = new Map();
const runs: Map<string, RunRecord> = new Map();
const terminals: Map<string, TerminalInfo> = new Map();
const sessionLogs: Map<string, { log: string; status: string }> = new Map();

let runCounter = 0;
let terminalCounter = 0;

// ── Helpers ────────────────────────────────────────────────────────────────

function activeSessionCount(simName: string): number {
  let count = 0;
  for (const s of sessions.values()) {
    if (s.simulator === simName) count++;
  }
  return count;
}

function toSimulatorSummary(info: SimulatorInfo): SimulatorSummary {
  return {
    ...info,
    active_session_count: activeSessionCount(info.name),
  };
}

// ── Express app ────────────────────────────────────────────────────────────

const app = express();
app.use(express.json());

// Overview
app.get("/api/overview", (_req, res) => {
  const data: OverviewData = {
    simulator_count: simulators.size,
    profile_count: profiles.size,
    session_count: sessions.size,
  };
  res.json(data);
});

// Simulators
app.get("/api/simulators", (_req, res) => {
  const list = Array.from(simulators.values()).map(toSimulatorSummary);
  res.json(list);
});

app.get("/api/simulators/:name", (req, res) => {
  const info = simulators.get(req.params.name);
  if (!info) return res.status(404).json({ error: "Simulator not found" });
  res.json(toSimulatorSummary(info));
});

// Profiles
app.get("/api/profiles", (req, res) => {
  const simulator = req.query.simulator as string | undefined;
  let list = Array.from(profiles.values());
  if (simulator) list = list.filter((p) => p.simulator === simulator);
  res.json(list);
});

app.post("/api/profiles", (req, res) => {
  const p: ProfileDef = req.body;
  if (!p.name) return res.json({ ok: false, error: "name is required" } satisfies ServiceResult);
  if (profiles.has(p.name)) return res.json({ ok: false, error: `profile '${p.name}' already exists` } satisfies ServiceResult);

  const sim = simulators.get(p.simulator);
  if (!sim) return res.json({ ok: false, error: `simulator '${p.simulator}' not found` } satisfies ServiceResult);
  if (!sim.supported_gpus.some((g) => g.name === p.gpu))
    return res.json({ ok: false, error: `GPU '${p.gpu}' not supported by ${p.simulator}` } satisfies ServiceResult);
  if (!sim.supported_modes.includes(p.mode))
    return res.json({ ok: false, error: `mode '${p.mode}' not supported by ${p.simulator}` } satisfies ServiceResult);

  profiles.set(p.name, { ...p, num_gpus: p.num_gpus || 1, num_nodes: p.num_nodes || 1 });
  res.json({ ok: true, error: "" } satisfies ServiceResult);
});

app.delete("/api/profiles/:name", (req, res) => {
  if (!profiles.has(req.params.name))
    return res.json({ ok: false, error: "profile not found" } satisfies ServiceResult);
  // Check no sessions reference this profile
  for (const s of sessions.values()) {
    if (s.profile === req.params.name)
      return res.json({ ok: false, error: `profile in use by session '${s.name}'` } satisfies ServiceResult);
  }
  profiles.delete(req.params.name);
  res.json({ ok: true, error: "" } satisfies ServiceResult);
});

// Sessions
app.get("/api/sessions", (req, res) => {
  const profileFilter = req.query.profile as string | undefined;
  let list: SessionSummary[] = Array.from(sessions.values()).map((s) => ({
    name: s.name,
    profile: s.profile,
    simulator: s.simulator,
    image: s.image,
    health_status: s.health_status,
  }));
  if (profileFilter) list = list.filter((s) => s.profile === profileFilter);
  res.json(list);
});

app.post("/api/sessions", (req, res) => {
  const { name, profile: profileName, image } = req.body as { name: string; profile: string; image: string };
  if (!name) return res.json({ ok: false, error: "name is required" } satisfies ServiceResult);
  if (sessions.has(name)) return res.json({ ok: false, error: `session '${name}' already exists` } satisfies ServiceResult);

  const prof = profiles.get(profileName);
  if (!prof) return res.json({ ok: false, error: `profile '${profileName}' not found` } satisfies ServiceResult);

  sessions.set(name, {
    name,
    profile: profileName,
    simulator: prof.simulator,
    image: image || "",
    health_status: "Healthy",
    created_at: Date.now(),
  });

  sessionLogs.set(name, { log: `Session '${name}' created.\nUsing profile '${profileName}'.\nSimulator: ${prof.simulator}\nReady.\n`, status: "ready" });

  res.json({ ok: true, error: "" } satisfies ServiceResult);
});

app.delete("/api/sessions/:name", (req, res) => {
  if (!sessions.has(req.params.name))
    return res.json({ ok: false, error: "session not found" } satisfies ServiceResult);
  sessions.delete(req.params.name);
  sessionLogs.delete(req.params.name);
  // Clean up terminals for this session
  for (const [id, t] of terminals) {
    if (t.session === req.params.name) terminals.delete(id);
  }
  res.json({ ok: true, error: "" } satisfies ServiceResult);
});

app.get("/api/sessions/:name/status", (req, res) => {
  const session = sessions.get(req.params.name);
  if (!session) return res.status(404).json({ error: "session not found" });

  const prof = profiles.get(session.profile);
  const uptimeSeconds = Math.floor((Date.now() - session.created_at) / 1000);

  const detail: SessionDetail = {
    name: session.name,
    profile: prof || { name: session.profile, simulator: session.simulator, mode: "Functional", gpu: "", num_gpus: 1, num_nodes: 1 },
    simulator: session.simulator,
    image: session.image,
    health: session.health_status,
    uptime: { seconds: uptimeSeconds, picoseconds: 0 },
    error_message: "",
    ticks: Math.floor(Math.random() * 100000),
    ipc: 1.5 + Math.random(),
    simulation_speed: 0.8 + Math.random() * 0.4,
    active_contexts: Math.floor(Math.random() * 8) + 1,
  };
  res.json(detail);
});

app.get("/api/sessions/:name/log", (req, res) => {
  const log = sessionLogs.get(req.params.name);
  res.json(log || { log: "", status: "" });
});

// Runs
app.get("/api/runs", (req, res) => {
  const sessionFilter = req.query.session as string | undefined;
  let list = Array.from(runs.values());
  if (sessionFilter) list = list.filter((r) => r.session === sessionFilter);
  res.json(list);
});

app.post("/api/runs", (req, res) => {
  const { session, command } = req.body as { session: string; command: string };
  if (!sessions.has(session))
    return res.json({ ok: false, error: `session '${session}' not found` });

  const id = `run-${++runCounter}`;
  const run: RunRecord = {
    id,
    session,
    command,
    status: "completed",
    exit_code: 0,
    output: `$ ${command}\n(simulated output)\n`,
  };
  runs.set(id, run);
  res.json({ ok: true, run });
});

// Terminals
app.get("/api/terminals", (_req, res) => {
  res.json(Array.from(terminals.values()));
});

app.post("/api/terminals", (req, res) => {
  const { session } = req.body as { session: string };
  if (!sessions.has(session))
    return res.json({ ok: false, error: `session '${session}' not found`, id: "" });

  const id = `term-${++terminalCounter}`;
  terminals.set(id, { id, session, alive: true });
  res.json({ ok: true, error: "", id });
});

app.delete("/api/terminals/:id", (req, res) => {
  terminals.delete(req.params.id);
  res.json({});
});

// ── Start ──────────────────────────────────────────────────────────────────

// Reset endpoint for testing
app.post("/api/__reset", (_req, res) => {
  profiles.clear();
  sessions.clear();
  runs.clear();
  terminals.clear();
  sessionLogs.clear();
  runCounter = 0;
  terminalCounter = 0;
  res.json({ ok: true });
});

const PORT = parseInt(process.env.PORT || "50051", 10);
app.listen(PORT, () => {
  console.log(`Mirage mock API server listening on http://localhost:${PORT}`);
});

export default app;
