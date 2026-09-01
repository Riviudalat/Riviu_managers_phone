import { useEffect, useRef, useState } from "react";
import { apiDocs } from "../api";
import { EmptyState, LoadingState, StatusNotice } from "../components/States";
import { describeError } from "../describeError";

/** The Local API's own documentation page. */
export function ApiPage() {
  const [docs, setDocs] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const loadTicket = useRef(0);

  const load = async () => {
    const ticket = ++loadTicket.current;
    setLoading(true);
    setError(null);
    try {
      const next = await apiDocs();
      if (ticket === loadTicket.current) setDocs(next);
    } catch (cause) {
      if (ticket === loadTicket.current) setError(describeError(cause));
    } finally {
      if (ticket === loadTicket.current) setLoading(false);
    }
  };

  useEffect(() => {
    void load();
    return () => {
      loadTicket.current += 1;
    };
  }, []);

  const isEmpty = docs !== null && docs.trim() === "";

  return (
    <div className="panel">
      {loading && <LoadingState label="Đang tải tài liệu API…" />}
      {!loading && error && (
        <StatusNotice
          tone="error"
          action={(
            <button type="button" className="ghost" onClick={() => void load()}>
              Thử lại
            </button>
          )}
        >
          Không tải được tài liệu API: {error}
        </StatusNotice>
      )}
      {!loading && !error && isEmpty && (
        <EmptyState
          title="Chưa có tài liệu API"
          hint="API cục bộ chưa trả về nội dung tài liệu."
          action={(
            <button type="button" className="ghost" onClick={() => void load()}>
              Tải lại
            </button>
          )}
        />
      )}
      {!loading && !error && !isEmpty && docs !== null && <pre className="api-docs">{docs}</pre>}
    </div>
  );
}
