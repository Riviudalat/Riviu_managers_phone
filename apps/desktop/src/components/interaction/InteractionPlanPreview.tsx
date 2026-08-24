import { Banner } from "../States";
import { groupPlanByCohort, type ThreadKind } from "../../interactionPlan";
import type { DeviceInfo, ThreadPreview } from "../../types";

/**
 * What is about to run, drawn from the backend's own plan.
 *
 * The popup used to draw this from a TypeScript reimplementation of `partition_actors`,
 * remainder-spreading included, so the preview and the plan were two programs that had to
 * agree by hand. Now the planner answers and this only renders.
 *
 * The capacity warning is the part that earns the round trip. Running more cohorts than the
 * stream budget is a **refusal, not a queue**: the ones past the limit fail with
 * `CapacityExhausted` rather than waiting their turn, and the cheapest moment to learn that
 * is before pressing the button.
 */
export function InteractionPlanPreview({
  preview,
  devices,
  deviceNumber,
  threadKind,
}: {
  preview: ThreadPreview | null;
  devices: DeviceInfo[];
  deviceNumber: Map<string, number>;
  threadKind: ThreadKind;
}) {
  const cohorts = groupPlanByCohort(preview?.plan);
  if (!preview || !cohorts.length) return null;

  const label = (udid: string) => {
    const number = deviceNumber.get(udid);
    const device = devices.find((entry) => entry.udid === udid);
    const name = device?.name || device?.model || udid.slice(0, 8);
    return number ? `${number} · ${name}` : name;
  };

  // **Only the cohorts that will actually run.** `cohortCount` is `partition_actors(...).len()`,
  // but `plan_threads` deals links round-robin and emits no assignments at all for a cohort
  // with no link. Fourteen phones, teams of three and **one** link gives `cohortCount = 4`
  // against a capacity of 2 — so the panel warned that four cohorts would run and the excess
  // would be refused, directly above a preview drawing exactly one, and advised raising the
  // cohort size to fix a problem that did not exist.
  const running = Math.min(preview.cohortCount, Math.max(1, preview.validTargetCount));
  const overCapacity = preview.streamCapacity > 0 && running > preview.streamCapacity;

  return (
    <div className="interaction-preview">
      <div className="nu-group-head">Sẽ chạy như thế này</div>
      {overCapacity && (
        <Banner tone="warn">
          {running} cụm chạy song song nhưng máy này chỉ mở được{" "}
          {preview.streamCapacity} luồng màn hình cùng lúc — phần vượt sẽ bị từ chối chứ không
          xếp hàng. Tăng số máy mỗi cụm để giảm số cụm.
        </Banner>
      )}
      {cohorts.map((team) => (
        <div key={team.cohort} className="interaction-cohort">
          <strong>
            Cụm {team.cohort + 1} · {team.actorUdids.length} máy · {team.targetKeys.length} link
          </strong>
          <small>{team.actorUdids.map(label).join(" · ")}</small>
          <small className="hint">
            {threadKind === "standalone"
              ? "mỗi máy một bình luận gốc"
              : threadKind === "star"
                ? `máy ${label(team.actorUdids[0])} mở bình luận, ${Math.max(
                    team.actorUdids.length - 1,
                    0,
                  )} máy còn lại cùng trả lời vào đó`
                : team.actorUdids.map(label).join(" → ")}
          </small>
        </div>
      ))}
    </div>
  );
}
