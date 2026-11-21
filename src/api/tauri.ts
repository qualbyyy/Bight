import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type {
  AppProfile,
  ClashConnections,
  ConnectionStatus,
  LogEntry,
  MergeReport,
  NodeConfig,
  PairingPayload,
  PingResult,
  ProfileMode,
  RoutingRule,
  SyncHostInfo,
  SyncPeer,
  Subscription,
} from "../types";

export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function getProfile(): Promise<AppProfile> {
  return invoke<AppProfile>("get_profile");
}

export async function listSubscriptions(): Promise<Subscription[]> {
  return invoke<Subscription[]>("list_subscriptions");
}

export async function listNodes(): Promise<NodeConfig[]> {
  return invoke<NodeConfig[]>("list_nodes");
}

export async function importSubscription(
  url: string,
  name?: string,
): Promise<Subscription> {
  return invoke<Subscription>("import_subscription", {
    url,
    name: name?.trim() || null,
  });
}

export async function refreshSubscription(
  subscriptionId: string,
): Promise<Subscription> {
  return invoke<Subscription>("refresh_subscription", { subscriptionId });
}

export async function deleteSubscription(subscriptionId: string): Promise<void> {
  return invoke("delete_subscription", { subscriptionId });
}

export async function createRule(input: {
  enabled: boolean;
  matchType: string;
  value: string;
  action: string;
  comment: string;
}): Promise<RoutingRule> {
  return invoke<RoutingRule>("create_rule", { input });
}

export async function deleteRule(ruleId: string): Promise<void> {
  return invoke("delete_rule", { ruleId });
}

export async function connect(
  nodeId: string | null,
  mode: ProfileMode,
): Promise<{ node_id: string; mode: ProfileMode; mixed_port: number; clash_port: number }> {
  return invoke("connect", { nodeId, mode });
}

export async function disconnect(): Promise<void> {
  return invoke("disconnect");
}

export async function getStatus(): Promise<ConnectionStatus> {
  return invoke<ConnectionStatus>("get_status");
}

export async function getLogs(limit = 120): Promise<LogEntry[]> {
  return invoke<LogEntry[]>("get_logs", { limit });
}

export async function setMode(mode: ProfileMode): Promise<void> {
  return invoke("set_mode", { mode });
}

export async function pingAll(
  testUrl = "https://www.gstatic.com/generate_204",
  timeoutMs = 2500,
): Promise<PingResult[]> {
  return invoke<PingResult[]>("ping_all", { testUrl, timeoutMs });
}

export async function getActiveConnections(): Promise<ClashConnections> {
  return invoke<ClashConnections>("get_active_connections");
}

export async function startSyncHost(): Promise<SyncHostInfo> {
  return invoke<SyncHostInfo>("sync_start_host");
}

export async function stopSyncHost(): Promise<void> {
  return invoke("sync_stop_host");
}

export async function discoverSyncPeers(timeoutMs = 3000): Promise<SyncPeer[]> {
  return invoke<SyncPeer[]>("sync_discover_peers", { timeoutMs });
}

export async function connectSyncPeer(pairingUri: string): Promise<MergeReport> {
  return invoke<MergeReport>("sync_connect_peer", { pairingUri });
}

export async function onEvent<T>(
  eventName: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (!isTauri) {
    return () => undefined;
  }
  return listen<T>(eventName, (event) => handler(event.payload));
}

export type { PairingPayload };
