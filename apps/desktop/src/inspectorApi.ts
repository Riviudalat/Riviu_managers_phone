import { invoke } from "@tauri-apps/api/core";
import type { InspectorElementSelector } from "./types";
export interface InspectorElement {index:number;parent:number|null;text:string;description:string;resourceId:string;className:string;x:number;y:number;width:number;height:number;enabled:boolean;clickable:boolean;selector:InspectorElementSelector|null}
export interface InspectorSnapshot {id:string;udid:string;package:string;version:string;locale:string;width:number;height:number;pngBase64:string;treeSha256:string;hierarchyXml?:string;elements:InspectorElement[]}
export interface InspectorRecording {id:string;udid:string;name:string;active:boolean;steps:{selector:InspectorElementSelector;expected?:InspectorElementSelector|null;beforeId:string;afterId:string;verified:boolean;error:string|null}[]}
export const inspectorObserve=(udid:string)=>invoke<InspectorSnapshot>("inspector_observe",{udid});
export const inspectorTap=(udid:string,selector:InspectorElementSelector)=>invoke<InspectorSnapshot>("inspector_tap",{udid,selector});
export const inspectorConfirmPostcondition=(udid:string,snapshotId:string,expected:InspectorElementSelector)=>invoke<InspectorRecording>("inspector_confirm_postcondition",{udid,snapshotId,expected});
export const inspectorRecording=(udid:string)=>invoke<InspectorRecording|null>("inspector_recording",{udid});
export const inspectorRecord=(udid:string,name:string,active:boolean)=>invoke<InspectorRecording>("inspector_record",{udid,name,active});
