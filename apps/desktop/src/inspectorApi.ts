import { invoke } from "@tauri-apps/api/core";
import type { InspectorElementSelector } from "./types";
export interface InspectorElement {
  index: number;
  parent: number | null;
  package?: string;
  text: string;
  description: string;
  resourceId: string;
  className: string;
  x: number;
  y: number;
  width: number;
  height: number;
  enabled: boolean;
  clickable: boolean;
  checkable?: boolean | null;
  checked?: boolean | null;
  selected?: boolean | null;
  focusable?: boolean | null;
  focused?: boolean | null;
  scrollable?: boolean | null;
  longClickable?: boolean | null;
  password?: boolean | null;
  selector: InspectorElementSelector | null;
  /** Only when `selector` is absent: the Nth sibling in its scrollable container, by explicit choice. */
  ordinalSelector?: InspectorElementSelector | null;
}
export interface InspectorSnapshot {id:string;udid:string;package:string;version:string;locale:string;width:number;height:number;pngBase64:string;treeSha256:string;hierarchyXml?:string;elements:InspectorElement[]}
export interface InspectorFraction {x:number;y:number}
export interface InspectorSwipe {from:InspectorFraction;to:InspectorFraction;durationMs:number}
export type InspectorClipboard = {mode:"arm"} | {mode:"read";name:string};
export interface InspectorRecordedStep {selector?:InspectorElementSelector|null;swipe?:InspectorSwipe|null;clipboard?:InspectorClipboard|null;expected?:InspectorElementSelector|null;sameScreen?:boolean;beforeId:string;afterId:string;verified:boolean;error:string|null}
export interface InspectorRecording {id:string;udid:string;name:string;active:boolean;steps:InspectorRecordedStep[]}
export interface InspectorClipboardStep {recording:InspectorRecording;preview:string|null}
export const inspectorObserve=(udid:string)=>invoke<InspectorSnapshot>("inspector_observe",{udid});
export const inspectorTap=(udid:string,selector:InspectorElementSelector)=>invoke<InspectorSnapshot>("inspector_tap",{udid,selector});
export const inspectorSwipe=(udid:string,swipe:InspectorSwipe)=>invoke<InspectorSnapshot>("inspector_swipe",{udid,swipe});
export const inspectorConfirmPostcondition=(udid:string,snapshotId:string,expected:InspectorElementSelector,sameScreen=false)=>invoke<InspectorRecording>("inspector_confirm_postcondition",{udid,snapshotId,expected,sameScreen});
export const inspectorRecordClipboard=(udid:string,step:InspectorClipboard)=>invoke<InspectorClipboardStep>("inspector_record_clipboard",{udid,step});
export const inspectorRecording=(udid:string)=>invoke<InspectorRecording|null>("inspector_recording",{udid});
export const inspectorRecord=(udid:string,name:string,active:boolean)=>invoke<InspectorRecording>("inspector_record",{udid,name,active});
