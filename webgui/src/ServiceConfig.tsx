import { useEffect, useState } from "react";
import { api, ApiError, type ServiceConfigFile } from "./api.ts";

export default function ServiceConfig({
  unit,
  path,
  onBack,
}: {
  unit: string;
  path: string;
  onBack: () => void;
}) {
  const [file, setFile] = useState<ServiceConfigFile | null>(null);
  const [content, setContent] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [conflict, setConflict] = useState(false);
  const [busy, setBusy] = useState(false);

  const load = () => {
    setFile(null);
    setError(null);
    setConflict(false);
    api
      .getServiceConfig(unit, path)
      .then((loaded) => {
        setFile(loaded);
        setContent(loaded.content);
      })
      .catch((err) =>
        setError(err instanceof ApiError ? err.message : String(err)),
      );
  };

  useEffect(load, [unit, path]);

  const save = async () => {
    if (!file) return;
    setBusy(true);
    setError(null);
    setConflict(false);
    try {
      const updated = await api.updateServiceConfig(
        unit,
        path,
        content,
        file.etag,
      );
      setFile(updated);
    } catch (err) {
      if (err instanceof ApiError && err.status === 409) {
        setConflict(true);
      } else {
        setError(err instanceof ApiError ? err.message : String(err));
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <section>
      <h2>
        {unit} · {path}
      </h2>
      <p>
        <a
          href="#"
          onClick={(event) => {
            event.preventDefault();
            onBack();
          }}
        >
          Back to services
        </a>
      </p>
      <p>Changes take effect the next time this service restarts.</p>
      {error && <p className="error">{error}</p>}
      {conflict && (
        <p className="error">
          This file changed since it was loaded.{" "}
          <a
            href="#"
            onClick={(event) => {
              event.preventDefault();
              load();
            }}
          >
            Reload
          </a>{" "}
          and try again.
        </p>
      )}
      {file === null && !error && <p>Loading…</p>}
      {file && (
        <>
          <textarea
            rows={24}
            cols={100}
            value={content}
            onChange={(event) => setContent(event.target.value)}
          />
          <div>
            <button onClick={() => void save()} disabled={busy}>
              {busy ? "Saving…" : "Save"}
            </button>
          </div>
        </>
      )}
    </section>
  );
}
