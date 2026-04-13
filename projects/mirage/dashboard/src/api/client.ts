/// gRPC-Web binary FlatBuffer client for the mirage Dashboard service.
///
/// Calls follow the gRPC-Web path convention:
///   POST /mirage.simulator.Dashboard/{MethodName}
///   Content-Type: application/x-flatbuffers
///   Body: raw FlatBuffer bytes
///
/// In development, Vite proxies these to the mirage-server
/// running on port 50051.

import * as flatbuffers from "flatbuffers";

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
  GpuFamily,
  SimulatorMode,
  HealthStatus,
} from "./types";

// ── Generated FlatBuffer types ─────────────────────────────────────────────
import { GetOverviewRequest } from "../generated/mirage/simulator/get-overview-request";
import { GetOverviewReply } from "../generated/mirage/simulator/get-overview-reply";
import { ListSimulatorsRequest } from "../generated/mirage/simulator/list-simulators-request";
import { ListSimulatorsReply } from "../generated/mirage/simulator/list-simulators-reply";
import { GetSimulatorRequest } from "../generated/mirage/simulator/get-simulator-request";
import { GetSimulatorReply } from "../generated/mirage/simulator/get-simulator-reply";
import { ListProfilesRequest } from "../generated/mirage/simulator/list-profiles-request";
import { ListProfilesReply } from "../generated/mirage/simulator/list-profiles-reply";
import { CreateProfileRequest } from "../generated/mirage/simulator/create-profile-request";
import { CreateProfileReply } from "../generated/mirage/simulator/create-profile-reply";
import { DeleteProfileRequest } from "../generated/mirage/simulator/delete-profile-request";
import { DeleteProfileReply } from "../generated/mirage/simulator/delete-profile-reply";
import { ListSessionsRequest } from "../generated/mirage/simulator/list-sessions-request";
import { ListSessionsReply } from "../generated/mirage/simulator/list-sessions-reply";
import { DashboardCreateSessionRequest } from "../generated/mirage/simulator/dashboard-create-session-request";
import { DashboardCreateSessionReply } from "../generated/mirage/simulator/dashboard-create-session-reply";
import { DashboardDeleteSessionRequest } from "../generated/mirage/simulator/dashboard-delete-session-request";
import { DashboardDeleteSessionReply } from "../generated/mirage/simulator/dashboard-delete-session-reply";
import { GetSessionDetailRequest } from "../generated/mirage/simulator/get-session-detail-request";
import { GetSessionDetailReply } from "../generated/mirage/simulator/get-session-detail-reply";
import { ListRunsRequest } from "../generated/mirage/simulator/list-runs-request";
import { ListRunsReply } from "../generated/mirage/simulator/list-runs-reply";
import { CreateRunRequest } from "../generated/mirage/simulator/create-run-request";
import { CreateRunReply } from "../generated/mirage/simulator/create-run-reply";
import { GetSessionLogRequest } from "../generated/mirage/simulator/get-session-log-request";
import { GetSessionLogReply } from "../generated/mirage/simulator/get-session-log-reply";
import { ListTerminalsRequest } from "../generated/mirage/simulator/list-terminals-request";
import { ListTerminalsReply } from "../generated/mirage/simulator/list-terminals-reply";
import { CreateTerminalRequest } from "../generated/mirage/simulator/create-terminal-request";
import { CreateTerminalReply } from "../generated/mirage/simulator/create-terminal-reply";
import { TerminalInputRequest } from "../generated/mirage/simulator/terminal-input-request";
import { TerminalInputReply as _TerminalInputReply } from "../generated/mirage/simulator/terminal-input-reply";
import { TerminalOutputRequest } from "../generated/mirage/simulator/terminal-output-request";
import { TerminalOutputReply } from "../generated/mirage/simulator/terminal-output-reply";
import { TerminalResizeRequest } from "../generated/mirage/simulator/terminal-resize-request";
import { TerminalResizeReply as _TerminalResizeReply } from "../generated/mirage/simulator/terminal-resize-reply";
import { CloseTerminalRequest } from "../generated/mirage/simulator/close-terminal-request";
import { CloseTerminalReply as _CloseTerminalReply } from "../generated/mirage/simulator/close-terminal-reply";

