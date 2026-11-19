import { nativeInvoke } from "./native";
import type {
  Session,
  CreateSessionRequest,
  Folder,
  CreateFolderRequest,
  SessionGroup,
  CredentialProfile,
  SearchResult,
  SerialPortInfo,
} from "../types";

// --- Sessions ---

export async function createSession(request: CreateSessionRequest): Promise<Session> {
  return nativeInvoke("create_session", { request });
}

export async function getSessions(folderId?: string): Promise<Session[]> {
  return nativeInvoke("get_sessions", { folderId });
}

export async function getSession(id: string): Promise<Session> {
  return nativeInvoke("get_session", { id });
}

export async function updateSession(request: Partial<Session> & { id: string; clear_folder?: boolean }): Promise<Session> {
  return nativeInvoke("update_session", { request });
}

export async function deleteSession(id: string): Promise<void> {
  return nativeInvoke("delete_session", { id });
}

export async function searchSessions(query: string): Promise<SearchResult[]> {
  return nativeInvoke("search_sessions", { query });
}

// --- Folders ---

export async function createFolder(request: CreateFolderRequest): Promise<Folder> {
  return nativeInvoke("create_folder", { request });
}

export async function getFolders(): Promise<Folder[]> {
  return nativeInvoke("get_folders");
}

export async function updateFolder(
  id: string,
  name?: string,
  parentId?: string,
  sortOrder?: number,
  clearParent = false,
): Promise<Folder> {
  return nativeInvoke("update_folder", { id, name, parentId, sortOrder, clearParent });
}

export async function deleteFolder(id: string, action: "move_to_parent" | "delete_all"): Promise<void> {
  return nativeInvoke("delete_folder", { id, action });
}

// --- Session groups ---

export async function createSessionGroup(name: string): Promise<SessionGroup> {
  return nativeInvoke("create_session_group", { request: { name } });
}

export async function getSessionGroups(): Promise<SessionGroup[]> {
  return nativeInvoke("get_session_groups");
}

export async function updateSessionGroup(id: string, name: string): Promise<SessionGroup> {
  return nativeInvoke("update_session_group", { id, name });
}

export async function deleteSessionGroup(id: string): Promise<void> {
  return nativeInvoke("delete_session_group", { id });
}

export async function getSessionGroupMembers(groupId: string): Promise<string[]> {
  return nativeInvoke("get_session_group_members", { groupId });
}

export async function setSessionGroupMembers(groupId: string, sessionIds: string[]): Promise<SessionGroup> {
  return nativeInvoke("set_session_group_members", {
    request: { group_id: groupId, session_ids: sessionIds },
  });
}

// --- Credentials ---

export async function storeCredential(request: {
  name: string;
  username: string;
  authentication_method: string;
  password?: string;
  description?: string;
}): Promise<CredentialProfile> {
  return nativeInvoke("store_credential", { request });
}

export async function getCredentialProfiles(): Promise<CredentialProfile[]> {
  return nativeInvoke("get_credential_profiles");
}

export async function getCredentialSecret(id: string): Promise<string | null> {
  return nativeInvoke("get_credential_secret", { id });
}

export async function updateCredential(request: {
  id: string;
  name: string;
  username: string;
  authentication_method: string;
  password?: string;
  description?: string;
}): Promise<CredentialProfile> {
  return nativeInvoke("update_credential", { request });
}

export async function deleteCredential(id: string): Promise<void> {
  return nativeInvoke("delete_credential", { id });
}

// --- Serial ---

export async function listSerialPorts(): Promise<SerialPortInfo[]> {
  return nativeInvoke("list_serial_ports");
}

// --- Graphical remote desktop ---

export async function launchVncViewer(host: string, port: number): Promise<void> {
  return nativeInvoke("launch_vnc_viewer", { host, port });
}

export async function startVncProxy(host: string, port: number): Promise<{ id: string; url: string }> {
  return nativeInvoke("start_vnc_proxy", { host, port });
}

export async function stopVncProxy(id: string): Promise<void> {
  return nativeInvoke("stop_vnc_proxy", { id });
}

export async function stopMonitorWorkerForUpdate(): Promise<void> {
  return nativeInvoke("stop_monitor_worker_for_update");
}

export async function restartMonitorWorkerAfterUpdateFailure(): Promise<void> {
  return nativeInvoke("restart_monitor_worker_after_update_failure");
}

// --- Network health monitoring ---

export interface MonitorDefinition {
  id: string;
  sessionId: string;
  sessionName: string;
  host: string;
  providerId: "arista-eos";
  monitorKind: "interfaces";
  interfaceName?: string;
  displayName: string;
  intervalSeconds: number;
  retentionHours: number;
  enabled: boolean;
  latestStatus?: "healthy" | "warning" | "critical" | "unknown" | "failed";
  latestObservedAt?: string;
  openIncidentCount: number;
  createdAt: string;
  updatedAt: string;
}

