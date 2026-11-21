import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ComponentType,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { QRCodeSVG } from "qrcode.react";
import {
  Activity,
  AlertTriangle,
  ArrowDown,
  ArrowUp,
  Check,
  ChevronDown,
  ChevronRight,
  CircleHelp,
  Clipboard,
  Command,
  Copy,
  Download,
  ExternalLink,
  FilePlus2,
  Gauge,
  Globe2,
  Keyboard,
  Laptop,
  Link2,
  ListFilter,
  LoaderCircle,
  Moon,
  Pause,
  Play,
  Plus,
  Power,
  QrCode,
  RefreshCcw,
  Router,
  ScanLine,
  Search,
  Server,
  Settings2,
  ShieldAlert,
  SlidersHorizontal,
  Sun,
  Terminal,
  Trash2,
  Wifi,
  X,
} from "lucide-react";

import { BightLogo } from "./BightLogo";
import {
  BarcodePermissionError,
  isMobile,
  isScanCancelled,
  openCameraSettings,
  scanPairingQr,
} from "./api/barcode";
import * as api from "./api/tauri";
import type {
  AppProfile,
  AppTab,
  ClashConnection,
  ClashConnections,
  ConnectionStatus,
  LogEntry,
  MergeReport,
  NodeConfig,
  ProfileMode,
  RoutingAction,
  RoutingMatch,
  Subscription,
  SyncHostInfo,
  SyncPeer,
} from "./types";

type Theme = "light" | "dark";
type FontPreset = "default" | "terminal" | "paper" | "system" | "custom";
type IconComponent = ComponentType<{
  size?: number;
  strokeWidth?: number;
  className?: string;
}>;
type ThemeStyle = CSSProperties & Record<`--${string}`, string>;
type ConfirmTarget =
  | { kind: "subscription"; id: string; name: string }
  | { kind: "rule"; id: string; name: string }
  | null;

type AccentPreset = {
  name: string;
  base: string;
  light: string;
};

const ACCENT_PRESETS: AccentPreset[] = [
  { name: "Sage", base: "#2F7D57", light: "#4BA378" },
  { name: "Cobalt", base: "#2B5B84", light: "#528BB8" },
  { name: "Ochre", base: "#9E6324", light: "#C98A42" },
  { name: "Terracotta", base: "#A34435", light: "#CC6656" },
  { name: "Plum", base: "#6E4773", light: "#9A6DA1" },
  { name: "Ink", base: "#1B1A18", light: "#ECEAE4" },
];

const FONT_OPTIONS: Array<{ value: FontPreset; label: string; detail: string }> = [
  { value: "default", label: "Default", detail: "Inter / JetBrains Mono" },
  { value: "terminal", label: "Terminal", detail: "Full Mono" },
  { value: "paper", label: "Paper", detail: "Newsreader / Georgia" },
  { value: "system", label: "System", detail: "Шрифт ОС" },
  { value: "custom", label: "Свой", detail: "Установленный шрифт" },
];

const TAB_ITEMS: Array<{ value: AppTab; label: string; icon: IconComponent }> = [
  { value: "overview", label: "Обзор", icon: Gauge },
  { value: "nodes", label: "Ноды", icon: Server },
  { value: "rules", label: "Правила", icon: ListFilter },
  { value: "traffic", label: "Трафик", icon: Activity },
  { value: "logs", label: "Логи", icon: Terminal },
  { value: "settings", label: "Настройки", icon: Settings2 },
];

const ROUTING_OPTIONS = [
  { value: "global", label: "Глобальный" },
  { value: "russia_direct", label: "Обход РФ" },
  { value: "custom", label: "Пользовательский" },
];

const MODE_OPTIONS: Array<{ value: ProfileMode; label: string }> = [
  { value: "tun", label: "TUN" },
  { value: "system_proxy", label: "System Proxy" },
];

const RULE_MATCH_OPTIONS: Array<{ value: RoutingMatch; label: string }> = [
  { value: "domain", label: "Домен" },
  { value: "domain_suffix", label: "Суффикс" },
  { value: "domain_keyword", label: "Ключевое слово" },
  { value: "ip_cidr", label: "IP / CIDR" },
  { value: "rule_set", label: "Rule set" },
];

const RULE_ACTION_OPTIONS: Array<{ value: RoutingAction; label: string }> = [
  { value: "direct", label: "Direct" },
  { value: "proxy", label: "Proxy" },
  { value: "reject", label: "Reject" },
];

function getStored<T>(key: string, fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return value === null ? fallback : (JSON.parse(value) as T);
  } catch (error) {
    console.warn(`[Bight] Failed to read localStorage key: ${key}`, error);
    return fallback;
  }
}

