import { useEffect, useState } from "react";
import { apiDocs } from "../api";
import { describeError } from "../describeError";

/** The Local API's own documentation page. */
export function ApiPage() {
  const [docs, setDocs] = useState("Loading…");
  useEffect(() => {
    apiDocs()
      .then(setDocs)
      .catch((e) => setDocs(describeError(e)));
  }, []);
  return (
    <div className="panel">
      <header className="panel-header">
        <h2>API</h2>
      </header>
      <pre className="api-docs">{docs}</pre>
    </div>
  );
}
