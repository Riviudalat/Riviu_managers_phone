import { useState } from "react";

import { deviceShell } from "../../api";
import { parseCurrentInputMethod, parseInputMethods, type InputMethod } from "../../imeList";
import { pushToast, toastError } from "../../toastStore";

/**
 * The input methods installed on one phone, and which one it is using.
 *
 * The other clean cluster inside `FocusStream`: two pieces of state, one loader, one setter,
 * and no overlap with anything else in that component. `imeList.ts` already parses the two
 * shell replies and is already tested; this is the state around it.
 */
export interface DeviceKeyboards {
  /// `null` until asked — the overlay only asks when the menu is opened.
  keyboards: InputMethod[] | null;
  current: string | null;
  load: () => Promise<void>;
  choose: (method: InputMethod) => Promise<void>;
}

export function useDeviceKeyboards(
  udid: string,
  /// Runs the shell work behind the overlay's busy flag; false means it refused because
  /// the phone was already doing something.
  runBusy: (work: () => Promise<void>) => Promise<boolean>,
): DeviceKeyboards {
  const [keyboards, setKeyboards] = useState<InputMethod[] | null>(null);
  const [current, setCurrent] = useState<string | null>(null);

  /// Read the phone's keyboards, and which one is current.
  ///
  /// Two shells rather than one round trip because they answer different questions and a
  /// failure of the second should not hide the first. The ids come back from the phone and
  /// are only ever sent back verbatim — see `imeList.ts`.
  const load = async () => {
    try {
      await runBusy(async () => {
        const listed = await deviceShell(udid, "ime list -s");
        setKeyboards(parseInputMethods(listed.stdout));
        try {
          const reply = await deviceShell(udid, "settings get secure default_input_method");
          setCurrent(parseCurrentInputMethod(reply.stdout));
        } catch {
          setCurrent(null);
        }
      });
    } catch (error) {
      setKeyboards([]);
      toastError("Không đọc được danh sách bàn phím", error);
    }
  };

  const choose = async (method: InputMethod) => {
    // Only an id the phone itself just printed, looked up in the parsed list rather than
    // taken from the event: the value reaches a real shell.
    const known = keyboards?.find((candidate) => candidate.id === method.id);
    if (!known) return;
    try {
      await runBusy(async () => {
        await deviceShell(udid, `ime set ${known.id}`);
        setCurrent(known.id);
      });
      pushToast("ok", "Đã đổi bàn phím", known.label);
    } catch (error) {
      toastError("Đổi bàn phím thất bại", error);
    }
  };

  return { keyboards, current, load, choose };
}