function setStored<T>(key: string, value: T): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch (error) {
    console.warn(`[Bight] Failed to write localStorage key: ${key}`, error);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function accentForeground(hex: string): string {
  const normalized = hex.replace("#", "");
  if (!/^[0-9a-f]{6}$/i.test(normalized)) {
    return "#131312";
  }
  const red = Number.parseInt(normalized.slice(0, 2), 16);
  const green = Number.parseInt(normalized.slice(2, 4), 16);
  const blue = Number.parseInt(normalized.slice(4, 6), 16);
  const brightness = (red * 299 + green * 587 + blue * 114) / 1000;
  return brightness > 155 ? "#131312" : "#FFFFFF";
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

function formatRate(value: number): string {
  return `${formatBytes(Math.max(0, value))}/s`;
}

function formatDuration(totalSeconds: number): string {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  return [hours, minutes, rest].map((part) => String(part).padStart(2, "0")).join(":");
}

function formatTtl(milliseconds: number): string {
  return formatDuration(Math.ceil(Math.max(0, milliseconds) / 1000));
}

function formatDate(timestamp?: number): string {
  if (!timestamp) return "—";
  return new Intl.DateTimeFormat("ru-RU", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(timestamp));
}

function formatPin(pin: string): string {
  return pin.length === 6 ? `${pin.slice(0, 3)} ${pin.slice(3)}` : pin;
}

function protocolLabel(node: NodeConfig): string {
  if (node.protocol === "vless") {
    return node.tls.reality ? "VLESS • REALITY" : "VLESS";
  }
  if (node.protocol === "hysteria2") return "HYSTERIA2";
  if (node.protocol === "trojan") return "TROJAN";
  return "SHADOWSOCKS";
}

function protocolShort(node: NodeConfig): string {
  return node.protocol === "hysteria2" ? "HY2" : node.protocol.toUpperCase();
}

function pingTone(delay?: number): string {
  if (delay === undefined) return "ping-neutral";
  if (delay < 80) return "ping-good";
  if (delay <= 200) return "ping-warn";
  return "ping-bad";
}

function pingLabel(delay?: number, error?: string): string {
  if (delay !== undefined) return `${delay} ms`;
  return error ? "Ошибка" : "—";
}

function humanMode(mode: ProfileMode): string {
  return mode === "tun" ? "TUN" : "System Proxy";
}

function metadataValue(connection: ClashConnection, keys: string[]): string | undefined {
  for (const key of keys) {
    const value = connection.metadata[key];
    if (typeof value === "string" && value.trim()) return value;
  }
  return undefined;
}

function connectionDomain(connection: ClashConnection): string {
  return metadataValue(connection, ["host", "destination", "hostname", "domain", "remoteDestination"]) ?? "Без домена";
}

function connectionProcess(connection: ClashConnection): string {
  return metadataValue(connection, ["process", "processName", "sourceProcess"]) ?? "локальный процесс";
}

function subscriptionTraffic(subscription: Subscription): number {
  if (!subscription.traffic_total || subscription.traffic_total <= 0) return 100;
  return Math.max(0, Math.min(100, 100 - ((subscription.traffic_used ?? 0) / subscription.traffic_total) * 100));
}

function subscriptionExpiry(subscription: Subscription): number {
  if (!subscription.expire_at) return 100;
  const thirtyDays = 30 * 24 * 60 * 60 * 1000;
  return Math.max(0, Math.min(100, ((subscription.expire_at - Date.now()) / thirtyDays) * 100));
}

function isInfrastructureError(message: string): boolean {
  return /cap_net_admin|wintun|permission denied|administrator|capability/i.test(message);
}

function useZoomGuard(): void {
  useEffect(() => {
    const onWheel = (event: WheelEvent) => {
      const target = event.target as HTMLElement | null;
      if (event.ctrlKey && !target?.closest("canvas")) event.preventDefault();
    };
    const onGesture = (event: Event) => {
      const target = event.target as HTMLElement | null;
      if (!target?.closest("canvas")) event.preventDefault();
    };
    document.addEventListener("wheel", onWheel, { passive: false });
    document.addEventListener("gesturestart", onGesture, { passive: false });
    return () => {
      document.removeEventListener("wheel", onWheel);
      document.removeEventListener("gesturestart", onGesture);
    };
  }, []);
}

export default function App() {
  useZoomGuard();

  const [theme, setTheme] = useState<Theme>(() => getStored<Theme>("bight.theme", "light"));
  const [accent, setAccent] = useState(() => getStored("bight.accent", ACCENT_PRESETS[0].base));
  const [tintBackground, setTintBackground] = useState(() => getStored("bight.tintBackground", false));
  const [fontPreset, setFontPreset] = useState<FontPreset>(() => getStored("bight.fontPreset", "default"));
  const [customFont, setCustomFont] = useState(() => getStored("bight.customFont", ""));
  const [customFontUrl, setCustomFontUrl] = useState(() => getStored("bight.customFontUrl", ""));
  const [activeTab, setActiveTab] = useState<AppTab>("overview");
  const [profile, setProfile] = useState<AppProfile | null>(null);
  const [nodes, setNodes] = useState<NodeConfig[]>([]);
  const [status, setStatus] = useState<ConnectionStatus>({ state: "disconnected" });
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [connections, setConnections] = useState<ClashConnections>({
    downloadTotal: 0,
    uploadTotal: 0,
    connections: [],
  });
  const [speed, setSpeed] = useState({ upload: 0, download: 0 });
  const [connectionStartedAt, setConnectionStartedAt] = useState<number | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [pingByNode, setPingByNode] = useState<Record<string, { delay?: number; error?: string }>>({});
  const [routingScenario, setRoutingScenario] = useState("global");
  const [pinging, setPinging] = useState(false);
  const [actionPending, setActionPending] = useState<"connecting" | "disconnecting" | null>(null);
  const [backendError, setBackendError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [syncOpen, setSyncOpen] = useState(false);
  const [syncHostInfo, setSyncHostInfo] = useState<SyncHostInfo | null>(null);
  const [confirmTarget, setConfirmTarget] = useState<ConfirmTarget>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [accentOpen, setAccentOpen] = useState(false);
  const [nowTick, setNowTick] = useState(() => Date.now());

  const refreshData = useCallback(async () => {
    if (!api.isTauri) return;
    const [profileResult, nodesResult, statusResult, logsResult, connectionsResult] = await Promise.allSettled([
      api.getProfile(),
      api.listNodes(),
      api.getStatus(),
      api.getLogs(),
      api.getActiveConnections(),
    ]);
    if (profileResult.status === "fulfilled") setProfile(profileResult.value);
    if (nodesResult.status === "fulfilled") setNodes(nodesResult.value);
    if (statusResult.status === "fulfilled") setStatus(statusResult.value);
    if (logsResult.status === "fulfilled") setLogs(logsResult.value);
    if (connectionsResult.status === "fulfilled") setConnections(connectionsResult.value);
    const rejected = [profileResult, nodesResult, statusResult].find((result) => result.status === "rejected");
    if (rejected?.status === "rejected") setBackendError(errorMessage(rejected.reason));
  }, []);

  useEffect(() => {
    void refreshData();
    let disposed = false;
    let cleanup: Array<() => void> = [];
    void Promise.all([
      api.onEvent<ConnectionStatus>("connection-status", (nextStatus) => {
        setStatus(nextStatus);
        if (nextStatus.state === "connected") setConnectionStartedAt((current) => current ?? Date.now());
        if (nextStatus.state === "disconnected") setConnectionStartedAt(null);
      }),
      api.onEvent<{ node_id: string; delay_ms?: number; error?: string }>("ping-result", (result) => {
        setPingByNode((current) => ({
          ...current,
          [result.node_id]: { delay: result.delay_ms, error: result.error },
        }));
      }),
      api.onEvent<LogEntry>("sing-box-log", (entry) => {
        setLogs((current) => [...current.slice(-119), entry]);
      }),
      api.onEvent<string>("sync-host-expired", () => setSyncHostInfo(null)),
    ])
      .then((unlisteners) => {
        if (disposed) unlisteners.forEach((unlisten) => unlisten());
        else cleanup = unlisteners;
      })
      .catch((error) => {
        if (!disposed) setBackendError(errorMessage(error));
      });
    return () => {
      disposed = true;
      cleanup.forEach((unlisten) => unlisten());
    };
  }, [refreshData]);

  useEffect(() => {
    const interval = window.setInterval(() => {
      setClock(Date.now());
      setNowTick(Date.now());
    }, 1000);
    return () => window.clearInterval(interval);
  }, []);

  useEffect(() => {
    if (!api.isTauri || status.state !== "connected") return;
    let alive = true;
    let previous: { upload: number; download: number; at: number } | null = null;
    const poll = async () => {
      try {
        const next = await api.getActiveConnections();
        if (!alive) return;
        const at = Date.now();
        if (previous) {
          const seconds = Math.max(0.1, (at - previous.at) / 1000);
          setSpeed({
            upload: Math.max(0, (next.uploadTotal - previous.upload) / seconds),
            download: Math.max(0, (next.downloadTotal - previous.download) / seconds),
          });
        }
        previous = { upload: next.uploadTotal, download: next.downloadTotal, at };
        setConnections(next);
      } catch (error) {
        console.warn("[Bight] Failed to refresh Clash connections", error);
      }
    };
    void poll();
    const interval = window.setInterval(() => void poll(), 3000);
    return () => {
      alive = false;
      window.clearInterval(interval);
    };
  }, [status.state]);

  useEffect(() => {
    if (!selectedNodeId && nodes.length > 0) {
      setSelectedNodeId(profile?.active_node_id ?? nodes[0].id);
    }
  }, [nodes, profile?.active_node_id, selectedNodeId]);

  useEffect(() => {
    setStored("bight.theme", theme);
  }, [theme]);
  useEffect(() => {
    setStored("bight.accent", accent);
  }, [accent]);
  useEffect(() => {
    setStored("bight.tintBackground", tintBackground);
  }, [tintBackground]);
  useEffect(() => {
    setStored("bight.fontPreset", fontPreset);
  }, [fontPreset]);
  useEffect(() => {
    setStored("bight.customFont", customFont);
  }, [customFont]);
  useEffect(() => {
    setStored("bight.customFontUrl", customFontUrl);
  }, [customFontUrl]);

  useEffect(() => {
    const id = "bight-custom-font-sheet";
    document.getElementById(id)?.remove();
    if (!customFontUrl.trim()) return;
    try {
      const url = new URL(customFontUrl);
      if (!/^https?:$/.test(url.protocol)) return;
      const link = document.createElement("link");
      link.id = id;
      link.rel = "stylesheet";
      link.href = url.toString();
      document.head.appendChild(link);
      return () => link.remove();
    } catch (error) {
      console.warn("[Bight] Invalid custom font URL", error);
    }
  }, [customFontUrl]);

  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen(true);
        return;
      }
      if (paletteOpen || importOpen || syncOpen || confirmTarget) return;
      const target = event.target as HTMLElement | null;
      if (target?.closest("input, textarea, button, [contenteditable='true']")) return;
      if ((event.key === "Enter" || event.key === " ") && selectedNodeId) {
        event.preventDefault();
        void toggleConnection();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  });

  const activeNodeId = status.state === "connected" ? status.node_id : profile?.active_node_id;
  const selectedNode = nodes.find((node) => node.id === selectedNodeId) ?? null;
  const activeNode = nodes.find((node) => node.id === activeNodeId) ?? selectedNode;
  const currentMode = profile?.mode ?? "system_proxy";
  const visualStatus = actionPending ?? status.state;
  const connectionAge = connectionStartedAt ? Math.max(0, clock - connectionStartedAt) : 0;
  const appStyle: ThemeStyle = {
    "--accent": accent,
    "--accent-fg": accentForeground(accent),
    "--font-ui":
      fontPreset === "terminal"
        ? "'JetBrains Mono', ui-monospace, SFMono-Regular, monospace"
        : fontPreset === "paper"
          ? "Newsreader, Georgia, serif"
          : fontPreset === "system"
            ? "system-ui, -apple-system, sans-serif"
            : fontPreset === "custom" && customFont.trim()
              ? `"${customFont.trim().replaceAll('"', "")}", ui-sans-serif, system-ui, sans-serif`
              : "Inter, 'IBM Plex Sans', ui-sans-serif, system-ui, sans-serif",
    "--font-mono": "'JetBrains Mono', 'IBM Plex Mono', ui-monospace, SFMono-Regular, monospace",
    "--bg": tintBackground ? "color-mix(in srgb, var(--bg-base) 93%, var(--accent) 7%)" : "var(--bg-base)",
    "--surface": tintBackground ? "color-mix(in srgb, var(--surface-base) 96%, var(--accent) 4%)" : "var(--surface-base)",
    "--border": tintBackground ? "color-mix(in srgb, var(--border-base) 86%, var(--accent) 14%)" : "var(--border-base)",
  };

  const subscriptionGroups = useMemo(() => {
    const groups: Array<{ id: string; name: string; subscription?: Subscription; nodes: NodeConfig[] }> = profile?.subscriptions
      .filter((subscription) => !subscription.is_deleted)
      .map((subscription) => ({
        id: subscription.id,
        name: subscription.name,
        subscription,
        nodes: subscription.nodes.filter((node) => !node.is_deleted),
      })) ?? [];
    const customNodes = profile?.custom_nodes.filter((node) => !node.is_deleted) ?? nodes.filter((node) => !profile);
    if (customNodes.length > 0) {
      groups.push({ id: "custom", name: "Свои ноды", subscription: undefined, nodes: customNodes });
    }
    return groups;
  }, [nodes, profile]);

  const runAction = useCallback(async (action: () => Promise<void>) => {
    setActionError(null);
    try {
      await action();
    } catch (error) {
      setActionError(errorMessage(error));
    }
  }, []);

  async function toggleConnection(): Promise<void> {
    if (actionPending) return;
    setActionError(null);
    if (status.state === "connected") {
      setActionPending("disconnecting");
      try {
        await api.disconnect();
        setStatus({ state: "disconnected" });
        setConnectionStartedAt(null);
        setSpeed({ upload: 0, download: 0 });
      } catch (error) {
        setActionError(errorMessage(error));
      } finally {
        setActionPending(null);
      }
      return;
    }
    if (!selectedNodeId) {
      setActionError("Сначала выберите ноду.");
      setActiveTab("nodes");
      return;
    }
    setActionPending("connecting");
    setStatus({ state: "connecting" });
    try {
      const result = await api.connect(selectedNodeId, currentMode);
      setStatus({ state: "connected", node_id: result.node_id, clash_port: result.clash_port, mode: result.mode });
      setConnectionStartedAt(Date.now());
      setProfile((current) => (current ? { ...current, active_node_id: result.node_id, mode: result.mode } : current));
    } catch (error) {
      const message = errorMessage(error);
      setStatus({ state: "error", message });
      setActionError(message);
    } finally {
      setActionPending(null);
    }
  }

  async function handleModeChange(mode: ProfileMode): Promise<void> {
    if (mode === currentMode) return;
    await runAction(async () => {
      await api.setMode(mode);
      setProfile((current) => (current ? { ...current, mode } : current));
    });
  }

  async function handlePing(): Promise<void> {
    if (pinging) return;
    setPinging(true);
    setActionError(null);
    try {
      const results = await api.pingAll();
      setPingByNode((current) => {
        const next = { ...current };
        for (const result of results) next[result.node_id] = { delay: result.delay_ms, error: result.error };
        return next;
      });
    } catch (error) {
      setActionError(errorMessage(error));
    } finally {
      setPinging(false);
    }
  }

  async function importFromClipboard(): Promise<void> {
    try {
      const text = await navigator.clipboard.readText();
      if (!text.trim()) {
        setActionError("Буфер обмена пуст.");
        return;
      }
      setImportOpen(true);
      setImportInitialUrl(text.trim());
    } catch (error) {
      console.warn("[Bight] Failed to read clipboard", error);
      setActionError("Не удалось прочитать буфер обмена.");
    }
  }

  async function refreshAfterMutation(): Promise<void> {
    await refreshData();
  }

  const [importInitialUrl, setImportInitialUrl] = useState("");
  const [importName, setImportName] = useState("");

  async function submitImport(): Promise<void> {
    if (!importInitialUrl.trim()) return;
    setActionError(null);
    try {
      await api.importSubscription(importInitialUrl.trim(), importName.trim() || undefined);
      setImportOpen(false);
      setImportInitialUrl("");
      setImportName("");
      await refreshAfterMutation();
      setActiveTab("nodes");
    } catch (error) {
      setActionError(errorMessage(error));
    }
  }

  async function confirmDelete(): Promise<void> {
    if (!confirmTarget) return;
    const target = confirmTarget;
    setConfirmTarget(null);
    setActionError(null);
    try {
      if (target.kind === "subscription") await api.deleteSubscription(target.id);
      else await api.deleteRule(target.id);
      await refreshAfterMutation();
    } catch (error) {
      setActionError(errorMessage(error));
    }
  }

  async function addTrafficRule(domain: string, action: RoutingAction): Promise<void> {
    if (!domain || domain === "Без домена") return;
    await runAction(async () => {
      await api.createRule({
        enabled: true,
        match_type: "domain",
        value: domain,
        action,
        comment: `Traffic inspector: ${action}`,
      });
      await refreshAfterMutation();
    });
  }

  const commandActions = useMemo(
    () => [
      { id: "ping", label: "Проверить пинг всех нод", meta: "Ping", icon: Activity, run: () => void handlePing() },
      { id: "sync", label: "Открыть LAN Sync", meta: "Sync", icon: QrCode, run: () => setSyncOpen(true) },
      { id: "logs", label: "Открыть логи sing-box", meta: "Logs", icon: Terminal, run: () => setActiveTab("logs") },
      { id: "theme", label: `Переключить тему: ${theme === "light" ? "тёмная" : "светлая"}`, meta: "Theme", icon: theme === "light" ? Moon : Sun, run: () => setTheme(theme === "light" ? "dark" : "light") },
      { id: "tint", label: `${tintBackground ? "Выключить" : "Включить"} тонирование фона`, meta: "Tint", icon: SlidersHorizontal, run: () => setTintBackground((value) => !value) },
      { id: "accent", label: "Сменить акцентный цвет", meta: "Accent", icon: SlidersHorizontal, run: () => { const currentIndex = ACCENT_PRESETS.findIndex((preset) => preset.base.toLowerCase() === accent.toLowerCase()); setAccent(ACCENT_PRESETS[(currentIndex + 1) % ACCENT_PRESETS.length].base); } },
      { id: "font", label: "Переключить шрифт интерфейса", meta: "Font", icon: Terminal, run: () => { const currentIndex = FONT_OPTIONS.findIndex((option) => option.value === fontPreset); setFontPreset(FONT_OPTIONS[(currentIndex + 1) % FONT_OPTIONS.length].value); } },
      { id: "mode-tun", label: "Переключить режим на TUN", meta: "Mode", icon: Router, run: () => void handleModeChange("tun") },
      { id: "mode-proxy", label: "Переключить режим на System Proxy", meta: "Mode", icon: Router, run: () => void handleModeChange("system_proxy") },
      { id: "nodes", label: "Перейти к нодам", meta: "Nodes", icon: Server, run: () => setActiveTab("nodes") },
      { id: "settings", label: "Открыть настройки", meta: "Settings", icon: Settings2, run: () => setActiveTab("settings") },
      ...nodes.map((node) => ({ id: `node-${node.id}`, label: `Выбрать ноду: ${node.name}`, meta: protocolLabel(node), icon: Server, run: () => { setSelectedNodeId(node.id); setActiveTab("nodes"); } })),
    ],
    [accent, fontPreset, handleModeChange, handlePing, nodes, theme, tintBackground],
  );

  return (
    <div className={`app-shell theme-${theme}`} style={appStyle}>
      <div className="app-frame">
        <header className="app-header">
          <button className="brand-lockup" type="button" onClick={() => setActiveTab("overview")} aria-label="Bight — обзор">
            <span className="brand-mark"><BightLogo /></span>
            <span>
              <strong>Bight</strong>
              <small>private connection</small>
            </span>
          </button>
          <nav className="desktop-nav" aria-label="Основная навигация">
            {TAB_ITEMS.map((item) => (
              <TabButton key={item.value} item={item} active={activeTab === item.value} onClick={() => setActiveTab(item.value)} />
            ))}
          </nav>
          <div className="header-actions">
            <button className="shortcut-button" type="button" onClick={() => setPaletteOpen(true)} title="Командная палитра">
              <Command size={14} />
              <span>Ctrl K</span>
            </button>
            <button className="icon-button" type="button" onClick={() => setActiveTab("settings")} title="Настройки">
              <Settings2 size={17} />
            </button>
          </div>
        </header>

        <main className="app-main">
          {backendError && !api.isTauri ? null : backendError ? <InlineNotice message={backendError} /> : null}
          {actionError ? <InlineNotice message={actionError} onClose={() => setActionError(null)} tone="error" /> : null}

          {activeTab !== "settings" && (
            <>
              <ConnectionBanner
                activeNode={activeNode}
                status={visualStatus}
                mode={currentMode}
                connectionAge={connectionAge}
                speed={speed}
                onToggle={() => void toggleConnection()}
                disabled={Boolean(actionPending)}
              />
              <div className="quick-controls">
                <div className="mode-toggle" aria-label="Режим подключения">
                  {MODE_OPTIONS.map((option) => (
                    <button
                      key={option.value}
                      className={currentMode === option.value ? "is-active" : ""}
                      type="button"
                      disabled={status.state === "connected" || actionPending !== null}
                      onClick={() => void handleModeChange(option.value)}
                    >
                      {option.label}
                    </button>
                  ))}
                </div>
                <CustomSelect value={routingScenario} options={ROUTING_OPTIONS} onChange={setRoutingScenario} compact />
                <button className="subtle-button" type="button" onClick={() => void handlePing()} disabled={pinging || nodes.length === 0}>
                  {pinging ? <LoaderCircle className="spin" size={15} /> : <Activity size={15} />}
                  Проверить пинг
                </button>
                <button className="subtle-button sync-quick" type="button" onClick={() => setSyncOpen(true)}>
                  <QrCode size={15} />
                  Sync
                </button>
              </div>
              {status.state === "error" && isInfrastructureError(status.message) && (
                <InfrastructureErrorCard message={status.message} onFallback={() => void handleModeChange("system_proxy")} />
              )}
            </>
          )}

          {activeTab === "overview" && (
            <OverviewScreen
              nodes={nodes}
              activeNodeId={activeNodeId}
              selectedNodeId={selectedNodeId}
              pingByNode={pingByNode}
              subscriptionGroups={subscriptionGroups}
              connections={connections}
              speed={speed}
              onSelectNode={setSelectedNodeId}
              onOpenNodes={() => setActiveTab("nodes")}
              onOpenTraffic={() => setActiveTab("traffic")}
              onOpenSync={() => setSyncOpen(true)}
              onImportClipboard={() => void importFromClipboard()}
              onPing={() => void handlePing()}
              onAddRule={addTrafficRule}
            />
          )}
          {activeTab === "nodes" && (
            <NodesScreen
              groups={subscriptionGroups}
              selectedNodeId={selectedNodeId}
              activeNodeId={activeNodeId}
              pingByNode={pingByNode}
              onSelectNode={setSelectedNodeId}
              onImport={() => {
                setImportInitialUrl("");
                setImportName("");
                setImportOpen(true);
              }}
              onRefreshSubscription={(subscriptionId) => void runAction(async () => {
                await api.refreshSubscription(subscriptionId);
                await refreshAfterMutation();
              })}
              onDeleteSubscription={(subscription) => setConfirmTarget({ kind: "subscription", id: subscription.id, name: subscription.name })}
              onPing={() => void handlePing()}
            />
          )}
          {activeTab === "rules" && (
            <RulesScreen
              rules={profile?.routing_rules.filter((rule) => !rule.is_deleted) ?? []}
              onDelete={(rule) => setConfirmTarget({ kind: "rule", id: rule.id, name: rule.value })}
              onCreated={refreshAfterMutation}
            />
          )}
          {activeTab === "traffic" && (
            <TrafficScreen
              connections={connections}
              pausedByDefault={false}
              onAddRule={addTrafficRule}
            />
          )}
          {activeTab === "logs" && <LogsScreen logs={logs} />}
          {activeTab === "settings" && (
            <SettingsScreen
              theme={theme}
              accent={accent}
              tintBackground={tintBackground}
              fontPreset={fontPreset}
              customFont={customFont}
              customFontUrl={customFontUrl}
              accentOpen={accentOpen}
              onThemeChange={setTheme}
              onAccentChange={(value) => {
                setAccent(value);
                setAccentOpen(false);
              }}
              onTintChange={setTintBackground}
              onFontPresetChange={setFontPreset}
              onCustomFontChange={setCustomFont}
              onCustomFontUrlChange={setCustomFontUrl}
              onAccentOpen={setAccentOpen}
              onReset={() => {
                setTheme("light");
                setAccent(ACCENT_PRESETS[0].base);
                setTintBackground(false);
                setFontPreset("default");
                setCustomFont("");
                setCustomFontUrl("");
              }}
            />
          )}
        </main>

        <nav className="bottom-tabbar" aria-label="Навигация на мобильном">
          {TAB_ITEMS.slice(0, 5).map((item) => (
            <TabButton key={item.value} item={item} active={activeTab === item.value} onClick={() => setActiveTab(item.value)} mobile />
          ))}
        </nav>
      </div>

      <ImportModal
        open={importOpen}
        url={importInitialUrl}
        name={importName}
        onUrlChange={setImportInitialUrl}
        onNameChange={setImportName}
        onClose={() => setImportOpen(false)}
        onSubmit={() => void submitImport()}
      />
      <SyncModal open={syncOpen} hostInfo={syncHostInfo} onHostInfo={setSyncHostInfo} onClose={() => setSyncOpen(false)} />
      <ConfirmModal
        target={confirmTarget}
        onClose={() => setConfirmTarget(null)}
        onConfirm={() => void confirmDelete()}
      />
      <CommandPalette
        open={paletteOpen}
        actions={commandActions}
        onClose={() => setPaletteOpen(false)}
      />
    </div>
  );
}

function TabButton({
  item,
  active,
  onClick,
  mobile = false,
}: {
  item: { value: AppTab; label: string; icon: IconComponent };
  active: boolean;
  onClick: () => void;
  mobile?: boolean;
}) {
  const Icon = item.icon;
  return (
    <button className={`tab-button ${active ? "is-active" : ""} ${mobile ? "is-mobile" : ""}`} type="button" onClick={onClick}>
      <Icon size={mobile ? 17 : 15} />
      <span>{item.label}</span>
    </button>
  );
}

function ConnectionBanner({
  activeNode,
  status,
  mode,
  connectionAge,
  speed,
  onToggle,
  disabled,
}: {
  activeNode: NodeConfig | null;
  status: string;
  mode: ProfileMode;
  connectionAge: number;
  speed: { upload: number; download: number };
  onToggle: () => void;
  disabled: boolean;
}) {
  const connected = status === "connected";
  const connecting = status === "connecting";
  const disconnecting = status === "disconnecting";
  const buttonLabel = connected ? "Отключить" : connecting ? "Подключение…" : disconnecting ? "Отключение…" : "Подключить";
  return (
    <section className={`connection-banner state-${status}`} aria-label="Состояние подключения">
      <div className="connection-summary">
        <div className="status-indicator" aria-hidden="true"><span /></div>
        <div className="connection-copy">
          <div className="connection-title-row">
            <strong title={activeNode?.name ?? "Нода не выбрана"}>{activeNode?.name ?? "Нода не выбрана"}</strong>
            {activeNode && <span className="technical-badge">{protocolLabel(activeNode)}</span>}
          </div>
          <div className="connection-telemetry">
            <span>{connected ? formatDuration(connectionAge / 1000) : "00:00:00"}</span>
            <span className="telemetry-separator">•</span>
            <span><ArrowUp size={12} /> {formatRate(speed.upload)}</span>
            <span><ArrowDown size={12} /> {formatRate(speed.download)}</span>
            <span className="mode-label">{humanMode(mode)}</span>
          </div>
        </div>
      </div>
      <button className="connect-button" type="button" onClick={onToggle} disabled={disabled || status === "connecting" || status === "disconnecting"}>
        {connecting || disconnecting ? <LoaderCircle className="spin" size={18} /> : <Power size={18} />}
        {buttonLabel}
      </button>
    </section>
  );
}

function OverviewScreen({
  nodes,
  activeNodeId,
  selectedNodeId,
  pingByNode,
  subscriptionGroups,
  connections,
  speed,
  onSelectNode,
  onOpenNodes,
  onOpenTraffic,
  onOpenSync,
  onImportClipboard,
  onPing,
  onAddRule,
}: {
  nodes: NodeConfig[];
  activeNodeId?: string;
  selectedNodeId: string | null;
  pingByNode: Record<string, { delay?: number; error?: string }>;
  subscriptionGroups: Array<{ id: string; name: string; subscription?: Subscription; nodes: NodeConfig[] }>;
  connections: ClashConnections;
  speed: { upload: number; download: number };
  onSelectNode: (id: string) => void;
  onOpenNodes: () => void;
  onOpenTraffic: () => void;
  onOpenSync: () => void;
  onImportClipboard: () => void;
  onPing: () => void;
  onAddRule: (domain: string, action: RoutingAction) => Promise<void>;
}) {
  if (nodes.length === 0) {
    return (
      <section className="empty-state card-surface">
        <div className="empty-icon"><Router size={24} /></div>
        <p className="eyebrow">BIGHT / READY</p>
        <h1>Добавьте первую ноду</h1>
        <p>Импортируйте подписку из буфера или перенесите профиль через LAN Sync.</p>
        <div className="empty-actions">
          <button className="primary-button" type="button" onClick={onImportClipboard}><Clipboard size={16} />Импорт из буфера</button>
          <button className="secondary-button" type="button" onClick={onOpenSync}><QrCode size={16} />Открыть Sync</button>
        </div>
        <div className="empty-hint"><Keyboard size={14} /> Ctrl K — командная палитра</div>
      </section>
    );
  }
  const recentNodes = subscriptionGroups.flatMap((group) => group.nodes).slice(0, 5);
  return (
    <div className="overview-grid">
      <section className="card-surface node-overview-card">
        <div className="section-heading">
          <div><p className="eyebrow">NODES / {nodes.length}</p><h2>Серверы</h2></div>
          <button className="text-button" type="button" onClick={onOpenNodes}>Все ноды <ChevronRight size={14} /></button>
        </div>
        <div className="mini-node-list">
          {recentNodes.map((node) => (
            <NodeRow
              key={node.id}
              node={node}
              active={node.id === activeNodeId}
              selected={node.id === selectedNodeId}
              ping={pingByNode[node.id]}
              onSelect={() => onSelectNode(node.id)}
              compact
            />
          ))}
        </div>
        <button className="subtle-button wide-button" type="button" onClick={onPing}><Activity size={15} /> Проверить все ноды</button>
      </section>
      <section className="card-surface traffic-overview-card">
        <div className="section-heading">
          <div><p className="eyebrow">CLASH API / LIVE</p><h2>Трафик</h2></div>
          <button className="icon-button" type="button" title="Открыть инспектор" onClick={onOpenTraffic}><ExternalLink size={15} /></button>
        </div>
        <div className="traffic-totals">
          <div><span>↑ отправлено</span><strong>{formatBytes(connections.uploadTotal)}</strong><small>{formatRate(speed.upload)}</small></div>
          <div><span>↓ получено</span><strong>{formatBytes(connections.downloadTotal)}</strong><small>{formatRate(speed.download)}</small></div>
        </div>
        <TrafficRows connections={connections.connections.slice(0, 3)} onAddRule={onAddRule} />
      </section>
      <section className="card-surface sync-overview-card">
        <div className="sync-card-copy"><p className="eyebrow">ZERO-CLOUD / LAN</p><h2>Синхронизация</h2><p>Профиль, правила и выбранная нода — напрямую между устройствами.</p></div>
        <button className="secondary-button" type="button" onClick={onOpenSync}><QrCode size={16} />Показать QR</button>
      </section>
    </div>
  );
}

function NodesScreen({
  groups,
  selectedNodeId,
  activeNodeId,
  pingByNode,
  onSelectNode,
  onImport,
  onRefreshSubscription,
  onDeleteSubscription,
  onPing,
}: {
  groups: Array<{ id: string; name: string; subscription?: Subscription; nodes: NodeConfig[] }>;
  selectedNodeId: string | null;
  activeNodeId?: string;
  pingByNode: Record<string, { delay?: number; error?: string }>;
  onSelectNode: (id: string) => void;
  onImport: () => void;
  onRefreshSubscription: (id: string) => void;
  onDeleteSubscription: (subscription: Subscription) => void;
  onPing: () => void;
}) {
  return (
    <section className="screen-section">
      <div className="screen-heading">
        <div><p className="eyebrow">SUBSCRIPTIONS / NODES</p><h1>Серверы</h1><p className="screen-subtitle">Выберите ноду и нажмите Enter или Space для подключения.</p></div>
        <div className="heading-actions"><button className="secondary-button" type="button" onClick={onImport}><FilePlus2 size={16} />Импорт</button><button className="subtle-button" type="button" onClick={onPing}><Activity size={15} />Пинг</button></div>
      </div>
      {groups.length === 0 ? <div className="card-surface compact-empty"><Server size={20} /><span>Подписок пока нет.</span><button className="text-button" type="button" onClick={onImport}>Добавить URL</button></div> : groups.map((group) => (
        <section className="subscription-group" key={group.id}>
          <div className="subscription-heading">
            <div><span className="group-dot" /><strong className="truncate" title={group.name}>{group.name}</strong><span className="count-badge">{group.nodes.length}</span></div>
            {group.subscription && <div className="subscription-actions"><span className="subscription-meta">{group.subscription.traffic_total ? `${formatBytes(group.subscription.traffic_used ?? 0)} / ${formatBytes(group.subscription.traffic_total)}` : "трафик —"}</span><button className="icon-button" type="button" title="Обновить подписку" onClick={() => onRefreshSubscription(group.subscription!.id)}><RefreshCcw size={15} /></button><button className="icon-button danger-icon" type="button" title="Удалить подписку" onClick={() => onDeleteSubscription(group.subscription!)}><Trash2 size={15} /></button></div>}
          </div>
          {group.subscription && <div className="subscription-bars" title={`Трафик: ${Math.round(subscriptionTraffic(group.subscription))}% · Срок: ${Math.round(subscriptionExpiry(group.subscription))}%`}><span style={{ width: `${subscriptionTraffic(group.subscription)}%` }} /><span style={{ width: `${subscriptionExpiry(group.subscription)}%` }} /></div>}
          <div className="node-list">
            {group.nodes.length === 0 ? <div className="node-empty">В подписке нет активных нод.</div> : group.nodes.map((node) => <NodeRow key={node.id} node={node} active={node.id === activeNodeId} selected={node.id === selectedNodeId} ping={pingByNode[node.id]} onSelect={() => onSelectNode(node.id)} />)}
          </div>
        </section>
      ))}
    </section>
  );
}

function NodeRow({
  node,
  active,
  selected,
  ping,
  onSelect,
  compact = false,
}: {
  node: NodeConfig;
  active: boolean;
  selected: boolean;
  ping?: { delay?: number; error?: string };
  onSelect: () => void;
  compact?: boolean;
}) {
  function onKeyDown(event: ReactKeyboardEvent<HTMLDivElement>): void {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect();
    }
  }
  return (
    <div className={`node-row ${active ? "is-active" : ""} ${selected ? "is-selected" : ""} ${compact ? "is-compact" : ""}`} tabIndex={0} onClick={onSelect} onKeyDown={onKeyDown} role="button" aria-label={`${node.name}, ${protocolLabel(node)}`}>
      <div className="node-leading"><span className="node-state-dot" /><div className="node-name-wrap"><strong className="truncate" title={node.name}>{node.name}</strong><CopyValue value={`${node.server}:${node.server_port}`} className="node-address-copy"><span className="node-address mono truncate" title={`${node.server}:${node.server_port}`}>{node.server}:{node.server_port}</span></CopyValue></div></div>
      <span className="technical-badge">{protocolShort(node)}</span>
      <span className={`ping-value ${pingTone(ping?.delay)}`}>{pingLabel(ping?.delay, ping?.error)}</span>
      {active && <span className="active-label">ACTIVE</span>}
      {selected && !active && <span className="selected-label">SELECTED</span>}
    </div>
  );
}

