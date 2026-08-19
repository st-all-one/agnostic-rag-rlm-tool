use std::collections::HashMap;

use crate::types::UsageSummary;

#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_per_1m: f64,
    pub output_per_1m: f64,
}

impl ModelPricing {
    #[must_use]
    pub fn new(input_per_1m: f64, output_per_1m: f64) -> Self {
        Self {
            input_per_1m,
            output_per_1m,
        }
    }

    #[must_use]
    pub fn cost_usd(&self, usage: &UsageSummary) -> f64 {
        let input_cost = (f64::from(usage.prompt_tokens) / 1_000_000.0) * self.input_per_1m;
        let output_cost = (f64::from(usage.completion_tokens) / 1_000_000.0) * self.output_per_1m;
        input_cost + output_cost
    }
}

#[derive(Debug, Clone)]
pub struct PricingTable {
    models: HashMap<String, ModelPricing>,
}

impl PricingTable {
    #[must_use]
    pub fn new() -> Self {
        let mut models = HashMap::new();

        // OpenAI models (USD per 1M tokens)
        models.insert("gpt-4o".to_string(), ModelPricing::new(2.50, 10.00));
        models.insert("gpt-4o-mini".to_string(), ModelPricing::new(0.15, 0.60));
        models.insert("gpt-4-turbo".to_string(), ModelPricing::new(10.00, 30.00));
        models.insert("gpt-4".to_string(), ModelPricing::new(30.00, 60.00));
        models.insert("gpt-3.5-turbo".to_string(), ModelPricing::new(0.50, 1.50));

        // Anthropic models
        models.insert(
            "claude-sonnet-4-20250514".to_string(),
            ModelPricing::new(3.00, 15.00),
        );
        models.insert(
            "claude-3-5-sonnet-20241022".to_string(),
            ModelPricing::new(3.00, 15.00),
        );
        models.insert(
            "claude-3-5-haiku-20241022".to_string(),
            ModelPricing::new(0.80, 4.00),
        );
        models.insert(
            "claude-3-opus-20240229".to_string(),
            ModelPricing::new(15.00, 75.00),
        );

        // Google Gemini models
        models.insert("gemini-1.5-pro".to_string(), ModelPricing::new(1.25, 5.00));
        models.insert(
            "gemini-1.5-flash".to_string(),
            ModelPricing::new(0.075, 0.30),
        );

        // DeepSeek models
        models.insert(
            "deepseek-v3".to_string(),
            ModelPricing::new(0.27, 1.10),
        );
        models.insert(
            "deepseek-r1".to_string(),
            ModelPricing::new(0.55, 2.19),
        );

        // MiMo (zero cost via local or API)
        models.insert("mimo".to_string(), ModelPricing::new(0.0, 0.0));

        // Ollama (local) - zero cost
        models.insert("llama3".to_string(), ModelPricing::new(0.0, 0.0));
        models.insert("codellama".to_string(), ModelPricing::new(0.0, 0.0));
        models.insert("mistral".to_string(), ModelPricing::new(0.0, 0.0));

        Self { models }
    }

    #[must_use]
    pub fn get(&self, model: &str) -> Option<&ModelPricing> {
        self.models.get(model)
    }

    pub fn register(&mut self, model: String, pricing: ModelPricing) {
        self.models.insert(model, pricing);
    }

    #[must_use]
    pub fn estimate_cost(&self, model: &str, usage: &UsageSummary) -> f64 {
        self.models.get(model).map_or(0.0, |p| p.cost_usd(usage))
    }
}

impl Default for PricingTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_pricing_cost() {
        let pricing = ModelPricing::new(10.0, 30.0);
        let usage = UsageSummary {
            prompt_tokens: 1_000_000,
            completion_tokens: 1_000_000,
            total_tokens: 2_000_000,
        };
        let cost = pricing.cost_usd(&usage);
        assert!((cost - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_model_pricing_cost_partial() {
        let pricing = ModelPricing::new(10.0, 30.0);
        let usage = UsageSummary {
            prompt_tokens: 100_000,
            completion_tokens: 50_000,
            total_tokens: 150_000,
        };
        let cost = pricing.cost_usd(&usage);
        assert!((cost - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_pricing_table_default() {
        let table = PricingTable::default();
        assert!(table.get("gpt-4o").is_some());
        assert!(table.get("claude-sonnet-4-20250514").is_some());
        assert!(table.get("unknown-model").is_none());
    }

    #[test]
    fn test_pricing_table_register() {
        let mut table = PricingTable::default();
        table.register("custom-model".to_string(), ModelPricing::new(5.0, 10.0));
        assert!(table.get("custom-model").is_some());
    }

    #[test]
    fn test_pricing_table_estimate_cost() {
        let table = PricingTable::default();
        let usage = UsageSummary {
            prompt_tokens: 1_000_000,
            completion_tokens: 1_000_000,
            total_tokens: 2_000_000,
        };
        let cost = table.estimate_cost("gpt-4o", &usage);
        assert!((cost - 12.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_pricing_table_unknown_model() {
        let table = PricingTable::default();
        let usage = UsageSummary {
            prompt_tokens: 1_000_000,
            completion_tokens: 1_000_000,
            total_tokens: 2_000_000,
        };
        let cost = table.estimate_cost("unknown-model", &usage);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_ollama_zero_cost() {
        let table = PricingTable::default();
        let usage = UsageSummary {
            prompt_tokens: 1_000_000,
            completion_tokens: 1_000_000,
            total_tokens: 2_000_000,
        };
        let cost = table.estimate_cost("llama3", &usage);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }
}
