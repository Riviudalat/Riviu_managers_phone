import { useCallback, useEffect, useRef, useState } from "react";

import { nurtureClearSessionLog, nurtureSessionLog } from "../../api";
import { describeError } from "../../describeError";
import type { SessionLogEntry } from "../../types";

/**
 * How often an open log refetches.
 *
 * Polling rather than riding the `nurtureStatus` event, and that is the point: the idle
 * sweeper writes into the same book without emitting a status, so a panel that only
 * refreshed on status changes would show a running session's history and miss every line
 * about a phone nobody is driving — which is exactly the phone somebody opens this to look
 * at. One device at a time and an in-memory read on the other end, so the cost is a
 * message hop.
 */
const REFRESH_MS = 2_500;

/** `14:22:07`, in the operator's own timezone. */
function clockOf(iso: string): string {
  const at = new Date(iso);
  return Number.isNaN(at.getTime())
    ? "--:--:--"
    : at.toLocaleTimeString(undefined, { hour12: false });
}

/**
 * How long a collapsed run has been repeating, when that is worth saying.
 *
 * A line said once has nothing to add. A line repeating for two minutes is the phone
 * telling you it is stuck, and the count alone (`×48`) does not carry that — the operator
 * would have to know the poll interval to read it as a duration.
 */
function spanOf(entry: SessionLogEntry): string | null {
  if (entry.repeats < 2) return null;
  const from = new Date(entry.at).getTime();
  const to = new Date(entry.lastAt).getTime();
  if (Number.isNaN(from) || Number.isNaN(to)) return null;
  const seconds = Math.round((to - from) / 1000);
  if (seconds < 5) return null;
  return seconds < 90 ? `${seconds}s` : `${Math.round(seconds / 60)} phút`;
}

type Props = {
  udid: string;
  /** Shown in the empty state, because "nothing yet" means different things either way. */
  running: boolean;
};

export function NurtureDeviceLog({ udid, running }: Props) {
  const [entries, setEntries] = useState<SessionLogEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const bottom = useRef<HTMLDivElement | null>(null);
  /**
   * Whether to keep the newest line in view.
   *
   * Follows only while the operator is already at the bottom. Scrolling up is a
   * deliberate act — they are reading something older — and yanking the view back every
   * 2.5 s would make the history unreadable precisely when it is being read.
   */
  const follow = useRef(true);

  const load = useCallback(async () => {
    try {
      setEntries(await nurtureSessionLog(udid));
      setError(null);
    } catch (e) {
      setError(describeError(e));
    }
  }, [udid]);

  useEffect(() => {
    // Opening a different row is a different phone: drop what is on screen rather than
    // showing the previous device's history until the first fetch lands.
    setEntries(null);
    follow.current = true;
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load]);

  useEffect(() => {
    if (follow.current) bottom.current?.scrollIntoView({ block: "nearest" });
  }, [entries]);

  return (
    <div className="nurture-log-panel">
      <div className="nurture-log-panel-head">
        <span className="nurture-log-panel-title">Nhật ký máy này</span>
        <div className="grow" />
        {entries?.length ? <span className="nurture-log-count">{entries.length} dòng</span> : null}
        <button
          type="button"
          className="ghost small"
          disabled={!entries?.length}
          onClick={async () => {
            try {
              await nurtureClearSessionLog(udid);
              await load();
            } catch (e) {
              setError(describeError(e));
            }
          }}
        >
          Xoá
        </button>
      </div>
      <div
        className="nurture-log-lines"
        onScroll={(event) => {
          const el = event.currentTarget;
          follow.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        {error ? (
          <p className="nurture-log-empty">{error}</p>
        ) : entries === null ? (
          <p className="nurture-log-empty">đang đọc…</p>
        ) : entries.length === 0 ? (
          <p className="nurture-log-empty">
            {running ? "chưa có dòng nào — phiên vừa bắt đầu" : "máy này chưa nói gì"}
          </p>
        ) : (
          <>
            {entries.map((entry, index) => {
              const span = spanOf(entry);
              return (
                <div className="nurture-log-line" key={`${entry.at}-${index}`}>
                  <time className="nurture-log-at" dateTime={entry.at}>
                    {clockOf(entry.at)}
                  </time>
                  <span className="nurture-log-text">{entry.text}</span>
                  {entry.repeats > 1 && (
                    <span
                      className="nurture-log-repeats"
                      title={
                        span
                          ? `lặp ${entry.repeats} lần, kéo dài ${span}`
                          : `lặp ${entry.repeats} lần`
                      }
                    >
                      ×{entry.repeats}
                      {span ? ` · ${span}` : ""}
                    </span>
                  )}
                </div>
              );
            })}
            <div ref={bottom} />
          </>
        )}
      </div>
    </div>
  );
}
