use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use vpn_core::{AppProfile, NodeConfig, RoutingRule, Subscription};

#[derive(Debug, Error)]
pub enum MergeError {
    #[error("could not serialize an LWW record: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeReport {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub conflicts: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPayload {
    pub profile: AppProfile,
}

impl SyncPayload {
    pub fn from_profile(profile: &AppProfile) -> Self {
        Self {
            profile: profile.clone(),
        }
    }
}

pub fn merge_payloads(
    local: &mut SyncPayload,
    remote: &SyncPayload,
) -> Result<MergeReport, MergeError> {
    merge_profiles(&mut local.profile, &remote.profile)
}

pub fn merge_profiles(
    local: &mut AppProfile,
    remote: &AppProfile,
) -> Result<MergeReport, MergeError> {
    let mut report = MergeReport::default();
    merge_subscriptions(&mut local.subscriptions, &remote.subscriptions, &mut report)?;
    merge_records(&mut local.custom_nodes, &remote.custom_nodes, &mut report)?;
    merge_records(&mut local.routing_rules, &remote.routing_rules, &mut report)?;

    if profile_remote_wins(local, remote)? {
        local.id = remote.id;
        local.name = remote.name.clone();
        local.active_node_id = remote.active_node_id;
        local.mode = remote.mode;
        local.updated_at_ms = remote.updated_at_ms;
        local.is_deleted = remote.is_deleted;
        if remote.is_deleted {
            report.deleted += 1;
        } else {
            report.updated += 1;
        }
    }

    local.subscriptions.sort_by_key(|item| item.id);
    local.custom_nodes.sort_by_key(|item| item.id);
    local.routing_rules.sort_by_key(|item| item.id);
    Ok(report)
}

trait LwwRecord: Clone + Serialize {
    fn id(&self) -> Uuid;
    fn updated_at_ms(&self) -> i64;
    fn is_deleted(&self) -> bool;
}

impl LwwRecord for NodeConfig {
    fn id(&self) -> Uuid {
        self.id
    }

    fn updated_at_ms(&self) -> i64 {
        self.updated_at_ms
    }

    fn is_deleted(&self) -> bool {
        self.is_deleted
    }
}

impl LwwRecord for RoutingRule {
    fn id(&self) -> Uuid {
        self.id
    }

    fn updated_at_ms(&self) -> i64 {
        self.updated_at_ms
    }

    fn is_deleted(&self) -> bool {
        self.is_deleted
    }
}

impl LwwRecord for Subscription {
    fn id(&self) -> Uuid {
        self.id
    }

    fn updated_at_ms(&self) -> i64 {
        self.updated_at_ms
    }

    fn is_deleted(&self) -> bool {
        self.is_deleted
    }
}

fn merge_subscriptions(
    local: &mut Vec<Subscription>,
    remote: &[Subscription],
    report: &mut MergeReport,
) -> Result<(), MergeError> {
    for remote_item in remote {
        let Some(index) = local.iter().position(|item| item.id == remote_item.id) else {
            local.push(remote_item.clone());
            count_add(remote_item, report);
            continue;
        };

        let local_item = local[index].clone();
        let remote_wins = record_wins(remote_item, &local_item)?;
        let mut merged = if remote_wins {
            count_update(remote_item, &local_item, report);
            remote_item.clone()
        } else {
            if canonical_bytes(remote_item)? != canonical_bytes(&local_item)? {
                report.conflicts += 1;
            }
            local_item.clone()
        };
        let other_nodes = if remote_wins {
            &local_item.nodes
        } else {
            &remote_item.nodes
        };
        merge_records(&mut merged.nodes, other_nodes, report)?;
        merged.nodes.sort_by_key(|item| item.id);
        local[index] = merged;
    }
    Ok(())
}

fn merge_records<T: LwwRecord>(
    local: &mut Vec<T>,
    remote: &[T],
    report: &mut MergeReport,
) -> Result<(), MergeError> {
    for remote_item in remote {
        let Some(index) = local.iter().position(|item| item.id() == remote_item.id()) else {
            local.push(remote_item.clone());
            count_add(remote_item, report);
            continue;
        };

        let local_item = &local[index];
        if record_wins(remote_item, local_item)? {
            count_update(remote_item, local_item, report);
            local[index] = remote_item.clone();
        } else if canonical_bytes(remote_item)? != canonical_bytes(local_item)? {
            report.conflicts += 1;
        }
    }
    local.sort_by_key(LwwRecord::id);
    Ok(())
}

fn record_wins<T: LwwRecord>(remote: &T, local: &T) -> Result<bool, MergeError> {
    if remote.updated_at_ms() != local.updated_at_ms() {
        return Ok(remote.updated_at_ms() > local.updated_at_ms());
    }
    if remote.is_deleted() != local.is_deleted() {
        return Ok(remote.is_deleted());
    }
    Ok(canonical_bytes(remote)? > canonical_bytes(local)?)
}

fn profile_remote_wins(local: &AppProfile, remote: &AppProfile) -> Result<bool, MergeError> {
    if remote.updated_at_ms != local.updated_at_ms {
        return Ok(remote.updated_at_ms > local.updated_at_ms);
    }
    if remote.is_deleted != local.is_deleted {
        return Ok(remote.is_deleted);
    }
    let remote_key =
        serde_json::to_vec(&(remote.id, &remote.name, remote.active_node_id, remote.mode))?;
    let local_key = serde_json::to_vec(&(local.id, &local.name, local.active_node_id, local.mode))?;
    Ok(remote_key > local_key)
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, MergeError> {
    Ok(serde_json::to_vec(value)?)
}

fn count_add<T: LwwRecord>(item: &T, report: &mut MergeReport) {
    report.added += 1;
    if item.is_deleted() {
        report.deleted += 1;
    }
}

fn count_update<T: LwwRecord>(remote: &T, local: &T, report: &mut MergeReport) {
    report.updated += 1;
    if remote.is_deleted() && !local.is_deleted() {
        report.deleted += 1;
    }
}
