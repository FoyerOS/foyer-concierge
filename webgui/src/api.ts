// Thin typed client for the concierge REST API. Types mirror
// crates/concierge-api; consider generating from /api/openapi.json once the
// API grows.

export interface ApiErrorBody {
  code: string;
  message: string;
}

export interface SessionInfo {
  username: string;
}

export interface SystemStatus {
  hostname: string;
  uptime_secs: number;
  load_avg: [number, number, number];
  memory: { total_kib: number; available_kib: number };
  systemd_version: string | null;
}

export class ApiError extends Error {
  constructor(
    public status: number,
    public body: ApiErrorBody,
  ) {
    super(body.message);
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (response.status === 204) {
    return undefined as T;
  }
  const body = await response.json();
  if (!response.ok) {
    throw new ApiError(response.status, body as ApiErrorBody);
  }
  return body as T;
}

export const api = {
  login: (username: string, password: string) =>
    request<SessionInfo>("/api/v1/auth/login", {
      method: "POST",
      body: JSON.stringify({ username, password }),
    }),
  logout: () => request<void>("/api/v1/auth/logout", { method: "POST" }),
  session: () => request<SessionInfo>("/api/v1/auth/session"),
  systemStatus: () => request<SystemStatus>("/api/v1/system/status"),
};
