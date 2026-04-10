import { useEffect, useState, useCallback } from "react";
import { listProfiles, createProfile, deleteProfile } from "../api/client";
import type { ProfileDef, SimulatorMode } from "../api/types";

export function ProfileListPage() {
  const [profiles, setProfiles] = useState<ProfileDef[]>([]);
  const [error, setError] = useState("");
  const [showForm, setShowForm] = useState(false);

  const refresh = useCallback(() => {
    listProfiles().then(setProfiles).catch((e) => setError(String(e)));
  }, []);

  useEffect(refresh, [refresh]);

  const handleCreate = async (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    const fd = new FormData(e.currentTarget);
    const profile: ProfileDef = {
      name: fd.get("name") as string,
      simulator: fd.get("simulator") as string,
      gpu: fd.get("gpu") as string,
      mode: (fd.get("mode") as SimulatorMode) || "Functional",
      num_gpus: Number(fd.get("num_gpus")) || 1,
      num_nodes: Number(fd.get("num_nodes")) || 1,
    };
    const res = await createProfile(profile);
    if (!res.ok) {
      setError(res.error);
    } else {
      setShowForm(false);
      refresh();
    }
  };

  const handleDelete = async (name: string) => {
    const res = await deleteProfile(name);
    if (!res.ok) {
      setError(res.error);
    } else {
      refresh();
    }
  };

  if (error) return <div className="error">{error}</div>;

  return (
    <div className="page">
      <div className="page-header">
        <h2>Profiles</h2>
        <button className="btn-primary" onClick={() => setShowForm(!showForm)}>
          {showForm ? "Cancel" : "+ New Profile"}
        </button>
      </div>

      {showForm && (
        <form className="create-form" onSubmit={handleCreate}>
          <input name="name" placeholder="Profile name" required />
          <input name="simulator" placeholder="Simulator" required />
          <input name="gpu" placeholder="GPU (e.g. MI300X)" required />
          <select name="mode" defaultValue="Functional">
            <option value="Functional">Functional</option>
            <option value="Clocked">Clocked</option>
            <option value="CycleAccurate">Cycle Accurate</option>
          </select>
          <input
            name="num_gpus"
            type="number"
            min="1"
            defaultValue="1"
            placeholder="GPUs"
          />
          <input
            name="num_nodes"
            type="number"
            min="1"
            defaultValue="1"
            placeholder="Nodes"
          />
          <button type="submit" className="btn-primary">
            Create
          </button>
        </form>
      )}

      {profiles.length === 0 ? (
        <p className="empty">No profiles.</p>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Simulator</th>
              <th>GPU</th>
              <th>Mode</th>
              <th>GPUs</th>
              <th>Nodes</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {profiles.map((p) => (
              <tr key={p.name}>
                <td><strong>{p.name}</strong></td>
                <td>{p.simulator}</td>
                <td><code>{p.gpu}</code></td>
                <td>{p.mode}</td>
                <td>{p.num_gpus}</td>
                <td>{p.num_nodes}</td>
                <td>
                  <button
                    className="btn-danger-sm"
                    onClick={() => handleDelete(p.name)}
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
