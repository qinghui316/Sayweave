//! Independent implementation of low-thinking request policy, informed by SayIt's
//! pinned provider behavior. No provider response text is logged or persisted.
use serde_json::{Value, json};
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Parameters {
    pub thinking: Value,
    pub completion_tokens: bool,
}
impl Parameters {
    pub fn initial(endpoint: &str, model: &str) -> Self {
        let host = reqwest::Url::parse(endpoint)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let thinking = if host == "api.openai.com"
            || host.ends_with(".openai.azure.com")
            || host.ends_with(".cognitiveservices.azure.com")
        {
            json!({})
        } else if model.to_ascii_lowercase().starts_with("glm-5.3") {
            low()
        } else if matches!(
            host.as_str(),
            "dashscope.aliyuncs.com" | "dashscope-intl.aliyuncs.com"
        ) {
            json!({"enable_thinking":false})
        } else if matches!(
            host.as_str(),
            "api.deepseek.com" | "api.xiaomimimo.com" | "open.bigmodel.cn"
        ) {
            json!({"thinking":{"type":"disabled"}})
        } else {
            json!({"enable_thinking":false,"thinking":{"type":"disabled"},"reasoning":{"effort":"none"},"reasoning_effort":"none"})
        };
        Self {
            thinking,
            completion_tokens: false,
        }
    }
    pub fn apply(&self, body: &mut Value, tokens: u32) {
        for (key, value) in self.thinking.as_object().unwrap() {
            body[key] = value.clone();
        }
        body[if self.completion_tokens {
            "max_completion_tokens"
        } else {
            "max_tokens"
        }] = json!(tokens);
    }
    pub fn adjust(&mut self, status: u16, body: &Value, thinking_adjusted: &mut bool) -> bool {
        if !matches!(status, 400 | 422) {
            return false;
        }
        if !self.completion_tokens && super::provider::unsupported_max_tokens(status, body) {
            self.completion_tokens = true;
            return true;
        }
        if *thinking_adjusted || self.thinking.as_object().is_none_or(|v| v.is_empty()) {
            return false;
        }
        let Some(error) = body.get("error") else {
            return false;
        };
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        let code = error.get("code").and_then(Value::as_str).unwrap_or("");
        let param = error.get("param").and_then(Value::as_str).unwrap_or("");
        let explicit = self.thinking.as_object().unwrap().keys().any(|field| {
            (param == field && matches!(code, "unsupported_parameter" | "unsupported_value"))
                || (message.contains(field)
                    && [
                        "unrecognized request argument",
                        "unknown parameter",
                        "unsupported",
                        "not supported",
                    ]
                    .iter()
                    .any(|word| message.contains(word)))
        });
        let forced = code == "1210"
            || message.contains("不支持关闭思考")
            || message.contains("始终思考")
            || message.contains("does not support disabling thinking")
            || message.contains("thinking cannot be disabled");
        if !explicit && !forced {
            return false;
        }
        let next = if forced { low() } else { json!({}) };
        if next == self.thinking {
            return false;
        }
        self.thinking = next;
        *thinking_adjusted = true;
        true
    }
}
fn low() -> Value {
    json!({"thinking":{"type":"enabled"},"reasoning_effort":"low"})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_and_custom_policy_and_strict_exceptions() {
        assert_eq!(
            Parameters::initial("https://api.openai.com/v1", "gpt").thinking,
            json!({})
        );
        assert_eq!(
            Parameters::initial("https://open.bigmodel.cn/api/paas/v4", "GLM-5.3").thinking,
            low()
        );
        assert_eq!(
            Parameters::initial("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen")
                .thinking,
            json!({"enable_thinking":false})
        );
        assert_eq!(
            Parameters::initial("https://proxy.example/v1", "model").thinking["reasoning_effort"],
            "none"
        );
    }
    #[test]
    fn only_explicit_pre_output_errors_allow_bounded_adjustments() {
        let mut p = Parameters::initial("https://proxy.example/v1", "model");
        let mut adjusted = false;
        assert!(!p.adjust(
            401,
            &json!({"error":{"param":"thinking","code":"unsupported_parameter"}}),
            &mut adjusted
        ));
        assert!(!p.adjust(
            400,
            &json!({"error":{"message":"model not found"}}),
            &mut adjusted
        ));
        assert!(p.adjust(
            400,
            &json!({"error":{"param":"max_tokens","code":"unsupported_parameter"}}),
            &mut adjusted
        ));
        assert!(p.adjust(
            400,
            &json!({"error":{"code":"1210","message":"该模型始终思考"}}),
            &mut adjusted
        ));
        assert_eq!(p.thinking, low());
        assert!(!p.adjust(
            400,
            &json!({"error":{"param":"reasoning_effort","code":"unsupported_value"}}),
            &mut adjusted
        ));
    }
}