function RulesScreen({
  rules,
  onDelete,
  onCreated,
}: {
  rules: Array<{ id: string; value: string; match_type: RoutingMatch; action: RoutingAction; enabled: boolean; comment: string }>;
  onDelete: (rule: { id: string; value: string }) => void;
  onCreated: () => Promise<void>;
}) {
  const [value, setValue] = useState("");
  const [matchType, setMatchType] = useState<RoutingMatch>("domain");
  const [action, setAction] = useState<RoutingAction>("direct");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function create(): Promise<void> {
    if (!value.trim()) return;
    setSaving(true);
    setError(null);
    try {
      await api.createRule({ enabled: true, match_type: matchType, value: value.trim(), action, comment: "" });
      setValue("");
      await onCreated();
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  }
  return (
    <section className="screen-section">
      <div className="screen-heading"><div><p className="eyebrow">ROUTE / POLICY</p><h1>Правила</h1><p className="screen-subtitle">Точные исключения поверх выбранного сценария.</p></div></div>
      <div className="card-surface rule-composer">
        <div className="field grow"><label htmlFor="rule-value">Значение</label><input id="rule-value" value={value} onChange={(event) => setValue(event.target.value)} placeholder="example.com или 10.0.0.0/8" onKeyDown={(event) => { if (event.key === "Enter") void create(); }} /></div>
        <div className="field"><label>Тип</label><CustomSelect value={matchType} options={RULE_MATCH_OPTIONS} onChange={(next) => setMatchType(next as RoutingMatch)} /></div>
        <div className="field"><label>Действие</label><CustomSelect value={action} options={RULE_ACTION_OPTIONS} onChange={(next) => setAction(next as RoutingAction)} /></div>
        <button className="primary-button" type="button" onClick={() => void create()} disabled={saving || !value.trim()}>{saving ? <LoaderCircle className="spin" size={16} /> : <Plus size={16} />}Добавить</button>
      </div>
      {error && <InlineNotice message={error} tone="error" />}
      <div className="rule-list">
        {rules.length === 0 ? <div className="card-surface compact-empty"><ListFilter size={20} /><span>Правил нет — локальный трафик остаётся под сценарием.</span></div> : rules.map((rule) => <div className="rule-row card-surface" key={rule.id}><span className={`rule-action-dot action-${rule.action}`} /><div className="rule-copy"><strong className="mono truncate" title={rule.value}>{rule.value}</strong><span>{rule.match_type.replaceAll("_", " ")} · {rule.action}</span></div><span className="technical-badge">{rule.enabled ? "ON" : "OFF"}</span><button className="icon-button danger-icon" type="button" title="Удалить правило" onClick={() => onDelete({ id: rule.id, value: rule.value })}><Trash2 size={15} /></button></div>)}
      </div>
    </section>
  );
}

function TrafficScreen({
  connections,
  onAddRule,
}: {
  connections: ClashConnections;
  pausedByDefault: boolean;
  onAddRule: (domain: string, action: RoutingAction) => Promise<void>;
}) {
  const [hoverPaused, setHoverPaused] = useState(false);
  const [manualPaused, setManualPaused] = useState(false);
  const paused = hoverPaused || manualPaused;
  return (
    <section className="screen-section">
      <div className="screen-heading"><div><p className="eyebrow">CLASH API / CONNECTIONS</p><h1>Инспектор трафика</h1><p className="screen-subtitle">Пауза сохраняет последний снимок соединений.</p></div><button className={`subtle-button ${manualPaused ? "is-active" : ""}`} type="button" onClick={() => setManualPaused((value) => !value)}>{manualPaused ? <Play size={15} /> : <Pause size={15} />}{manualPaused ? "Продолжить" : "Пауза"}</button></div>
      <div className="traffic-inspector card-surface" onMouseEnter={() => setHoverPaused(true)} onMouseLeave={() => setHoverPaused(false)}>
        {(hoverPaused || manualPaused) && <span className="pause-badge"><Pause size={12} /> {hoverPaused ? "PAUSED ON HOVER" : "PAUSED"}</span>}
        <div className="traffic-inspector-head"><span>Домен</span><span>Процесс</span><span>Маршрут</span></div>
        {connections.connections.length === 0 ? <div className="compact-empty"><Activity size={18} /><span>Активных соединений нет.</span></div> : <TrafficRows connections={connections.connections} onAddRule={onAddRule} />}
      </div>
    </section>
  );
}

function TrafficRows({ connections, onAddRule }: { connections: ClashConnection[]; onAddRule: (domain: string, action: RoutingAction) => Promise<void> }) {
  return <div className="traffic-rows">{connections.map((connection) => {
    const domain = connectionDomain(connection);
    return <div className="traffic-row" key={connection.id}><span className="traffic-domain mono truncate" title={domain}><Globe2 size={14} />{domain}</span><span className="traffic-process truncate" title={connectionProcess(connection)}>{connectionProcess(connection)}</span><div className="traffic-actions"><button type="button" onClick={() => void onAddRule(domain, "direct")}>→ Direct</button><button type="button" onClick={() => void onAddRule(domain, "proxy")}>→ Proxy</button></div></div>;
  })}</div>;
}

function LogsScreen({ logs }: { logs: LogEntry[] }) {
  return (
    <section className="screen-section">
      <div className="screen-heading"><div><p className="eyebrow">SING-BOX / OUTPUT</p><h1>Логи</h1><p className="screen-subtitle">Последние сообщения процесса и системные события.</p></div></div>
      <div className="log-panel card-surface">
        {logs.length === 0 ? <div className="compact-empty"><Terminal size={18} /><span>Логи появятся после запуска подключения.</span></div> : logs.map((entry, index) => <CopyValue key={`${entry.timestamp_ms}-${index}`} value={entry.line} className="log-row"><span className="log-time mono">{formatDate(entry.timestamp_ms)}</span><span className={`log-stream stream-${entry.stream}`}>{entry.stream}</span><span className="log-line mono truncate" title={entry.line}>{entry.line}</span></CopyValue>)}
      </div>
    </section>
  );
}

function SettingsScreen({
  theme,
  accent,
  tintBackground,
  fontPreset,
  customFont,
  customFontUrl,
  accentOpen,
  onThemeChange,
  onAccentChange,
  onTintChange,
  onFontPresetChange,
  onCustomFontChange,
  onCustomFontUrlChange,
  onAccentOpen,
  onReset,
}: {
  theme: Theme;
  accent: string;
  tintBackground: boolean;
  fontPreset: FontPreset;
  customFont: string;
  customFontUrl: string;
  accentOpen: boolean;
  onThemeChange: (theme: Theme) => void;
  onAccentChange: (accent: string) => void;
  onTintChange: (value: boolean) => void;
  onFontPresetChange: (value: FontPreset) => void;
  onCustomFontChange: (value: string) => void;
  onCustomFontUrlChange: (value: string) => void;
  onAccentOpen: (value: boolean) => void;
  onReset: () => void;
}) {
  return (
    <section className="screen-section settings-screen">
      <div className="screen-heading"><div><p className="eyebrow">BIGHT / SYSTEM</p><h1>Настройки</h1><p className="screen-subtitle">Локальные настройки интерфейса сохраняются на этом устройстве.</p></div><button className="subtle-button" type="button" onClick={onReset}>Сбросить</button></div>
      <div className="settings-grid">
        <section className="card-surface setting-card"><div className="setting-heading"><div><p className="eyebrow">APPEARANCE</p><h2>Тема</h2></div><Sun size={17} /></div><div className="segmented-control"><button type="button" className={theme === "light" ? "is-active" : ""} onClick={() => onThemeChange("light")}><Sun size={15} />Светлая</button><button type="button" className={theme === "dark" ? "is-active" : ""} onClick={() => onThemeChange("dark")}><Moon size={15} />Тёмная</button></div><label className="check-row"><input type="checkbox" checked={tintBackground} onChange={(event) => onTintChange(event.target.checked)} /><span className="fake-check">{tintBackground && <Check size={12} />}</span><span>Тонировать фон</span></label></section>
        <section className="card-surface setting-card"><div className="setting-heading"><div><p className="eyebrow">ACCENT</p><h2>Ключевой цвет</h2></div><span className="accent-preview" style={{ background: accent }} /></div><div className="accent-grid">{ACCENT_PRESETS.map((preset) => <button key={preset.name} className={`accent-swatch ${accent.toLowerCase() === preset.base.toLowerCase() ? "is-active" : ""}`} type="button" onClick={() => onAccentChange(preset.base)} title={preset.name} style={{ "--swatch": preset.base } as CSSProperties}><span style={{ background: preset.base }} /><small>{preset.name}</small></button>)}<AccentPopover open={accentOpen} accent={accent} onClose={() => onAccentOpen(false)} onChange={onAccentChange} onOpen={() => onAccentOpen(true)} /></div></section>
        <section className="card-surface setting-card"><div className="setting-heading"><div><p className="eyebrow">TYPE / SYSTEM</p><h2>Типографика</h2></div><Terminal size={17} /></div><div className="font-options">{FONT_OPTIONS.map((option) => <button key={option.value} type="button" className={`font-option ${fontPreset === option.value ? "is-active" : ""}`} onClick={() => onFontPresetChange(option.value)}><span>{option.label}</span><small>{option.detail}</small></button>)}</div>{fontPreset === "custom" && <div className="custom-font-fields"><div className="field"><label htmlFor="custom-font">Имя шрифта</label><input id="custom-font" value={customFont} onChange={(event) => onCustomFontChange(event.target.value)} placeholder="Fira Code" /></div><div className="field"><label htmlFor="custom-font-url">CSS URL</label><input id="custom-font-url" value={customFontUrl} onChange={(event) => onCustomFontUrlChange(event.target.value)} placeholder="https://…/font.css" /></div></div>}</section>
      </div>
      <div className="settings-footer mono"><Keyboard size={14} /> Ctrl K — действия · Esc — закрыть · Space / Enter — подключить выбранную ноду</div>
    </section>
  );
}

function AccentPopover({ open, accent, onClose, onChange, onOpen }: { open: boolean; accent: string; onClose: () => void; onChange: (value: string) => void; onOpen: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  const [value, setValue] = useState(accent);
  const valid = /^#[0-9a-f]{6}$/i.test(value);
  useEffect(() => setValue(accent), [accent]);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => { if (ref.current && !ref.current.contains(event.target as Node)) onClose(); };
    const onKeyDown = (event: globalThis.KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => { document.removeEventListener("pointerdown", onPointerDown); document.removeEventListener("keydown", onKeyDown); };
  }, [onClose, open]);
  return <div className="accent-custom-wrap" ref={ref}><button type="button" className={`accent-custom-button ${open ? "is-active" : ""}`} onClick={onOpen} title="Свой цвет"><Plus size={18} /></button>{open && <div className="popover accent-popover"><div className="popover-title"><span>Свой цвет</span><button className="icon-button" type="button" onClick={onClose}><X size={14} /></button></div><input className="color-input" type="color" value={valid ? value : "#2F7D57"} onChange={(event) => { setValue(event.target.value.toUpperCase()); onChange(event.target.value.toUpperCase()); }} aria-label="Палитра цвета" /><div className="field"><label htmlFor="custom-hex">HEX</label><input id="custom-hex" className="mono" value={value} onChange={(event) => setValue(event.target.value.toUpperCase())} onBlur={() => { if (valid) onChange(value); }} placeholder="#RRGGBB" maxLength={7} /><small className={valid ? "field-valid" : "field-error"}>{valid ? "Применяется сразу" : "Формат #RRGGBB"}</small></div></div>}</div>;
}

function ImportModal({ open, url, name, onUrlChange, onNameChange, onClose, onSubmit }: { open: boolean; url: string; name: string; onUrlChange: (value: string) => void; onNameChange: (value: string) => void; onClose: () => void; onSubmit: () => void }) {
  return <Modal open={open} onClose={onClose} title="Импорт подписки"><div className="modal-form"><div className="field"><label htmlFor="subscription-url">URL подписки</label><input id="subscription-url" value={url} onChange={(event) => onUrlChange(event.target.value)} placeholder="https://…" autoFocus /></div><div className="field"><label htmlFor="subscription-name">Название <span>необязательно</span></label><input id="subscription-name" value={name} onChange={(event) => onNameChange(event.target.value)} placeholder="Домашний профиль" /></div><div className="modal-actions"><button className="secondary-button" type="button" onClick={onClose}>Отмена</button><button className="primary-button" type="button" disabled={!url.trim()} onClick={onSubmit}><Download size={16} />Импортировать</button></div></div></Modal>;
}

function SyncModal({ open, hostInfo, onHostInfo, onClose }: { open: boolean; hostInfo: SyncHostInfo | null; onHostInfo: (value: SyncHostInfo | null) => void; onClose: () => void }) {
  const [tab, setTab] = useState<"host" | "client">("host");
  const [peers, setPeers] = useState<SyncPeer[]>([]);
  const [pairingUri, setPairingUri] = useState("");
  const [report, setReport] = useState<MergeReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [scannerPermissionDenied, setScannerPermissionDenied] = useState(false);
  const [tick, setTick] = useState(() => Date.now());
  useEffect(() => {
    if (!open) return;
    setError(null);
    const interval = window.setInterval(() => setTick(Date.now()), 1000);
    if (api.isTauri) void discoverPeers();
    if (api.isTauri && tab === "host" && !hostInfo) void createHost();
    return () => window.clearInterval(interval);
  }, [open, tab]);
  async function createHost(): Promise<void> {
    setBusy(true);
    setError(null);
    try { onHostInfo(await api.startSyncHost()); } catch (reason) { setError(errorMessage(reason)); } finally { setBusy(false); }
  }
  async function refreshHost(): Promise<void> {
    setBusy(true);
    setError(null);
    try { await api.stopSyncHost(); onHostInfo(await api.startSyncHost()); } catch (reason) { setError(errorMessage(reason)); } finally { setBusy(false); }
  }
  async function discoverPeers(): Promise<void> {
    try { setPeers(await api.discoverSyncPeers()); } catch (reason) { setError(errorMessage(reason)); }
  }
  async function connectPeer(): Promise<void> {
    if (!pairingUri.trim()) return;
    setBusy(true);
    setError(null);
    setReport(null);
    try { setReport(await api.connectSyncPeer(pairingUri.trim())); } catch (reason) { setError(errorMessage(reason)); } finally { setBusy(false); }
  }
  async function scanAndConnect(): Promise<void> {
    if (!isMobile) return;
    setBusy(true);
    setError(null);
    setReport(null);
    setScannerPermissionDenied(false);
    try {
      const uri = await scanPairingQr();
      setPairingUri(uri);
      setReport(await api.connectSyncPeer(uri));
    } catch (reason) {
      if (reason instanceof BarcodePermissionError) {
        setScannerPermissionDenied(true);
        setError(reason.permission === "denied" ? "Доступ к камере запрещён. Разрешите его в настройках Bight." : "Bight не получил доступ к камере.");
      } else if (!isScanCancelled(reason)) {
        setError(errorMessage(reason));
      }
    } finally {
      setBusy(false);
    }
  }
  async function openScannerSettings(): Promise<void> {
    try {
      await openCameraSettings();
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }
  async function stopHost(): Promise<void> {
    setBusy(true);
    setError(null);
    try {
      await api.stopSyncHost();
      onHostInfo(null);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setBusy(false);
    }
  }
  const remaining = hostInfo ? Math.max(0, hostInfo.pairing.expires_at_ms - tick) : 0;
  const progress = hostInfo ? Math.max(0, Math.min(100, (remaining / (180 * 1000)) * 100)) : 0;
  return <Modal open={open} onClose={onClose} title="LAN Sync" wide><div className="sync-modal"><div className="sync-tabs"><button type="button" className={tab === "host" ? "is-active" : ""} onClick={() => setTab("host")}><QrCode size={15} />Показать QR <span>Host</span></button><button type="button" className={tab === "client" ? "is-active" : ""} onClick={() => setTab("client")}><Link2 size={15} />Подключиться <span>Client</span></button></div>{error && <InlineNotice message={error} tone="error" />}{tab === "host" ? <div className="sync-host-layout"><div className="qr-frame">{hostInfo ? <QRCodeSVG value={hostInfo.qr_payload} size={220} bgColor="#FCFBF8" fgColor="#1B1A18" level="H" includeMargin /> : <div className="qr-placeholder">{busy ? <LoaderCircle className="spin" size={22} /> : <QrCode size={34} />}<span>{busy ? "Создание сессии" : "Нет активной сессии"}</span></div>}</div><div className="sync-details"><p className="eyebrow">PAIRING / 180 SEC</p><h2>Поднесите телефон к QR</h2><p className="sync-muted">Ключ остаётся в QR-пayload и не отправляется в облако.</p>{hostInfo ? <><div className="pin-block"><span>PIN</span><CopyValue value={hostInfo.pin} className="pin-value mono">{formatPin(hostInfo.pin)}</CopyValue></div><div className="ttl-block"><div><span>Сессия истекает через</span><strong className="mono">{formatTtl(remaining)}</strong></div><div className="ttl-track"><span style={{ width: `${progress}%` }} /></div></div><CopyValue value={hostInfo.qr_payload} className="pairing-code mono"><span className="truncate">{hostInfo.qr_payload}</span></CopyValue><div className="sync-actions"><button className="secondary-button" type="button" onClick={() => void refreshHost()} disabled={busy}><RefreshCcw size={15} />Обновить сессию</button><button className="subtle-button" type="button" onClick={() => void stopHost()} disabled={busy}><X size={15} />Остановить</button></div></> : <button className="primary-button" type="button" onClick={() => void createHost()} disabled={busy}><Play size={15} />Запустить Host</button>}</div></div> : <div className="sync-client-layout"><div className="client-entry card-surface"><div className="entry-heading"><div><p className="eyebrow">CLIENT / PAIRING URI</p><h2>Введите код устройства</h2></div>{isMobile && <button className="secondary-button" type="button" onClick={() => void scanAndConnect()} disabled={busy}><ScanLine size={16} />Сканировать QR</button>}</div>{scannerPermissionDenied && <button className="text-button scanner-settings-button" type="button" onClick={() => void openScannerSettings()}>Открыть настройки камеры</button>}<textarea value={pairingUri} onChange={(event) => setPairingUri(event.target.value)} placeholder="Вставьте pairing URI из QR-кода или списка пиров" rows={4} /><button className="primary-button" type="button" onClick={() => void connectPeer()} disabled={busy || !pairingUri.trim()}>{busy ? <LoaderCircle className="spin" size={16} /> : <Link2 size={16} />}Подключить и смержить</button></div><div className="peer-list card-surface"><div className="entry-heading"><div><p className="eyebrow">MDNS / PEERS</p><h2>Устройства рядом</h2></div><button className="icon-button" type="button" title="Обновить список" onClick={() => void discoverPeers()}><RefreshCcw size={15} /></button></div>{peers.length === 0 ? <div className="compact-empty"><Wifi size={18} /><span>Пиры не найдены.</span></div> : peers.map((peer) => <div className="peer-row" key={peer.pairing.session_id}><span className="peer-icon"><Laptop size={16} /></span><div><strong>{peer.instance_name}</strong><span className="mono">{peer.pairing.ips[0]}:{peer.pairing.port}</span></div><button className="text-button" type="button" onClick={() => setPairingUri(JSON.stringify(peer.pairing))}>Выбрать</button></div>)}</div></div>}{report && <div className="merge-report"><span className="eyebrow">MERGE REPORT</span><div><strong>{report.added}</strong><small>добавлено</small></div><div><strong>{report.updated}</strong><small>обновлено</small></div><div><strong>{report.deleted}</strong><small>удалено</small></div><div><strong>{report.conflicts}</strong><small>конфликтов</small></div></div>}</div></Modal>;
}

function CommandPalette({ open, actions, onClose }: { open: boolean; actions: Array<{ id: string; label: string; meta: string; icon: IconComponent; run: () => void }>; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => { if (open) { setQuery(""); window.setTimeout(() => inputRef.current?.focus(), 0); } }, [open]);
  const filtered = actions.filter((action) => `${action.label} ${action.meta}`.toLowerCase().includes(query.toLowerCase()));
  return <Modal open={open} onClose={onClose} title="Командная палитра" compact><div className="command-palette"><div className="command-search"><Search size={16} /><input ref={inputRef} value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Найти действие или ноду…" /></div><div className="command-list">{filtered.length === 0 ? <div className="compact-empty">Ничего не найдено.</div> : filtered.map((action, index) => { const Icon = action.icon; return <button key={action.id} type="button" className="command-row" onClick={() => { action.run(); onClose(); }}><Icon size={16} /><span>{action.label}</span><small>{index === 0 && <kbd>Ctrl K</kbd>} {action.meta}</small></button>; })}</div></div></Modal>;
}

function Modal({ open, onClose, title, children, wide = false, compact = false }: { open: boolean; onClose: () => void; title: string; children: ReactNode; wide?: boolean; compact?: boolean }) {
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: globalThis.KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose, open]);
  if (!open) return null;
  return <div className="modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose(); }}><section className={`modal-sheet ${wide ? "is-wide" : ""} ${compact ? "is-compact" : ""}`} role="dialog" aria-modal="true" aria-label={title}><div className="modal-header"><div><p className="eyebrow">BIGHT / PANEL</p><h2>{title}</h2></div><button className="icon-button" type="button" title="Закрыть" onClick={onClose}><X size={17} /></button></div>{children}</section></div>;
}

