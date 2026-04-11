/// gRPC-Web JSON client for the mirage Dashboard service.
///
/// Calls follow the gRPC-Web path convention:
///   POST /mirage.simulator.Dashboard/{MethodName}
///
/// In development, Vite proxies these to the mirage-server
/// running on port 50051.

import type {
  OverviewData,
  SimulatorSummary,
  ProfileDef,
  SessionSummary,
  SessionDetail,
  ServiceResult,
  SessionDef,
  RunRecord,
  TerminalInfo,
} from "./types";

const SERVICE = "/mirage.simulator.Dashboard";

async function rpc<T>(method: string, body: unknown = {}): Promise<T> {
  const res = await fetch(`${SERVICE}/${method}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`RPC ${method} failed (${res.status}): ${text}`);
  }
  return res.json() as Promise<T>;
}

// ── Overview ───────────────────────────────────────────────────────────────

export async function getOverview(): Promise<OverviewData> {
  return rpc<OverviewData>("GetOverview");
}

// ── Simulators ─────────────────────────────────────────────────────────────

export async function listSimulators(): Promise<SimulatorSummary[]> {
  const res = await rpc<{ simulators: SimulatorSummary[] }>("ListSimulators");
  return res.simulators ?? [];
}

export async function getSimulator(
  name: string
): Promise<SimulatorSummary | null> {
  const res = await rpc<{ simulator: SimulatorSummary }>("GetSimulator", {
    name,
  });
  return res.simulator ?? null;
}

// ── Profiles ───────────────────────────────────────────────────────────────

export async function listProfiles(
  simulatorFilter?: string
): Promise<ProfileDef[]> {
  const res = await rpc<{ profiles: ProfileDef[] }>("ListProfiles", {
    simulator_filter: simulatorFilter ?? "",
  });
  return res.profiles ?? [];
}

export async function createProfile(
  profile: ProfileDef
): Promise<ServiceResult> {
  return rpc<ServiceResult>("CreateProfile", profile);
}

export async function deleteProfile(name: string): Promise<ServiceResult> {
  return rpc<ServiceResult>("DeleteProfile", { name });
}

// ── Sessions ───────────────────────────────────────────────────────────────

export async function listSessions(
  profileFilter?: string
): Promise<SessionSummary[]> {
  const res = await rpc<{ sessions: SessionSummary[] }>("ListSessions", {
    profile_filter: profileFilter ?? "",
  });
  return res.sessions ?? [];
}

export async function createSession(
  session: SessionDef
): Promise<ServiceResult> {
  return rpc<ServiceResult>("CreateSession", session);
}

export async function deleteSession(name: string): Promise<ServiceResult> {
  return rpc<ServiceResult>("DeleteSession", { name });
}

export async function getSessionDetail(
  name: string
): Promise<SessionDetail | null> {
  try {
    return await rpc<SessionDetail>("GetSessionDetail", { name });
  } catch {
    return null;
  }
}

// ── Runs ───────────────────────────────────────────────────────────────────

export async function listRuns(
  sessionFilter?: string
): Promise<RunRecord[]> {
  const res = await rpc<{ runs: RunRecord[] }>("ListRuns", {
    session_filter: sessionFilter ?? "",
  });
  return res.runs ?? [];
}

export async function createRun(
  session: string,
  command: string
): Promise<{ ok: boolean; error?: string; run?: RunRecord }> {
  return rpc<{ ok: boolean; error?: string; run?: RunRecord }>("CreateRun", {
    session,
    command,
  });
}

// ── Session log ────────────────────────────────────────────────────────────

export async function getSessionLog(
  name: string
): Promise<{ log: string; status: string }> {
  const res = await fetch("/api/session/log", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name }),
  });
  if (!res.ok) return { log: "", status: "" };
  return res.json() as Promise<{ log: string; status: string }>;
}

// ── Terminals ──────────────────────────────────────────────────────────────

async function terminalRpc<T>(
  endpoint: string,
  body: unknown = {}
): Promise<T> {
  const res = await fetch(`/api/terminal/${endpoint}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new Error(`Terminal ${endpoint} failed (${res.status})`);
  }
  return res.json() as Promise<T>;
}

export async function listTerminals(): Promise<TerminalInfo[]> {
  const res = await terminalRpc<{ terminals: TerminalInfo[] }>("list");
  return res.terminals ?? [];
}

export async function createTerminal(
  session: string
): Promise<{ ok: boolean; error: string; id: string }> {
  return terminalRpc<{ ok: boolean; error: string; id: string }>("create", {
    session,
  });
}

export async function terminalInput(
  id: string,
  data: string
): Promise<void> {
  // data is already base64-encoded
  await terminalRpc("input", { id, data });
}

export async function terminalOutput(
  id: string
): Promise<{ data: string; alive: boolean }> {
  return terminalRpc<{ data: string; alive: boolean }>("output", { id });
}

export async function terminalResize(
  id: string,
  rows: number,
  cols: number
): Promise<void> {
  await terminalRpc("resize", { id, rows, cols });
}

export async function closeTerminal(id: string): Promise<void> {
  await terminalRpc("close", { id });
}
