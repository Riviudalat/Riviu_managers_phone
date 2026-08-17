import type { StreamPlaceholder as Placeholder } from "../streamPlaceholder";

interface Props {
  view: Placeholder;
  /// Shown in the loading label so a screen reader says which phone is waiting.
  deviceName: string;
  onRetry: () => void;
}

/// The one thing drawn over a phone picture that is not showing one.
///
/// Shared by the grid tile and the control overlay so the two cannot drift again: the tile
/// grew a logo-and-spinner treatment and the overlay kept a bare string, which meant the same
/// condition looked like two different problems depending on where you were standing.
export function StreamPlaceholder({ view, deviceName, onRetry }: Props) {
  if (view.kind === "none") return null;

  if (view.kind === "loading") {
    return (
      <div
        className="dev-phone-loading"
        role="status"
        aria-label={`Đang mở stream ${deviceName}`}
        title="Đang mở stream…"
      >
        <img src="/logo.jpg" alt="" />
      </div>
    );
  }

  return (
    <div className="dev-phone-empty">
      <span>{view.reason}</span>
      {/* No retry when retrying cannot help. The codec refusing every candidate is not a
          transient fault, and a button that cannot succeed is worse than no button. */}
      {view.canRetry && (
        <button
          type="button"
          className="link"
          onClick={(event) => {
            event.stopPropagation();
            onRetry();
          }}
        >
          Thử lại
        </button>
      )}
    </div>
  );
}
