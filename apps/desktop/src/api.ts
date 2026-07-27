import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AnalyticsSummary,
  AppLibraryItem,
  AppleIdConfig,
  AuthSession,
  DeviceGroup,
  DeviceInfo,
  DeviceMeta,
  JobRecord,
  LocalUser,
  MaterialItem,
  OpLog,
  ProxyConfig,
  PublishTask,
  ScheduleItem,
  StreamSettings,
  NurtureCommentCost,
  NurtureCostSummary,
  NurtureSessionStatus,
  NurtureSettings,
} from "./types";

export async function listDevices() {
  return invoke<DeviceInfo[]>("list_devices");
}

export async function refreshDevices() {
  return invoke<DeviceInfo[]>("refresh_devices");
}

export async function prepareDevice(udid: string) {
  return invoke<DeviceInfo>("prepare_device", { udid });
}

export async function installIpa(udid: string, path: string) {
  return invoke<void>("install_ipa", { udid, path });
}

export async function uninstallApp(udid: string, bundleId: string) {
  return invoke<void>("uninstall_app", { udid, bundleId });
}

export async function screenshot(udid: string) {
  return invoke<string>("screenshot", { udid });
}

export async function syslog(udid: string, lines = 80) {
  return invoke<string>("syslog", { udid, lines });
}

export async function rebootDevice(udid: string) {
  return invoke<void>("reboot_device", { udid });
}

export async function deviceTap(
  udid: string,
  x: number,
  y: number,
  imageW?: number,
  imageH?: number,
) {
  return invoke<void>("device_tap", {
    udid,
    x,
    y,
    imageW: imageW ?? null,
    imageH: imageH ?? null,
  });
}

export async function deviceSwipe(
  udid: string,
  fromX: number,
  fromY: number,
  toX: number,
  toY: number,
  imageW?: number,
  imageH?: number,
) {
  return invoke<void>("device_swipe", {
    udid,
    gesture: {
      from: { x: fromX, y: fromY },
      to: { x: toX, y: toY },
      durationMs: 280,
    },
    imageW: imageW ?? null,
    imageH: imageH ?? null,
  });
}

export async function deviceTypeText(udid: string, text: string) {
  return invoke<void>("device_type_text", { udid, text });
}

export async function deviceHome(udid: string) {
  return invoke<void>("device_home", { udid });
}

export async function groupInput(payload: {
  udids: string[];
  kind: string;
  x?: number;
  y?: number;
  toX?: number;
  toY?: number;
  text?: string;
  imageW?: number;
  imageH?: number;
}) {
  return invoke<void>("group_input", {
    udids: payload.udids,
    kind: payload.kind,
    x: payload.x ?? null,
    y: payload.y ?? null,
    toX: payload.toX ?? null,
    toY: payload.toY ?? null,
    text: payload.text ?? null,
    imageW: payload.imageW ?? null,
    imageH: payload.imageH ?? null,
  });
}

export async function getStreamSettings() {
  return invoke<StreamSettings>("get_stream_settings");
}

export async function setStreamSettings(settings: StreamSettings) {
  return invoke<StreamSettings>("set_stream_settings", { settings });
}

export async function latestFrame(udid: string) {
  return invoke<string | null>("latest_frame", { udid });
}

export async function listJobs() {
  return invoke<JobRecord[]>("list_jobs");
}

export async function runScript(scriptJson: string, udids: string[]) {
  return invoke<JobRecord>("run_script", { scriptJson, udids });
}

export async function cancelJob(jobId: string) {
  return invoke<void>("cancel_job", { jobId });
}

export async function listScripts() {
  return invoke<[string, string][]>("list_scripts");
}

export async function saveScript(name: string, bodyJson: string) {
  return invoke<void>("save_script", { name, bodyJson });
}

export async function exampleScript() {
  return invoke<string>("example_script");
}

export async function getAppleId() {
  return invoke<AppleIdConfig>("get_apple_id");
}

export async function setAppleId(email: string, password: string) {
  return invoke<void>("set_apple_id", { email, password });
}

export async function clearAppleId() {
  return invoke<void>("clear_apple_id");
}

export async function resignWda(udid: string) {
  return invoke<string>("resign_wda", { udid });
}

export async function bulkResignWda(udids: string[]) {
  return invoke<string[]>("bulk_resign_wda", { udids });
}

export async function driverMode() {
  return invoke<string>("driver_mode");
}

export async function authSession() {
  return invoke<AuthSession>("auth_session");
}

