import { useEffect, useState, useCallback } from "react";
import { Link } from "react-router-dom";
import { listSessions, createSession, deleteSession } from "../api/client";
import type { SessionSummary } from "../api/types";
import { StatusBadge } from "../components/StatusBadge";

export function SessionListPage() {
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [error, setError] = useState("");
  const [showForm, setShowForm] = useState(false);

  const refresh = useCallback(() => {
    listSessions().then(setSessions).catch((e) => setError(String(e)));
  }, []);

  useEffect(refresh, [refresh]);

  const handleCreate = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const fd = new FormData(e.currentTarget);
    const session = {
      name: fd.get("name") as string,
      profile: fd.get("profile") as string,
      image: fd.get("image") as string,
    };
    const res = await createSession(session);
    if (!res.ok) {
      setError(res.error);
    } else {
      setShowForm(false);
      refresh();
    }
  };

  const handleDelete = async (name: string) => {
    const res = await deleteSession(name);
    if (!res.ok) {
      setError(res.error);
    } else {
      refresh();
    }
  };

  return (
    <div className="page">
      <div className="page-header">
        <h2>Sessions</h2>
        <button className="btn-primary" onClick={() => setShowForm(!showForm)}>
          {showForm ? "Cancel" : "+ New Session"}
        </button>
      </div>

      {error && <div className="error">{error}</div>}

      {showForm && (
        <form className="create-form" onSubmit={handleCreate}>
          <input name="name" placeholder="Session name" required />
          <input name="profile" placeholder="Profile name" required />
          <input
            name="image"
            placeholder="Container image (e.g. pytorch:latest)"
          />
          <button type="submit" className="btn-primary">
            Create
          </button>
        </form>
      )}

      {sessions.length === 0 ? (
        <p className="empty">No sessions.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Profile</th>
              <th>Simulator</th>
              <th>Image</th>
              <th>Health</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {sessions.map((s) => (
              <tr key={s.name}>
                <td>
                  <Link to={`/sessions/${encodeURIComponent(s.name)}`}>
                    <strong>{s.name}</strong>
                  </Link>
                </td>
                <td>{s.profile}</td>
                <td>{s.simulator}</td>
                <td>
                  <code>{s.image}</code>
                </td>
                <td>
                  <StatusBadge status={s.health_status} />
                </td>
                <td>
                  <button
                    className="btn-danger-sm"
                    onClick={() => handleDelete(s.name)}
                  >
                    Delete
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
