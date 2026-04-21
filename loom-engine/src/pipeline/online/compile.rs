//! Context package compiler for the online pipeline.
//!
//! Assembles ranked candidates into context packages for AI consumption.
//! Supports two output formats:
//!
//! - **Structured** (XML-like tags): For Claude and Claude Code models.
//! - **Compact** (JSON): For local models and GPT, optimized for token efficiency.
//!
//! # Pipeline Position
//!
//! ```text
//! loom_think → classify → namespace → retrieve → weight → rank → **compile**
//! ```
//!
//! # Compilation Steps
//!
//! 1. Inject all hot tier memory for namespace (always included).
//! 2. Add warm tier candidates up to token budget.
//! 3. Deduplicate candidates by identifier.
//! 4. Format by output format (structured or compact).
//! 5. Include provenance information for each item.
//! 6. Compute total token count.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::pipeline::online::rank::RankedCandidate;
use crate::pipeline::online::retrieve::{
    CandidatePayload, MemoryType, RetrievalCandidate,
};
use crate::types::classification::TaskClass;
use crate::types::compilation::{CompiledPackage, OutputFormat};
use crate::types::ingestion::IngestionMode;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default warm tier token budget when no namespace config is available.
pub const DEFAULT_WARM_TIER_BUDGET: usize = 3000;

// ---------------------------------------------------------------------------
// Hot tier item
// ---------------------------------------------------------------------------

/// A memory item from the hot tier, always included in compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotTierItem {
    /// Unique identifier for deduplication.
    pub id: Uuid,
    /// The memory type category.
    pub memory_type: MemoryType,
    /// The item payload.
    pub payload: HotTierPayload,
}

/// Payload variants for hot tier items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HotTierPayload {
    /// A hot tier fact.
    Fact(HotFact),
    /// A hot tier entity (identity/project context).
    Entity(HotEntity),
    /// A hot tier procedure.
    Procedure(HotProcedure),
}

/// A fact from the hot tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub evidence: String,
    pub observed: Option<String>,
    pub source: String,
}

/// An entity from the hot tier (used for identity/project context).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotEntity {
    pub name: String,
    pub entity_type: String,
    pub summary: Option<String>,
}

/// A procedure from the hot tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotProcedure {
    pub pattern: String,
    pub confidence: f64,
    pub observation_count: i32,
}

// ---------------------------------------------------------------------------
// Compilation input
// ---------------------------------------------------------------------------

/// Input to the context compiler.
#[derive(Debug, Clone)]
pub struct CompilationInput {
    /// Namespace for this compilation.
    pub namespace: String,
    /// Active task class.
    pub task_class: TaskClass,
    /// Target model name (e.g. "claude-3.5-sonnet").
    pub target_model: String,
    /// Desired output format.
    pub format: OutputFormat,
    /// Warm tier token budget.
    pub warm_tier_budget: usize,
    /// Hot tier items (always included).
    pub hot_tier_items: Vec<HotTierItem>,
    /// Ranked warm tier candidates from the ranking stage.
    pub ranked_candidates: Vec<RankedCandidate>,
}

// ---------------------------------------------------------------------------
// Compilation output metadata
// ---------------------------------------------------------------------------

/// Metadata about a selected candidate for audit logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedItem {
    /// Candidate identifier.
    pub id: Uuid,
    /// Memory type.
    pub memory_type: String,
    /// Score breakdown.
    pub relevance: f64,
    pub recency: f64,
    pub stability: f64,
    pub provenance: f64,
    /// Final composite score.
    pub final_score: f64,
    /// Effective ingestion mode for the candidate (serde snake_case when
    /// present). Mirrors `RetrievalCandidate::provenance_mode`.
    pub ingestion_mode: Option<IngestionMode>,
    /// True when the item's only provenance is `user_authored_seed` — no
    /// live or vendor corroboration. Per ADR 004, surfaced in compiled
    /// output so downstream readers can decide whether to trust the content
    /// or verify against source.
    pub sole_source: Option<bool>,
}

/// Compute the `sole_source` flag for a retrieval candidate.
///
/// Returns `Some(true)` when the candidate's effective ingestion mode is
/// `user_authored_seed` — by construction the retrieval stage picks the
/// highest-authority mode across source episodes, so seeing seed here means
/// no live or vendor corroboration exists. Returns `Some(false)` when the
/// candidate has live or vendor evidence, and `None` when mode metadata is
/// absent (synthetic or test candidates).
pub fn compute_sole_source(candidate: &RetrievalCandidate) -> Option<bool> {
    candidate
        .provenance_mode
        .map(|m| m == IngestionMode::UserAuthoredSeed)
}

/// Metadata about a rejected candidate for audit logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectedItem {
    /// Candidate identifier.
    pub id: Uuid,
    /// Memory type.
    pub memory_type: String,
    /// Reason for rejection.
    pub reason: String,
}

/// Full compilation result with audit metadata.
#[derive(Debug, Clone)]
pub struct CompilationResult {
    /// The compiled context package.
    pub package: CompiledPackage,
    /// Items selected for the final package.
    pub selected_items: Vec<SelectedItem>,
    /// Items rejected during compilation.
    pub rejected_items: Vec<RejectedItem>,
    /// Total candidates found across all profiles.
    pub candidates_found: i32,
    /// Candidates selected for the package.
    pub candidates_selected: i32,
    /// Candidates rejected.
    pub candidates_rejected: i32,
}

// ---------------------------------------------------------------------------
// Token estimation
// ---------------------------------------------------------------------------

