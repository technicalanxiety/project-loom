//! Memory weight modifiers per task class.
//!
//! Applies task-specific weights to candidate relevance scores based on the
//! active [`TaskClass`]. Each (task class, memory type) pair has a weight
//! in `[0.0, 1.0]`. A weight of `0.0` means **hard exclusion** — the
//! candidate is removed from the list entirely.
//!
//! # Pipeline Position
//!
//! ```text
//! loom_think → classify → namespace → retrieve → **weight** → rank → compile
//! ```

use crate::pipeline::online::retrieve::{MemoryType, RetrievalCandidate};
use crate::types::classification::TaskClass;

// ---------------------------------------------------------------------------
// Weight matrix
// ---------------------------------------------------------------------------

/// Memory weights for a given task class.
///
/// Tuple order: `(episodic, semantic, procedural)`.
/// Graph candidates are treated as semantic for weighting purposes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryWeights {
    /// Weight for episodic memory (raw episodes).
    pub episodic: f64,
    /// Weight for semantic memory (facts and graph results).
    pub semantic: f64,
    /// Weight for procedural memory (behavioral patterns).
    pub procedural: f64,
}

/// Return the memory weight modifiers for a given task class.
///
/// | Task Class     | Episodic | Semantic | Procedural |
/// |----------------|----------|----------|------------|
/// | Debug          | 1.0      | 0.7      | 0.8        |
/// | Architecture   | 0.5      | 1.0      | 0.3        |
/// | Compliance     | 1.0      | 0.8      | 0.0        |
/// | Writing        | 0.3      | 1.0      | 0.6        |
/// | Chat           | 0.4      | 1.0      | 0.3        |
pub fn memory_weights(class: &TaskClass) -> MemoryWeights {
    match class {
        TaskClass::Debug => MemoryWeights {
            episodic: 1.0,
            semantic: 0.7,
            procedural: 0.8,
        },
        TaskClass::Architecture => MemoryWeights {
            episodic: 0.5,
            semantic: 1.0,
            procedural: 0.3,
        },
        TaskClass::Compliance => MemoryWeights {
            episodic: 1.0,
            semantic: 0.8,
            procedural: 0.0,
        },
        TaskClass::Writing => MemoryWeights {
            episodic: 0.3,
            semantic: 1.0,
            procedural: 0.6,
        },
        TaskClass::Chat => MemoryWeights {
            episodic: 0.4,
            semantic: 1.0,
            procedural: 0.3,
        },
    }
}

/// Look up the weight for a specific memory type under a task class.
///
/// [`MemoryType::Graph`] is treated as semantic for weighting purposes.
pub fn weight_for_memory_type(class: &TaskClass, memory_type: &MemoryType) -> f64 {
    let w = memory_weights(class);
    match memory_type {
        MemoryType::Episodic => w.episodic,
        MemoryType::Semantic | MemoryType::Graph => w.semantic,
        MemoryType::Procedural => w.procedural,
    }
}

// ---------------------------------------------------------------------------
// Weight application
// ---------------------------------------------------------------------------

/// A retrieval candidate with its weight-modified relevance score.
#[derive(Debug, Clone)]
pub struct WeightedCandidate {
    /// The original retrieval candidate.
    pub candidate: RetrievalCandidate,
    /// The weight applied to this candidate's score.
    pub weight: f64,
    /// The weight-modified relevance score (`original_score * weight`).
    pub weighted_score: f64,
}

