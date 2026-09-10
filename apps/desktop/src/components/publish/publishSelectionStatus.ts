/** Shared gate for the visible button and the preflight request. No state is repaired here. */
export function publishSelectionStatus(input: {
  selectedIds: string[];
  bundles: { id: string; name: string; caption: string }[];
  assignments: Record<string, string>;
  captions: Record<string, string>;
  eligible: string[];
  ready: string[];
  blockingReason?: string;
}) {
  const selected = input.selectedIds.map(id => input.bundles.find(bundle => bundle.id === id));
  const mapped = selected.filter(bundle => bundle && input.assignments[bundle.id]).length;
  let reason = "";
  const used = new Set<string>();
  if (!selected.length) reason = "Chọn bài và máy, hoặc bấm Chọn nhanh để ghép vừa đủ.";
  for (const bundle of selected) {
    if (reason) break;
    if (!bundle) { reason = "Bài đã chọn không còn trong nguồn. Quét lại thư mục."; break; }
    const udid = input.assignments[bundle.id];
    if (!udid) reason = `${bundle.name}: chưa ghép máy. Bấm Chọn nhanh hoặc chọn máy nhận bài.`;
    else if (!input.eligible.includes(udid)) reason = `${bundle.name}: máy đã gán không còn trong phạm vi hoặc đang mất kết nối.`;
    else if (!input.ready.includes(udid)) reason = `${bundle.name}: máy đã gán chưa sẵn sàng. Cặp gán vẫn được giữ.`;
    else if (used.has(udid)) reason = `${bundle.name}: máy đang nhận nhiều hơn một bài.`;
    else if (!(input.captions[bundle.id] ?? bundle.caption).trim()) reason = `${bundle.name}: nội dung chữ đang trống.`;
    used.add(udid);
  }
  if (!reason && input.blockingReason) reason = input.blockingReason;
  return { ready: !reason, reason, mapped };
}
