import { useEffect, useState } from "react";
import { clearAppleId, getAppleId, setAppleId } from "../../api";
import { describeError } from "../../toastStore";

/** Apple ID for the legacy stock-agent path. */
export function LegacyAgentSection() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [hasPassword, setHasPassword] = useState(false);
  const [legacyMessage, setLegacyMessage] = useState<string | null>(null);

  useEffect(() => {
    getAppleId()
      .then((config) => {
        setEmail(config.email);
        setHasPassword(config.hasPassword);
      })
      .catch(() => undefined);
  }, []);
  return (
    <section className="settings-section">
      <h3>Legacy stock agent</h3>
      <p className="hint">
        Apple ID signing chỉ dành cho rollback/debug stock WDA; không phải đường bình luận chữ.
      </p>
      <label>
        Email
        <input value={email} onChange={(event) => setEmail(event.target.value)} />
      </label>
      <label>
        Password {hasPassword ? "(saved)" : ""}
        <input
          type="password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          placeholder={hasPassword ? "••••••••" : ""}
        />
      </label>
      {legacyMessage && <p className="hint">{legacyMessage}</p>}
      <div className="row">
        <button
          type="button"
          className="primary"
          onClick={async () => {
            try {
              await setAppleId(email, password);
              setHasPassword(true);
              setPassword("");
              setLegacyMessage("Saved to OS credential store");
            } catch (error) {
              setLegacyMessage(describeError(error));
            }
          }}
        >
          Save
        </button>
        <button
          type="button"
          className="ghost"
          onClick={async () => {
            try {
              await clearAppleId();
              setEmail("");
              setPassword("");
              setHasPassword(false);
              setLegacyMessage("Cleared");
            } catch (error) {
              setLegacyMessage(describeError(error));
            }
          }}
        >
          Clear
        </button>
      </div>
    </section>
  );
}
