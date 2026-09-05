import type { OperationRunKind, OperationRunSummary, PageId } from "./types";

/** Navigation identity only. Opening a source never dispatches or retries a run. */
export interface OperationSourceRef {
  operationId: string;
  kind: OperationRunKind;
  sourceId: string;
  itemId?: string;
  udid?: string;
}

const SOURCE_PAGE: Record<OperationRunKind, PageId> = {
  script: "jobs", flow: "scripts", orchestration: "scripts", nurture: "nurture",
  interaction: "interaction", publish: "publish", appInstall: "apps", materialTransfer: "material",
};

export function operationSourcePage(source: OperationSourceRef): PageId {
  return SOURCE_PAGE[source.kind];
}

export function operationSourceFor(run: OperationRunSummary): OperationSourceRef {
  return { operationId: run.id, sourceId: run.sourceId, kind: run.kind };
}
