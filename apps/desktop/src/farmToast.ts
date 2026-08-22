import { pushToast, toastError } from "./toastStore";

/**
 * Report the outcome of a farm action. Single choke point for the farm pages,
 * so they all notify through the in-app toast stack instead of an OS dialog.
 */
export function flash(msg: string) {
  const [title, ...rest] = msg.split("\n");
  pushToast("info", title, rest.join("\n") || undefined);
}

/** Report a failed farm action; normalises Tauri/Error/string throwables. */
export function flashError(cause: unknown, title = "Thao tác thất bại") {
  toastError(title, cause);
}
