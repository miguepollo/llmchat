use crate::config::Settings;
use crate::types::{Message, Role};
use futures_util::StreamExt;
use std::time::Duration;

/// Eventos que el hilo de red envía hacia la interfaz.
#[derive(Debug)]
pub enum StreamEvent {
    /// Fragmento de razonamiento del modelo ("thinking").
    Reasoning(String),
    /// Fragmento de texto de la respuesta final.
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

/// Convierte los mensajes de la app al formato de la API. Los mensajes de
/// usuario con imágenes adjuntas usan `content` como array de partes
/// (`text` + `image_url` con data URI base64), el estándar multimodal OpenAI.
fn build_api_messages(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            let has_images = m.attachments.iter().any(|a| a.is_sendable_image());
            if has_images {
                let mut parts = Vec::new();
                parts.push(serde_json::json!({ "type": "text", "text": m.content }));
                for a in &m.attachments {
                    if let Some(data_uri) = &a.image_data {
                        parts.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": { "url": data_uri }
                        }));
                    }
                }
                serde_json::json!({ "role": role, "content": parts })
            } else {
                serde_json::json!({ "role": role, "content": m.content })
            }
        })
        .collect()
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

    let api_messages = build_api_messages(messages);

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
    let mut sse = SseAccumulator::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(bytes) => {
                for event in sse.push(&bytes) {
                    if let Some(delta) = parse_sse_chunk(&event) {
                        if forward_delta(delta, &tx) {
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
    // Si el servidor omitió la línea en blanco final, procesamos lo sobrante.
    for event in sse.finish() {
        if let Some(delta) = parse_sse_chunk(&event) {
            if forward_delta(delta, &tx) {
                return;
            }
        }
    }
    let _ = tx.send(StreamEvent::Done);
}

/// Reenvía un delta hacia la interfaz. Devuelve `true` si llegó `[DONE]`.
fn forward_delta(
    delta: StreamDelta,
    tx: &tokio::sync::mpsc::UnboundedSender<StreamEvent>,
) -> bool {
    match delta {
        StreamDelta::Reasoning(r) => {
            let _ = tx.send(StreamEvent::Reasoning(r));
        }
        StreamDelta::Content(c) => {
            let _ = tx.send(StreamEvent::Chunk(c));
        }
        StreamDelta::Done => {
            let _ = tx.send(StreamEvent::Done);
            return true;
        }
    }
    false
}

/// Acumulador de eventos SSE.
///
/// Los eventos SSE se separan con una línea en blanco (`\n\n` o `\r\n\r\n`),
/// pero un mismo evento puede llegar troceado en varios paquetes de red. Este
/// acumulador espera a tener el evento completo antes de devolverlo, evitando
/// que se pierdan caracteres (o se rompa el UTF-8) al cortar por la mitad.
struct SseAccumulator {
    buf: Vec<u8>,
}

impl SseAccumulator {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Añade un trozo de red y devuelve los eventos `data: ...` ya completos.
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = Self::event_end(&self.buf) {
            let event = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            self.buf.drain(..end);
            events.push(event);
        }
        events
    }

    /// Devuelve los bytes restantes sin separador final (normalmente ninguno).
    fn finish(&mut self) -> Vec<String> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let rest = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        vec![rest]
    }

    /// Posición justo después del separador de evento (`\n\n` o `\r\n\r\n`).
    fn event_end(buf: &[u8]) -> Option<usize> {
        for (i, &b) in buf.iter().enumerate() {
            if b == b'\n' {
                let mut j = i + 1;
                if j < buf.len() && buf[j] == b'\r' {
                    j += 1;
                }
                if j < buf.len() && buf[j] == b'\n' {
                    return Some(j + 1);
                }
            }
        }
        None
    }
}

/// Construye la URL base limpia (sin barra final) a partir de la URL que
/// introduce el usuario, soportando tanto la raíz como un endpoint de chat.
fn base_url_for(base: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.trim_end_matches("/chat/completions").to_string()
    } else {
        base.to_string()
    }
}

/// Llama al endpoint `/models` (OpenAI-compatible) y devuelve la lista de ids.
/// La API key se envía como bearer auth; si está vacía, se omite (útil para
/// servidores locales como Ollama).
pub async fn fetch_models(base_url: &str, api_key: &str) -> Result<Vec<String>, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("No se pudo crear el cliente HTTP: {e}"))?;

    let url = format!("{}/models", base_url_for(base_url));
    let mut request = client.get(&url);
    let key = api_key.trim();
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Error de conexión: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {text}"));
    }

    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Respuesta JSON inválida: {e}"))?;

    let models: Vec<String> = value["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if models.is_empty() {
        Err("No se encontraron modelos en la respuesta.".to_string())
    } else {
        Ok(models)
    }
}

enum StreamDelta {
    /// Fragmento de razonamiento (`reasoning_content`) del modelo.
    Reasoning(String),
    /// Fragmento de la respuesta final.
    Content(String),
    Done,
}

