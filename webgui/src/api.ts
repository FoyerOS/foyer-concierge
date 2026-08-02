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

export type ServiceHealth =
  | "ok"
  | "failed"
  | "transitioning"
  | "inactive"
  | "unknown";

export interface ServiceInfo {
  name: string;
  description: string;
  load_state: string;
  active_state: string;
  sub_state: string;
  unit_file_state: string | null;
  enabled: boolean;
  active: boolean;
  health: ServiceHealth;
  config_paths: string[];
}

export interface ServiceConfigFile {
  path: string;
  content: string;
  etag: string;
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
  changePassword: (username: string, currentPassword: string, newPassword: string) =>
    request<SessionInfo>("/api/v1/auth/change-password", {
      method: "POST",
      body: JSON.stringify({
        username,
        current_password: currentPassword,
        new_password: newPassword,
      }),
    }),
  logout: () => request<void>("/api/v1/auth/logout", { method: "POST" }),
  session: () => request<SessionInfo>("/api/v1/auth/session"),
  systemStatus: () => request<SystemStatus>("/api/v1/system/status"),
  listServices: () => request<ServiceInfo[]>("/api/v1/services"),
  enableService: (name: string) =>
    request<ServiceInfo>(`/api/v1/services/${encodeURIComponent(name)}/enable`, {
      method: "POST",
    }),
  disableService: (name: string) =>
    request<ServiceInfo>(`/api/v1/services/${encodeURIComponent(name)}/disable`, {
      method: "POST",
    }),
  getServiceConfig: (name: string, path: string) =>
    request<ServiceConfigFile>(
      `/api/v1/services/${encodeURIComponent(name)}/config?path=${encodeURIComponent(path)}`,
    ),
  updateServiceConfig: (name: string, path: string, content: string, etag: string) =>
    request<ServiceConfigFile>(
      `/api/v1/services/${encodeURIComponent(name)}/config?path=${encodeURIComponent(path)}`,
      { method: "PUT", body: JSON.stringify({ content, etag }) },
    ),
};
