import { useEffect, useState } from "react";
import { exampleScript, listScripts, saveScript } from "../api";

interface Props {
  onUseInJobs: (json: string) => void;
}

export function ScriptsPanel({ onUseInJobs }: Props) {
  const [name, setName] = useState("demo");
  const [body, setBody] = useState("");
  const [saved, setSaved] = useState<[string, string][]>([]);
  const [message, setMessage] = useState<string | null>(null);

  const reload = async () => {
    setSaved(await listScripts());
  };

  useEffect(() => {
    exampleScript().then(setBody).catch(() => undefined);
    reload().catch(() => undefined);
  }, []);

  return (
    <div className="panel">
      <header className="panel-header">
        <h2>Scripts</h2>
        <button
          type="button"
          className="ghost"
          onClick={async () => setBody(await exampleScript())}
        >
          Load example
        </button>
      </header>
      <div className="panel-grid">
        <section>
          <label>
            Name
            <input value={name} onChange={(e) => setName(e.target.value)} />
          </label>
          <textarea rows={18} value={body} onChange={(e) => setBody(e.target.value)} />
          {message && <p className="hint">{message}</p>}
          <div className="row">
            <button
              type="button"
              onClick={async () => {
                await saveScript(name, body);
                setMessage("Saved");
                await reload();
              }}
            >
              Save
            </button>
            <button type="button" className="ghost" onClick={() => onUseInJobs(body)}>
              Use in Jobs
            </button>
          </div>
        </section>
        <section>
          <h3>Saved</h3>
          <ul className="script-list">
            {saved.map(([n, json]) => (
              <li key={n}>
                <button
                  type="button"
                  className="linkish"
                  onClick={() => {
                    setName(n);
                    setBody(json);
                  }}
                >
                  {n}
                </button>
              </li>
            ))}
            {!saved.length && <p className="hint">No saved scripts.</p>}
          </ul>
        </section>
      </div>
    </div>
  );
}
