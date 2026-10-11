import { Check } from "lucide-react";

export type PublishStep = 1 | 2 | 3 | 4;

const STEPS: ReadonlyArray<readonly [PublishStep, string]> = [
  [1, "Nội dung"],
  [2, "Ghép máy–tài khoản"],
  [3, "Xem lại & xác nhận"],
  [4, "Theo dõi"],
];

/**
 * The publish step bar (PLAN §7): always the same four steps, so the operator knows where
 * the public post happens. Only step 3's dialog confirms a post; the bar itself never acts.
 */
export function PublishSteps({ current }: { current: PublishStep }) {
  return <ol className="publish-steps" aria-label="Các bước đăng bài">
    {STEPS.map(([step, label]) => <li key={step} className={step === current ? "is-current" : step < current ? "is-done" : undefined}
      aria-current={step === current ? "step" : undefined}>
      <span className="publish-step-index" aria-hidden="true">{step < current ? <Check size={13}/> : step}</span>
      <span>{label}</span>
    </li>)}
  </ol>;
}