import { ProfileDef as FbProfileDef } from "../generated/mirage/fb/profile-def";
import { SessionDef as FbSessionDef } from "../generated/mirage/fb/session-def";
import { GpuFamily as FbGpuFamily } from "../generated/mirage/fb/gpu-family";
import { SimulatorMode as FbSimulatorMode } from "../generated/mirage/fb/simulator-mode";
import { HealthStatus as FbHealthStatus } from "../generated/mirage/fb/health-status";

// ── Enum maps ──────────────────────────────────────────────────────────────

const GPU_FAMILY_NAMES: Record<number, GpuFamily> = {
  [FbGpuFamily.Unknown]: "Unknown",
  [FbGpuFamily.AmdCdna]: "AmdCdna",
  [FbGpuFamily.AmdRdna]: "AmdRdna",
  [FbGpuFamily.RiscV]: "RiscV",
};

const SIM_MODE_NAMES: Record<number, SimulatorMode> = {
  [FbSimulatorMode.Functional]: "Functional",
  [FbSimulatorMode.Clocked]: "Clocked",
  [FbSimulatorMode.CycleAccurate]: "CycleAccurate",
};

const SIM_MODE_VALUES: Record<string, FbSimulatorMode> = {
  Functional: FbSimulatorMode.Functional,
  Clocked: FbSimulatorMode.Clocked,
  CycleAccurate: FbSimulatorMode.CycleAccurate,
};

const HEALTH_NAMES: Record<number, HealthStatus> = {
  [FbHealthStatus.Unknown]: "Unknown",
  [FbHealthStatus.Healthy]: "Healthy",
  [FbHealthStatus.Unhealthy]: "Unhealthy",
};

// ── Transport ──────────────────────────────────────────────────────────────

const SERVICE = "/mirage.simulator.Dashboard";

/** Build a finished FlatBuffer and return the payload bytes. */
function finish(
  build: (b: flatbuffers.Builder) => flatbuffers.Offset
): Uint8Array {
  const b = new flatbuffers.Builder(256);
  const off = build(b);
  b.finish(off);
  return b.asUint8Array();
}

/** Send binary FlatBuffer RPC, return response ArrayBuffer. */
async function rpc(method: string, body: Uint8Array): Promise<ArrayBuffer> {
  const res = await fetch(`${SERVICE}/${method}`, {
    method: "POST",
    headers: { "Content-Type": "application/x-flatbuffers" },
    body: body as unknown as BodyInit,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`RPC ${method} failed (${res.status}): ${text}`);
  }
  return res.arrayBuffer();
}

function buf(ab: ArrayBuffer): flatbuffers.ByteBuffer {
  return new flatbuffers.ByteBuffer(new Uint8Array(ab));
}

// ── Overview ───────────────────────────────────────────────────────────────

export async function getOverview(): Promise<OverviewData> {
  const body = finish((b) => GetOverviewRequest.createGetOverviewRequest(b));
  const ab = await rpc("GetOverview", body);
  const r = GetOverviewReply.getRootAsGetOverviewReply(buf(ab));
  return {
    simulator_count: r.simulatorCount(),
    profile_count: r.profileCount(),
    session_count: r.sessionCount(),
  };
}

// ── Simulators ─────────────────────────────────────────────────────────────

