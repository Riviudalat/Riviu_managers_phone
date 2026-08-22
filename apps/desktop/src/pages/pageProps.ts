import type { DeviceInfo } from "../types";

/**
 * What a farm page needs to act on a selection.
 *
 * Three of the five pages take exactly this and nothing else, which is the clearest
 * evidence they were never one component: they shared a props type, not a state.
 */
export type SelProps = {
  devices: DeviceInfo[];
  selected: string[];
  onSelectUdids: (udids: string[]) => void;
};