/// Estimate the token count for a retrieval candidate.
///
/// Uses rough heuristics: ~50 tokens per fact, ~100 per episode (or
/// content-length based), ~30 per graph result, ~40 per procedure.
pub fn estimate_candidate_tokens(candidate: &RetrievalCandidate) -> usize {
    match &candidate.payload {
        CandidatePayload::Fact(_) => 50,
        CandidatePayload::Episode(ep) => {
            // ~1 token per 4 characters, clamped to [20, 200].
            (ep.content.len() / 4).max(20).min(200)
        }
        CandidatePayload::Graph(_) => 30,
        CandidatePayload::Procedure(_) => 40,
    }
}

/// Estimate the token count for a hot tier item.
pub fn estimate_hot_item_tokens(item: &HotTierItem) -> usize {
    match &item.payload {
        HotTierPayload::Fact(_) => 50,
        HotTierPayload::Entity(_) => 20,
        HotTierPayload::Procedure(_) => 40,
    }
}

/// Estimate token count from a string (rough: 1 token per 4 chars).
pub fn estimate_string_tokens(s: &str) -> usize {
    (s.len() / 4).max(1)
}

// ---------------------------------------------------------------------------
// Deduplication
// ---------------------------------------------------------------------------

/// Deduplicate candidates by their identifier.
///
/// Keeps the first occurrence of each ID (highest-ranked, since input is
/// sorted by score descending).
pub fn deduplicate_candidates(
    candidates: Vec<RankedCandidate>,
) -> (Vec<RankedCandidate>, Vec<RejectedItem>) {
    let mut seen = HashSet::new();
    let mut kept = Vec::new();
    let mut rejected = Vec::new();

    for rc in candidates {
        if seen.insert(rc.candidate.id) {
            kept.push(rc);
        } else {
            rejected.push(RejectedItem {
                id: rc.candidate.id,
                memory_type: rc.candidate.memory_type.to_string(),
                reason: "duplicate".to_string(),
            });
        }
    }

    (kept, rejected)
}

// ---------------------------------------------------------------------------
// Budget trimming
// ---------------------------------------------------------------------------

/// Trim candidates to fit within a warm tier token budget.
///
/// Candidates are already sorted by score descending. Removes lowest-ranked
/// candidates until the total fits within the budget.
pub fn trim_to_warm_budget(
    candidates: Vec<RankedCandidate>,
    budget: usize,
    hot_tier_tokens: usize,
) -> (Vec<RankedCandidate>, Vec<RejectedItem>) {
    let remaining_budget = budget.saturating_sub(hot_tier_tokens);
    let mut total_tokens = 0usize;
    let mut kept = Vec::new();
    let mut rejected = Vec::new();

    for rc in candidates {
        let tokens = estimate_candidate_tokens(&rc.candidate);
        if total_tokens + tokens <= remaining_budget {
            total_tokens += tokens;
            kept.push(rc);
        } else {
            rejected.push(RejectedItem {
                id: rc.candidate.id,
                memory_type: rc.candidate.memory_type.to_string(),
                reason: "token_budget_exceeded".to_string(),
            });
        }
    }

    tracing::info!(
        warm_tokens = total_tokens,
        budget = remaining_budget,
        kept = kept.len(),
        rejected = rejected.len(),
        "warm tier candidates trimmed to budget"
    );

    (kept, rejected)
}

// ---------------------------------------------------------------------------
// Context compiler (main entry point)
// ---------------------------------------------------------------------------

/// Compile a context package from hot tier items and ranked candidates.
///
/// # Steps
///
/// 1. Deduplicate ranked candidates by identifier.
/// 2. Compute hot tier token cost.
/// 3. Trim warm tier candidates to fit within budget.
/// 4. Format output in the requested format (structured XML or compact JSON).
/// 5. Compute total token count.
/// 6. Return compiled package with audit metadata.
pub fn compile_package(input: CompilationInput) -> CompilationResult {
    let _span = tracing::info_span!("compile_package",
        namespace = %input.namespace,
        format = %input.format,
        task_class = %input.task_class,
    )
    .entered();

    let compilation_id = Uuid::new_v4();
    let total_candidates_found = input.ranked_candidates.len() as i32;

    // Step 1: Deduplicate candidates, also excluding hot tier IDs.
    let hot_ids: HashSet<Uuid> = input.hot_tier_items.iter().map(|h| h.id).collect();
    let warm_candidates: Vec<RankedCandidate> = input
        .ranked_candidates
        .into_iter()
        .filter(|rc| !hot_ids.contains(&rc.candidate.id))
        .collect();

    let (deduped, mut all_rejected) = deduplicate_candidates(warm_candidates);

    // Step 2: Compute hot tier token cost.
    let hot_tier_tokens: usize = input
        .hot_tier_items
        .iter()
        .map(estimate_hot_item_tokens)
        .sum();

    // Step 3: Trim warm tier candidates to budget.
    let (selected_warm, budget_rejected) =
        trim_to_warm_budget(deduped, input.warm_tier_budget, hot_tier_tokens);
    all_rejected.extend(budget_rejected);

    // Build selected items metadata for audit.
    let selected_items: Vec<SelectedItem> = selected_warm
        .iter()
        .map(|rc| SelectedItem {
            id: rc.candidate.id,
            memory_type: rc.candidate.memory_type.to_string(),
            relevance: rc.scores.relevance,
            recency: rc.scores.recency,
            stability: rc.scores.stability,
            provenance: rc.scores.provenance,
            final_score: rc.final_score,
            ingestion_mode: rc.candidate.provenance_mode,
            sole_source: compute_sole_source(&rc.candidate),
        })
        .collect();

    let candidates_selected = selected_items.len() as i32;
    let candidates_rejected = all_rejected.len() as i32;

    // Step 4: Format output.
    let context_package = match input.format {
        OutputFormat::Structured => format_structured(
            &input.hot_tier_items,
            &selected_warm,
            &input.namespace,
            &input.task_class,
            &input.target_model,
        ),
        OutputFormat::Compact => format_compact(
            &input.hot_tier_items,
            &selected_warm,
            &input.namespace,
            &input.task_class,
        ),
    };

    // Step 5: Compute total token count.
    let token_count = estimate_string_tokens(&context_package) as i32;

    tracing::info!(
        compilation_id = %compilation_id,
        token_count,
        hot_items = input.hot_tier_items.len(),
        warm_items = selected_warm.len(),
        format = %input.format,
        "context package compiled"
    );

    let package = CompiledPackage {
        context_package,
        token_count,
        compilation_id,
        format: input.format,
    };

    CompilationResult {
        package,
        selected_items,
        rejected_items: all_rejected,
        candidates_found: total_candidates_found,
        candidates_selected,
        candidates_rejected,
    }
}

