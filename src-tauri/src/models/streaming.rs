//! Strict incremental Chat Completions framing; partial text is never a result.
use super::types::ModelError;
use crate::contracts::processing::TokenUsage;
use serde_json::Value;

const BODY_LIMIT: usize = 1024 * 1024;
const OUTPUT_LIMIT: usize = 16 * 1024;
fn choice_index(choice: &Value) -> Result<(), ModelError> {
    if choice.get("index").and_then(Value::as_u64) == Some(0) {
        Ok(())
    } else {
        Err(ModelError::new("protocol"))
    }
}
fn assistant_role(role: Option<&Value>, required: bool) -> Result<(), ModelError> {
    match role {
        Some(v) if v.as_str() == Some("assistant") => Ok(()),
        None if !required => Ok(()),
        _ => Err(ModelError::new("protocol")),
    }
}
#[derive(Default)]
pub(super) struct Decoder {
    pending: Vec<u8>,
    data: Vec<String>,
    text: String,
    usage: TokenUsage,
    bytes: usize,
    first_line: bool,
    stopped: bool,
    done: bool,
}
impl Decoder {
    pub fn is_done(&self) -> bool {
        self.done
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<usize, ModelError> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or(ModelError::new("responseTooLarge"))?;
        if self.bytes > BODY_LIMIT {
            return Err(ModelError::new("responseTooLarge"));
        }
        let before = self.text.len();
        self.pending.extend_from_slice(bytes);
        self.lines(false)?;
        Ok(self.text.len() - before)
    }
    fn lines(&mut self, eof: bool) -> Result<(), ModelError> {
        while let Some(end) = self
            .pending
            .iter()
            .position(|b| matches!(*b, b'\r' | b'\n'))
        {
            if self.pending[end] == b'\r' && end + 1 == self.pending.len() && !eof {
                break;
            }
            let separator =
                if self.pending[end] == b'\r' && self.pending.get(end + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            let line = String::from_utf8(self.pending[..end].to_vec())
                .map_err(|_| ModelError::new("protocol"))?;
            self.pending.drain(..end + separator);
            self.line(&line)?;
        }
        Ok(())
    }
    fn line(&mut self, line: &str) -> Result<(), ModelError> {
        let line = if !self.first_line {
            self.first_line = true;
            line.strip_prefix('\u{feff}').unwrap_or(line)
        } else {
            line
        };
        if line.is_empty() {
            if !self.data.is_empty() {
                let event = self.data.join("\n");
                self.data.clear();
                self.event(&event)?;
            }
        } else if !line.starts_with(':') {
            let (field, value) = line.split_once(':').unwrap_or((line, ""));
            if field == "data" {
                self.data
                    .push(value.strip_prefix(' ').unwrap_or(value).into());
            }
        }
        Ok(())
    }
    fn event(&mut self, event: &str) -> Result<(), ModelError> {
        if self.done {
            return Err(ModelError::new("protocol"));
        }
        if event == "[DONE]" {
            if !self.stopped {
                return Err(ModelError::new("incompleteText"));
            }
            self.done = true;
            return Ok(());
        }
        let value: Value = serde_json::from_str(event).map_err(|_| ModelError::new("protocol"))?;
        if value.get("error").is_some() {
            return Err(ModelError::new("service"));
        }
        read_usage(&value, &mut self.usage)?;
        let choices = value
            .get("choices")
            .and_then(Value::as_array)
            .ok_or(ModelError::new("protocol"))?;
        if choices.is_empty() {
            if value.get("usage").is_none_or(Value::is_null) {
                return Err(ModelError::new("protocol"));
            }
            return Ok(());
        }
        if self.stopped || choices.len() != 1 {
            return Err(ModelError::new("protocol"));
        }
        let choice = &choices[0];
        choice_index(choice)?;
        let delta = choice
            .get("delta")
            .and_then(Value::as_object)
            .ok_or(ModelError::new("protocol"))?;
        if delta.get("tool_calls").is_some_and(|v| !v.is_null())
            || delta.get("function_call").is_some_and(|v| !v.is_null())
        {
            return Err(ModelError::new("unexpectedAction"));
        }
        if delta
            .get("refusal")
            .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
        {
            return Err(ModelError::new("refused"));
        }
        assistant_role(delta.get("role"), false)?;
        match delta.get("content") {
            None | Some(Value::Null) => {}
            Some(Value::String(text)) => {
                if self.text.len() + text.len() > OUTPUT_LIMIT {
                    return Err(ModelError::new("responseTooLarge"));
                }
                self.text.push_str(text);
            }
            _ => return Err(ModelError::new("protocol")),
        }
        match choice.get("finish_reason") {
            None | Some(Value::Null) => {}
            Some(Value::String(reason)) if reason == "stop" => self.stopped = true,
            Some(Value::String(reason)) if reason == "length" => {
                return Err(ModelError::new("truncatedText"));
            }
            Some(Value::String(reason)) if reason == "content_filter" => {
                return Err(ModelError::new("refused"));
            }
            _ => return Err(ModelError::new("unexpectedAction")),
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<(String, TokenUsage), ModelError> {
        self.lines(true)?;
        // SSE events must terminate with an empty line. EOF is not a completion.
        if !self.pending.is_empty() || !self.data.is_empty() || !self.done || !self.stopped {
            return Err(ModelError::new("incompleteText"));
        }
        if self.text.trim().is_empty() {
            return Err(ModelError::new("emptyText"));
        }
        Ok((self.text, self.usage))
    }
}
fn read_usage(value: &Value, usage: &mut TokenUsage) -> Result<(), ModelError> {
    if let Some(value) = value.get("usage").filter(|v| !v.is_null()) {
        if !value.is_object() {
            return Err(ModelError::new("protocol"));
        }
        let count = |key| match value.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(n) => n.as_u64().map(Some).ok_or(ModelError::new("protocol")),
        };
        *usage = TokenUsage {
            input: count("prompt_tokens")?,
            output: count("completion_tokens")?,
            total: count("total_tokens")?,
        };
    }
    Ok(())
}
pub(super) fn complete_json(bytes: &[u8]) -> Result<(String, TokenUsage), ModelError> {
    if bytes.len() > BODY_LIMIT {
        return Err(ModelError::new("responseTooLarge"));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| ModelError::new("protocol"))?;
    let choices = value
        .get("choices")
        .and_then(Value::as_array)
        .ok_or(ModelError::new("protocol"))?;
    if choices.len() != 1 {
        return Err(ModelError::new("protocol"));
    }
    let choice = &choices[0];
    choice_index(choice)?;
    match choice.get("finish_reason").and_then(Value::as_str) {
        Some("stop") => {}
        Some("length") => return Err(ModelError::new("truncatedText")),
        Some("content_filter") => return Err(ModelError::new("refused")),
        _ => return Err(ModelError::new("incompleteText")),
    }
    let message = choice
        .get("message")
        .and_then(Value::as_object)
        .ok_or(ModelError::new("protocol"))?;
    assistant_role(message.get("role"), true)?;
    if message.get("tool_calls").is_some_and(|v| !v.is_null())
        || message.get("function_call").is_some_and(|v| !v.is_null())
    {
        return Err(ModelError::new("unexpectedAction"));
    }
    if message
        .get("refusal")
        .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
    {
        return Err(ModelError::new("refused"));
    }
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .ok_or(ModelError::new("protocol"))?;
    if text.len() > OUTPUT_LIMIT {
        return Err(ModelError::new("responseTooLarge"));
    }
    if text.trim().is_empty() {
        return Err(ModelError::new("emptyText"));
    }
    let mut usage = TokenUsage::default();
    read_usage(&value, &mut usage)?;
    Ok((text.into(), usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream() -> Vec<u8> {
        concat!("\u{feff}: keepalive\r\n\r\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"中文🙂\"},\"finish_reason\":null}]}\r\n\r\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"。\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":20,\"completion_tokens\":5,\"total_tokens\":25}}\r\r",
            "data: [DONE]\n\n").as_bytes().to_vec()
    }
    #[test]
    fn every_utf8_line_and_json_network_split_has_one_complete_result() {
        let bytes = stream();
        for split in 0..=bytes.len() {
            let mut decoder = Decoder::default();
            decoder.push(&bytes[..split]).unwrap();
            decoder.push(&bytes[split..]).unwrap();
            let (text, usage) = decoder.finish().unwrap();
            assert_eq!(text, "中文🙂。");
            assert_eq!(usage.total, Some(25));
        }
        let mut decoder = Decoder::default();
        for byte in bytes {
            decoder.push(&[byte]).unwrap();
        }
        assert_eq!(decoder.finish().unwrap().0, "中文🙂。");
    }
    #[test]
    fn eof_done_without_stop_and_partial_frame_never_succeed() {
        let bytes = stream();
        for end in 0..bytes.len() {
            let mut d = Decoder::default();
            if d.push(&bytes[..end]).is_ok() {
                assert!(d.finish().is_err(), "end={end}");
            }
        }
        let mut d = Decoder::default();
        assert_eq!(
            d.push(b"data: [DONE]\n\n").unwrap_err().code,
            "incompleteText"
        );
    }
    #[test]
    fn rejected_truncated_malformed_and_tool_results() {
        for (json, code) in [
            (
                r#"{"choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":"length"}]}"#,
                "truncatedText",
            ),
            (
                r#"{"choices":[{"index":0,"delta":{"refusal":"no"},"finish_reason":null}]}"#,
                "refused",
            ),
            (
                r#"{"choices":[{"index":0,"delta":{"tool_calls":[]},"finish_reason":null}]}"#,
                "unexpectedAction",
            ),
            (
                r#"{"choices":[{"index":0,"delta":{"content":{}},"finish_reason":null}]}"#,
                "protocol",
            ),
        ] {
            let mut d = Decoder::default();
            assert_eq!(
                d.push(format!("data: {json}\n\n").as_bytes())
                    .unwrap_err()
                    .code,
                code
            );
        }
        assert_eq!(
            complete_json(
                br#"{"choices":[{"index":0,"message":{"role":"assistant","content":"partial"}}]}"#
            )
            .err()
            .unwrap()
            .code,
            "incompleteText"
        );
    }
}
