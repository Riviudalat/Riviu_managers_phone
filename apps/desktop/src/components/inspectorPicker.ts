/** Window event the system bar's Inspector launcher listens for to open its phone picker. */
export const INSPECTOR_PICKER_REQUEST = "riviu:inspector-picker";

/** Opens the launcher's phone picker, as its button does (Ctrl+K "Inspector"). Nothing starts on a phone. */
export function requestInspectorPicker() {
  window.dispatchEvent(new Event(INSPECTOR_PICKER_REQUEST));
}