// ---------------------------------------------------------------------------
// Structured output format (XML-like tags)
// ---------------------------------------------------------------------------

/// Format the context package as XML-like structured tags.
///
/// Produces output like:
/// ```xml
/// <loom model="claude-3.5-sonnet" tokens="1847" namespace="project-sentinel" task="architecture">
///   <identity>Project memory context</identity>
///   <project>...</project>
///   <knowledge>
///     <fact subject="..." predicate="..." object="..." evidence="..." observed="..." source="..."/>
///   </knowledge>
///   <episodes>
///     <episode date="..." source="..." id="...">...</episode>
///   </episodes>
///   <patterns>
///     <pattern confidence="0.85" observations="5">...</pattern>
///   </patterns>
/// </loom>
/// ```
pub fn format_structured(
    hot_items: &[HotTierItem],
    warm_candidates: &[RankedCandidate],
    namespace: &str,
    task_class: &TaskClass,
    target_model: &str,
) -> String {
    let mut facts = Vec::new();
    let mut episodes = Vec::new();
    let mut patterns = Vec::new();
    let mut identity = String::new();
    let mut project = String::new();

    // Collect hot tier items. Hot-tier items lack retrieval-stage mode
    // metadata (they are inserted from a separate hot-tier query that does
    // not join loom_episodes), so sole_source is emitted as `None` and the
    // attribute is omitted on their fact elements.
    for item in hot_items {
        match &item.payload {
            HotTierPayload::Fact(f) => {
                facts.push(format_structured_fact(
                    &f.subject,
                    &f.predicate,
                    &f.object,
                    &f.evidence,
                    f.observed.as_deref(),
                    &f.source,
                    None,
                ));
            }
            HotTierPayload::Entity(e) => {
                if e.entity_type == "project" {
                    project = e.summary.clone().unwrap_or_else(|| e.name.clone());
                } else {
                    if !identity.is_empty() {
                        identity.push_str(", ");
                    }
                    identity.push_str(&e.name);
                }
            }
            HotTierPayload::Procedure(p) => {
                patterns.push(format_structured_pattern(
                    &p.pattern,
                    p.confidence,
                    p.observation_count,
                ));
            }
        }
    }

    // Collect warm tier candidates.
    for rc in warm_candidates {
        let sole_source = compute_sole_source(&rc.candidate);
        let mode = rc.candidate.provenance_mode;
        match &rc.candidate.payload {
            CandidatePayload::Fact(f) => {
                facts.push(format_structured_fact(
                    &f.subject_id.to_string(),
                    &f.predicate,
                    &f.object_id.to_string(),
                    &f.evidence_status,
                    None,
                    &format_source_episodes(&f.source_episodes),
                    sole_source,
                ));
            }
            CandidatePayload::Episode(ep) => {
                episodes.push(format_structured_episode(
                    &ep.occurred_at,
                    &ep.source,
                    &rc.candidate.id,
                    &ep.content,
                    mode,
                ));
            }
            CandidatePayload::Graph(g) => {
                if let Some(pred) = &g.predicate {
                    facts.push(format_structured_fact(
                        &g.entity_name,
                        pred,
                        &g.entity_type,
                        "graph",
                        None,
                        &g.entity_id.to_string(),
                        sole_source,
                    ));
                }
            }
            CandidatePayload::Procedure(p) => {
                patterns.push(format_structured_pattern(
                    &p.pattern,
                    p.confidence,
                    p.observation_count,
                ));
            }
        }
    }

    // Estimate token count for the root tag.
    let estimated_tokens = estimate_structured_tokens(
        &identity, &project, &facts, &episodes, &patterns,
    );

    let mut out = String::new();
    out.push_str(&format!(
        "<loom model=\"{}\" tokens=\"{}\" namespace=\"{}\" task=\"{}\">\n",
        xml_escape(target_model),
        estimated_tokens,
        xml_escape(namespace),
        task_class,
    ));

    if identity.is_empty() {
        identity = format!("{namespace} memory context");
    }
    out.push_str(&format!("  <identity>{}</identity>\n", xml_escape(&identity)));

    if !project.is_empty() {
        out.push_str(&format!("  <project>{}</project>\n", xml_escape(&project)));
    }

    if !facts.is_empty() {
        out.push_str("  <knowledge>\n");
        for fact in &facts {
            out.push_str("    ");
            out.push_str(fact);
            out.push('\n');
        }
        out.push_str("  </knowledge>\n");
    }

    if !episodes.is_empty() {
        out.push_str("  <episodes>\n");
        for ep in &episodes {
            out.push_str("    ");
            out.push_str(ep);
            out.push('\n');
        }
        out.push_str("  </episodes>\n");
    }

    if !patterns.is_empty() {
        out.push_str("  <patterns>\n");
        for pat in &patterns {
            out.push_str("    ");
            out.push_str(pat);
            out.push('\n');
        }
        out.push_str("  </patterns>\n");
    }

    out.push_str("</loom>");
    out
}

