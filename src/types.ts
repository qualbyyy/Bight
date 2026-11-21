export type ProfileMode = "tun" | "system_proxy";

export type ConnectionStatus =
  | { state: "disconnected" }
  | { state: "connecting" }
  | { state: "connected"; node_id: string; clash_port: number; mode: ProfileMode }
  | { state: "error"; message: string };

export type TransportConfig =
  | { type: "tcp" }
  | { type: "web_socket"; path: string; host?: string }
  | { type: "grpc"; service_name: string }
  | { type: "http_upgrade"; path: string; host?: string };

export type RealitySettings = {
  public_key: string;
  short_id: string;
  spider_x?: string;
};

export type TlsSettings = {
  enabled: boolean;
  server_name?: string;
  insecure?: boolean;
  fingerprint?: string;
  reality?: RealitySettings;
};

export type NodeProtocol =
  | {
      protocol: "vless";
      uuid: string;
      flow?: string;
      tls: TlsSettings;
      transport: TransportConfig;
    }
  | {
      protocol: "hysteria2";
      password: string;
      tls: TlsSettings;
      obfs?: { kind: string; password: string };
      up_mbps?: number;
      down_mbps?: number;
    }
  | {
      protocol: "trojan";
      password: string;
      tls: TlsSettings;
      transport: TransportConfig;
    }
  | {
      protocol: "shadowsocks";
      method: string;
      password: string;
      plugin?: string;
      plugin_opts?: string;
    };

export type NodeConfig = {
  id: string;
  name: string;
  server: string;
  server_port: number;
  updated_at_ms: number;
  is_deleted: boolean;
} & NodeProtocol;

export type Subscription = {
  id: string;
  name: string;
  url: string;
  nodes: NodeConfig[];
  traffic_used?: number;
  traffic_total?: number;
  expire_at?: number;
  last_fetched_at?: number;
  updated_at_ms: number;
  is_deleted: boolean;
};

export type RoutingMatch =
  | "domain"
  | "domain_suffix"
  | "domain_keyword"
  | "ip_cidr"
  | "rule_set";

export type RoutingAction = "proxy" | "direct" | "reject";

export type RoutingRule = {
  id: string;
  enabled: boolean;
  match_type: RoutingMatch;
  value: string;
  action: RoutingAction;
  comment: string;
  updated_at_ms: number;
  is_deleted: boolean;
};

export type AppProfile = {
  id: string;
  name: string;
  subscriptions: Subscription[];
  custom_nodes: NodeConfig[];
  routing_rules: RoutingRule[];
  active_node_id?: string;
  mode: ProfileMode;
  updated_at_ms: number;
  is_deleted: boolean;
};

export type PingResult = {
  node_id: string;
  node_name: string;
  delay_ms?: number;
  error?: string;
};

export type LogEntry = {
  timestamp_ms: number;
  stream: "stdout" | "stderr" | "system";
  line: string;
};

export type ClashConnection = {
  id: string;
  metadata: Record<string, unknown>;
};

export type ClashConnections = {
  downloadTotal: number;
  uploadTotal: number;
  connections: ClashConnection[];
};

export type PairingPayload = {
  version: number;
  ips: string[];
  port: number;
  public_key: string;
  session_id: string;
  pin: string;
  expires_at_ms: number;
};

export type SyncHostInfo = {
  pairing: PairingPayload;
  qr_payload: string;
  pin: string;
};

export type SyncPeer = {
  instance_name: string;
  pairing: PairingPayload;
};

export type MergeReport = {
  added: number;
  updated: number;
  deleted: number;
  conflicts: number;
};

export type AppTab = "overview" | "nodes" | "rules" | "traffic" | "logs" | "settings";
