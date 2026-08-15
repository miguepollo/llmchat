use serde::{Deserialize, Serialize};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Tipo de archivo adjunto en un mensaje.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    Pdf,
    Epub,
    Text,
    Other,
}

impl AttachmentKind {
    /// Etiqueta corta legible para la interfaz.
    pub fn label(self) -> &'static str {
        match self {
            AttachmentKind::Image => "Imagen",
            AttachmentKind::Pdf => "PDF",
            AttachmentKind::Epub => "EPUB",
            AttachmentKind::Text => "Texto",
            AttachmentKind::Other => "Archivo",
        }
    }
}

/// Un archivo adjuntado a un mensaje. El texto extraído (PDF/EPUB/TXT) se
/// guarda dentro de `Message::content`; aquí solo queda la referencia ligera.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub kind: AttachmentKind,
    pub name: String,
    /// Línea corta que se muestra al lado del adjunto.
    pub summary: String,
    /// Ruta del archivo si es una imagen (para mostrar la vista previa).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_file: Option<String>,
    /// Data URI base64 de la imagen: `data:image/png;base64,...`.
    /// Se usa para enviar la imagen al modelo (multimodal) y para mostrarla.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_data: Option<String>,
}

impl Attachment {
    /// `true` si el adjunto es una imagen lista para enviar al modelo.
    pub fn is_sendable_image(&self) -> bool {
        self.kind == AttachmentKind::Image && self.image_data.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Texto de razonamiento ("thinking") previo a la respuesta final.
    /// Solo lo usan los modelos que soportan `reasoning_content`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reasoning: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), reasoning: String::new(), attachments: Vec::new() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), reasoning: String::new(), attachments: Vec::new() }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), reasoning: String::new(), attachments: Vec::new() }
    }

    pub fn user_with_attachments(content: impl Into<String>, attachments: Vec<Attachment>) -> Self {
        Self { role: Role::User, content: content.into(), reasoning: String::new(), attachments }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub messages: Vec<Message>,
}

impl Conversation {
    pub fn new() -> Self {
        let now = now_millis();
        Self {
            id: format!("{now:x}-{:x}", rand_ish()),
            title: "Nueva conversación".to_string(),
            created: now,
            updated: now,
            messages: Vec::new(),
        }
    }

    pub fn touch(&mut self) {
        self.updated = now_millis();
    }
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn rand_ish() -> u64 {
    let mut hasher = RandomState::new().build_hasher();
    std::process::id().hash(&mut hasher);
    hasher.finish()
}
