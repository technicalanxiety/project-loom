// loom-engine/src/llm/client.rs
// Ollama HTTP client (reqwest) with Azure OpenAI fallback.
// Uses OpenAI-compatible API format (/v1/chat/completions, /v1/embeddings).

// TODO: Implement
// - OllamaClient struct (reqwest::Client, base_url)
// - chat_completion(model, messages, temperature) -> ChatResponse
// - embed(model, input) -> Vec<f64>
// - AzureOpenAIClient for fallback
// - LlmClient trait abstracting both backends
