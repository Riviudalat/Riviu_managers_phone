import { useEffect, useState } from "react";
import { Image as ImageIcon, Video, RefreshCw } from "lucide-react";
import { publishImagePreview } from "../../api";
import type { PublishBundle } from "../../types";
import { describeError } from "../../describeError";

const cache = new Map<string, Promise<string>>();
export function PublishMedia({
  bundle,
  index = 0,
  expanded = false,
}: {
  bundle: PublishBundle;
  index?: number;
  expanded?: boolean;
}) {
  const item = bundle.images[index];
  const key = item ? `${bundle.sourcePath}|${item.path}|${item.sha256}` : "";
  const [result, setResult] = useState<{
    key: string;
    url?: string;
    error?: string;
  }>({ key: "" });
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!item) return;
    let active = true;
    let request = cache.get(key);
    if (!request) {
      request = publishImagePreview(bundle.sourcePath, item.path, item.sha256);
      if (cache.size >= 80) cache.delete(cache.keys().next().value!);
      cache.set(key, request);
      void request.catch(() => {
        if (cache.get(key) === request) cache.delete(key);
      });
    }
    void request
      .then((url) => {
        if (active) setResult({ key, url });
      })
      .catch((error) => {
        if (active) setResult({ key, error: describeError(error) });
      });
    return () => {
      active = false;
    };
  }, [key, retry, item, bundle.sourcePath]);
  if (bundle.mediaKind === "video")
    return (
      <span className="publish-media-placeholder" aria-label="Video MP4">
        <Video size={expanded ? 38 : 22} />
        <small>MP4</small>
      </span>
    );
  if (result.key === key && result.url)
    return (
      <img
        className="publish-media"
        src={result.url}
        alt={expanded ? (item?.fileName ?? bundle.name) : ""}
        draggable={false}
      />
    );
  return (
    <span
      className="publish-media-placeholder"
      title={result.key === key ? result.error : undefined}
    >
      <ImageIcon size={expanded ? 38 : 20} aria-hidden="true" />
      {expanded && (
        <span>
          {result.key === key && result.error
            ? "Chưa tải được ảnh xem trước"
            : item
              ? "Đang tải ảnh…"
              : "Chưa có ảnh xem trước"}
        </span>
      )}
      {expanded && result.key === key && result.error && (
        <button
          type="button"
          className="ghost"
          onClick={() => setRetry((n) => n + 1)}
        >
          <RefreshCw size={14} /> Thử lại
        </button>
      )}
    </span>
  );
}
