import { invoke } from "@tauri-apps/api/core";
export const appWorkflowSchedule=(id:string,revision:number,target:import('./types').TargetRef,name:string,everyMinutes:number)=>invoke<import('./types').AutomationSchedule>("app_workflow_schedule",{id,revision,target,name,everyMinutes});
import type { AutomationKind, FlowViewport, JsonObject, JsonValue, TargetRef, OrchestrationRunDetail } from "./types";
export interface AppWorkflowNode { id:string; action:string; position:{x:number;y:number}; config:JsonObject }
export interface AppWorkflowEdge { id:string; source:string; port:string; target:string }
export interface AppWorkflowV1 { schemaVersion:1;id:string;revision:number;name:string;kind:AutomationKind;entryNodeId:string;nodes:AppWorkflowNode[];edges:AppWorkflowEdge[];viewport:FlowViewport;profileConfig:JsonValue }
export interface AppWorkflowSummary {id:string;name:string;kind:AutomationKind;latestRevision:number;updatedAt:string}
export interface AppStepDefinition { action:string;label:string;category:string;ports:string[];defaultConfig:JsonObject }
export const appWorkflowList=()=>invoke<AppWorkflowSummary[]>("app_workflow_list");
export const appWorkflowTemplate=(kind:AutomationKind)=>invoke<AppWorkflowV1>("app_workflow_template",{kind});
export const appWorkflowCatalog=(kind:AutomationKind)=>invoke<AppStepDefinition[]>("app_workflow_catalog",{kind});
export const appWorkflowGet=(id:string,revision:number|null=null)=>invoke<AppWorkflowV1|null>("app_workflow_get",{id,revision});
export const appWorkflowValidate=(document:AppWorkflowV1)=>invoke<void>("app_workflow_validate",{document});
export const appWorkflowSave=(document:AppWorkflowV1)=>invoke<AppWorkflowV1>("app_workflow_save",{document,expectedRevision:document.revision||null});
export const appWorkflowArchive=(id:string,revision:number)=>invoke<void>("app_workflow_archive",{id,revision});
export const appWorkflowRun=(id:string,revision:number,target:TargetRef)=>invoke<OrchestrationRunDetail>("app_workflow_run",{id,revision,target});