function ConfirmModal({ target, onClose, onConfirm }: { target: ConfirmTarget; onClose: () => void; onConfirm: () => void }) {
  if (!target) return null;
  return <Modal open={Boolean(target)} onClose={onClose} title="Подтвердите действие" compact><div className="confirm-copy"><div className="confirm-icon"><AlertTriangle size={20} /></div><p>Удалить <strong>{target.name}</strong>? Это создаст tombstone для следующей синхронизации.</p></div><div className="modal-actions"><button className="secondary-button" type="button" onClick={onClose}>Отмена</button><button className="danger-button" type="button" onClick={onConfirm}><Trash2 size={15} />Удалить</button></div></Modal>;
}

function CustomSelect({ value, options, onChange, compact = false }: { value: string; options: Array<{ value: string; label: string }>; onChange: (value: string) => void; compact?: boolean }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => { if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false); };
    const onKeyDown = (event: globalThis.KeyboardEvent) => { if (event.key === "Escape") setOpen(false); };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => { document.removeEventListener("pointerdown", onPointerDown); document.removeEventListener("keydown", onKeyDown); };
  }, [open]);
  const current = options.find((option) => option.value === value) ?? options[0];
  return <div className={`custom-select ${compact ? "is-compact" : ""}`} ref={ref}><button className="select-trigger" type="button" aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen((state) => !state)}><span>{current?.label}</span><ChevronDown size={14} /></button>{open && <div className="select-menu" role="listbox">{options.map((option) => <button key={option.value} type="button" role="option" aria-selected={option.value === value} className={option.value === value ? "is-selected" : ""} onClick={() => { onChange(option.value); setOpen(false); }}>{option.label}{option.value === value && <Check size={14} />}</button>)}</div>}</div>;
}

