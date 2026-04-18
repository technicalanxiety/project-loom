// loom-engine/src/api/dashboard.rs
// Dashboard API endpoints (read-only + 2 writes).
// Serves data for the React dashboard SPA.

// TODO: Implement
// - GET  /dashboard/api/health -> PipelineHealth
// - GET  /dashboard/api/entities?namespace= -> Vec<Entity>
// - GET  /dashboard/api/facts?namespace= -> Vec<Fact>
// - GET  /dashboard/api/traces?namespace=&limit=&offset= -> Vec<AuditLogEntry>
// - GET  /dashboard/api/conflicts -> Vec<ResolutionConflict>
// - GET  /dashboard/api/candidates -> Vec<PredicateCandidate>
// - GET  /dashboard/api/metrics?namespace= -> RetrievalMetrics
// - POST /dashboard/api/conflicts/:id/resolve -> ResolutionResult
// - POST /dashboard/api/candidates/:id/resolve -> CandidateResult