export async function listSimulators(): Promise<SimulatorSummary[]> {
  const body = finish((b) =>
    ListSimulatorsRequest.createListSimulatorsRequest(b)
  );
  const ab = await rpc("ListSimulators", body);
  const r = ListSimulatorsReply.getRootAsListSimulatorsReply(buf(ab));
  const out: SimulatorSummary[] = [];
  for (let i = 0; i < r.simulatorsLength(); i++) {
    const s = r.simulators(i)!;
    const gpus = [];
    for (let j = 0; j < s.supportedGpusLength(); j++) {
      const g = s.supportedGpus(j)!;
      gpus.push({
        name: g.name() ?? "",
        arch: g.arch() ?? "",
        family: GPU_FAMILY_NAMES[g.family()] ?? "Unknown",
        description: g.description() ?? "",
      });
    }
    const modes: SimulatorMode[] = [];
    for (let j = 0; j < s.supportedModesLength(); j++)
      modes.push(SIM_MODE_NAMES[s.supportedModes(j)!] ?? "Functional");
    out.push({
      name: s.name() ?? "",
      version: s.version() ?? "",
      description: s.description() ?? "",
      supported_gpus: gpus,
      supports_custom_gpus: s.supportsCustomGpus(),
      supported_modes: modes,
      active_session_count: s.activeSessionCount(),
    });
  }
  return out;
}

export async function getSimulator(
  name: string
): Promise<SimulatorSummary | null> {
  const body = finish((b) => {
    const n = b.createString(name);
    return GetSimulatorRequest.createGetSimulatorRequest(b, n);
  });
  const ab = await rpc("GetSimulator", body);
  const r = GetSimulatorReply.getRootAsGetSimulatorReply(buf(ab));
  const s = r.simulator();
  if (!s) return null;
  const gpus = [];
  for (let j = 0; j < s.supportedGpusLength(); j++) {
    const g = s.supportedGpus(j)!;
    gpus.push({
      name: g.name() ?? "",
      arch: g.arch() ?? "",
      family: GPU_FAMILY_NAMES[g.family()] ?? ("Unknown" as GpuFamily),
      description: g.description() ?? "",
    });
  }
  const modes: SimulatorMode[] = [];
  for (let j = 0; j < s.supportedModesLength(); j++)
    modes.push(SIM_MODE_NAMES[s.supportedModes(j)!] ?? "Functional");
  return {
    name: s.name() ?? "",
    version: s.version() ?? "",
    description: s.description() ?? "",
    supported_gpus: gpus,
    supports_custom_gpus: s.supportsCustomGpus(),
    supported_modes: modes,
    active_session_count: s.activeSessionCount(),
  };
}

// ── Profiles ───────────────────────────────────────────────────────────────

function readProfile(p: FbProfileDef): ProfileDef {
  return {
    name: p.name() ?? "",
    simulator: p.simulator() ?? "",
    mode: SIM_MODE_NAMES[p.mode()] ?? "Functional",
    gpu: p.gpu() ?? "",
    num_gpus: p.numGpus(),
    num_nodes: p.numNodes(),
  };
}

export async function listProfiles(
  simulatorFilter?: string
): Promise<ProfileDef[]> {
  const body = finish((b) => {
    const f = b.createString(simulatorFilter ?? "");
    return ListProfilesRequest.createListProfilesRequest(b, f);
  });
  const ab = await rpc("ListProfiles", body);
  const r = ListProfilesReply.getRootAsListProfilesReply(buf(ab));
  const out: ProfileDef[] = [];
  for (let i = 0; i < r.profilesLength(); i++)
    out.push(readProfile(r.profiles(i)!));
  return out;
}

export async function createProfile(
  profile: ProfileDef
): Promise<ServiceResult> {
  const body = finish((b) => {
    const nm = b.createString(profile.name);
    const sim = b.createString(profile.simulator);
    const gpu = b.createString(profile.gpu);
    const p = FbProfileDef.createProfileDef(
      b,
      nm,
      sim,
      SIM_MODE_VALUES[profile.mode] ?? FbSimulatorMode.Functional,
      gpu,
      profile.num_gpus,
      profile.num_nodes
    );
    return CreateProfileRequest.createCreateProfileRequest(b, p);
  });
  const ab = await rpc("CreateProfile", body);
  const r = CreateProfileReply.getRootAsCreateProfileReply(buf(ab));
  return { ok: r.ok(), error: r.error() ?? "" };
}

