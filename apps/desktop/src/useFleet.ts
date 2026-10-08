import { useCallback, useEffect, useRef, useState } from "react";

import {
  androidToolProblems,
  androidUnavailableReason,
  appLogDirectory,
  driverDegradedReason,
  listDeviceMetas,
  ensureDeviceNumbers,
  listDevices,
  listGroups,
  listJobs,
  listenRiviuEvents,
  retryStartup,
  startupError,
} from "./api";
import { readQueryClient } from "./readQuery";
import { describeError } from "./describeError";
import { NurtureFailureWatch } from "./nurtureFailureWatch";
import { announceAdbServerNotice } from "./adbServerNotice";
import type { DeviceGroup, DeviceInfo, DeviceMeta, JobRecord } from "./types";

/**
 * One watch for the whole app, outside the hook.
 *
 * Module scope rather than a `useRef`: it batches failures over a couple of seconds, and a
 * per-mount instance would split one fleet run's failures across two batches if the hook
 * ever remounted — which is the shape that turns one toast into several.
 */
const failureWatch = new NurtureFailureWatch();
class SupersededMetadataRead extends Error {}

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
  refreshMetas: () => Promise<DeviceMeta[]>;
  numberAllocationError: string | null;
  retryDeviceNumbers: () => Promise<void>;
  retryingNumbers: boolean;
  jobs: JobRecord[];
  /// Re-read devices, jobs, groups and records from the backend.
  reload: () => Promise<void>;

  /// Non-null when the backend refused to start; the shell shows nothing else.
  startupIssue: string | null | undefined;
  /// The backend is up but a call failed.
  bootError: string | null;
  /// True after the first complete fleet read, including a failed read.
  fleetSettled: boolean;
  /// The device sidecar is degraded, so an empty fleet has a cause worth naming.
  driverIssue: string | null;
  /// Android specifically is unavailable; asked apart because the two halves fail apart.
  androidIssue: string | null;
  androidToolProblems: string[];
  logDirectory: string | null;
  retryingStartup: boolean;
  /// Ask the backend to start again, and resubscribe if it does.
  retry: () => Promise<void>;
}

