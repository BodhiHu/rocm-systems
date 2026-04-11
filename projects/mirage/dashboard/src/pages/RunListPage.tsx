import { useEffect, useState, useCallback } from "react";
import { listRuns, createRun, listSessions } from "../api/client";
import type { RunRecord, SessionSummary } from "../api/types";

export function RunListPage() {
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState("");
  const [showForm, setShowForm] = useState(false);
  const [submitting, setSubmitting] = useState(false);

  const refresh = useCallback(() => {
    listRuns().then(setRuns).catch((e) => setError(String(e)));
  }, []);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    listSessions().then(setSessions).catch(() => {});
  }, []);

  const handleCreate = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    setSubmitting(true);
    setError("");
    const fd = new FormData(e.currentTarget);
    const session = fd.get("session") as string;
    const command = fd.get("command") as string;

    try {
      const res = await createRun(session, command);
      if (!res.ok) {
        setError(res.error ?? "Run failed");
      } else {
        setShowForm(false);
        refresh();
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  const runningSessions = sessions.filter(
    (s) => s.health_status === "Healthy"
  );

  return (
    <div className="page">
      <div className="page-header">
        <h2>Runs</h2>
        <button className="btn-primary" onClick={() => setShowForm(!showForm)}>
          {showForm ? "Cancel" : "+ New Run"}
        </button>
      </div>

      {error && <div className="error">{error}</div>}

      {showForm && (
        <form className="create-form labeled-form" onSubmit={handleCreate}>
          <div className="form-field">
            <label htmlFor="rf-session">Session</label>
            <select id="rf-session" name="session" required>
              <option value="" disabled selected>
                Select running session
              </option>
              {runningSessions.map((s) => (
                <option key={s.name} value={s.name}>
                  {s.name} — {s.simulator} / {s.profile}
                </option>
              ))}
            </select>
          </div>
          <div className="form-field">
            <label htmlFor="rf-command">Command</label>
            <input
              id="rf-command"
              name="command"
              required
              placeholder="e.g. ls -la /workspace"
            />
          </div>
          <div className="form-field form-actions">
            <button
              type="submit"
              className="btn-primary"
              disabled={submitting}
            >
              {submitting ? "Running…" : "Run"}
            </button>
          </div>
        </form>
      )}

      {runs.length === 0 ? (
        <p className="empty">No runs yet.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>ID</th>
              <th>Session</th>
              <th>Command</th>
              <th>Status</th>
              <th>Exit Code</th>
              <th>Output</th>
            </tr>
          </thead>
          <tbody>
            {[...runs].reverse().map((r) => (
              <tr key={r.id}>
                <td>
                  <strong>{r.id}</strong>
                </td>
                <td>{r.session}</td>
                <td>
                  <code>{r.command}</code>
                </td>
                <td>
                  <span
                    className={`badge ${
                      r.exit_code === 0 ? "badge-healthy" : "badge-unhealthy"
                    }`}
                  >
                    {r.status}
                  </span>
                </td>
                <td>{r.exit_code}</td>
                <td>
                  <pre className="run-output">{r.output || "—"}</pre>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
