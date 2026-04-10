import { NavLink, Outlet } from "react-router-dom";

export function Layout() {
  return (
    <div className="layout">
      <aside className="sidebar">
        <div className="sidebar-header">
          <h1>mirage</h1>
          <span className="subtitle">GPU Simulator</span>
        </div>
        <nav>
          <NavLink to="/" end>
            Overview
          </NavLink>
          <NavLink to="/simulators">Simulators</NavLink>
          <NavLink to="/profiles">Profiles</NavLink>
          <NavLink to="/sessions">Sessions</NavLink>
        </nav>
      </aside>
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}
