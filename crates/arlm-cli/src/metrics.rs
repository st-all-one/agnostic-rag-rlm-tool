use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
struct Inner {
    requests_total: AtomicU64,
    search_results_count: AtomicU64,
    cache_hits_total: AtomicU64,
    nodes_total: AtomicU64,
    agent_requests: parking_lot::Mutex<HashMap<String, u64>>,
    agent_tokens: parking_lot::Mutex<HashMap<String, u64>>,
}

#[derive(Debug, Clone)]
pub struct ArlmMetrics {
    inner: Arc<Inner>,
}

impl ArlmMetrics {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                requests_total: AtomicU64::new(0),
                search_results_count: AtomicU64::new(0),
                cache_hits_total: AtomicU64::new(0),
                nodes_total: AtomicU64::new(0),
                agent_requests: parking_lot::Mutex::new(HashMap::new()),
                agent_tokens: parking_lot::Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn record_request(&self) {
        self.inner.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_search(&self, count: u64) {
        self.inner
            .search_results_count
            .fetch_add(count, Ordering::Relaxed);
    }

    #[allow(dead_code)]
    pub fn record_cache_hit(&self) {
        self.inner.cache_hits_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_node(&self) {
        self.inner.nodes_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record request for a specific agent.
    pub fn record_agent_request(&self, agent: &str, tokens: u64) {
        let mut requests = self.inner.agent_requests.lock();
        *requests.entry(agent.to_string()).or_insert(0) += 1;
        let mut tokens_map = self.inner.agent_tokens.lock();
        *tokens_map.entry(agent.to_string()).or_insert(0) += tokens;
    }

    #[must_use]
    pub fn render(&self) -> String {
        let requests = self.inner.requests_total.load(Ordering::Relaxed);
        let search = self.inner.search_results_count.load(Ordering::Relaxed);
        let cache = self.inner.cache_hits_total.load(Ordering::Relaxed);
        let nodes = self.inner.nodes_total.load(Ordering::Relaxed);

        let mut out = String::with_capacity(256);
        let _ = writeln!(out, "# HELP arlm_requests_total Total HTTP requests");
        let _ = writeln!(out, "# TYPE arlm_requests_total counter");
        let _ = writeln!(out, "arlm_requests_total {requests}");
        let _ = writeln!(
            out,
            "# HELP arlm_search_results_total Total search results returned"
        );
        let _ = writeln!(out, "# TYPE arlm_search_results_total counter");
        let _ = writeln!(out, "arlm_search_results_total {search}");
        let _ = writeln!(out, "# HELP arlm_cache_hits_total Total cache hits");
        let _ = writeln!(out, "# TYPE arlm_cache_hits_total counter");
        let _ = writeln!(out, "arlm_cache_hits_total {cache}");
        let _ = writeln!(out, "# HELP arlm_nodes_total Total nodes visited");
        let _ = writeln!(out, "# TYPE arlm_nodes_total counter");
        let _ = writeln!(out, "arlm_nodes_total {nodes}");

        // Agent-specific metrics
        let agent_requests = self.inner.agent_requests.lock();
        let agent_tokens = self.inner.agent_tokens.lock();

        if !agent_requests.is_empty() {
            let _ = writeln!(
                out,
                "# HELP arlm_agent_requests_total Requests per agent"
            );
            let _ = writeln!(out, "# TYPE arlm_agent_requests_total counter");
            for (agent, count) in agent_requests.iter() {
                let _ = writeln!(out, "arlm_agent_requests_total{{agent=\"{agent}\"}} {count}");
            }

            let _ = writeln!(
                out,
                "# HELP arlm_agent_tokens_total Tokens per agent"
            );
            let _ = writeln!(out, "# TYPE arlm_agent_tokens_total counter");
            for (agent, tokens) in agent_tokens.iter() {
                let _ = writeln!(out, "arlm_agent_tokens_total{{agent=\"{agent}\"}} {tokens}");
            }
        }

        out
    }
}

impl Default for ArlmMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_metrics_zero() {
        let m = ArlmMetrics::new();
        let rendered = m.render();
        assert!(rendered.contains("arlm_requests_total 0"));
        assert!(rendered.contains("arlm_search_results_total 0"));
        assert!(rendered.contains("arlm_cache_hits_total 0"));
        assert!(rendered.contains("arlm_nodes_total 0"));
    }

    #[test]
    fn test_record_request() {
        let m = ArlmMetrics::new();
        m.record_request();
        m.record_request();
        let rendered = m.render();
        assert!(rendered.contains("arlm_requests_total 2"));
    }

    #[test]
    fn test_record_search() {
        let m = ArlmMetrics::new();
        m.record_search(5);
        let rendered = m.render();
        assert!(rendered.contains("arlm_search_results_total 5"));
    }

    #[test]
    fn test_record_cache_hit() {
        let m = ArlmMetrics::new();
        m.record_cache_hit();
        m.record_cache_hit();
        m.record_cache_hit();
        let rendered = m.render();
        assert!(rendered.contains("arlm_cache_hits_total 3"));
    }

    #[test]
    fn test_record_node() {
        let m = ArlmMetrics::new();
        m.record_node();
        let rendered = m.render();
        assert!(rendered.contains("arlm_nodes_total 1"));
    }

    #[test]
    fn test_render_prometheus_format() {
        let m = ArlmMetrics::new();
        m.record_request();
        let rendered = m.render();
        assert!(rendered.starts_with("# HELP"));
        assert!(rendered.contains("# TYPE arlm_requests_total counter"));
        assert!(rendered.contains("# TYPE arlm_search_results_total counter"));
        assert!(rendered.contains("# TYPE arlm_cache_hits_total counter"));
        assert!(rendered.contains("# TYPE arlm_nodes_total counter"));
    }

    #[test]
    fn test_metrics_clone_shares_state() {
        let m1 = ArlmMetrics::new();
        let m2 = m1.clone();
        m1.record_request();
        let rendered = m2.render();
        assert!(rendered.contains("arlm_requests_total 1"));
    }
}
