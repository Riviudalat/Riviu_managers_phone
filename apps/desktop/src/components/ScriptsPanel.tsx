import { useEffect, useState } from "react";
import { exampleScript, listScripts, saveScript } from "../api";
import { describeError } from "../describeError";

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
        <h2>Kịch bản</h2>
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
                // The backend parses the script before storing it, so this rejects on
                // exactly the input an operator most needs told about -- a script with a
                // syntax error. It used to reject into nothing: no "Saved", no reason, and
                // the panel still showing the text that was never stored.
                try {
                  await saveScript(name, body);
                  setMessage("Đã lưu");
                  await reload();
                } catch (error) {
                  setMessage(`Không lưu được: ${describeError(error)}`);
                }
              }}
            >
              Save
            </button>
            <button type="button" className="ghost" onClick={() => onUseInJobs(body)}>
              Dùng ở Tác vụ
            </button>
          </div>
        </section>
        <section>
          <h3>Đã lưu</h3>
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
