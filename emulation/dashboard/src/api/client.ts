/// JSON REST client for the Mirage Daemon dashboard API.
///
/// In development, Vite proxies `/api` to the mock server on port 50051.

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

// ── Transport ──────────────────────────────────────────────────────────────

const API = "/api";

async function get<T>(path: string): Promise<T> {
  const res = await fetch(`${API}${path}`);
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`GET ${path} failed (${res.status}): ${text}`);
  }
  return res.json();
}

async function post<T>(path: string, body?: unknown): Promise<T> {
  const res = await fetch(`${API}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`POST ${path} failed (${res.status}): ${text}`);
  }
  return res.json();
}

async function del<T>(path: string): Promise<T> {
  const res = await fetch(`${API}${path}`, { method: "DELETE" });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`DELETE ${path} failed (${res.status}): ${text}`);
  }
  return res.json();
}

// ── Overview ───────────────────────────────────────────────────────────────

export async function getOverview(): Promise<OverviewData> {
  return get("/overview");
}

// ── Simulators ─────────────────────────────────────────────────────────────

export async function listSimulators(): Promise<SimulatorSummary[]> {
  return get("/simulators");
}

export async function getSimulator(
  name: string
): Promise<SimulatorSummary | null> {
  try {
    return await get(`/simulators/${encodeURIComponent(name)}`);
  } catch {
    return null;
  }
}

// ── Profiles ───────────────────────────────────────────────────────────────

export async function listProfiles(
  simulatorFilter?: string
): Promise<ProfileDef[]> {
  const qs = simulatorFilter ? `?simulator=${encodeURIComponent(simulatorFilter)}` : "";
  return get(`/profiles${qs}`);
}

export async function createProfile(
  profile: ProfileDef
): Promise<ServiceResult> {
  return post("/profiles", profile);
}

export async function deleteProfile(name: string): Promise<ServiceResult> {
  return del(`/profiles/${encodeURIComponent(name)}`);
}

// ── Sessions ───────────────────────────────────────────────────────────────

export async function listSessions(
  profileFilter?: string
): Promise<SessionSummary[]> {
  const qs = profileFilter ? `?profile=${encodeURIComponent(profileFilter)}` : "";
  return get(`/sessions${qs}`);
}

export async function createSession(
  session: SessionDef
): Promise<ServiceResult> {
  return post("/sessions", session);
}

export async function deleteSession(name: string): Promise<ServiceResult> {
  return del(`/sessions/${encodeURIComponent(name)}`);
}

export async function getSessionDetail(
  name: string
): Promise<SessionDetail | null> {
  try {
    return await get(`/sessions/${encodeURIComponent(name)}/status`);
  } catch {
    return null;
  }
}

// ── Runs ───────────────────────────────────────────────────────────────────

export async function listRuns(
  sessionFilter?: string
): Promise<RunRecord[]> {
  const qs = sessionFilter ? `?session=${encodeURIComponent(sessionFilter)}` : "";
  return get(`/runs${qs}`);
}

export async function createRun(
  session: string,
  command: string
): Promise<{ ok: boolean; error?: string; run?: RunRecord }> {
  return post("/runs", { session, command });
}

// ── Session log ────────────────────────────────────────────────────────────

export async function getSessionLog(
  name: string
): Promise<{ log: string; status: string }> {
  return get(`/sessions/${encodeURIComponent(name)}/log`);
}

// ── Terminals ──────────────────────────────────────────────────────────────

export async function listTerminals(): Promise<TerminalInfo[]> {
  return get("/terminals");
}

export async function createTerminal(
  session: string
): Promise<{ ok: boolean; error: string; id: string }> {
  return post("/terminals", { session });
}

export async function closeTerminal(id: string): Promise<void> {
  await del(`/terminals/${encodeURIComponent(id)}`);
}
