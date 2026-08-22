import { useCallback, useEffect, useState } from "react";

import {
  androidUnavailableReason,
  driverDegradedReason,
  listDeviceMetas,
  listDevices,
  listGroups,
  listJobs,
  listenRiviuEvents,
  retryStartup,
  startupError,
} from "./api";
import { describeError } from "./describeError";
import type { DeviceGroup, DeviceInfo, DeviceMeta, JobRecord } from "./types";

/**
 * The fleet as the shell sees it, and whether the backend came up at all.
 *
 * Startup health and the device roster are one hook rather than two because they are one
 * effect, and deliberately so: the effect that asks `startup_error` is the same effect that
 * subscribes to `riviu://event`, since there is nothing to subscribe to until startup has
 * succeeded. Splitting them would mean splitting that effect, and the comment on its
 * dependency list records what a previous split cost — a retry that cleared the error, ran
 * `reload()` by hand, and left the session with no subscription for the rest of its life.
 */
export interface Fleet {
  devices: DeviceInfo[];
  groups: DeviceGroup[];
  metas: DeviceMeta[];
  setMetas: React.Dispatch<React.SetStateAction<DeviceMeta[]>>;
  jobs: JobRecord[];
  /// Re-read devices, jobs, groups and records from the backend.
  reload: () => Promise<void>;

  /// Non-null when the backend refused to start; the shell shows nothing else.
  startupIssue: string | null | undefined;
  /// The backend is up but a call failed.
  bootError: string | null;
  /// The device sidecar is degraded, so an empty fleet has a cause worth naming.
  driverIssue: string | null;
  /// Android specifically is unavailable; asked apart because the two halves fail apart.
  androidIssue: string | null;
  retryingStartup: boolean;
  /// Ask the backend to start again, and resubscribe if it does.
  retry: () => Promise<void>;
}

export function useFleet(): Fleet {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [metas, setMetas] = useState<DeviceMeta[]>([]);
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [bootError, setBootError] = useState<string | null>(null);
  const [driverIssue, setDriverIssue] = useState<string | null>(null);
  const [androidIssue, setAndroidIssue] = useState<string | null>(null);
  const [startupIssue, setStartupIssue] = useState<string | null | undefined>(undefined);
  const [retryingStartup, setRetryingStartup] = useState(false);
  /// Bumped by the retry button, and read by the boot effect below as a reason to run
  /// again. A counter rather than `startupIssue` in that effect's dependencies: the
  /// effect *sets* the issue, so depending on it makes every ordinary startup run the
  /// whole thing twice — two `startup_error` calls, two subscriptions, one of them
  /// immediately torn down.
  const [startupAttempt, setStartupAttempt] = useState(0);

  const reload = useCallback(async () => {
    try {
      const [d, j] = await Promise.all([listDevices(), listJobs()]);
      setDevices(d);
      setJobs(j);
      // Groups are auxiliary and load separately, on purpose. Inside the Promise.all
      // above, a group-listing failure rejected the whole reload and left the grid empty
      // — the fleet blanked because a tab strip could not be drawn. Caught by e2e, which
      // had no handler registered for it. Losing the tabs is a smaller loss than losing
      // every phone, so this failure degrades to "no groups".
      setGroups(await listGroups().catch(() => []));
      // Same reasoning as the groups above, and the same failure mode to avoid: a records
      // read that throws must cost the grid its labels, never its phones.
      setMetas(await listDeviceMetas().catch(() => []));
      setBootError(null);
      // An empty list can mean "nothing plugged in" or "the device sidecar never
      // started". Ask which, so the UI does not report the wrong one.
      setDriverIssue(await driverDegradedReason().catch(() => null));
      // Asked separately, because the two halves of the fleet fail for different
      // reasons and an Android phone that never appears used to say nothing at all.
      setAndroidIssue(await androidUnavailableReason().catch(() => null));
    } catch (e) {
      setBootError(describeError(e));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    startupError()
      .then((issue) => {
        if (cancelled) return;
        setStartupIssue(issue);
        if (issue) return;

        void reload();
        void listenRiviuEvents((event) => {
          if (event.type === "devicesUpdated") {
            setDevices(event.devices);
          } else if (event.type === "deviceUpdated") {
            const { device } = event;
            setDevices((prev) => {
              const idx = prev.findIndex((d) => d.udid === device.udid);
              if (idx === -1) return [...prev, device];
              const next = [...prev];
              next[idx] = device;
              return next;
            });
          } else if (event.type === "jobUpdated") {
            const { job } = event;
            setJobs((prev) => {
              const idx = prev.findIndex((j) => j.id === job.id);
              if (idx === -1) return [job, ...prev];
              const next = [...prev];
              next[idx] = job;
              return next;
            });
          }
        }).then((fn) => {
          if (cancelled) {
            fn();
          } else {
            unlisten = fn;
          }
        });
      })
      .catch((error) => {
        if (cancelled) return;
        setStartupIssue(null);
        setBootError(describeError(error));
        void reload();
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
    // **`startupAttempt` is a dependency so a successful retry gets a subscription.**
    //
    // This effect returns early when startup failed, so nothing is listening. The retry
    // button cleared the issue and the app rendered — but the effect never ran again, so
    // `devicesUpdated`, `deviceUpdated` and `jobUpdated` were never subscribed for the rest
    // of the session. The retry handler knew half of this: it replayed `reload()` by hand,
    // with a comment saying the boot effect had already run. It could not replay the
    // subscription, and that is the half that matters — without it the grid moves only on
    // the three-second poll and no tile ever learns a frame arrived.
  }, [reload, startupAttempt]);

  const retry = useCallback(async () => {
    setRetryingStartup(true);
    try {
      const stillBlocked = await retryStartup();
      setStartupIssue(stillBlocked);
      // Came up: run the boot effect again, which loads the fleet *and* subscribes to
      // events. This used to call `reload()` by hand instead, which did the first and
      // could not do the second.
      if (!stillBlocked) setStartupAttempt((attempt) => attempt + 1);
    } catch (error) {
      setStartupIssue(describeError(error));
    } finally {
      setRetryingStartup(false);
    }
  }, []);

  return {
    devices,
    groups,
    metas,
    setMetas,
    jobs,
    reload,
    startupIssue,
    bootError,
    driverIssue,
    androidIssue,
    retryingStartup,
    retry,
  };
}
