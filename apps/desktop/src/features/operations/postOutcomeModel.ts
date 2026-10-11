import type { OperationRunItem, PublishCampaignState, PublishSheetDeliveryProgress } from "../../types";

/**
 * One post-result vocabulary (PLAN §6): Đã gửi → Đã xác minh → Đã ghi Sheet, with Chưa rõ as its
 * own state. ACK is not proof: a row reaches "verified" only on the backend's verified state,
 * which is `succeeded` + `post.publicationVerified === true` + a canonical TikTok link. The
 * backend sets that flag only from a capture that proved the link, the account, the caption and
 * the time for this run (crates/core publish verification). A link without the flag, or the
 * `alreadyVerified` capability alone (it only checks the link's shape), is never verified here.
 */
export type PostLayer = "notSent" | "sending" | "verifying" | "verified" | "uncertain";
export type SheetLayer = "off" | "waiting" | "pending" | "failed" | "sent" | "superseded" | "unknown";

export interface PostProof {
  method: string | null;
  account: string | null;
  submittedAt: string | null;
  checkedAt: string | null;
  captionMatched: boolean | null;
  timeMatched: boolean | null;
  candidateUrl: string | null;
  raw: string | null;
}

export interface PostOutcome {
  post: PostLayer;
  /** "Chưa gửi", "Đang đăng", "Đang xác minh", "Đã xác minh" or "Chưa rõ". */
  postLabel: string;
  /** What is missing or why the result is unknown; never shown for a verified post. */
  postReason?: string;
  /** The canonical link, only when verified. */
  url: string | null;
  sheet: SheetLayer;
  sheetLabel: string;
  sheetReason?: string;
  /** Đã gửi: the Post was submitted (verifying, verified or unknown). Never a success count. */
  sent: boolean;
  verified: boolean;
  /** Đã ghi Sheet: the backend confirmed the write for a verified post. */
  sheetDelivered: boolean;
  uncertain: boolean;
  proof: PostProof;
}

export interface PostOutcomeInput {
  state: PublishCampaignState;
  effectIntent?: string | null;
  evidenceJson?: string | null;
  errorCode?: string | null;
  sheetDelivery?: PublishSheetDeliveryProgress | null;
}

const MAX_EVIDENCE = 262_144;

