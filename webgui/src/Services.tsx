import { useEffect, useState } from "react";
import { api, ApiError, type ServiceInfo } from "./api.ts";

export default function Services({
  onConfigure,
}: {
  onConfigure: (unit: string, path: string) => void;
}) {
  const [services, setServices] = useState<ServiceInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = () => {
    api
      .listServices()
      .then(setServices)
      .catch((err) =>
        setError(err instanceof ApiError ? err.message : String(err)),
      );
  };

  useEffect(refresh, []);

  const toggle = async (service: ServiceInfo) => {
    setBusy(service.name);
    setError(null);
    try {
      if (service.enabled) {
        await api.disableService(service.name);
      } else {
        await api.enableService(service.name);
      }
      refresh();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section>
      <h2>Services</h2>
      {error && <p className="error">{error}</p>}
      {services === null && !error && <p>Loading…</p>}
      {services && (
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>Health</th>
              <th>Enabled</th>
              <th>Active</th>
              <th>Config</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {services.map((service) => (
              <tr key={service.name}>
                <td>
                  {service.name}
                  <div>
                    <small>{service.description}</small>
                  </div>
                </td>
                <td>{service.health}</td>
                <td>{service.enabled ? "yes" : "no"}</td>
                <td>{service.active ? "yes" : "no"}</td>
                <td>
                  {service.config_paths.map((path) => (
                    <div key={path}>
                      <a
                        href="#"
                        onClick={(event) => {
                          event.preventDefault();
                          onConfigure(service.name, path);
                        }}
                      >
                        {path}
                      </a>
                    </div>
                  ))}
                </td>
                <td>
                  <button
                    onClick={() => void toggle(service)}
                    disabled={busy === service.name}
                  >
                    {service.enabled ? "Disable" : "Enable"}
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