export function useFleet(): Fleet {
  const [devices, setDevices] = useState<DeviceInfo[]>([]);
  const [groups, setGroups] = useState<DeviceGroup[]>([]);
  const [metas, setMetasState] = useState<DeviceMeta[]>([]);
  const metadataEpoch = useRef({ version: 0, mounted: true });
  useEffect(() => {
    const epoch = metadataEpoch.current;
    epoch.mounted = true;
    return () => { epoch.mounted = false; epoch.version++; };
  }, []);
  const refreshMetas = useCallback(async () => {
    const revision = ++metadataEpoch.current.version;
    // Do not join a cached/in-flight read started before the acknowledging mutation.
    await readQueryClient.cancelQueries({ queryKey: ["deviceMetadata"] });
    if (revision !== metadataEpoch.current.version || !metadataEpoch.current.mounted) throw new SupersededMetadataRead("Đã có lượt đọc metadata mới hơn; giữ dữ liệu hiện tại.");
    await readQueryClient.invalidateQueries({ queryKey: ["deviceMetadata"], refetchType: "none" });
    if (revision !== metadataEpoch.current.version || !metadataEpoch.current.mounted) throw new SupersededMetadataRead();
    const rows = await listDeviceMetas();
    if (!metadataEpoch.current.mounted || revision !== metadataEpoch.current.version) throw new SupersededMetadataRead("Đã có lượt đọc metadata mới hơn; giữ dữ liệu hiện tại.");
    setMetasState(rows);
    return rows;
  }, []);
  const [numberAllocationError, setNumberAllocationError] = useState<string | null>(null);
  const [retryingNumbers, setRetryingNumbers] = useState(false);
  const numberRetry = useRef(false);
  const allocationQueue = useRef<Promise<void>>(Promise.resolve());
  const allocateNumbers = useCallback((serials: string[], active: () => boolean) => {
    const task = allocationQueue.current.catch(() => undefined).then(async () => {
      if (!active()) return;
      // The mutation response is a historical snapshot, never a publishable read model.
      await ensureDeviceNumbers(serials);
      if (active()) {
        await refreshMetas();
        setNumberAllocationError(null);
      }
    });
    allocationQueue.current = task;
    return task;
  }, [refreshMetas]);
  const [jobs, setJobs] = useState<JobRecord[]>([]);
  const [bootError, setBootError] = useState<string | null>(null);
  const [fleetSettled, setFleetSettled] = useState(false);
  const [driverIssue, setDriverIssue] = useState<string | null>(null);
  const [androidIssue, setAndroidIssue] = useState<string | null>(null);
  // Named apart from the imported command on purpose: calling the state variable the same
  // thing shadows it, and `await androidToolProblems()` then calls an array instead.
  const [toolProblems, setToolProblems] = useState<string[]>([]);
  const [logDirectory, setLogDirectory] = useState<string | null>(null);
  const [startupIssue, setStartupIssue] = useState<string | null | undefined>(undefined);
  const [retryingStartup, setRetryingStartup] = useState(false);
  /// Bumped by the retry button, and read by the boot effect below as a reason to run
  /// again. A counter rather than `startupIssue` in that effect's dependencies: the
  /// effect *sets* the issue, so depending on it makes every ordinary startup run the
  /// whole thing twice — two `startup_error` calls, two subscriptions, one of them
  /// immediately torn down.
  const [startupAttempt, setStartupAttempt] = useState(0);

  // Allocation is keyed by serials, not arrival order; the backend owns the transaction.
  const rosterSerials = JSON.stringify([...new Set(devices.map(device => device.udid))].sort());
  useEffect(() => {
    const serials: string[] = JSON.parse(rosterSerials);
    if (!serials.length) return;
    let cancelled = false;
    void allocateNumbers(serials, () => !cancelled && metadataEpoch.current.mounted).catch(error => {
      if (!cancelled && !(error instanceof SupersededMetadataRead)) setNumberAllocationError(`Chưa xác nhận gán số máy: ${describeError(error)}`);
    });
    return () => { cancelled = true; };
  }, [rosterSerials, allocateNumbers]);

  const retryDeviceNumbers = useCallback(async () => {
    if (numberRetry.current) return;
    numberRetry.current = true;
    setRetryingNumbers(true);
    try {
      await allocationQueue.current.catch(() => undefined);
      // Reconcile a possibly committed allocation before requesting missing serials only.
      const rows = await refreshMetas();
      const missing = (JSON.parse(rosterSerials) as string[]).filter(id =>
        !rows.some(row => row.udid === id && row.number != null && row.number > 0));
      if (missing.length) await allocateNumbers(missing, () => metadataEpoch.current.mounted);
      if (metadataEpoch.current.mounted) setNumberAllocationError(null);
    } catch (error) {
      if (metadataEpoch.current.mounted) setNumberAllocationError(`Chưa xác nhận gán số máy: ${describeError(error)}`);
    } finally {
      numberRetry.current = false;
      if (metadataEpoch.current.mounted) setRetryingNumbers(false);
    }
  }, [rosterSerials, refreshMetas, allocateNumbers]);

  const reload = useCallback(async () => {
    setFleetSettled(false);
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
      // failed read keeps committed labels and reports stale metadata.
      try {
        await refreshMetas();
        if (metadataEpoch.current.mounted) setBootError(null);
      } catch (error) {
        if (metadataEpoch.current.mounted && !(error instanceof SupersededMetadataRead)) setBootError(`Chưa cập nhật số/tên máy; giữ dữ liệu đã đọc: ${describeError(error)}`);
      }
      // An empty list can mean "nothing plugged in" or "the device sidecar never
      // started". Ask which, so the UI does not report the wrong one.
      setDriverIssue(await driverDegradedReason().catch(() => null));
      // Asked separately, because the two halves of the fleet fail for different
      // reasons and an Android phone that never appears used to say nothing at all.
      setAndroidIssue(await androidUnavailableReason().catch(() => null));
      // Asked here too, because a bundle that lost a file lists phones normally and only
      // fails when one is driven — so nothing else in this hook would ever notice.
      setToolProblems(await androidToolProblems().catch(() => []));
      setLogDirectory(await appLogDirectory().catch(() => null));
    } catch (e) {
      setBootError(describeError(e));
    } finally {
      setFleetSettled(true);
    }
  }, [refreshMetas]);

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
          } else if (event.type === "nurtureStatus") {
            // **The always-mounted listener, and that is the point.** This event had exactly
            // one subscriber and it lived inside `NurturePopup`, so a session that failed
            // while the panel was closed was seen by nothing at all. On 23/08/2026 two of
            // fourteen phones failed on their lock screens while the fleet chip still read
            // "14 sẵn sàng" — that chip counts phones that stream, and a locked phone
            // streams its lock screen perfectly.
            failureWatch.observe(event.status);
          } else if (event.type === "adbServerNotice") {
            // Same always-mounted listener: a server restart blacks out every tile at once,
            // and the reason has to reach the activity history whichever page is open.
            announceAdbServerNotice(event);
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
    refreshMetas,
    numberAllocationError,
    retryDeviceNumbers,
    retryingNumbers,
    jobs,
    reload,
    startupIssue,
    bootError,
    fleetSettled,
    driverIssue,
    androidIssue,
    androidToolProblems: toolProblems,
    logDirectory,
    retryingStartup,
    retry,
  };
}