/// Analiza un evento SSE completo: recoge todas las líneas `data:` del evento
/// (pueden ser varias) y las une antes de interpretar el JSON.
fn parse_sse_chunk(chunk: &str) -> Option<StreamDelta> {
    let mut data_lines: Vec<String> = Vec::new();
    for line in chunk.lines() {
        let line = line.trim();
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if !data.is_empty() {
                data_lines.push(data.to_string());
            }
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    let data = data_lines.join("\n");
    if data == "[DONE]" {
        return Some(StreamDelta::Done);
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data) {
        let delta = &value["choices"][0]["delta"];
        // Razonamiento ("thinking") primero: algunos modelos emiten solo esto.
        if let Some(r) = delta["reasoning_content"].as_str() {
            if !r.is_empty() {
                return Some(StreamDelta::Reasoning(r.to_string()));
            }
        }
        if let Some(c) = delta["content"].as_str() {
            if !c.is_empty() {
                return Some(StreamDelta::Content(c.to_string()));
            }
        }
        // Fallback a la forma no-streaming del mensaje.
        let message = &value["choices"][0]["message"];
        if let Some(r) = message["reasoning_content"].as_str() {
            if !r.is_empty() {
                return Some(StreamDelta::Reasoning(r.to_string()));
            }
        }
        if let Some(c) = message["content"].as_str() {
            if !c.is_empty() {
                return Some(StreamDelta::Content(c.to_string()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_delta_content() {
        let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"Hola\"}}]}\n\n";
        assert!(
            matches!(parse_sse_chunk(chunk), Some(StreamDelta::Content(s)) if s == "Hola")
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
    fn reassembles_events_split_across_network_chunks() {
        let mut acc = SseAccumulator::new();
        // El JSON del evento llega partido por la mitad, como ocurre en TCP.
        assert!(
            acc.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"Ho")
                .is_empty()
        );
        let events = acc.push(b"la\"}}]}\n\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            parse_sse_chunk(&events[0]),
            Some(StreamDelta::Content(s)) if s == "Hola"
        ));
    }

    #[test]
    fn handles_crlf_and_multiple_events_per_chunk() {
        let mut acc = SseAccumulator::new();
        let events =
            acc.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\r\n\r\ndata: [DONE]\r\n\r\n");
        assert_eq!(events.len(), 2);
        assert!(matches!(
            parse_sse_chunk(&events[0]),
            Some(StreamDelta::Content(s)) if s == "A"
        ));
        assert!(matches!(parse_sse_chunk(&events[1]), Some(StreamDelta::Done)));
    }

    #[test]
    fn finish_returns_remaining_event_without_blank_line() {
        let mut acc = SseAccumulator::new();
        assert!(acc.push(b"data: [DONE]").is_empty());
        let rest = acc.finish();
        assert_eq!(rest.len(), 1);
        assert!(matches!(parse_sse_chunk(&rest[0]), Some(StreamDelta::Done)));
    }

    #[test]
    fn preserves_utf8_split_across_chunks() {
        let mut acc = SseAccumulator::new();
        // "é" son 2 bytes (0xC3 0xA9): si se parte, no debe salir "�".
        acc.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"caf\xC3");
        let events = acc.push(b"\xA9\"}}]}\n\n");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            parse_sse_chunk(&events[0]),
            Some(StreamDelta::Content(s)) if s == "café"
        ));
    }

    #[test]
    fn builds_models_base_url() {
        assert_eq!(base_url_for("https://api.openai.com/v1"), "https://api.openai.com/v1");
        assert_eq!(
            base_url_for("http://localhost:11434/v1/"),
            "http://localhost:11434/v1"
        );
        assert_eq!(
            base_url_for("http://localhost:8000/v1/chat/completions"),
            "http://localhost:8000/v1"
        );
    }

    #[test]
    fn handles_reasoning_content() {
        let chunk = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"pensando\"}}]}\n\n";
        assert!(
            matches!(parse_sse_chunk(chunk), Some(StreamDelta::Reasoning(s)) if s == "pensando")
        );
    }

    #[test]
    fn distinguishes_reasoning_from_content() {
        let mut acc = SseAccumulator::new();
        let events = acc.push(
            b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"pienso\"}}]}\n\n\
              data: {\"choices\":[{\"delta\":{\"content\":\"respondo\"}}]}\n\n",
        );
        assert_eq!(events.len(), 2);
        assert!(matches!(
            parse_sse_chunk(&events[0]),
            Some(StreamDelta::Reasoning(s)) if s == "pienso"
        ));
        assert!(matches!(
            parse_sse_chunk(&events[1]),
            Some(StreamDelta::Content(s)) if s == "respondo"
        ));
    }

    #[test]
    fn parses_non_streaming_message_reasoning() {
        // Forma no-streaming: `choices[0].message.reasoning_content`.
        let chunk = "data: {\"choices\":[{\"message\":{\"reasoning_content\":\"reflexion\",\"content\":\"respuesta\"}}]}\n\n";
        assert!(matches!(
            parse_sse_chunk(chunk),
            Some(StreamDelta::Reasoning(s)) if s == "reflexion"
        ));
    }

    #[test]
    fn builds_multimodal_content_for_images() {
        use crate::types::{Attachment, AttachmentKind};
        let mut msg = Message::user("¿Qué hay en esta foto?");
        msg.attachments = vec![Attachment {
            kind: AttachmentKind::Image,
            name: "foto.png".into(),
            summary: "Imagen: foto.png".into(),
            image_file: None,
            image_data: Some("data:image/png;base64,AAAA".into()),
        }];
        let built = build_api_messages(&[msg]);
        assert_eq!(built[0]["role"], "user");
        let content = &built[0]["content"];
        assert!(content.is_array());
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "¿Qué hay en esta foto?");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,AAAA");
    }

    #[test]
    fn keeps_plain_content_without_images() {
        let msg = Message::user("hola");
        let built = build_api_messages(&[msg]);
        assert_eq!(built[0]["content"], "hola");
    }
}

