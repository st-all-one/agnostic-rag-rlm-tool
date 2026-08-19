use serde::{Deserialize, Serialize};

use crate::types::Action;

/// Sampling parameters for LLM requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingArgs {
    pub temperature: f32,
    pub top_p: f32,
    pub top_k: Option<u32>,
}

impl SamplingArgs {
    /// Create sampling args tailored to the node type in the RLM tree.
    #[must_use]
    pub fn for_node_type(action: Action) -> Self {
        match action {
            Action::Solve => Self {
                temperature: 0.3,
                top_p: 0.9,
                top_k: None,
            },
            Action::Decompose => Self {
                temperature: 0.1,
                top_p: 0.85,
                top_k: None,
            },
        }
    }

    /// Apply these sampling args to a `CompletionRequest` by setting the
    /// temperature field. Returns the request unchanged if temperature is
    /// already set.
    #[must_use]
    pub fn apply_to_request(
        self,
        mut req: arlm_llm::CompletionRequest,
    ) -> arlm_llm::CompletionRequest {
        if req.temperature.is_none() {
            req.temperature = Some(self.temperature);
        }
        req
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_for_solve_action() {
        let args = SamplingArgs::for_node_type(Action::Solve);
        assert!((args.temperature - 0.3).abs() < f32::EPSILON);
        assert!((args.top_p - 0.9).abs() < f32::EPSILON);
        assert!(args.top_k.is_none());
    }

    #[test]
    fn test_for_decompose_action() {
        let args = SamplingArgs::for_node_type(Action::Decompose);
        assert!((args.temperature - 0.1).abs() < f32::EPSILON);
        assert!((args.top_p - 0.85).abs() < f32::EPSILON);
        assert!(args.top_k.is_none());
    }

    #[test]
    fn test_apply_to_request_sets_temperature() {
        use arlm_llm::{CompletionRequest, Message, Role};

        let args = SamplingArgs {
            temperature: 0.5,
            top_p: 0.9,
            top_k: Some(40),
        };
        let req = CompletionRequest {
            model: "test".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "hi".to_string(),
            }],
            temperature: None,
            max_tokens: None,
            stop: None,
        };
        let updated = args.apply_to_request(req);
        assert!((updated.temperature.unwrap() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_apply_to_request_preserves_existing_temperature() {
        use arlm_llm::{CompletionRequest, Message, Role};

        let args = SamplingArgs {
            temperature: 0.5,
            top_p: 0.9,
            top_k: None,
        };
        let req = CompletionRequest {
            model: "test".to_string(),
            messages: vec![Message {
                role: Role::User,
                content: "hi".to_string(),
            }],
            temperature: Some(0.9),
            max_tokens: None,
            stop: None,
        };
        let updated = args.apply_to_request(req);
        assert!((updated.temperature.unwrap() - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let args = SamplingArgs::for_node_type(Action::Decompose);
        let json = serde_json::to_string(&args).unwrap();
        let deserialized: SamplingArgs = serde_json::from_str(&json).unwrap();
        assert_eq!(args.temperature, deserialized.temperature);
        assert_eq!(args.top_p, deserialized.top_p);
    }
}
