use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: &str = "0.1.0";
pub const TOMBSTONE_SCHEMA: &str = "https://git-cortex.dev/schemas/v0.1/tombstone.json";
pub const HANDOFF_SCHEMA: &str = "https://git-cortex.dev/schemas/v0.1/handoff.json";
pub const INVARIANT_SCHEMA: &str = "https://git-cortex.dev/schemas/v0.1/invariant.json";
pub const EVENT_SCHEMA: &str = "https://git-cortex.dev/schemas/v0.1/event.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSymbol {
    pub symbol_uri: String,
    pub ast_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub command: String,
    pub exit_code: i32,
    pub error_signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationPolicy {
    pub invalidate_on_any_ast_change: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CaptureMode {
    AutoRevertDetected,
    #[default]
    Manual,
    RewindSynthesized,
    HookEnforced,
}

/// A Tombstone captures a disproved hypothesis or failed modification
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TombstoneRecord {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub id: String,
    pub schema_version: String,
    pub created_at: DateTime<Utc>,
    pub author_harness: String,
    pub branch_context: String,
    pub base_commit_sha: String,
    pub capture_mode: CaptureMode,
    pub target_symbols: Vec<TargetSymbol>,
    pub attempted_approach: String,
    pub failure_reason: String,
    pub verification_evidence: VerificationEvidence,
    pub invalidation_policy: InvalidationPolicy,

    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl TombstoneRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        author_harness: String,
        branch_context: String,
        base_commit_sha: String,
        capture_mode: CaptureMode,
        target_symbols: Vec<TargetSymbol>,
        attempted_approach: String,
        failure_reason: String,
        evidence: VerificationEvidence,
        invalidate_symbols: Vec<String>,
    ) -> Self {
        Self {
            schema: TOMBSTONE_SCHEMA.to_string(),
            id,
            schema_version: SCHEMA_VERSION.to_string(),
            created_at: Utc::now(),
            author_harness,
            branch_context,
            base_commit_sha,
            capture_mode,
            target_symbols,
            attempted_approach,
            failure_reason,
            verification_evidence: evidence,
            invalidation_policy: InvalidationPolicy {
                invalidate_on_any_ast_change: invalidate_symbols,
                status: Some("ACTIVE".to_string()),
            },
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallGraphEdge {
    pub caller: String,
    pub callee: String,
    pub relationship: String,
    pub verified_ast_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActiveHypothesis {
    pub hypothesis_id: String,
    pub statement: String,
    pub confidence: f64,
    pub status: String,
    pub next_verification_step: String,
}

/// EpistemicHandoff compresses working memory into a <= 4KB payload
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpistemicHandoff {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub handoff_id: String,
    pub schema_version: String,
    pub head_commit_sha: String,
    pub working_tree_dirty_hash: String,
    pub objective: String,
    pub verified_call_graph: Vec<CallGraphEdge>,
    pub active_hypotheses: Vec<ActiveHypothesis>,
    pub active_tombstone_ids: Vec<String>,
    pub do_not_re_read_unchanged: Vec<String>,

    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl EpistemicHandoff {
    pub fn new(
        handoff_id: String,
        head_commit_sha: String,
        working_tree_dirty_hash: String,
        objective: String,
    ) -> Self {
        Self {
            schema: HANDOFF_SCHEMA.to_string(),
            handoff_id,
            schema_version: SCHEMA_VERSION.to_string(),
            head_commit_sha,
            working_tree_dirty_hash,
            objective,
            verified_call_graph: Vec::new(),
            active_hypotheses: Vec::new(),
            active_tombstone_ids: Vec::new(),
            do_not_re_read_unchanged: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InvariantSeverity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpstreamDependency {
    pub symbol_uri: String,
    pub expected_ast_hash: String,
}

/// Causal Invariant Lock attached to architectural constraints
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvariantContract {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub invariant_id: String,
    pub schema_version: String,
    pub commit_sha: String,
    pub guarded_symbol: TargetSymbol,
    pub severity: InvariantSeverity,
    pub assertion: String,
    pub upstream_dependencies: Vec<UpstreamDependency>,
    pub verification_test_cmd: String,

    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl InvariantContract {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        invariant_id: String,
        commit_sha: String,
        guarded_symbol: TargetSymbol,
        severity: InvariantSeverity,
        assertion: String,
        upstream_dependencies: Vec<UpstreamDependency>,
        verification_test_cmd: String,
    ) -> Self {
        Self {
            schema: INVARIANT_SCHEMA.to_string(),
            invariant_id,
            schema_version: SCHEMA_VERSION.to_string(),
            commit_sha,
            guarded_symbol,
            severity,
            assertion,
            upstream_dependencies,
            verification_test_cmd,
            extensions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EventType {
    Revoke,
    Reaffirm,
    Supersede,
}

/// Append-only lifecycle event (reaffirming or revoking an invariant or tombstone)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleEvent {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub event_id: String,
    pub target_id: String,
    pub event_type: EventType,
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub author: String,

    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_json::Value>,
}

impl LifecycleEvent {
    pub fn new(
        event_id: String,
        target_id: String,
        event_type: EventType,
        reason: String,
        author: String,
    ) -> Self {
        Self {
            schema: EVENT_SCHEMA.to_string(),
            event_id,
            target_id,
            event_type,
            reason,
            created_at: Utc::now(),
            author,
            extensions: BTreeMap::new(),
        }
    }
}

/// Query-time evaluated status of a Tombstone or Invariant
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DerivedStatus {
    Active,
    Stale { changed_symbols: Vec<String> },
    Orphaned { missing_symbols: Vec<String> },
    Revoked { reason: String },
}

impl DerivedStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Stale { .. } => "STALE",
            Self::Orphaned { .. } => "ORPHANED",
            Self::Revoked { .. } => "REVOKED",
        }
    }
}