export async function deleteProfile(name: string): Promise<ServiceResult> {
  const body = finish((b) => {
    const n = b.createString(name);
    return DeleteProfileRequest.createDeleteProfileRequest(b, n);
  });
  const ab = await rpc("DeleteProfile", body);
  const r = DeleteProfileReply.getRootAsDeleteProfileReply(buf(ab));
  return { ok: r.ok(), error: r.error() ?? "" };
}

// ── Sessions ───────────────────────────────────────────────────────────────

export async function listSessions(
  profileFilter?: string
): Promise<SessionSummary[]> {
  const body = finish((b) => {
    const f = b.createString(profileFilter ?? "");
    return ListSessionsRequest.createListSessionsRequest(b, f);
  });
  const ab = await rpc("ListSessions", body);
  const r = ListSessionsReply.getRootAsListSessionsReply(buf(ab));
  const out: SessionSummary[] = [];
  for (let i = 0; i < r.sessionsLength(); i++) {
    const s = r.sessions(i)!;
    out.push({
      name: s.name() ?? "",
      profile: s.profile() ?? "",
      simulator: s.simulator() ?? "",
      image: s.image() ?? "",
      health_status: HEALTH_NAMES[s.healthStatus()] ?? "Unknown",
    });
  }
  return out;
}

export async function createSession(
  session: SessionDef
): Promise<ServiceResult> {
  const body = finish((b) => {
    const nm = b.createString(session.name);
    const prof = b.createString(session.profile);
    const img = b.createString(session.image);
    const sd = FbSessionDef.createSessionDef(b, nm, prof, img);
    return DashboardCreateSessionRequest.createDashboardCreateSessionRequest(
      b,
      sd
    );
  });
  const ab = await rpc("CreateSession", body);
  const r = DashboardCreateSessionReply.getRootAsDashboardCreateSessionReply(
    buf(ab)
  );
  return { ok: r.ok(), error: r.error() ?? "" };
}

export async function deleteSession(name: string): Promise<ServiceResult> {
  const body = finish((b) => {
    const n = b.createString(name);
    return DashboardDeleteSessionRequest.createDashboardDeleteSessionRequest(
      b,
      n
    );
  });
  const ab = await rpc("DeleteSession", body);
  const r = DashboardDeleteSessionReply.getRootAsDashboardDeleteSessionReply(
    buf(ab)
  );
  return { ok: r.ok(), error: r.error() ?? "" };
}

export async function getSessionDetail(
  name: string
): Promise<SessionDetail | null> {
  const body = finish((b) => {
    const n = b.createString(name);
    return GetSessionDetailRequest.createGetSessionDetailRequest(b, n);
  });
  try {
    const ab = await rpc("GetSessionDetail", body);
    const r = GetSessionDetailReply.getRootAsGetSessionDetailReply(buf(ab));
    const prof = r.profile();
    return {
      name: r.name() ?? "",
      profile: prof ? readProfile(prof) : { name: "", simulator: "", mode: "Functional", gpu: "", num_gpus: 1, num_nodes: 1 },
      simulator: r.simulator() ?? "",
      image: r.image() ?? "",
      health: HEALTH_NAMES[r.health()] ?? "Unknown",
      uptime: { seconds: Number(r.uptime()), picoseconds: 0 },
      error_message: r.errorMessage() ?? "",
      ticks: Number(r.ticks()),
      ipc: r.ipc(),
      simulation_speed: r.simulationSpeed(),
      active_contexts: r.activeContexts(),
    };
  } catch {
    return null;
  }
}

// ── Runs ───────────────────────────────────────────────────────────────────

function readRunRecord(r: { id(): string | null; session(): string | null; command(): string | null; status(): string | null; exitCode(): number; output(): string | null }): RunRecord {
  return {
    id: r.id() ?? "",
    session: r.session() ?? "",
    command: r.command() ?? "",
    status: r.status() ?? "",
    exit_code: r.exitCode(),
    output: r.output() ?? "",
  };
}

