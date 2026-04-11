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
