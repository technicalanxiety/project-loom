//! Four-dimension ranking and trimming for the online pipeline.
//!
//! Scores each weighted candidate on four interpretable dimensions and
//! computes a composite score for final ordering:
//!
//! | Dimension     | Weight | Basis                                                |
//! |---------------|--------|------------------------------------------------------|
//! | **Relevance** | 0.40   | Weight-modified similarity score from retrieval       |
//! | **Recency**   | 0.25   | Time-based decay from `occurred_at` / `valid_from`   |
//! | **Stability** | 0.20   | Non-superseded status, evidence authority, salience   |
//! | **Provenance**| 0.15   | Source episode count, evidence status authority       |
//!
//! Each dimension is scored in `[0.0, 1.0]`. The final composite score is:
//!
//! ```text
//! final = relevance × 0.40 + recency × 0.25 + stability × 0.20 + provenance × 0.15
//! ```
//!
//! Candidates are sorted by final score descending.
//!
//! # Pipeline Position
//!
//! ```text
//! loom_think → classify → namespace → retrieve → weight → **rank** → compile
//! ```

use chrono::Utc;

use crate::pipeline::online::retrieve::{
    CandidatePayload, MemoryType, RetrievalCandidate,
};
use crate::pipeline::online::weight::WeightedCandidate;
use crate::types::compilation::RankingScore;

// ---------------------------------------------------------------------------
// Dimension weights (constants)
// ---------------------------------------------------------------------------

/// Weight for the relevance dimension.
pub const RELEVANCE_WEIGHT: f64 = 0.40;
/// Weight for the recency dimension.
pub const RECENCY_WEIGHT: f64 = 0.25;
/// Weight for the stability dimension.
pub const STABILITY_WEIGHT: f64 = 0.20;
/// Weight for the provenance dimension.
pub const PROVENANCE_WEIGHT: f64 = 0.15;

/// Recency half-life in days for ranking decay.
const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;

// ---------------------------------------------------------------------------
// Ranked candidate
// ---------------------------------------------------------------------------

/// A candidate with its four-dimension score breakdown and composite score.
#[derive(Debug, Clone)]
pub struct RankedCandidate {
    /// The original retrieval candidate.
    pub candidate: RetrievalCandidate,
    /// Per-dimension score breakdown.
    pub scores: RankingScore,
    /// Composite final score.
    pub final_score: f64,
}

// ---------------------------------------------------------------------------
// Evidence status authority ordering
// ---------------------------------------------------------------------------

/// Return an authority score in `[0.0, 1.0]` for an evidence status string.
///
/// Higher values indicate more authoritative evidence:
/// - `user_asserted` → 1.0
/// - `observed` → 0.8
/// - `extracted` → 0.6
/// - `inferred` → 0.4
/// - `promoted` → 0.7
/// - `deprecated` → 0.2
/// - `superseded` → 0.1
///
/// Unknown statuses default to 0.5.
pub fn evidence_authority(status: &str) -> f64 {
    match status {
        "user_asserted" => 1.0,
        "observed" => 0.8,
        "promoted" => 0.7,
        "extracted" => 0.6,
        "inferred" => 0.4,
        "deprecated" => 0.2,
        "superseded" => 0.1,
        _ => 0.5,
    }
}

// ---------------------------------------------------------------------------
// Dimension scorers
// ---------------------------------------------------------------------------

/// Score the **relevance** dimension for a candidate.
///
/// Uses the weight-modified score from the weighting stage. The score is
/// already in `[0.0, 1.0]` after weight application.
pub fn score_relevance(weighted: &WeightedCandidate) -> f64 {
    weighted.weighted_score.clamp(0.0, 1.0)
}