export async function authLogin(email: string, password: string) {
  return invoke<LocalUser>("auth_login", { email, password });
}

export async function authRegister(email: string, password: string) {
  return invoke<LocalUser>("auth_register", { email, password });
}

export async function getDeviceMeta(udid: string) {
  return invoke<DeviceMeta>("get_device_meta", { udid });
}

export async function saveDeviceMeta(meta: DeviceMeta) {
  return invoke<void>("save_device_meta", { meta });
}

export async function listGroups() {
  return invoke<DeviceGroup[]>("list_groups");
}

export async function saveGroup(group: DeviceGroup) {
  return invoke<DeviceGroup>("save_group", { group });
}

export async function deleteGroup(id: string) {
  return invoke<void>("delete_group", { id });
}

export async function listProxies() {
  return invoke<ProxyConfig[]>("list_proxies");
}

export async function saveProxy(proxy: ProxyConfig) {
  return invoke<ProxyConfig>("save_proxy", { proxy });
}

export async function deleteProxy(id: string) {
  return invoke<void>("delete_proxy", { id });
}

export async function exportProxyConfig(id: string) {
  return invoke<string>("export_proxy_config", { id });
}

export async function listMaterials() {
  return invoke<MaterialItem[]>("list_materials");
}

export async function addMaterial(sourcePath: string, name?: string) {
  return invoke<MaterialItem>("add_material", { sourcePath, name: name ?? null });
}

export async function deleteMaterial(id: string) {
  return invoke<void>("delete_material", { id });
}

export async function pushMaterial(udid: string, materialId: string) {
  return invoke<string>("push_material", { udid, materialId });
}

export async function listAppsLibrary() {
  return invoke<AppLibraryItem[]>("list_apps_library");
}

export async function addAppLibrary(
  sourcePath: string,
  name?: string,
  bundleId?: string,
  version?: string,
) {
  return invoke<AppLibraryItem>("add_app_library", {
    sourcePath,
    name: name ?? null,
    bundleId: bundleId ?? null,
    version: version ?? null,
  });
}

export async function deleteAppLibrary(id: string) {
  return invoke<void>("delete_app_library", { id });
}

export async function installLibraryApp(udid: string, appId: string) {
  return invoke<void>("install_library_app", { udid, appId });
}

export async function listSchedules() {
  return invoke<ScheduleItem[]>("list_schedules");
}

export async function saveSchedule(schedule: ScheduleItem) {
  return invoke<ScheduleItem>("save_schedule", { schedule });
}

export async function deleteSchedule(id: string) {
  return invoke<void>("delete_schedule", { id });
}

export async function listPublishTasks() {
  return invoke<PublishTask[]>("list_publish_tasks");
}

export async function createPublishTask(
  name: string,
  scriptName: string,
  materialIds: string[],
  udids: string[],
) {
  return invoke<PublishTask>("create_publish_task", {
    name,
    scriptName,
    materialIds,
    udids,
  });
}

export async function listOpLogs(limit = 100) {
  return invoke<OpLog[]>("list_op_logs", { limit });
}

export async function listUsers() {
  return invoke<LocalUser[]>("list_users");
}

export async function analyticsSummary() {
  return invoke<AnalyticsSummary>("analytics_summary");
}

export async function apiDocs() {
  return invoke<string>("api_docs");
}

export async function nurtureGetSettings() {
  return invoke<NurtureSettings>("nurture_get_settings");
}

export async function nurtureSaveSettings(settings: NurtureSettings) {
  return invoke<NurtureSettings>("nurture_save_settings", { settings });
}

export async function nurtureListCosts(limit = 100) {
  return invoke<NurtureCommentCost[]>("nurture_list_costs", { limit });
}

export async function nurtureCostSummary() {
  return invoke<NurtureCostSummary>("nurture_cost_summary");
}

export async function nurtureSessionStatus() {
  return invoke<NurtureSessionStatus[]>("nurture_session_status");
}

export async function nurtureStart(udids: string[], durationMinutes?: number | null) {
  return invoke<string[]>("nurture_start", {
    udids,
    durationMinutes: durationMinutes ?? null,
  });
}

export async function nurtureStop(udids: string[] = []) {
  return invoke<void>("nurture_stop", { udids });
}

export function listenRiviuEvents(handler: (payload: unknown) => void): Promise<UnlistenFn> {
  return listen("riviu://event", (event) => handler(event.payload));
}
