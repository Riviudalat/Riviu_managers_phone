import { useState } from "react";
import { updateCheck, updateInstall } from "../../api";
import { describeError } from "../../toastStore";
import { updateView } from "../../updateView";
import type { UpdateStatus } from "../../types";

/** Checking for and installing a new build of this app. */
export function UpdateSection() {
  const [update, setUpdate] = useState<UpdateStatus | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [installingUpdate, setInstallingUpdate] = useState(false);

  const updateStatusView = updateView(update, updateError, installingUpdate);
  return (
    <section className="settings-section">
      <h3>Bản cập nhật</h3>
      <p>
        <span className={`chip ${updateStatusView.tone}`}>{updateStatusView.headline}</span>
      </p>
      {updateStatusView.detail && <p className="hint">{updateStatusView.detail}</p>}
      <div className="row">
        <button
          type="button"
          className="ghost"
          disabled={checkingUpdate || installingUpdate}
          onClick={async () => {
            setCheckingUpdate(true);
            setUpdateError(null);
            try {
              setUpdate(await updateCheck());
            } catch (error) {
              setUpdate(null);
              setUpdateError(describeError(error));
            } finally {
              setCheckingUpdate(false);
            }
          }}
        >
          {checkingUpdate ? "Đang kiểm..." : "Kiểm bản mới"}
        </button>
        <button
          type="button"
          className="primary"
          disabled={!updateStatusView.canInstall}
          onClick={async () => {
            setInstallingUpdate(true);
            setUpdateError(null);
            try {
              await updateInstall();
              // Reached on macOS only: the archive is unpacked in place and the app has
              // to be reopened. On Windows the process is gone before this line.
              setUpdateError("Đã cài xong — mở lại app để dùng bản mới.");
            } catch (error) {
              setUpdateError(describeError(error));
            } finally {
              setInstallingUpdate(false);
            }
          }}
        >
          Tải và cài đặt
        </button>
      </div>
    </section>
  );
}