/// Score the **recency** dimension for a candidate.
///
/// Uses exponential decay based on the candidate's timestamp:
/// - Episodes: `occurred_at`
/// - Facts: `valid_from` (approximated by creation time)
/// - Graph: hop-based (closer hops are more "recent" in traversal terms)
/// - Procedures: `last_observed` (approximated)
///
/// Recent items score close to 1.0; items older than the half-life score
/// below 0.5.
pub fn score_recency(candidate: &RetrievalCandidate) -> f64 {
    let now = Utc::now();

    match &candidate.payload {
        CandidatePayload::Episode(ep) => {
            let age_days = (now - ep.occurred_at).num_hours().max(0) as f64 / 24.0;
            compute_recency_decay(age_days)
        }
        CandidatePayload::Fact(_) => {
            // Facts don't carry a timestamp in the candidate payload.
            // Use a moderate default recency (assume relatively recent).
            0.6
        }
        CandidatePayload::Graph(g) => {
            // Graph candidates: closer hops are more relevant.
            match g.hop_depth {
                0 | 1 => 0.8,
                2 => 0.6,
                _ => 0.4,
            }
        }
        CandidatePayload::Procedure(_) => {
            // Procedures are long-lived patterns; moderate recency.
            0.5
        }
    }
}

/// Compute exponential recency decay.
///
/// Returns a value in `[0.0, 1.0]` where recent items (age ≈ 0) score
/// close to 1.0 and items older than the half-life score below 0.5.
fn compute_recency_decay(age_days: f64) -> f64 {
    let decay = (-age_days * (2.0_f64.ln()) / RECENCY_HALF_LIFE_DAYS).exp();
    decay.clamp(0.0, 1.0)
}

/// Score the **stability** dimension for a candidate.
///
/// Considers:
/// - Whether the item is current (non-superseded): base 0.5 if current
/// - Evidence status authority: contributes up to 0.3
/// - Salience score: contributes up to 0.2 (default 0.5 if unknown)
pub fn score_stability(candidate: &RetrievalCandidate) -> f64 {
    match &candidate.payload {
        CandidatePayload::Fact(f) => {
            // Non-superseded facts get a base stability.
            let is_current = f.evidence_status != "superseded"
                && f.evidence_status != "deprecated";
            let base = if is_current { 0.5 } else { 0.1 };
            let authority = evidence_authority(&f.evidence_status) * 0.3;
            // Default salience for facts (actual salience would come from
            // fact_state, but we use a reasonable default here).
            let salience = 0.5 * 0.2;
            (base + authority + salience).clamp(0.0, 1.0)
        }
        CandidatePayload::Episode(_) => {
            // Episodes are immutable evidence — high stability.
            0.8
        }
        CandidatePayload::Graph(g) => {
            // Graph results: stability based on hop depth (closer = more stable).
            match g.hop_depth {
                0 | 1 => 0.7,
                2 => 0.5,
                _ => 0.3,
            }
        }
        CandidatePayload::Procedure(p) => {
            // Procedures: stability based on confidence and observation count.
            let confidence_component = p.confidence * 0.5;
            let observation_component =
                (p.observation_count as f64 / 10.0).min(1.0) * 0.5;
            (confidence_component + observation_component).clamp(0.0, 1.0)
        }
    }
}

/// Score the **provenance** dimension for a candidate.
///
/// Considers:
/// - Source episode count: more sources = stronger provenance
/// - Evidence status authority: user_asserted > observed > extracted > inferred
pub fn score_provenance(candidate: &RetrievalCandidate) -> f64 {
    match &candidate.payload {
        CandidatePayload::Fact(f) => {
            // Source episode count: normalize to [0, 1] (cap at 5 episodes).
            let episode_score =
                (f.source_episodes.len() as f64 / 5.0).min(1.0) * 0.5;
            let authority = evidence_authority(&f.evidence_status) * 0.5;
            (episode_score + authority).clamp(0.0, 1.0)
        }
        CandidatePayload::Episode(_) => {
            // Episodes are primary evidence — high provenance.
            0.8
        }
        CandidatePayload::Graph(_) => {
            // Graph results: moderate provenance (derived from facts).
            0.5
        }
        CandidatePayload::Procedure(p) => {
            // Procedures: provenance based on observation count.
            let obs_score =
                (p.observation_count as f64 / 10.0).min(1.0) * 0.6;
            let confidence_score = p.confidence * 0.4;
            (obs_score + confidence_score).clamp(0.0, 1.0)
        }
    }
}

