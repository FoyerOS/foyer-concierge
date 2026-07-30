import { useEffect, useState } from "react";
import { api, ApiError, type SessionInfo, type SystemStatus } from "./api.ts";
import Services from "./Services.tsx";
import ServiceConfig from "./ServiceConfig.tsx";

export default function App() {
  const [session, setSession] = useState<SessionInfo | null | undefined>();

  useEffect(() => {
    api
      .session()
      .then(setSession)
      .catch(() => setSession(null));
  }, []);

  if (session === undefined) {
    return <main className="card">Loading…</main>;
  }
  if (session === null) {
    return <Login onLogin={setSession} />;
  }
  return (
    <Dashboard
      session={session}
      onLogout={() => {
        void api.logout().finally(() => setSession(null));
      }}
    />
  );
}

function Login({ onLogin }: { onLogin: (session: SessionInfo) => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onLogin(await api.login(username, password));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "cannot reach daemon");
    } finally {
      setBusy(false);
    }
  };

  return (
    <main className="card">
      <h1>Foyer Concierge</h1>
      <form onSubmit={submit}>
        <label>
          Username
          <input
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="username"
            required
          />
        </label>
        <label>
          Password
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="current-password"
            required
          />
        </label>
        <button type="submit" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </button>
        {error && <p className="error">{error}</p>}
      </form>
    </main>
  );
}

type View =
  | { kind: "status" }
  | { kind: "services" }
  | { kind: "config"; unit: string; path: string };

function Dashboard({
  session,
  onLogout,
}: {
  session: SessionInfo;
  onLogout: () => void;
}) {
  const [view, setView] = useState<View>({ kind: "status" });

  return (
    <main className="card">
      <header>
        <h1>Foyer Concierge</h1>
        <span>
          {session.username} · <button onClick={onLogout}>Sign out</button>
        </span>
      </header>
      <nav>
        <a
          href="#"
          onClick={(event) => {
            event.preventDefault();
            setView({ kind: "status" });
          }}
        >
          Status
        </a>{" "}
        ·{" "}
        <a
          href="#"
          onClick={(event) => {
            event.preventDefault();
            setView({ kind: "services" });
          }}
        >
          Services
        </a>
      </nav>
      {view.kind === "status" && <StatusView />}
      {view.kind === "services" && (
        <Services
          onConfigure={(unit, path) => setView({ kind: "config", unit, path })}
        />
      )}
      {view.kind === "config" && (
        <ServiceConfig
          unit={view.unit}
          path={view.path}
          onBack={() => setView({ kind: "services" })}
        />
      )}
    </main>
  );
}

function StatusView() {
  const [status, setStatus] = useState<SystemStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .systemStatus()
      .then(setStatus)
      .catch((err) =>
        setError(err instanceof Error ? err.message : String(err)),
      );
  }, []);

  return (
    <section>
      <h2>System status</h2>
      {error && <p className="error">{error}</p>}
      {status ? (
        <dl>
          <dt>Hostname</dt>
          <dd>{status.hostname}</dd>
          <dt>Uptime</dt>
          <dd>{formatUptime(status.uptime_secs)}</dd>
          <dt>Load average</dt>
          <dd>{status.load_avg.map((l) => l.toFixed(2)).join(" ")}</dd>
          <dt>Memory</dt>
          <dd>
            {Math.round(status.memory.available_kib / 1024)} MiB free of{" "}
            {Math.round(status.memory.total_kib / 1024)} MiB
          </dd>
          <dt>systemd</dt>
          <dd>{status.systemd_version ?? "unreachable"}</dd>
        </dl>
      ) : (
        !error && <p>Loading…</p>
      )}
    </section>
  );
}

function formatUptime(secs: number): string {
  const days = Math.floor(secs / 86_400);
  const hours = Math.floor((secs % 86_400) / 3_600);
  const minutes = Math.floor((secs % 3_600) / 60);
  return days > 0 ? `${days}d ${hours}h ${minutes}m` : `${hours}h ${minutes}m`;
}
