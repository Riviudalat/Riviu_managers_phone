export const THREADS_ACTIONS = { like: "Thích", reply: "Trả lời", repost: "Đăng lại", quote: "Trích dẫn" } as const;
export type ThreadsAction = keyof typeof THREADS_ACTIONS;
export type ThreadsInteractionRow = {
  id: string; udid: string; account: string; url: string;
  action: ThreadsAction; text: string;
};
export type ThreadsInteractionDraft = { rows: ThreadsInteractionRow[]; runAt: string };

/** A direct post URL identifies a target, not proof of ownership or publication. */
export function parseThreadsTarget(raw: string) {
  const url = new URL(raw.trim());
  if (url.protocol !== "https:" || url.username || url.password || url.port
    || !["threads.com", "www.threads.com", "threads.net", "www.threads.net"].includes(url.hostname)) {
    throw new Error("Dùng link bài HTTPS trên threads.com hoặc threads.net.");
  }
  const match = /^\/@([A-Za-z0-9_.]{1,30})\/post\/([A-Za-z0-9_-]+)\/?$/.exec(url.pathname);
  if (!match) throw new Error("Cần link bài dạng https://www.threads.com/@taikhoan/post/MA_BAI; link rút gọn chưa hỗ trợ.");
  return { author: match[1].toLowerCase(), postCode: match[2], url: `https://www.threads.com/@${match[1].toLowerCase()}/post/${match[2]}` };
}

export function decodeThreadsDraft(value: unknown): ThreadsInteractionDraft {
  const empty = { rows: [], runAt: "" };
  if (!value || typeof value !== "object") return empty;
  const candidate = value as Partial<ThreadsInteractionDraft>;
  if (!Array.isArray(candidate.rows) || candidate.rows.length > 100 || typeof candidate.runAt !== "string") return empty;
  const ids = new Set<string>();
  const rows: ThreadsInteractionRow[] = [];
  for (const row of candidate.rows) {
    if (!row || typeof row !== "object"
      || ![row.id, row.udid, row.account, row.url, row.text].every(v => typeof v === "string")
      || !Object.hasOwn(THREADS_ACTIONS, row.action) || !row.id || ids.has(row.id)) return empty;
    ids.add(row.id);
    rows.push({ id: row.id, udid: row.udid, account: row.account, url: row.url, action: row.action, text: row.text });
  }
  return { rows, runAt: candidate.runAt };
}

export function checkThreadsPlan(draft: ThreadsInteractionDraft, readyUdids: string[], now = Date.now()) {
  const issues: string[] = [];
  const seen = new Set<string>();
  if (!draft.rows.length) issues.push("Thêm ít nhất một dòng tương tác.");
  if (draft.rows.length > 100) issues.push("Mỗi bản nháp tối đa 100 dòng.");
  if (draft.runAt && (!Number.isFinite(Date.parse(draft.runAt)) || Date.parse(draft.runAt) <= now)) issues.push("Giờ dự kiến phải hợp lệ và ở tương lai.");
  const rows = draft.rows.map((row, index) => {
    const prefix = `Dòng ${index + 1}: `;
    const account = row.account.trim().replace(/^@/, "").toLowerCase();
    if (!/^[a-z0-9_.]{1,30}$/.test(account)) issues.push(prefix + "nhập username Threads thực hiện.");
    if (!readyUdids.includes(row.udid)) issues.push(prefix + "chọn máy Android sẵn sàng trong phạm vi.");
    if (!Object.hasOwn(THREADS_ACTIONS, row.action)) issues.push(prefix + "hành động không hợp lệ.");
    if (row.action === "reply" || row.action === "quote") {
      if (!row.text.trim()) issues.push(prefix + "nhập nội dung trả lời/trích dẫn.");
      if (Array.from(row.text).length > 500) issues.push(prefix + "nội dung vượt giới hạn 500 ký tự của luồng này.");
    } else if (row.text.trim()) issues.push(prefix + "Thích/Đăng lại không gửi nội dung; chọn Trả lời/Trích dẫn hoặc xóa nội dung.");
    try {
      const target = parseThreadsTarget(row.url);
      // Same account on two phones or an old/new domain must not duplicate an action.
      const key = JSON.stringify([account, target.postCode, row.action]);
      if (seen.has(key)) issues.push(prefix + "trùng tài khoản, bài và hành động với dòng trước.");
      seen.add(key);
      return { ...row, account, url: target.url, targetKey: target.postCode };
    } catch (error) {
      issues.push(prefix + (error instanceof Error ? error.message : "link không hợp lệ"));
      return { ...row, account, targetKey: "" };
    }
  });
  return { rows, issues, canExecute: false as const };
}