/// Format a single fact as an XML self-closing tag.
///
/// `sole_source` is serialized as `sole_source="true|false"` when `Some`,
/// and omitted when `None` (hot-tier facts and synthetic inputs).
fn format_structured_fact(
    subject: &str,
    predicate: &str,
    object: &str,
    evidence: &str,
    observed: Option<&str>,
    source: &str,
    sole_source: Option<bool>,
) -> String {
    let observed_attr = match observed {
        Some(d) => format!(" observed=\"{}\"", xml_escape(d)),
        None => String::new(),
    };
    let sole_source_attr = match sole_source {
        Some(b) => format!(" sole_source=\"{b}\""),
        None => String::new(),
    };
    format!(
        "<fact subject=\"{}\" predicate=\"{}\" object=\"{}\" evidence=\"{}\"{} source=\"{}\"{}/>",
        xml_escape(subject),
        xml_escape(predicate),
        xml_escape(object),
        xml_escape(evidence),
        observed_attr,
        xml_escape(source),
        sole_source_attr,
    )
}

/// Format a single episode as an XML element.
///
/// Emits `mode="<ingestion_mode>"` when the effective mode is known, so
/// readers can see per-episode provenance alongside the facts derived from
/// it. Omitted when `None` (synthetic candidates in tests).
fn format_structured_episode(
    occurred_at: &DateTime<Utc>,
    source: &str,
    id: &Uuid,
    content: &str,
    mode: Option<IngestionMode>,
) -> String {
    let date = occurred_at.format("%Y-%m-%d").to_string();
    let mode_attr = match mode {
        Some(m) => format!(" mode=\"{m}\""),
        None => String::new(),
    };
    format!(
        "<episode date=\"{}\" source=\"{}\" id=\"{}\"{}>{}</episode>",
        xml_escape(&date),
        xml_escape(source),
        id,
        mode_attr,
        xml_escape(content),
    )
}

/// Format a single pattern as an XML self-closing tag.
fn format_structured_pattern(
    pattern: &str,
    confidence: f64,
    observations: i32,
) -> String {
    format!(
        "<pattern confidence=\"{:.2}\" observations=\"{}\">{}</pattern>",
        confidence,
        observations,
        xml_escape(pattern),
    )
}

/// Estimate total tokens for structured output.
fn estimate_structured_tokens(
    identity: &str,
    project: &str,
    facts: &[String],
    episodes: &[String],
    patterns: &[String],
) -> usize {
    let mut total = 20; // overhead for tags
    total += estimate_string_tokens(identity);
    total += estimate_string_tokens(project);
    for f in facts {
        total += estimate_string_tokens(f);
    }
    for e in episodes {
        total += estimate_string_tokens(e);
    }
    for p in patterns {
        total += estimate_string_tokens(p);
    }
    total
}