export interface MonitorSample {
  id: number;
  monitorId: string;
  observedAt: string;
  status: "healthy" | "warning" | "critical" | "unknown" | "failed";
  durationMs: number;
  summary: { interfaceCount?: number; downCount?: number; errorCount?: number; logCount?: number };
  error?: string;
}

export interface MonitorSampleDetail {
  sample: MonitorSample;
  interfaces: Array<{
    name: string;
    adminStatus?: string;
    operStatus?: string;
    inputErrors?: number;
    outputErrors?: number;
    inputDiscards?: number;
    outputDiscards?: number;
  }>;
  rawOutput?: string;
}

export interface MonitorIncident {
  id: string;
  monitorId: string;
  fingerprint: string;
  severity: "info" | "warning" | "critical";
  state: "open" | "acknowledged" | "resolved";
  title: string;
  details?: string;
  startedAt: string;
  lastSeenAt: string;
  resolvedAt?: string;
  occurrenceCount: number;
}

export interface MonitorBaseline {
  id: string;
  monitorId: string;
  name: string;
  sourceObservedAt: string;
  capturedAt: string;
}

export interface BaselineComparison {
  baseline: MonitorBaseline;
  currentObservedAt: string;
  addedInterfaces: string[];
  removedInterfaces: string[];
  changedInterfaces: string[];
}

export interface MaintenanceMarker {
  id: string;
  sessionId?: string;
  startsAt: string;
  endsAt: string;
  reason: string;
  createdAt: string;
}

export async function createMonitorDefinition(request: {
  sessionId: string;
  providerId: "arista-eos";
  monitorKind: "interfaces";
  interfaceName: string;
  displayName: string;
  intervalSeconds: number;
  retentionHours: number;
}): Promise<MonitorDefinition> {
  return nativeInvoke("create_monitor_definition", { request });
}

export async function getMonitorDefinitions(): Promise<MonitorDefinition[]> {
  return nativeInvoke("get_monitor_definitions");
}

export async function setMonitorEnabled(id: string, enabled: boolean): Promise<MonitorDefinition> {
  return nativeInvoke("set_monitor_enabled", { id, enabled });
}

export async function deleteMonitorDefinition(id: string): Promise<void> {
  return nativeInvoke("delete_monitor_definition", { id });
}

export async function runMonitorNow(id: string): Promise<MonitorDefinition> {
  return nativeInvoke("run_monitor_now", { id });
}

export async function getMonitorSamples(monitorId: string, limit = 100): Promise<MonitorSample[]> {
  return nativeInvoke("get_monitor_samples", { monitorId, limit });
}

export async function getMonitorSampleDetail(sampleId: number): Promise<MonitorSampleDetail> {
  return nativeInvoke("get_monitor_sample_detail", { sampleId });
}

export async function getMonitorIncidents(includeResolved = false): Promise<MonitorIncident[]> {
  return nativeInvoke("get_monitor_incidents", { includeResolved });
}

export async function acknowledgeMonitorIncident(id: string): Promise<void> {
  return nativeInvoke("acknowledge_monitor_incident", { id });
}

export async function captureMonitorBaseline(monitorId: string, name: string): Promise<MonitorBaseline> {
  return nativeInvoke("capture_monitor_baseline", { monitorId, name });
}

export async function getMonitorBaselines(monitorId: string): Promise<MonitorBaseline[]> {
  return nativeInvoke("get_monitor_baselines", { monitorId });
}

export async function compareMonitorBaseline(id: string): Promise<BaselineComparison> {
  return nativeInvoke("compare_monitor_baseline", { id });
}

export async function deleteMonitorBaseline(id: string): Promise<void> {
  return nativeInvoke("delete_monitor_baseline", { id });
}

export async function createMonitorMaintenance(request: {
  sessionId?: string;
  startsAt: string;
  endsAt: string;
  reason: string;
}): Promise<MaintenanceMarker> {
  return nativeInvoke("create_monitor_maintenance", { request });
}

export async function getMonitorMaintenance(): Promise<MaintenanceMarker[]> {
  return nativeInvoke("get_monitor_maintenance");
}

export async function deleteMonitorMaintenance(id: string): Promise<void> {
  return nativeInvoke("delete_monitor_maintenance", { id });
}

// --- Data maintenance ---

export async function clearRecentSessions(): Promise<void> {
  return nativeInvoke("clear_recent_sessions");
}

export async function resetApplicationData(): Promise<void> {
  return nativeInvoke("reset_application_data");
}