// ---------------------------------------------------------------------------
// Composite ranking
// ---------------------------------------------------------------------------

/// Compute the composite final score from four dimension scores.
///
/// ```text
/// final = relevance × 0.40 + recency × 0.25 + stability × 0.20 + provenance × 0.15
/// ```
pub fn compute_final_score(scores: &RankingScore) -> f64 {
    scores.relevance * RELEVANCE_WEIGHT
        + scores.recency * RECENCY_WEIGHT
        + scores.stability * STABILITY_WEIGHT
        + scores.provenance * PROVENANCE_WEIGHT
}

/// Rank a list of weighted candidates using the four-dimension scoring model.
///
/// Computes relevance, recency, stability, and provenance scores for each
/// candidate, combines them into a composite score, and sorts descending.
///
/// Returns a list of [`RankedCandidate`] sorted by `final_score` descending.
pub fn rank_candidates(weighted: Vec<WeightedCandidate>) -> Vec<RankedCandidate> {
    let mut ranked: Vec<RankedCandidate> = weighted
        .into_iter()
        .map(|wc| {
            let relevance = score_relevance(&wc);
            let recency = score_recency(&wc.candidate);
            let stability = score_stability(&wc.candidate);
            let provenance = score_provenance(&wc.candidate);

            let scores = RankingScore {
                relevance,
                recency,
                stability,
                provenance,
            };

            let final_score = compute_final_score(&scores);

            RankedCandidate {
                candidate: wc.candidate,
                scores,
                final_score,
            }
        })
        .collect();

    // Sort by final score descending.
    ranked.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    tracing::info!(
        ranked_count = ranked.len(),
        top_score = ranked.first().map(|r| r.final_score).unwrap_or(0.0),
        "candidates ranked by four-dimension scoring"
    );

    ranked
}

/// Trim ranked candidates to fit within a token budget.
///
/// Estimates token count per candidate and removes lowest-ranked candidates
/// until the total fits within the budget. Returns the trimmed list.
pub fn trim_to_budget(
    ranked: Vec<RankedCandidate>,
    token_budget: usize,
) -> Vec<RankedCandidate> {
    // Simple token estimation: ~50 tokens per fact, ~100 per episode,
    // ~30 per graph result, ~40 per procedure.
    let mut total_tokens = 0usize;
    let mut trimmed = Vec::new();

    for candidate in ranked {
        let estimated_tokens = estimate_tokens(&candidate.candidate);
        if total_tokens + estimated_tokens <= token_budget {
            total_tokens += estimated_tokens;
            trimmed.push(candidate);
        } else {
            tracing::debug!(
                candidate_id = %candidate.candidate.id,
                final_score = candidate.final_score,
                estimated_tokens,
                total_tokens,
                token_budget,
                "candidate trimmed to fit token budget"
            );
        }
    }

    tracing::info!(
        total_tokens,
        token_budget,
        candidates_kept = trimmed.len(),
        "ranked candidates trimmed to token budget"
    );

    trimmed
}