function object(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function text(value: unknown, max = 512): string | null {
  return typeof value === "string" && value.trim() ? value.trim().slice(0, max) : null;
}

export function readPublishEvidence(raw: string | null | undefined): Record<string, unknown> {
  // Evidence may include large artifacts; decline oversized/malformed envelopes.
  if (!raw || raw.length > MAX_EVIDENCE) return {};
  try { return object(JSON.parse(raw)); } catch { return {}; }
}

/** A canonical TikTok post link (https, tiktok.com, /@account/video|photo/<id>), or null. */
export function canonicalPostUrl(value: unknown): string | null {
  if (typeof value !== "string") return null;
  try {
    const parsed = new URL(value.trim());
    return parsed.protocol === "https:"
      && ["www.tiktok.com", "tiktok.com"].includes(parsed.hostname)
      && /^\/@[^/]+\/(?:video|photo)\/\d+\/?$/.test(parsed.pathname) ? parsed.href : null;
  } catch {
    return null;
  }
}

const METHOD_LABELS: Record<string, string> = {
  ownProfileCaptionAndCanonicalLink: "Hồ sơ đúng tài khoản, caption khớp và giờ đăng sau lúc bấm Đăng",
  videoCanonicalPublicMetadata: "Metadata công khai của TikTok: tác giả, caption và thời điểm cấp mã bài trong lượt",
  expandedPhotoCaptionCanonicalPublicMetadata: "Caption đầy đủ và metadata công khai của TikTok: tác giả, caption, thời điểm cấp mã bài trong lượt",
  candidateCanonicalPublicMetadata: "Đối chiếu lại link đã chép qua metadata công khai của TikTok",
  canonical_link_from_caption_identified_own_post: "Link chép từ bài của đúng tài khoản, nhận diện bằng caption ngay sau khi Đăng",
};

function boolOrNull(value: unknown): boolean | null {
  return typeof value === "boolean" ? value : null;
}

/** Older rows kept the post facts at the evidence root; newer ones under `post`. */
function postOf(evidence: Record<string, unknown>): Record<string, unknown> {
  return evidence.post && typeof evidence.post === "object" && !Array.isArray(evidence.post) ? object(evidence.post) : evidence;
}

/** A TikTok handle compared the way TikTok compares it: no "@", no surrounding space, any case. */
function normalizedHandle(value: unknown): string | null {
  const handle = text(value, 64)?.replace(/^@+/, "").trim().toLocaleLowerCase("en");
  return handle ? handle : null;
}

/**
 * The account frozen before Post and every other account the row claims. The effect intent is
 * the authority; the post evidence and the account diagnostic only stand in for a legacy record
 * without an intent account. Any claim that names another account is a contradiction.
 */
function accountsOf(input: PostOutcomeInput, evidence: Record<string, unknown>): { expected: string | null; conflict: string | null } {
  const diagnostic = object(evidence.accountDiagnostic);
  const frozen = normalizedHandle(readPublishEvidence(input.effectIntent).expectedAccount);
  const claims = [postOf(evidence).expectedAccount, diagnostic.expectedAccount, diagnostic.observedAccount]
    .map(normalizedHandle)
    .filter((claim): claim is string => claim !== null);
  const expected = frozen ?? claims[0] ?? null;
  return { expected, conflict: expected ? claims.find(claim => claim !== expected) ?? null : null };
}

function proofOf(input: PostOutcomeInput, evidence: Record<string, unknown>, raw: string | null | undefined): PostProof {
  const post = postOf(evidence);
  const status = object(evidence.verificationStatus);
  const diagnostic = object(evidence.verificationDiagnostic);
  const publication = object(diagnostic.publicationEvidence);
  const budget = object(evidence.verificationBudget);
  const methodKey = text(post.verificationMethod) ?? text(post.postConfirmedBy);
  const account = accountsOf(input, evidence).expected;
  return {
    method: methodKey ? METHOD_LABELS[methodKey] ?? "Phương thức xác minh chưa nhận diện" : null,
    account: account ? `@${account}` : null,
    submittedAt: text(post.submittedAt, 64),
    checkedAt: text(status.checkedAt, 64),
    captionMatched: boolOrNull(publication.captionMatched),
    timeMatched: boolOrNull(publication.submissionTimeMatched),
    candidateUrl: canonicalPostUrl(publication.provenCandidateUrl) ?? canonicalPostUrl(budget.provenCandidateUrl),
    raw: raw && raw.length <= MAX_EVIDENCE ? raw : null,
  };
}

/**
 * The backend's verified state, read from the same facts it writes together, and refused when
 * the row itself contradicts it: a true flag never outranks a recorded caption or time mismatch,
 * a missing account frozen before Post, evidence naming another account than the frozen intent,
 * or a link whose author is another account. The time proof
 * is id-allocation-time evidence (the canonical content id was allocated inside this run's window,
 * after its prepared_at), not a publication timestamp; docs/development/contracts.md.
 */
function verificationGate(input: PostOutcomeInput, evidence: Record<string, unknown>, proof: PostProof)
  : { url: string } | { url: null; contradiction?: string } {
  const post = postOf(evidence);
  const url = canonicalPostUrl(post.postUrl);
  if (input.state !== "succeeded" || post.publicationVerified !== true || !url) return { url: null };
  if (proof.captionMatched === false) return { url: null, contradiction: "Caption trên TikTok khác caption của lượt này; không tính là đã xác minh." };
  if (proof.timeMatched === false) return { url: null, contradiction: "Mã bài không được cấp trong khoảng thời gian của lượt này; không tính là đã xác minh." };
  const { expected: account, conflict } = accountsOf(input, evidence);
  if (!account) return { url: null, contradiction: "Thiếu tài khoản đã ghi trước khi Đăng; chưa xác nhận link thuộc đúng tài khoản." };
  if (conflict) {
    return { url: null, contradiction: `Bằng chứng ghi tài khoản @${conflict}, khác tài khoản đã ghi trước khi Đăng @${account}; không tính là đã xác minh.` };
  }
  const author = normalizedHandle(/^\/@([^/]+)\//.exec(new URL(url).pathname)?.[1]);
  if (author !== account) {
    return { url: null, contradiction: `Link thuộc @${author ?? "?"}, khác tài khoản của lượt @${account}; không tính là đã xác minh.` };
  }
  return { url };
}

/** Why a submitted post is not verified yet, naming the proof that is missing. */
function missingProof(input: PostOutcomeInput, evidence: Record<string, unknown>, proof: PostProof): string {
  const status = object(evidence.verificationStatus);
  const url = canonicalPostUrl(postOf(evidence).postUrl);
  if (url) return "Đã có link nhưng chưa có xác nhận của lượt này về đúng tài khoản, caption và thời điểm; không tính là đã xác minh.";
  const reason = text(status.reason);
  if (proof.captionMatched === true && proof.timeMatched === true) {
    return `Caption và thời điểm đã khớp; chưa có link bài chuẩn để xác nhận tài khoản.${reason ? ` ${reason}` : ""}`;
  }
  if (reason) return reason;
  if (input.state === "succeeded") return "Chưa có link bài chuẩn và xác nhận tài khoản, caption, thời điểm của lượt này.";
  return "Chờ TikTok hoàn tất; Riviu tự kiểm tra link, tài khoản, caption và thời điểm khi máy rảnh.";
}

const NOT_SENT_REASON: Partial<Record<PublishCampaignState, string>> = {
  queued: "Đang chờ",
  scheduled: "Đã lên lịch",
  preparing: "Đang kiểm tra máy",
  ready: "Sẵn sàng đăng",
  transferring: "Đang chuyển nội dung",
  imported: "Đã chuyển nội dung, chờ đăng",
  failedBeforeDispatch: "Dừng trước khi bấm Đăng",
  cancelled: "Đã huỷ trước khi bấm Đăng",
  missed: "Lỡ lịch",
};

function sheetOf(verified: boolean, delivery: PublishSheetDeliveryProgress | null | undefined, sheetRequired: boolean | undefined)
  : Pick<PostOutcome, "sheet" | "sheetLabel" | "sheetReason" | "sheetDelivered"> {
  if (delivery?.state === "superseded") {
    return { sheet: "superseded", sheetLabel: "Đợt báo cáo đã đóng · giữ lịch sử trong app", sheetDelivered: false };
  }
  if (!verified) {
    // A Sheet row never comes before the verified link it reports.
    if (sheetRequired === false) return { sheet: "off", sheetLabel: "Không ghi Sheet", sheetDelivered: false };
    return { sheet: "waiting", sheetLabel: "Chờ bài được xác minh", sheetDelivered: false };
  }
  if (delivery?.state === "sent") return { sheet: "sent", sheetLabel: "Đã ghi Sheet", sheetDelivered: true };
  if (delivery?.state === "failed" && delivery.nextAttemptAtMs != null) {
    return { sheet: "pending", sheetLabel: "Đang chờ ghi Sheet", sheetReason: delivery.lastError ?? undefined, sheetDelivered: false };
  }
  if (delivery?.state === "failed") {
    return { sheet: "failed", sheetLabel: "Lỗi ghi Sheet", sheetReason: delivery.lastError ? delivery.lastError : "Sheet chưa nhận dòng này; bài đã xác minh vẫn giữ link.", sheetDelivered: false };
  }
  if (delivery?.state === "pending") {
    return { sheet: "pending", sheetLabel: "Đang chờ ghi Sheet", sheetReason: delivery.lastError ?? undefined, sheetDelivered: false };
  }
  if (sheetRequired === false) return { sheet: "off", sheetLabel: "Không ghi Sheet", sheetDelivered: false };
  return { sheet: "unknown", sheetLabel: "Chưa rõ Sheet", sheetReason: "Chưa đọc được kết quả ghi Sheet cho bài này.", sheetDelivered: false };
}

export function postOutcome(input: PostOutcomeInput, options: { sheetRequired?: boolean } = {}): PostOutcome {
  const evidence = readPublishEvidence(input.evidenceJson);
  const proof = proofOf(input, evidence, input.evidenceJson);
  const gate = verificationGate(input, evidence, proof);
  const url = gate.url;
  const verified = url !== null;
  const sheet = sheetOf(verified, input.sheetDelivery, options.sheetRequired);
  const base = { url, proof, ...sheet };
  if (verified) {
    return { ...base, post: "verified", postLabel: "Đã xác minh", sent: true, verified: true, uncertain: false };
  }
  const review = input.state === "uncertain" && input.errorCode === "post_verification_needs_review";
  if (input.state === "uncertain" || (input.effectIntent && input.state === "cancelled")) {
    const status = object(evidence.verificationStatus);
    const reason = review
      ? text(status.reason) ?? "Chưa có đủ bằng chứng xác nhận bài; tự kiểm tra đã dừng."
      : "Có thể đã bấm Đăng nhưng chưa xác nhận bài đã lên.";
    return { ...base, post: "uncertain", postLabel: "Chưa rõ", postReason: reason, sent: true, verified: false, uncertain: true };
  }
  if (input.state === "succeeded") {
    // The backend calls it done, but the row lacks or contradicts the proof: say which.
    const reason = ("contradiction" in gate && gate.contradiction) || missingProof(input, evidence, proof);
    return { ...base, post: "uncertain", postLabel: "Chưa rõ", postReason: reason, sent: true, verified: false, uncertain: true };
  }
  if (input.state === "verifying") {
    return { ...base, post: "verifying", postLabel: "Đang xác minh", postReason: missingProof(input, evidence, proof), sent: true, verified: false, uncertain: false };
  }
  if (input.state === "posting") {
    return { ...base, post: "sending", postLabel: "Đang đăng", postReason: "Đang bấm Đăng; chưa tính là đã gửi.", sent: false, verified: false, uncertain: false };
  }
  return { ...base, post: "notSent", postLabel: "Chưa gửi", postReason: NOT_SENT_REASON[input.state], sent: false, verified: false, uncertain: false };
}

export interface PostOutcomeTotals {
  total: number;
  sent: number;
  verifying: number;
  verified: number;
  sheetDelivered: number;
  uncertain: number;
  /** False when no row writes to a Sheet; the Sheet count is then not shown as 0. */
  sheetUsed: boolean;
}

export function postOutcomeTotals(outcomes: readonly PostOutcome[]): PostOutcomeTotals {
  const totals: PostOutcomeTotals = { total: outcomes.length, sent: 0, verifying: 0, verified: 0, sheetDelivered: 0, uncertain: 0, sheetUsed: false };
  for (const outcome of outcomes) {
    if (outcome.sent) totals.sent += 1;
    if (outcome.post === "verifying") totals.verifying += 1;
    if (outcome.verified) totals.verified += 1;
    if (outcome.sheetDelivered) totals.sheetDelivered += 1;
    if (outcome.uncertain) totals.uncertain += 1;
    if (outcome.sheet !== "off") totals.sheetUsed = true;
  }
  return totals;
}

/** "20 đã gửi · 18 đã xác minh · 17 đã ghi Sheet · 2 chưa rõ": separate numbers, never a sum. */
export function postOutcomeTotalsText(totals: PostOutcomeTotals): string {
  return [
    `${totals.sent} đã gửi`,
    `${totals.verified} đã xác minh`,
    totals.sheetUsed ? `${totals.sheetDelivered} đã ghi Sheet` : "không ghi Sheet",
    `${totals.uncertain} chưa rõ`,
    totals.verifying ? `${totals.verifying} đang xác minh` : null,
  ].filter(Boolean).join(" · ");
}

/**
 * A Lượt chạy / monitor item carries the assignment's evidence but only the run-level state.
 * Mapped conservatively: `succeeded` still needs the evidence, a pending post is verifying,
 * and nothing here can make an item more certain than its own record.
 */
export function runItemsPostOutcome(items: readonly Pick<OperationRunItem, "state" | "evidence" | "errorCode">[]): PostOutcome | null {
  const outcomes = items.map(item => postOutcome({
    state: item.state === "succeeded" ? "succeeded"
      : item.state === "uncertain" ? "uncertain"
        : item.errorCode === "post_verification_pending" ? "verifying"
          : item.state === "failed" ? "failedBeforeDispatch"
            : item.state === "cancelled" || item.state === "skipped" ? "cancelled" : "queued",
    evidenceJson: item.evidence,
    errorCode: item.errorCode,
  }));
  if (!outcomes.length) return null;
  if (outcomes.every(outcome => outcome.verified)) return outcomes[0];
  return outcomes.find(outcome => outcome.uncertain) ?? outcomes.find(outcome => outcome.post === "verifying") ?? null;
}
