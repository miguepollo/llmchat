use crate::config::Settings;
use crate::types::{Message, Role};
use futures_util::StreamExt;
use std::time::Duration;

/// Eventos que el hilo de red envía hacia la interfaz.
#[derive(Debug)]
pub enum StreamEvent {
    Chunk(String),
    Done,
    Error(String),
}

fn endpoint_for(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

/// Llama a un endpoint OpenAI-compatible con `stream: true` y va reenviando
/// los fragmentos de texto por el canal `tx`.
pub async fn stream_chat(
    settings: &Settings,
    messages: &[Message],
    tx: tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!(
                "No se pudo crear el cliente HTTP: {e}"
            )));
            return;
        }
    };

    let api_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            serde_json::json!({ "role": role, "content": m.content })
        })
        .collect();

    let body = serde_json::json!({
        "model": settings.model,
        "messages": api_messages,
        "stream": true,
        "temperature": settings.temperature,
    });

    let mut request = client.post(endpoint_for(&settings.base_url)).json(&body);
    let key = settings.api_key.trim();
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }

    let response = match request.send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!("Error de conexión: {e}")));
            return;
        }
    };

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let _ = tx.send(StreamEvent::Error(format!("HTTP {status}: {text}")));
        return;
    }

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                if let Some(delta) = parse_sse_chunk(&text) {
                    match delta {
                        StreamDelta::Delta(content) => {
                            let _ = tx.send(StreamEvent::Chunk(content));
                        }
                        StreamDelta::Done => {
                            let _ = tx.send(StreamEvent::Done);
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!(
                    "Error leyendo la respuesta: {e}"
                )));
                return;
            }
        }
    }
    let _ = tx.send(StreamEvent::Done);
}

enum StreamDelta {
    Delta(String),
    Done,
}

/// Analiza un trozo de SSE: las líneas empiezan con `data:` y llevan JSON.
/// Devuelve `None` si el trozo no contenía nada relevante.
fn parse_sse_chunk(chunk: &str) -> Option<StreamDelta> {
    let mut result: Option<StreamDelta> = None;
    for line in chunk.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                return Some(StreamDelta::Done);
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                let content = value["choices"][0]["delta"]["content"]
                    .as_str()
                    .map(String::from)
                    .or_else(|| {
                        value["choices"][0]["delta"]["reasoning_content"]
                            .as_str()
                            .map(String::from)
                    })
                    .or_else(|| {
                        value["choices"][0]["message"]["content"]
                            .as_str()
                            .map(String::from)
                    });
                if let Some(c) = content {
                    result = Some(StreamDelta::Delta(c));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_delta_content() {
        let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\n";
        assert!(
            matches!(parse_sse_chunk(chunk), Some(StreamDelta::Delta(s)) if s == "Hola")
        );
    }

    #[test]
    fn parses_done_marker() {
        assert!(matches!(
            parse_sse_chunk("data: [DONE]\n\n"),
            Some(StreamDelta::Done)
        ));
    }

    #[test]
    fn ignores_non_data_lines() {
        assert!(parse_sse_chunk(": keepalive\n\n").is_none());
    }

    #[test]
    fn handles_reasoning_content() {
        let chunk = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"pensando\"}}]}\n\n";
        assert!(
            matches!(parse_sse_chunk(chunk), Some(StreamDelta::Delta(s)) if s == "pensando")
        );
    }
}