/// Estimate the token count for a candidate based on its memory type.
fn estimate_tokens(candidate: &RetrievalCandidate) -> usize {
    match &candidate.payload {
        CandidatePayload::Fact(_) => 50,
        CandidatePayload::Episode(ep) => {
            // Rough estimate: 1 token per 4 characters.
            (ep.content.len() / 4).max(20).min(200)
        }
        CandidatePayload::Graph(_) => 30,
        CandidatePayload::Procedure(_) => 40,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::online::retrieve::{
        CandidatePayload, EpisodeCandidate, FactCandidate,
        ProcedureCandidate, RetrievalCandidate, RetrievalProfile,
    };
    use crate::pipeline::online::weight::WeightedCandidate;
    use crate::types::compilation::RankingScore;
    use chrono::Utc;
    use uuid::Uuid;

    /// Helper to create a weighted candidate with given parameters.
    fn make_weighted_fact(
        score: f64,
        weight: f64,
        evidence_status: &str,
        source_episode_count: usize,
    ) -> WeightedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::FactLookup,
            memory_type: MemoryType::Semantic,
            payload: CandidatePayload::Fact(FactCandidate {
                subject_id: Uuid::new_v4(),
                predicate: "uses".to_string(),
                object_id: Uuid::new_v4(),
                evidence_status: evidence_status.to_string(),
                source_episodes: (0..source_episode_count)
                    .map(|_| Uuid::new_v4())
                    .collect(),
                namespace: "default".to_string(),
            }),
        };
        WeightedCandidate {
            candidate,
            weight,
            weighted_score: score * weight,
        }
    }

    fn make_weighted_episode(
        score: f64,
        weight: f64,
        occurred_at: chrono::DateTime<Utc>,
    ) -> WeightedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::EpisodeRecall,
            memory_type: MemoryType::Episodic,
            payload: CandidatePayload::Episode(EpisodeCandidate {
                source: "test".to_string(),
                content: "test content for episode".to_string(),
                occurred_at,
                namespace: "default".to_string(),
            }),
        };
        WeightedCandidate {
            candidate,
            weight,
            weighted_score: score * weight,
        }
    }

    fn make_weighted_procedure(
        score: f64,
        weight: f64,
        confidence: f64,
        observation_count: i32,
    ) -> WeightedCandidate {
        let candidate = RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: RetrievalProfile::ProcedureAssist,
            memory_type: MemoryType::Procedural,
            payload: CandidatePayload::Procedure(ProcedureCandidate {
                pattern: "test pattern".to_string(),
                confidence,
                observation_count,
                namespace: "default".to_string(),
            }),
        };
        WeightedCandidate {
            candidate,
            weight,
            weighted_score: score * weight,
        }
    }

    #[test]
    fn composite_score_formula_is_correct() {
        let scores = RankingScore {
            relevance: 0.8,
            recency: 0.6,
            stability: 0.7,
            provenance: 0.5,
        };
        let expected = 0.8 * 0.40 + 0.6 * 0.25 + 0.7 * 0.20 + 0.5 * 0.15;
        let actual = compute_final_score(&scores);
        assert!(
            (actual - expected).abs() < 1e-10,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn composite_score_matches_ranking_score_composite() {
        let scores = RankingScore {
            relevance: 0.9,
            recency: 0.4,
            stability: 0.6,
            provenance: 0.8,
        };
        let from_fn = compute_final_score(&scores);
        let from_method = scores.composite();
        assert!(
            (from_fn - from_method).abs() < 1e-10,
            "compute_final_score and RankingScore::composite should agree"
        );
    }

    #[test]
    fn rank_candidates_sorts_descending() {
        let candidates = vec![
            make_weighted_fact(0.3, 1.0, "extracted", 1),
            make_weighted_fact(0.9, 1.0, "user_asserted", 3),
            make_weighted_fact(0.6, 1.0, "observed", 2),
        ];

        let ranked = rank_candidates(candidates);

        for i in 1..ranked.len() {
            assert!(
                ranked[i - 1].final_score >= ranked[i].final_score,
                "candidates should be sorted descending: {} >= {}",
                ranked[i - 1].final_score,
                ranked[i].final_score
            );
        }
    }

    #[test]
    fn evidence_authority_ordering() {
        assert!(evidence_authority("user_asserted") > evidence_authority("observed"));
        assert!(evidence_authority("observed") > evidence_authority("extracted"));
        assert!(evidence_authority("extracted") > evidence_authority("inferred"));
        assert!(evidence_authority("inferred") > evidence_authority("deprecated"));
        assert!(evidence_authority("deprecated") > evidence_authority("superseded"));
    }

    #[test]
    fn recency_decay_recent_is_high() {
        let decay = compute_recency_decay(0.0);
        assert!(
            (decay - 1.0).abs() < 1e-10,
            "zero-age should have decay 1.0"
        );
    }

    #[test]
    fn recency_decay_old_is_low() {
        let decay = compute_recency_decay(90.0);
        assert!(
            decay < 0.2,
            "90-day-old item should have low recency: {decay}"
        );
    }

    #[test]
    fn recency_decay_at_half_life() {
        let decay = compute_recency_decay(RECENCY_HALF_LIFE_DAYS);
        assert!(
            (decay - 0.5).abs() < 0.01,
            "at half-life, decay should be ~0.5: {decay}"
        );
    }

    #[test]
    fn score_relevance_clamps_to_unit() {
        let wc = make_weighted_fact(1.5, 1.0, "extracted", 1);
        let rel = score_relevance(&wc);
        assert!(rel <= 1.0, "relevance should be clamped to 1.0");
    }

    #[test]
    fn score_stability_superseded_is_low() {
        let wc = make_weighted_fact(0.8, 1.0, "superseded", 1);
        let stab = score_stability(&wc.candidate);
        assert!(
            stab < 0.5,
            "superseded facts should have low stability: {stab}"
        );
    }

    #[test]
    fn score_provenance_more_episodes_is_higher() {
        let wc1 = make_weighted_fact(0.8, 1.0, "extracted", 1);
        let wc5 = make_weighted_fact(0.8, 1.0, "extracted", 5);
        let p1 = score_provenance(&wc1.candidate);
        let p5 = score_provenance(&wc5.candidate);
        assert!(
            p5 > p1,
            "more source episodes should increase provenance: {p5} > {p1}"
        );
    }

    #[test]
    fn trim_to_budget_respects_limit() {
        let candidates = vec![
            make_weighted_fact(0.9, 1.0, "extracted", 1),
            make_weighted_fact(0.8, 1.0, "extracted", 1),
            make_weighted_fact(0.7, 1.0, "extracted", 1),
            make_weighted_fact(0.6, 1.0, "extracted", 1),
        ];

        let ranked = rank_candidates(candidates);
        // Each fact is ~50 tokens, so budget of 120 should keep ~2.
        let trimmed = trim_to_budget(ranked, 120);
        assert!(
            trimmed.len() <= 3,
            "should trim to fit budget: got {} candidates",
            trimmed.len()
        );
    }

    #[test]
    fn empty_candidates_ranks_to_empty() {
        let ranked = rank_candidates(vec![]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn four_dimension_score_with_known_inputs() {
        let scores = RankingScore {
            relevance: 1.0,
            recency: 1.0,
            stability: 1.0,
            provenance: 1.0,
        };
        let final_score = compute_final_score(&scores);
        let expected = 0.40 + 0.25 + 0.20 + 0.15;
        assert!(
            (final_score - expected).abs() < 1e-10,
            "all-1.0 scores should sum to {expected}, got {final_score}"
        );
    }

    #[test]
    fn four_dimension_score_all_zero() {
        let scores = RankingScore {
            relevance: 0.0,
            recency: 0.0,
            stability: 0.0,
            provenance: 0.0,
        };
        let final_score = compute_final_score(&scores);
        assert!(
            final_score.abs() < 1e-10,
            "all-0.0 scores should sum to 0.0, got {final_score}"
        );
    }

    #[test]
    fn episode_recency_recent_scores_high() {
        let now = Utc::now();
        let wc = make_weighted_episode(0.8, 1.0, now);
        let recency = score_recency(&wc.candidate);
        assert!(
            recency > 0.9,
            "just-now episode should have high recency: {recency}"
        );
    }

    #[test]
    fn procedure_stability_high_confidence() {
        let wc = make_weighted_procedure(0.8, 1.0, 0.95, 10);
        let stab = score_stability(&wc.candidate);
        assert!(
            stab > 0.7,
            "high-confidence, high-observation procedure should have high stability: {stab}"
        );
    }
}