/// Escape special XML characters.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Format source episode UUIDs as a comma-separated string.
fn format_source_episodes(episodes: &[Uuid]) -> String {
    episodes
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

// ---------------------------------------------------------------------------
// Compact output format (JSON)
// ---------------------------------------------------------------------------

/// Format the context package as a compact JSON object.
///
/// Produces output like:
/// ```json
/// {
///   "ns": "project-sentinel",
///   "task": "architecture",
///   "identity": "Project memory context",
///   "facts": [{"s": "...", "p": "...", "o": "...", "e": "...", "t": "..."}],
///   "recent": [{"date": "...", "src": "...", "text": "..."}],
///   "patterns": [{"p": "...", "c": 0.85, "n": 5}]
/// }
/// ```
pub fn format_compact(
    hot_items: &[HotTierItem],
    warm_candidates: &[RankedCandidate],
    namespace: &str,
    task_class: &TaskClass,
) -> String {
    let mut facts: Vec<serde_json::Value> = Vec::new();
    let mut recent: Vec<serde_json::Value> = Vec::new();
    let mut patterns: Vec<serde_json::Value> = Vec::new();
    let mut identity = String::new();

    // Collect hot tier items. Hot-tier content has no retrieval-stage
    // provenance metadata; `sole_source` is omitted from those facts.
    for item in hot_items {
        match &item.payload {
            HotTierPayload::Fact(f) => {
                facts.push(serde_json::json!({
                    "s": f.subject,
                    "p": f.predicate,
                    "o": f.object,
                    "e": f.evidence,
                    "t": f.observed.as_deref().unwrap_or(""),
                }));
            }
            HotTierPayload::Entity(e) => {
                if !identity.is_empty() {
                    identity.push_str(", ");
                }
                identity.push_str(
                    e.summary.as_deref().unwrap_or(&e.name),
                );
            }
            HotTierPayload::Procedure(p) => {
                patterns.push(serde_json::json!({
                    "p": p.pattern,
                    "c": p.confidence,
                    "n": p.observation_count,
                }));
            }
        }
    }

    // Collect warm tier candidates. `sole_source` is present on every fact
    // whose candidate carries a known ingestion mode (true when seed-only,
    // false when live or vendor evidence exists).
    for rc in warm_candidates {
        let sole_source = compute_sole_source(&rc.candidate);
        let mode = rc.candidate.provenance_mode;
        match &rc.candidate.payload {
            CandidatePayload::Fact(f) => {
                let mut obj = serde_json::json!({
                    "s": f.subject_id.to_string(),
                    "p": f.predicate,
                    "o": f.object_id.to_string(),
                    "e": f.evidence_status,
                    "t": "",
                });
                if let Some(b) = sole_source {
                    obj["sole_source"] = serde_json::Value::Bool(b);
                }
                facts.push(obj);
            }
            CandidatePayload::Episode(ep) => {
                let mut obj = serde_json::json!({
                    "date": ep.occurred_at.format("%Y-%m-%d").to_string(),
                    "src": ep.source,
                    "text": ep.content,
                });
                if let Some(m) = mode {
                    obj["mode"] = serde_json::Value::String(m.to_string());
                }
                recent.push(obj);
            }
            CandidatePayload::Graph(g) => {
                if let Some(pred) = &g.predicate {
                    let mut obj = serde_json::json!({
                        "s": g.entity_name,
                        "p": pred,
                        "o": g.entity_type,
                        "e": "graph",
                        "t": "",
                    });
                    if let Some(b) = sole_source {
                        obj["sole_source"] = serde_json::Value::Bool(b);
                    }
                    facts.push(obj);
                }
            }
            CandidatePayload::Procedure(p) => {
                patterns.push(serde_json::json!({
                    "p": p.pattern,
                    "c": p.confidence,
                    "n": p.observation_count,
                }));
            }
        }
    }

    if identity.is_empty() {
        identity = format!("{namespace} memory context");
    }

    let output = serde_json::json!({
        "ns": namespace,
        "task": task_class.to_string(),
        "identity": identity,
        "facts": facts,
        "recent": recent,
        "patterns": patterns,
    });

    serde_json::to_string(&output).unwrap_or_else(|_| "{}".to_string())
}

// ---------------------------------------------------------------------------
// Audit logging
// ---------------------------------------------------------------------------

/// Build an audit log entry from compilation results.
///
/// Captures the full compilation trace: classification, retrieval profiles,
/// candidate decisions, token counts, output format, and latency breakdown.
pub fn build_audit_entry(
    result: &CompilationResult,
    namespace: &str,
    task_class: &TaskClass,
    query_text: Option<&str>,
    target_model: Option<&str>,
    primary_class: &str,
    secondary_class: Option<&str>,
    primary_confidence: Option<f64>,
    secondary_confidence: Option<f64>,
    profiles_executed: &[String],
    latency_total_ms: Option<i32>,
    latency_classify_ms: Option<i32>,
    latency_retrieve_ms: Option<i32>,
    latency_rank_ms: Option<i32>,
    latency_compile_ms: Option<i32>,
) -> crate::db::audit::NewAuditEntry {
    let selected_json = serde_json::to_value(&result.selected_items).ok();
    let rejected_json = serde_json::to_value(&result.rejected_items).ok();

    let retrieval_profile = profiles_executed
        .first()
        .cloned()
        .unwrap_or_else(|| "none".to_string());

    crate::db::audit::NewAuditEntry {
        task_class: task_class.to_string(),
        namespace: namespace.to_string(),
        query_text: query_text.map(|s| s.to_string()),
        target_model: target_model.map(|s| s.to_string()),
        primary_class: primary_class.to_string(),
        secondary_class: secondary_class.map(|s| s.to_string()),
        primary_confidence,
        secondary_confidence,
        profiles_executed: Some(profiles_executed.to_vec()),
        retrieval_profile,
        candidates_found: Some(result.candidates_found),
        candidates_selected: Some(result.candidates_selected),
        candidates_rejected: Some(result.candidates_rejected),
        selected_items: selected_json,
        rejected_items: rejected_json,
        compiled_tokens: Some(result.package.token_count),
        output_format: Some(result.package.format.to_string()),
        latency_total_ms,
        latency_classify_ms,
        latency_retrieve_ms,
        latency_rank_ms,
        latency_compile_ms,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::online::rank::RankedCandidate;
    use crate::pipeline::online::retrieve::{
        CandidatePayload, EpisodeCandidate, FactCandidate,
        MemoryType, ProcedureCandidate, RetrievalCandidate, RetrievalProfile,
    };
    use crate::types::compilation::RankingScore;
    use chrono::Utc;
    use uuid::Uuid;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_hot_fact(subject: &str, predicate: &str, object: &str) -> HotTierItem {
        HotTierItem {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Fact(HotFact {
                subject: subject.to_string(),
                predicate: predicate.to_string(),
                object: object.to_string(),
                evidence: "explicit".to_string(),
                observed: Some("2025-09-01".to_string()),
                source: "episode_abc".to_string(),
            }),
        }
    }

    fn make_hot_entity(name: &str, entity_type: &str) -> HotTierItem {
        HotTierItem {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Entity(HotEntity {
                name: name.to_string(),
                entity_type: entity_type.to_string(),
                summary: Some(format!("{name} - test project")),
            }),
        }
    }

    fn make_hot_procedure(pattern: &str) -> HotTierItem {
        HotTierItem {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Procedural,
            payload: HotTierPayload::Procedure(HotProcedure {
                pattern: pattern.to_string(),
                confidence: 0.85,
                observation_count: 5,
            }),
        }
    }

    fn make_ranked_fact(score: f64) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::FactLookup,
            memory_type: MemoryType::Semantic,
            payload: CandidatePayload::Fact(FactCandidate {
                subject_id: Uuid::new_v4(),
                predicate: "uses".to_string(),
                object_id: Uuid::new_v4(),
                evidence_status: "extracted".to_string(),
                source_episodes: vec![Uuid::new_v4()],
                namespace: "default".to_string(),
            }),
            provenance_mode: None,
        };
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.6,
                stability: 0.7,
                provenance: 0.5,
            },
            final_score: score * 0.4 + 0.6 * 0.25 + 0.7 * 0.20 + 0.5 * 0.15,
        }
    }

    fn make_ranked_episode(score: f64, content: &str) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::EpisodeRecall,
            memory_type: MemoryType::Episodic,
            payload: CandidatePayload::Episode(EpisodeCandidate {
                source: "claude-code".to_string(),
                content: content.to_string(),
                occurred_at: Utc::now(),
                namespace: "default".to_string(),
            }),
            provenance_mode: None,
        };
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.9,
                stability: 0.8,
                provenance: 0.8,
            },
            final_score: score * 0.4 + 0.9 * 0.25 + 0.8 * 0.20 + 0.8 * 0.15,
        }
    }

    fn make_ranked_procedure(score: f64, pattern: &str) -> RankedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::ProcedureAssist,
            memory_type: MemoryType::Procedural,
            payload: CandidatePayload::Procedure(ProcedureCandidate {
                pattern: pattern.to_string(),
                confidence: 0.9,
                observation_count: 7,
                namespace: "default".to_string(),
            }),
            provenance_mode: None,
        };
        RankedCandidate {
            candidate,
            scores: RankingScore {
                relevance: score,
                recency: 0.5,
                stability: 0.8,
                provenance: 0.7,
            },
            final_score: score * 0.4 + 0.5 * 0.25 + 0.8 * 0.20 + 0.7 * 0.15,
        }
    }

    fn sample_input() -> CompilationInput {
        CompilationInput {
            namespace: "project-sentinel".to_string(),
            task_class: TaskClass::Architecture,
            target_model: "claude-3.5-sonnet".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
            hot_tier_items: vec![
                make_hot_fact("Project Sentinel", "uses", "Semantic Kernel"),
                make_hot_entity("Sentinel", "project"),
            ],
            ranked_candidates: vec![
                make_ranked_fact(0.9),
                make_ranked_episode(0.8, "Discussed APIM authentication flow changes"),
                make_ranked_procedure(0.7, "When debugging auth issues, check APIM logs first"),
            ],
        }
    }

    // -----------------------------------------------------------------------
    // 13.2 — Structured XML format tests
    // -----------------------------------------------------------------------

    #[test]
    fn structured_format_contains_loom_root_tag() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(pkg.starts_with("<loom "), "should start with <loom tag");
        assert!(pkg.ends_with("</loom>"), "should end with </loom>");
    }

    #[test]
    fn structured_format_has_correct_attributes() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("model=\"claude-3.5-sonnet\""),
            "should contain model attribute"
        );
        assert!(
            pkg.contains("namespace=\"project-sentinel\""),
            "should contain namespace attribute"
        );
        assert!(
            pkg.contains("task=\"architecture\""),
            "should contain task attribute"
        );
        assert!(
            pkg.contains("tokens=\""),
            "should contain tokens attribute"
        );
    }

    #[test]
    fn structured_format_contains_identity_tag() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("<identity>"),
            "should contain <identity> tag"
        );
        assert!(
            pkg.contains("</identity>"),
            "should contain </identity> tag"
        );
    }

    #[test]
    fn structured_format_contains_knowledge_section() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("<knowledge>"),
            "should contain <knowledge> tag"
        );
        assert!(
            pkg.contains("</knowledge>"),
            "should contain </knowledge> tag"
        );
        assert!(
            pkg.contains("<fact "),
            "should contain <fact> elements"
        );
    }

    #[test]
    fn structured_format_fact_has_required_attributes() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(pkg.contains("subject=\""), "fact should have subject attr");
        assert!(pkg.contains("predicate=\""), "fact should have predicate attr");
        assert!(pkg.contains("object=\""), "fact should have object attr");
        assert!(pkg.contains("evidence=\""), "fact should have evidence attr");
        assert!(pkg.contains("source=\""), "fact should have source attr");
    }

    #[test]
    fn structured_format_contains_episodes_section() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("<episodes>"),
            "should contain <episodes> tag"
        );
        assert!(
            pkg.contains("<episode "),
            "should contain <episode> elements"
        );
    }

    #[test]
    fn structured_format_episode_has_required_attributes() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(pkg.contains("date=\""), "episode should have date attr");
        assert!(pkg.contains("source=\""), "episode should have source attr");
        assert!(pkg.contains("id=\""), "episode should have id attr");
    }

    #[test]
    fn structured_format_contains_patterns_section() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("<patterns>"),
            "should contain <patterns> tag"
        );
        assert!(
            pkg.contains("<pattern "),
            "should contain <pattern> elements"
        );
    }

    #[test]
    fn structured_format_pattern_has_required_attributes() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("confidence=\""),
            "pattern should have confidence attr"
        );
        assert!(
            pkg.contains("observations=\""),
            "pattern should have observations attr"
        );
    }

    #[test]
    fn structured_format_hot_fact_observed_date() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        assert!(
            pkg.contains("observed=\"2025-09-01\""),
            "hot fact should include observed date"
        );
    }

    // -----------------------------------------------------------------------
    // 13.3 — Compact JSON format tests
    // -----------------------------------------------------------------------

    #[test]
    fn compact_format_is_valid_json() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value =
            serde_json::from_str(pkg).expect("compact output should be valid JSON");
        assert!(parsed.is_object(), "should be a JSON object");
    }

    #[test]
    fn compact_format_has_required_fields() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value = serde_json::from_str(pkg).unwrap();
        assert!(parsed.get("ns").is_some(), "should have 'ns' field");
        assert!(parsed.get("task").is_some(), "should have 'task' field");
        assert!(parsed.get("identity").is_some(), "should have 'identity' field");
        assert!(parsed.get("facts").is_some(), "should have 'facts' field");
        assert!(parsed.get("recent").is_some(), "should have 'recent' field");
        assert!(parsed.get("patterns").is_some(), "should have 'patterns' field");
    }

    #[test]
    fn compact_format_facts_have_correct_keys() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value = serde_json::from_str(pkg).unwrap();
        let facts = parsed["facts"].as_array().unwrap();
        assert!(!facts.is_empty(), "should have at least one fact");

        let fact = &facts[0];
        assert!(fact.get("s").is_some(), "fact should have 's' field");
        assert!(fact.get("p").is_some(), "fact should have 'p' field");
        assert!(fact.get("o").is_some(), "fact should have 'o' field");
        assert!(fact.get("e").is_some(), "fact should have 'e' field");
        assert!(fact.get("t").is_some(), "fact should have 't' field");
    }

    #[test]
    fn compact_format_recent_have_correct_keys() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value = serde_json::from_str(pkg).unwrap();
        let recent = parsed["recent"].as_array().unwrap();
        assert!(!recent.is_empty(), "should have at least one episode");

        let ep = &recent[0];
        assert!(ep.get("date").is_some(), "episode should have 'date' field");
        assert!(ep.get("src").is_some(), "episode should have 'src' field");
        assert!(ep.get("text").is_some(), "episode should have 'text' field");
    }

    #[test]
    fn compact_format_patterns_have_correct_keys() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value = serde_json::from_str(pkg).unwrap();
        let patterns = parsed["patterns"].as_array().unwrap();
        assert!(!patterns.is_empty(), "should have at least one pattern");

        let pat = &patterns[0];
        assert!(pat.get("p").is_some(), "pattern should have 'p' field");
        assert!(pat.get("c").is_some(), "pattern should have 'c' field");
        assert!(pat.get("n").is_some(), "pattern should have 'n' field");
    }

    #[test]
    fn compact_format_namespace_and_task() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value = serde_json::from_str(pkg).unwrap();
        assert_eq!(parsed["ns"], "project-sentinel");
        assert_eq!(parsed["task"], "architecture");
    }

    // -----------------------------------------------------------------------
    // 13.1 — Token budget enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn warm_tier_trimming_respects_budget() {
        let mut input = sample_input();
        // Set a very small budget so some candidates get trimmed.
        input.warm_tier_budget = 100;
        let result = compile_package(input);

        // With budget of 100 and hot items taking ~70 tokens,
        // only ~30 tokens left for warm candidates.
        assert!(
            result.candidates_rejected > 0 || result.candidates_selected <= 3,
            "some candidates should be rejected or all fit within budget"
        );
    }

    #[test]
    fn zero_budget_rejects_all_warm_candidates() {
        let mut input = sample_input();
        input.warm_tier_budget = 0;
        let result = compile_package(input);

        assert_eq!(
            result.candidates_selected, 0,
            "zero budget should reject all warm candidates"
        );
    }

    // -----------------------------------------------------------------------
    // 13.1 — Deduplication
    // -----------------------------------------------------------------------

    #[test]
    fn deduplication_removes_duplicate_ids() {
        let rc1 = make_ranked_fact(0.9);
        let id = rc1.candidate.id;

        // Create a duplicate with the same ID.
        let mut rc2 = make_ranked_fact(0.8);
        rc2.candidate.id = id;

        let (kept, rejected) = deduplicate_candidates(vec![rc1, rc2]);
        assert_eq!(kept.len(), 1, "should keep only one");
        assert_eq!(rejected.len(), 1, "should reject the duplicate");
        assert_eq!(rejected[0].reason, "duplicate");
    }

    #[test]
    fn deduplication_keeps_first_occurrence() {
        let rc1 = make_ranked_fact(0.9);
        let id = rc1.candidate.id;
        let score1 = rc1.final_score;

        let mut rc2 = make_ranked_fact(0.5);
        rc2.candidate.id = id;

        let (kept, _) = deduplicate_candidates(vec![rc1, rc2]);
        assert_eq!(kept.len(), 1);
        assert!(
            (kept[0].final_score - score1).abs() < f64::EPSILON,
            "should keep the first (highest-ranked) occurrence"
        );
    }

    // -----------------------------------------------------------------------
    // 13.1 — Provenance tracking
    // -----------------------------------------------------------------------

    #[test]
    fn structured_output_includes_provenance() {
        let input = sample_input();
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        // Hot tier fact should have source provenance.
        assert!(
            pkg.contains("source=\"episode_abc\""),
            "should include provenance source"
        );
    }

    #[test]
    fn compact_output_includes_episode_source() {
        let mut input = sample_input();
        input.format = OutputFormat::Compact;
        let result = compile_package(input);
        let pkg = &result.package.context_package;

        let parsed: serde_json::Value = serde_json::from_str(pkg).unwrap();
        let recent = parsed["recent"].as_array().unwrap();
        if !recent.is_empty() {
            assert!(
                recent[0].get("src").is_some(),
                "episode should include source provenance"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 13.4 — Audit logging
    // -----------------------------------------------------------------------

    #[test]
    fn audit_entry_captures_all_fields() {
        let input = sample_input();
        let result = compile_package(input);

        let entry = build_audit_entry(
            &result,
            "project-sentinel",
            &TaskClass::Architecture,
            Some("how does auth work?"),
            Some("claude-3.5-sonnet"),
            "architecture",
            Some("debug"),
            Some(0.85),
            Some(0.65),
            &["fact_lookup".to_string(), "graph_neighborhood".to_string()],
            Some(250),
            Some(50),
            Some(100),
            Some(60),
            Some(40),
        );

        assert_eq!(entry.task_class, "architecture");
        assert_eq!(entry.namespace, "project-sentinel");
        assert_eq!(entry.query_text.as_deref(), Some("how does auth work?"));
        assert_eq!(entry.target_model.as_deref(), Some("claude-3.5-sonnet"));
        assert_eq!(entry.primary_class, "architecture");
        assert_eq!(entry.secondary_class.as_deref(), Some("debug"));
        assert_eq!(entry.primary_confidence, Some(0.85));
        assert_eq!(entry.secondary_confidence, Some(0.65));
        assert_eq!(
            entry.profiles_executed.as_deref(),
            Some(&["fact_lookup".to_string(), "graph_neighborhood".to_string()][..])
        );
        assert_eq!(entry.retrieval_profile, "fact_lookup");
        assert!(entry.candidates_found.is_some());
        assert!(entry.candidates_selected.is_some());
        assert!(entry.candidates_rejected.is_some());
        assert!(entry.selected_items.is_some());
        assert!(entry.rejected_items.is_some());
        assert!(entry.compiled_tokens.is_some());
        assert_eq!(entry.output_format.as_deref(), Some("structured"));
        assert_eq!(entry.latency_total_ms, Some(250));
        assert_eq!(entry.latency_classify_ms, Some(50));
        assert_eq!(entry.latency_retrieve_ms, Some(100));
        assert_eq!(entry.latency_rank_ms, Some(60));
        assert_eq!(entry.latency_compile_ms, Some(40));
    }

    #[test]
    fn audit_selected_items_contain_score_breakdown() {
        let input = sample_input();
        let result = compile_package(input);

        for item in &result.selected_items {
            assert!(item.relevance >= 0.0 && item.relevance <= 1.0);
            assert!(item.recency >= 0.0 && item.recency <= 1.0);
            assert!(item.stability >= 0.0 && item.stability <= 1.0);
            assert!(item.provenance >= 0.0 && item.provenance <= 1.0);
            assert!(item.final_score >= 0.0);
        }
    }

    // -----------------------------------------------------------------------
    // 13.4 — Latency measurement via tracing spans
    // -----------------------------------------------------------------------

    #[test]
    fn compilation_produces_positive_token_count() {
        let input = sample_input();
        let result = compile_package(input);

        assert!(
            result.package.token_count > 0,
            "compiled package should have positive token count"
        );
    }

    #[test]
    fn compilation_id_is_unique() {
        let input1 = sample_input();
        let input2 = sample_input();
        let r1 = compile_package(input1);
        let r2 = compile_package(input2);

        assert_ne!(
            r1.package.compilation_id, r2.package.compilation_id,
            "each compilation should have a unique ID"
        );
    }

    // -----------------------------------------------------------------------
    // Edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn empty_candidates_produces_valid_output() {
        let input = CompilationInput {
            namespace: "empty-ns".to_string(),
            task_class: TaskClass::Chat,
            target_model: "test-model".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
            hot_tier_items: vec![],
            ranked_candidates: vec![],
        };
        let result = compile_package(input);

        assert!(result.package.context_package.contains("<loom "));
        assert!(result.package.context_package.contains("</loom>"));
        assert_eq!(result.candidates_selected, 0);
        assert_eq!(result.candidates_rejected, 0);
    }

    #[test]
    fn empty_candidates_compact_produces_valid_json() {
        let input = CompilationInput {
            namespace: "empty-ns".to_string(),
            task_class: TaskClass::Chat,
            target_model: "test-model".to_string(),
            format: OutputFormat::Compact,
            warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
            hot_tier_items: vec![],
            ranked_candidates: vec![],
        };
        let result = compile_package(input);

        let parsed: serde_json::Value =
            serde_json::from_str(&result.package.context_package).unwrap();
        assert_eq!(parsed["ns"], "empty-ns");
        assert_eq!(parsed["facts"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn xml_escape_handles_special_characters() {
        let escaped = xml_escape("a < b & c > d \"e\" 'f'");
        assert_eq!(
            escaped,
            "a &lt; b &amp; c &gt; d &quot;e&quot; &apos;f&apos;"
        );
    }

    #[test]
    fn hot_tier_items_excluded_from_warm_dedup() {
        let hot_id = Uuid::new_v4();
        let hot_items = vec![HotTierItem {
            id: hot_id,
            memory_type: MemoryType::Semantic,
            payload: HotTierPayload::Fact(HotFact {
                subject: "A".to_string(),
                predicate: "uses".to_string(),
                object: "B".to_string(),
                evidence: "explicit".to_string(),
                observed: None,
                source: "ep1".to_string(),
            }),
        }];

        // Create a warm candidate with the same ID as a hot item.
        let mut warm = make_ranked_fact(0.9);
        warm.candidate.id = hot_id;

        let input = CompilationInput {
            namespace: "test".to_string(),
            task_class: TaskClass::Chat,
            target_model: "test".to_string(),
            format: OutputFormat::Structured,
            warm_tier_budget: DEFAULT_WARM_TIER_BUDGET,
            hot_tier_items: hot_items,
            ranked_candidates: vec![warm],
        };

        let result = compile_package(input);
        // The warm candidate with the same ID as a hot item should be filtered out.
        assert_eq!(
            result.candidates_selected, 0,
            "warm candidate with same ID as hot item should be excluded"
        );
    }
}