export async function listRuns(
  sessionFilter?: string
): Promise<RunRecord[]> {
  const body = finish((b) => {
    const f = b.createString(sessionFilter ?? "");
    return ListRunsRequest.createListRunsRequest(b, f);
  });
  const ab = await rpc("ListRuns", body);
  const r = ListRunsReply.getRootAsListRunsReply(buf(ab));
  const out: RunRecord[] = [];
  for (let i = 0; i < r.runsLength(); i++)
    out.push(readRunRecord(r.runs(i)!));
  return out;
}

export async function createRun(
  session: string,
  command: string
): Promise<{ ok: boolean; error?: string; run?: RunRecord }> {
  const body = finish((b) => {
    const s = b.createString(session);
    const c = b.createString(command);
    return CreateRunRequest.createCreateRunRequest(b, s, c);
  });
  const ab = await rpc("CreateRun", body);
  const r = CreateRunReply.getRootAsCreateRunReply(buf(ab));
  const run = r.run();
  return {
    ok: r.ok(),
    error: r.error() ?? undefined,
    run: run ? readRunRecord(run) : undefined,
  };
}

// ── Session log ────────────────────────────────────────────────────────────

export async function getSessionLog(
  name: string
): Promise<{ log: string; status: string }> {
  const body = finish((b) => {
    const n = b.createString(name);
    return GetSessionLogRequest.createGetSessionLogRequest(b, n);
  });
  const ab = await rpc("GetSessionLog", body);
  const r = GetSessionLogReply.getRootAsGetSessionLogReply(buf(ab));
  return { log: r.log() ?? "", status: r.status() ?? "" };
}

// ── Terminals ──────────────────────────────────────────────────────────────

export async function listTerminals(): Promise<TerminalInfo[]> {
  const body = finish((b) =>
    ListTerminalsRequest.createListTerminalsRequest(b)
  );
  const ab = await rpc("ListTerminals", body);
  const r = ListTerminalsReply.getRootAsListTerminalsReply(buf(ab));
  const out: TerminalInfo[] = [];
  for (let i = 0; i < r.terminalsLength(); i++) {
    const t = r.terminals(i)!;
    out.push({ id: t.id() ?? "", session: t.session() ?? "", alive: t.alive() });
  }
  return out;
}

export async function createTerminal(
  session: string
): Promise<{ ok: boolean; error: string; id: string }> {
  const body = finish((b) => {
    const s = b.createString(session);
    return CreateTerminalRequest.createCreateTerminalRequest(b, s);
  });
  const ab = await rpc("CreateTerminal", body);
  const r = CreateTerminalReply.getRootAsCreateTerminalReply(buf(ab));
  return { ok: r.ok(), error: r.error() ?? "", id: r.id() ?? "" };
}

export async function terminalInput(
  id: string,
  data: string
): Promise<void> {
  const body = finish((b) => {
    const i = b.createString(id);
    const d = b.createString(data);
    return TerminalInputRequest.createTerminalInputRequest(b, i, d);
  });
  await rpc("TerminalInput", body);
}

export async function terminalOutput(
  id: string
): Promise<{ data: string; alive: boolean }> {
  const body = finish((b) => {
    const i = b.createString(id);
    return TerminalOutputRequest.createTerminalOutputRequest(b, i);
  });
  const ab = await rpc("TerminalOutput", body);
  const r = TerminalOutputReply.getRootAsTerminalOutputReply(buf(ab));
  return { data: r.data() ?? "", alive: r.alive() };
}

export async function terminalResize(
  id: string,
  rows: number,
  cols: number
): Promise<void> {
  const body = finish((b) => {
    const i = b.createString(id);
    return TerminalResizeRequest.createTerminalResizeRequest(b, i, rows, cols);
  });
  await rpc("TerminalResize", body);
}

export async function closeTerminal(id: string): Promise<void> {
  const body = finish((b) => {
    const i = b.createString(id);
    return CloseTerminalRequest.createCloseTerminalRequest(b, i);
  });
  await rpc("CloseTerminal", body);
}
