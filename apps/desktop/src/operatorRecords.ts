import { invoke } from "@tauri-apps/api/core";
import type { JsonObject } from "./types";
export type OperatorRecordKind = "account" | "network" | "savedTask";
export interface OperatorRecord { id: string; kind: OperatorRecordKind; name: string; revision: number; data: JsonObject; archived: boolean; createdAt: string; updatedAt: string }
export function operatorList(kind: OperatorRecordKind, search = "") { return invoke<OperatorRecord[]>("operator_list", { kind, search }); }
export function operatorSave(input: { id: string; kind: OperatorRecordKind; name: string; expectedRevision: number | null; data: JsonObject }) { return invoke<OperatorRecord>("operator_save", { input }); }
export function operatorArchive(record: OperatorRecord) { return invoke<void>("operator_archive", { id: record.id, expectedRevision: record.revision }); }
export function operatorNetworkProbe(id:string){return invoke<{reachable:boolean;elapsedMs:number;host:string;port:number;protocolValidated:boolean}>("operator_network_probe",{id});}
export function operatorNetworkApply(id:string,clear=false){return invoke<{udid:string;confirmed:boolean;observed?:string;error?:unknown}[]>("operator_network_apply",{id,clear});}
export function operatorImport(kind:OperatorRecordKind,records:{name:string;data:JsonObject}[]){return invoke<OperatorRecord[]>("operator_import",{inputs:records.map(record=>({...record,id:crypto.randomUUID(),kind,expectedRevision:null}))});}