/// Apply memory weight modifiers to a list of retrieval candidates.
///
/// Candidates whose weight is `0.0` are **hard-excluded** (removed from the
/// list). All other candidates have their relevance score multiplied by the
/// weight for their memory type under the active task class.
///
/// Returns a new list of [`WeightedCandidate`] with hard-excluded items
/// removed.
pub fn apply_weights(
    candidates: Vec<RetrievalCandidate>,
    class: &TaskClass,
) -> Vec<WeightedCandidate> {
    let weights = memory_weights(class);

    let result: Vec<WeightedCandidate> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let w = match candidate.memory_type {
                MemoryType::Episodic => weights.episodic,
                MemoryType::Semantic | MemoryType::Graph => weights.semantic,
                MemoryType::Procedural => weights.procedural,
            };

            // Hard-exclude candidates with weight 0.0.
            if w == 0.0 {
                tracing::debug!(
                    candidate_id = %candidate.id,
                    memory_type = %candidate.memory_type,
                    task_class = %class,
                    "hard-excluded candidate (weight 0.0)"
                );
                return None;
            }

            let weighted_score = candidate.score * w;

            Some(WeightedCandidate {
                candidate,
                weight: w,
                weighted_score,
            })
        })
        .collect();

    tracing::info!(
        task_class = %class,
        episodic_weight = weights.episodic,
        semantic_weight = weights.semantic,
        procedural_weight = weights.procedural,
        candidates_after_weighting = result.len(),
        "memory weight modifiers applied"
    );

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::online::retrieve::{
        CandidatePayload, EpisodeCandidate, FactCandidate, ProcedureCandidate,
    };
    use chrono::Utc;
    use uuid::Uuid;

    /// Helper to create a candidate with a given memory type and score.
    fn make_candidate(memory_type: MemoryType, score: f64) -> RetrievalCandidate {
        let profile = match &memory_type {
            MemoryType::Episodic => crate::pipeline::online::retrieve::RetrievalProfile::EpisodeRecall,
            MemoryType::Semantic => crate::pipeline::online::retrieve::RetrievalProfile::FactLookup,
            MemoryType::Graph => crate::pipeline::online::retrieve::RetrievalProfile::GraphNeighborhood,
            MemoryType::Procedural => crate::pipeline::online::retrieve::RetrievalProfile::ProcedureAssist,
        };

        let payload = match &memory_type {
            MemoryType::Episodic => CandidatePayload::Episode(EpisodeCandidate {
                source: "test".to_string(),
                content: "test content".to_string(),
                occurred_at: Utc::now(),
                namespace: "default".to_string(),
            }),
            MemoryType::Semantic => CandidatePayload::Fact(FactCandidate {
                subject_id: Uuid::new_v4(),
                predicate: "uses".to_string(),
                object_id: Uuid::new_v4(),
                evidence_status: "extracted".to_string(),
                source_episodes: vec![Uuid::new_v4()],
                namespace: "default".to_string(),
            }),
            MemoryType::Graph => CandidatePayload::Graph(
                crate::pipeline::online::retrieve::GraphCandidate {
                    entity_id: Uuid::new_v4(),
                    entity_name: "test".to_string(),
                    entity_type: "service".to_string(),
                    fact_id: None,
                    predicate: None,
                    hop_depth: 1,
                },
            ),
            MemoryType::Procedural => CandidatePayload::Procedure(ProcedureCandidate {
                pattern: "test pattern".to_string(),
                confidence: 0.9,
                observation_count: 5,
                namespace: "default".to_string(),
            }),
        };

        RetrievalCandidate {
            id: Uuid::new_v4(),
            score,
            source_profile: profile,
            memory_type,
            payload,
        }
    }

    #[test]
    fn debug_weights_are_correct() {
        let w = memory_weights(&TaskClass::Debug);
        assert!((w.episodic - 1.0).abs() < f64::EPSILON);
        assert!((w.semantic - 0.7).abs() < f64::EPSILON);
        assert!((w.procedural - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn architecture_weights_are_correct() {
        let w = memory_weights(&TaskClass::Architecture);
        assert!((w.episodic - 0.5).abs() < f64::EPSILON);
        assert!((w.semantic - 1.0).abs() < f64::EPSILON);
        assert!((w.procedural - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn compliance_weights_are_correct() {
        let w = memory_weights(&TaskClass::Compliance);
        assert!((w.episodic - 1.0).abs() < f64::EPSILON);
        assert!((w.semantic - 0.8).abs() < f64::EPSILON);
        assert!((w.procedural - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn writing_weights_are_correct() {
        let w = memory_weights(&TaskClass::Writing);
        assert!((w.episodic - 0.3).abs() < f64::EPSILON);
        assert!((w.semantic - 1.0).abs() < f64::EPSILON);
        assert!((w.procedural - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn chat_weights_are_correct() {
        let w = memory_weights(&TaskClass::Chat);
        assert!((w.episodic - 0.4).abs() < f64::EPSILON);
        assert!((w.semantic - 1.0).abs() < f64::EPSILON);
        assert!((w.procedural - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn hard_exclusion_removes_procedural_for_compliance() {
        let candidates = vec![
            make_candidate(MemoryType::Episodic, 0.9),
            make_candidate(MemoryType::Procedural, 0.95),
            make_candidate(MemoryType::Semantic, 0.8),
        ];

        let weighted = apply_weights(candidates, &TaskClass::Compliance);

        // Procedural should be excluded.
        assert_eq!(weighted.len(), 2);
        for wc in &weighted {
            assert_ne!(
                wc.candidate.memory_type,
                MemoryType::Procedural,
                "procedural candidates should be hard-excluded for compliance"
            );
        }
    }

    #[test]
    fn weight_multiplies_score() {
        let candidates = vec![make_candidate(MemoryType::Episodic, 0.8)];

        let weighted = apply_weights(candidates, &TaskClass::Architecture);

        assert_eq!(weighted.len(), 1);
        // Architecture episodic weight = 0.5, so 0.8 * 0.5 = 0.4
        let expected = 0.8 * 0.5;
        assert!(
            (weighted[0].weighted_score - expected).abs() < f64::EPSILON,
            "expected {expected}, got {}",
            weighted[0].weighted_score
        );
    }

    #[test]
    fn graph_uses_semantic_weight() {
        let candidates = vec![make_candidate(MemoryType::Graph, 0.9)];

        let weighted = apply_weights(candidates, &TaskClass::Debug);

        assert_eq!(weighted.len(), 1);
        // Debug semantic weight = 0.7, graph uses semantic weight
        let expected = 0.9 * 0.7;
        assert!(
            (weighted[0].weighted_score - expected).abs() < f64::EPSILON,
            "expected {expected}, got {}",
            weighted[0].weighted_score
        );
    }

    #[test]
    fn weight_1_0_preserves_score() {
        let candidates = vec![make_candidate(MemoryType::Episodic, 0.75)];

        let weighted = apply_weights(candidates, &TaskClass::Debug);

        assert_eq!(weighted.len(), 1);
        // Debug episodic weight = 1.0
        assert!(
            (weighted[0].weighted_score - 0.75).abs() < f64::EPSILON,
            "weight 1.0 should preserve original score"
        );
    }

    #[test]
    fn empty_candidates_returns_empty() {
        let weighted = apply_weights(vec![], &TaskClass::Chat);
        assert!(weighted.is_empty());
    }

    #[test]
    fn weight_for_memory_type_returns_correct_values() {
        assert!((weight_for_memory_type(&TaskClass::Debug, &MemoryType::Episodic) - 1.0).abs() < f64::EPSILON);
        assert!((weight_for_memory_type(&TaskClass::Compliance, &MemoryType::Procedural) - 0.0).abs() < f64::EPSILON);
        assert!((weight_for_memory_type(&TaskClass::Architecture, &MemoryType::Graph) - 1.0).abs() < f64::EPSILON);
    }
}