function CopyValue({ value, children, className = "" }: { value: string; children: ReactNode; className?: string }) {
  const [copied, setCopied] = useState(false);
  async function copy(event: React.MouseEvent<HTMLButtonElement>): Promise<void> {
    event.stopPropagation();
    try {
      await navigator.clipboard.writeText(value);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (error) {
      console.warn("[Bight] Failed to copy value", error);
    }
  }
  return <button className={`copy-value ${className}`} type="button" onClick={(event) => void copy(event)} title="Скопировать">{children}{copied ? <Check size={14} className="copy-state" /> : <Copy size={14} className="copy-state" />}</button>;
}

function InfrastructureErrorCard({ message, onFallback }: { message: string; onFallback: () => void }) {
  const isWindows = /wintun/i.test(message);
  const command = isWindows ? "Положите wintun.dll рядом с sing-box.exe и перезапустите Bight." : "pkexec setcap cap_net_admin,cap_net_bind_service=+ep /path/to/sing-box";
  return <section className="infra-error-card"><ShieldAlert size={19} /><div><h3>{isWindows ? "Не найден wintun.dll" : "Не хватает CAP_NET_ADMIN"}</h3><p>{isWindows ? "Для TUN на Windows библиотека должна лежать рядом с sidecar-бинарником." : "TUN требует capability на sidecar. Можно один раз применить команду ниже или продолжить через System Proxy."}</p><CopyValue value={command} className="fix-command"><span className="mono">{command}</span></CopyValue><div className="infra-actions"><button className="secondary-button" type="button" onClick={onFallback}><Router size={15} />Переключить на System Proxy</button></div></div></section>;
}

function InlineNotice({ message, onClose, tone = "info" }: { message: string; onClose?: () => void; tone?: "info" | "error" }) {
  return <div className={`inline-notice notice-${tone}`}><ShieldAlert size={15} /><span>{message}</span>{onClose && <button className="icon-button" type="button" onClick={onClose}><X size={14} /></button>}</div>;
}
