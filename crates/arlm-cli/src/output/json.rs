use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct JsonOutput {
    status: String,
    data: Option<serde_json::Value>,
    metadata: HashMap<String, serde_json::Value>,
}

impl JsonOutput {
    #[must_use]
    pub fn ok() -> Self {
        Self {
            status: "ok".to_string(),
            data: None,
            metadata: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_data(mut self, data: serde_json::Value) -> Self {
        self.data = Some(data);
        self
    }

    #[must_use]
    pub fn with_metadata(mut self, key: &str, value: impl Into<serde_json::Value>) -> Self {
        self.metadata.insert(key.to_string(), value.into());
        self
    }

    pub fn print(&self) {
        println!("{}", self.to_json_string());
    }

    #[must_use]
    pub fn to_json_string(&self) -> String {
        let mut map = serde_json::Map::new();
        map.insert("status".to_string(), serde_json::json!(self.status));
        if let Some(ref data) = self.data {
            map.insert("data".to_string(), data.clone());
        }
        if !self.metadata.is_empty() {
            map.insert("metadata".to_string(), serde_json::json!(self.metadata));
        }
        let output = serde_json::Value::Object(map);
        serde_json::to_string_pretty(&output)
            .unwrap_or_else(|_| r#"{"status":"ok"}"#.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_output_ok() {
        let output = JsonOutput::ok();
        let s = format!("{}", output.status);
        assert_eq!(s, "ok");
    }

    #[test]
    fn test_json_output_with_data() {
        let output = JsonOutput::ok().with_data(serde_json::json!({"count": 42}));
        let json = output.to_json_string();
        assert!(json.contains("42"));
    }
}
