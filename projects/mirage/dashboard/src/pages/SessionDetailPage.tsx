import { useEffect, useState } from "react";
import { useParams, Link } from "react-router-dom";
import { getSessionDetail } from "../api/client";
import type { SessionDetail } from "../api/types";
import { StatusBadge } from "../components/StatusBadge";

function formatUptime(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return `${h}h ${m}m`;
}

export function SessionDetailPage() {
  const { name } = useParams<{ name: string }>();
  const [detail, setDetail] = useState<SessionDetail | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!name) return;
    getSessionDetail(name)
      .then(setDetail)
      .catch((e) => setError(String(e)));

    // Poll every 5 seconds
    const id = setInterval(() => {
      getSessionDetail(name).then(setDetail).catch(() => {});
    }, 5000);
    return () => clearInterval(id);
  }, [name]);

  if (error) return <div className="error">{error}</div>;
  if (!detail) return <div className="loading">Loading...</div>;

  return (
    <div className="page">
      <Link to="/sessions" className="back-link">
        &larr; Sessions
      </Link>
      <div className="page-header">
        <h2>{detail.name}</h2>
        <StatusBadge status={detail.health} />
      </div>

      {detail.error_message && (
        <div className="error">{detail.error_message}</div>
      )}

      <div className="detail-grid">
        <div className="detail-item">
          <label>Simulator</label>
          <span>{detail.simulator}</span>
        </div>
        <div className="detail-item">
          <label>Profile</label>
          <span>{detail.profile.name}</span>
        </div>
        <div className="detail-item">
          <label>GPU</label>
          <span>
            <code>{detail.profile.gpu}</code>
          </span>
        </div>
        <div className="detail-item">
          <label>Mode</label>
          <span>{detail.profile.mode}</span>
        </div>
        <div className="detail-item">
          <label>Cluster</label>
          <span>
            {detail.profile.num_gpus} GPU(s) &times;{" "}
            {detail.profile.num_nodes} node(s)
          </span>
        </div>
        <div className="detail-item">
          <label>Image</label>
          <span>
            <code>{detail.image}</code>
          </span>
        </div>
      </div>

      <h3>Performance</h3>
      <div className="card-grid">
        <div className="stat-card">
          <span className="stat-value">
            {formatUptime(detail.uptime.seconds)}
          </span>
          <span className="stat-label">Uptime</span>
        </div>
        <div className="stat-card">
          <span className="stat-value">{detail.ticks.toLocaleString()}</span>
          <span className="stat-label">Ticks</span>
        </div>
        <div className="stat-card">
          <span className="stat-value">{detail.ipc.toFixed(2)}</span>
          <span className="stat-label">IPC</span>
        </div>
        <div className="stat-card">
          <span className="stat-value">
            {detail.simulation_speed.toFixed(2)}x
          </span>
          <span className="stat-label">Speed</span>
        </div>
        <div className="stat-card">
          <span className="stat-value">{detail.active_contexts}</span>
          <span className="stat-label">Active Contexts</span>
        </div>
      </div>
    </div>
  );
}
