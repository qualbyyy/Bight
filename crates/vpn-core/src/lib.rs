mod clash_api;
mod models;
mod parser;
pub mod singbox;

pub use clash_api::{ClashApiClient, ClashApiError, ClashConnection, ClashConnections};
pub use models::{
    AppProfile, HysteriaObfs, NodeConfig, NodeProtocol, ProfileMode, RealitySettings,
    RoutingAction, RoutingMatch, RoutingRule, Subscription, TlsSettings, TransportConfig,
};
pub use parser::{
    ParseError, SubscriptionDecode, SubscriptionUsage, decode_subscription, parse_node_uri,
    parse_subscription_userinfo,
};
pub use singbox::{
    BuildOptions, ConfigError, TargetPlatform, build_config, build_config_json, validate_config,
};
