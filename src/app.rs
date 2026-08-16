use crate::api::{self, StreamEvent};
use crate::config::{self, Settings};
use crate::files;
use crate::markdown::{self, scaled_size as fs};
use crate::types::{Attachment, Conversation, Message, Role};
use crate::update::{self, ReleaseInfo};
use eframe::egui;
use eframe::egui::{Align2, Color32, CornerRadius, FontId, Margin, Stroke, StrokeKind};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Paleta de colores del tema oscuro personalizado
// ---------------------------------------------------------------------------
const BG: Color32 = Color32::from_rgb(17, 17, 23);
const SIDEBAR_BG: Color32 = Color32::from_rgb(21, 21, 29);
const CARD_BG: Color32 = Color32::from_rgb(27, 27, 37);
const CARD_HOVER: Color32 = Color32::from_rgb(36, 36, 49);
const CARD_SELECTED: Color32 = Color32::from_rgb(42, 42, 58);
const INPUT_BG: Color32 = Color32::from_rgb(24, 24, 33);
const BUBBLE_USER: Color32 = Color32::from_rgb(109, 82, 211);
const BUBBLE_ASSISTANT: Color32 = Color32::from_rgb(29, 29, 39);
const BUBBLE_BORDER: Color32 = Color32::from_rgb(46, 46, 60);
const ACCENT: Color32 = Color32::from_rgb(109, 82, 211);
const ACCENT_HOVER: Color32 = Color32::from_rgb(128, 101, 228);
const TEXT_MAIN: Color32 = Color32::from_rgb(236, 238, 246);
const TEXT_DIM: Color32 = Color32::from_rgb(143, 146, 163);
const TEXT_FAINT: Color32 = Color32::from_rgb(102, 105, 121);

/// Resultado de cargar los modelos disponibles desde el servidor.
enum ModelEvent {
    Loaded(Vec<String>),
    Error(String),
}

/// Resultado de importar un archivo (procesado en segundo plano).
enum ImportEvent {
    Done { attachments: Vec<Attachment>, extracted: Option<String> },
    Error(String),
}

/// Acción producida desde la barra lateral (tarjeta de conversación).
enum SidebarAction {
    Select(String),
    Delete(String),
    /// El usuario pulsó el botón de renombrar de la tarjeta.
    Rename(String),
    /// El usuario confirmó el nuevo título (Enter o clic fuera).
    CommitRename(String, String),
}

/// Resultado de las tareas de autoactualización (comprobación / instalación).
enum UpdateEvent {
    /// Comprobación terminada: `Ok(Some)` si hay actualización, `Ok(None)` si no.
    Checked(Result<Option<ReleaseInfo>, String>),
    /// La descarga + instalación terminó (o falló). En caso de éxito la app
    /// se cierra sola para que el reemplazo del binario pueda completarse.
    Install(Result<(), String>),
}

/// Estado de la autoactualización, tal y como se muestra en Ajustes.
enum UpdateStatus {
    /// No se ha comprobado aún.
    Idle,
    /// Comprobando / descargando en segundo plano.
    Checking,
    /// No hay versiones más nuevas.
    UpToDate,
    /// Hay una versión más nueva lista para instalar.
    Available(ReleaseInfo),
    /// Descargando e instalando la actualización.
    Downloading,
    /// Fallo al comprobar o instalar.
    Error(String),
}

pub struct App {
    settings: Settings,
    conversations: Vec<Conversation>,
    current_id: Option<String>,
    draft: String,
    streaming: bool,
    stream_conv_id: Option<String>,
    rx: Option<mpsc::UnboundedReceiver<StreamEvent>>,
    models_rx: Option<mpsc::UnboundedReceiver<ModelEvent>>,
    fetching_models: bool,
    model_query: String,
    import_rx: Option<mpsc::UnboundedReceiver<ImportEvent>>,
    importing_files: bool,
    images: HashMap<String, egui::TextureHandle>,
    join: Option<tokio::task::JoinHandle<()>>,
    show_settings: bool,
    status: Option<String>,
    input_focus: bool,
    needs_save: bool,
    /// Conversación cuyo título se está renombrando (edición en línea en la barra lateral).
    renaming_id: Option<String>,
    /// Borrador del título mientras se renombra.
    rename_draft: String,
    /// Pide el foco y la selección del campo de renombrado en el siguiente frame.
    rename_focus_pending: bool,
    runtime: tokio::runtime::Handle,
    /// Estilo base (sin escala) usado como plantilla para aplicar `font_scale`.
    base_style: egui::Style,
    /// Último `font_scale` aplicado al estilo (evita re-aplicarlo cada frame).
    applied_font_scale: f32,
    /// Estado de la autoactualización para la UI de Ajustes.
    update_status: UpdateStatus,
    /// Canal con los resultados de las tareas de actualización.
    update_rx: Option<mpsc::UnboundedReceiver<UpdateEvent>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, runtime: tokio::runtime::Handle) -> Self {
        // ---------- Tema oscuro personalizado ----------
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = BG;
        visuals.window_fill = Color32::from_rgb(24, 24, 32);
        visuals.extreme_bg_color = Color32::from_rgb(30, 30, 40);
        visuals.faint_bg_color = Color32::from_rgb(23, 23, 31);
        visuals.code_bg_color = Color32::from_rgb(14, 14, 20);
        visuals.override_text_color = Some(TEXT_MAIN);
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke = Stroke::new(1.0_f32, ACCENT_HOVER);
        visuals.hyperlink_color = Color32::from_rgb(146, 126, 255);
        visuals.window_corner_radius = CornerRadius::same(10);
        visuals.widgets.inactive.corner_radius = CornerRadius::same(6);
        visuals.widgets.hovered.corner_radius = CornerRadius::same(6);
        visuals.widgets.active.corner_radius = CornerRadius::same(6);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 34, 45);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(30, 30, 40);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(198, 201, 216));
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(45, 45, 58);
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(41, 41, 53);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.active.bg_fill = Color32::from_rgb(54, 54, 68);
        visuals.widgets.active.weak_bg_fill = Color32::from_rgb(50, 50, 63);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_DIM);
        cc.egui_ctx.set_visuals(visuals);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        // El estilo base (sin escala) se guarda como plantilla para poder
        // reconstruir los tamaños de fuente al cambiar `font_scale`.
        cc.egui_ctx.set_style(style.clone());
        let base_style = style;

        let mut conversations = config::load_conversations();
        if conversations.is_empty() {
            conversations.push(Conversation::new());
        }
        let current_id = conversations.first().map(|c| c.id.clone());
        let mut app = Self {
            settings: config::load_settings(),
            conversations,
            current_id,
            draft: String::new(),
            streaming: false,
            stream_conv_id: None,
            rx: None,
            models_rx: None,
            fetching_models: false,
            model_query: String::new(),
            import_rx: None,
            importing_files: false,
            images: HashMap::new(),
            join: None,
            show_settings: false,
            status: None,
            input_focus: true,
            needs_save: false,
            renaming_id: None,
            rename_draft: String::new(),
            rename_focus_pending: false,
            runtime,
            base_style,
            applied_font_scale: 1.0,
            update_status: UpdateStatus::Idle,
            update_rx: None,
        };

        // Comprueba actualizaciones en segundo plano al arrancar.
        app.trigger_update_check();

        app
    }

    fn conversation(&self, id: &str) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.id == id)
    }

    fn conversation_mut(&mut self, id: &str) -> Option<&mut Conversation> {
        self.conversations.iter_mut().find(|c| c.id == id)
    }

    fn new_conversation(&mut self) {
        self.cancel_rename();
        let conv = Conversation::new();
        let id = conv.id.clone();
        self.conversations.push(conv);
        self.current_id = Some(id);
        self.draft.clear();
        self.input_focus = true;
        self.needs_save = true;
    }

    fn delete_conversation(&mut self, id: &str) {
        self.conversations.retain(|c| c.id != id);
        if self.renaming_id.as_deref() == Some(id) {
            self.cancel_rename();
        }
        if self.current_id.as_deref() == Some(id) {
            self.current_id = self.conversations.first().map(|c| c.id.clone());
        }
        if self.conversations.is_empty() {
            self.conversations.push(Conversation::new());
            self.current_id = self.conversations.first().map(|c| c.id.clone());
        }
        self.needs_save = true;
    }

    /// Inicia el renombrado en línea de una conversación (botón ✎ de la tarjeta
    /// o de la cabecera). Rellena el borrador con el título actual.
    fn start_rename(&mut self, id: String) {
        self.renaming_id = Some(id.clone());
        self.rename_draft = self
            .conversation(&id)
            .map(|c| c.title.clone())
            .unwrap_or_default();
        self.rename_focus_pending = true;
        self.current_id = Some(id);
    }

    /// Confirma el nuevo título y persiste los cambios.
    fn commit_rename(&mut self, id: &str, title: &str) {
        if let Some(conv) = self.conversation_mut(id) {
            let t = title.trim();
            conv.title = if t.is_empty() {
                "Nueva conversación".to_string()
            } else {
                t.to_string()
            };
            conv.touch();
        }
        self.cancel_rename();
        self.needs_save = true;
    }

    /// Cierra el renombrado en curso sin guardar cambios.
    fn cancel_rename(&mut self) {
        self.renaming_id = None;
        self.rename_draft.clear();
        self.rename_focus_pending = false;
    }

    /// Lanza una tarea en segundo plano que consulta `/models` y rellena la lista.
    fn fetch_models(&mut self) {
        if self.fetching_models {
            return;
        }
        let base_url = self.settings.base_url.clone();
        let api_key = self.settings.api_key.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        self.models_rx = Some(rx);
        self.fetching_models = true;
        self.status = Some("Cargando modelos...".to_string());

        self.runtime.spawn(async move {
            let result = api::fetch_models(&base_url, &api_key).await;
            let _ = tx.send(match result {
                Ok(models) => ModelEvent::Loaded(models),
                Err(e) => ModelEvent::Error(e),
            });
        });
    }

    /// Recoge el resultado pendiente de la consulta de modelos.
    fn poll_models(&mut self) {
        let Some(rx) = &mut self.models_rx else {
            return;
        };
        match rx.try_recv() {
            Ok(ModelEvent::Loaded(models)) => {
                let count = models.len();
                self.settings.models = models;
                if let Some(first) = self.settings.models.first().cloned() {
                    self.settings.model = first;
                }
                self.needs_save = true;
                self.status = Some(format!("{count} modelos cargados."));
            }
            Ok(ModelEvent::Error(e)) => {
                self.status = Some(e);
            }
            Err(_) => return, // todavía no hay respuesta
        }
        self.models_rx = None;
        self.fetching_models = false;
    }

    /// Abre el selector de archivos y lanza la importación en segundo plano.
    fn pick_files_to_import(&mut self) {
        if self.importing_files {
            self.status = Some("Ya se está importando un archivo...".to_string());
            return;
        }
        let mut dialog = rfd::FileDialog::new().set_title("Adjuntar archivos");
        dialog = dialog.add_filter(
            "Archivos admitidos",
            &[
                "png", "jpg", "jpeg", "gif", "webp", "bmp", "tiff", "tif", "pdf", "epub",
                "txt", "md", "markdown", "json", "csv", "log", "toml", "yaml", "yml",
            ],
        );
        let Some(files) = dialog.pick_files() else {
            return;
        };
        let files: Vec<std::path::PathBuf> = files;
        if files.is_empty() {
            return;
        }

        let (tx, rx) = mpsc::unbounded_channel();
        self.import_rx = Some(rx);
        self.importing_files = true;
        self.status = Some("Procesando archivos...".to_string());

        let handle = self.runtime.clone();
        handle.spawn(async move {
            for path in files {
                match files::import_file(&path) {
                    Ok(imported) => {
                        let _ = tx.send(ImportEvent::Done {
                            attachments: imported.attachments,
                            extracted: imported.extracted_text,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(ImportEvent::Error(e));
                    }
                }
            }
        });
    }

    /// Añade unos adjuntos como mensaje del usuario en la conversación actual.
    fn add_user_attachment(&mut self, attachments: Vec<Attachment>, extracted: Option<String>) {
        let conv_id = match self.current_id.clone() {
            Some(id) => id,
            None => {
                self.new_conversation();
                self.current_id.clone().unwrap()
            }
        };
        let summary = attachments
            .first()
            .map(|a| a.summary.clone())
            .unwrap_or_else(|| "Archivo adjuntado".to_string());
        let mut content = summary.clone();
        if let Some(text) = extracted {
            if !text.trim().is_empty() {
                content.push_str(&format!("\n\n<contenido extraído>\n{text}"));
            }
        }
        let msg = Message::user_with_attachments(content, attachments.clone());

        if let Some(conv) = self.conversation_mut(&conv_id) {
            conv.touch();
            conv.messages.push(msg);
        }
        self.needs_save = true;
        self.status = Some(format!("Archivo adjuntado: {summary}"));
    }

    /// Carga en una textura de GPU una imagen a partir de su data URI (si no está ya).
    fn ensure_image_loaded(&mut self, ctx: &egui::Context, data_uri: &str) {
        if self.images.contains_key(data_uri) {
            return;
        }
        if let Some(color) = decode_data_uri(data_uri) {
            let tex = ctx.load_texture(data_uri.to_string(), color, egui::TextureOptions::LINEAR);
            self.images.insert(data_uri.to_string(), tex);
        }
    }

    /// Recoge los resultados de la importación y actualiza la UI.
    fn poll_imports(&mut self, ctx: &egui::Context) {
        let Some(mut rx) = self.import_rx.take() else {
            return;
        };
        let mut disconnected = false;
        let mut events: Vec<ImportEvent> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(ev) => events.push(ev),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if !disconnected {
            self.import_rx = Some(rx);
        } else {
            self.importing_files = false;
        }
        for ev in events {
            match ev {
                ImportEvent::Done { attachments, extracted } => {
                    for attach in &attachments {
                        if let Some(data_uri) = &attach.image_data {
                            self.ensure_image_loaded(ctx, data_uri);
                        }
                    }
                    self.add_user_attachment(attachments, extracted);
                }
                ImportEvent::Error(e) => {
                    self.status = Some(e);
                }
            }
        }
    }

    fn send(&mut self) {
        if self.streaming {
            return;
        }
        let draft = self.draft.trim().to_string();
        if draft.is_empty() {
            return;
        }
        if self.settings.model.trim().is_empty() {
            self.status = Some("Indica un modelo en Ajustes antes de enviar.".to_string());
            return;
        }
        if self.current_id.is_none() || self.conversations.is_empty() {
            self.new_conversation();
        }
        let conv_id = match self.current_id.clone() {
            Some(id) => id,
            None => return,
        };

        let system_prompt = self.settings.system_prompt.clone();

        // Registra el mensaje del usuario y prepara el payload a enviar.
        let messages_to_send = {
            let conv = match self.conversation_mut(&conv_id) {
                Some(c) => c,
                None => return,
            };
            let user_msg_count = conv.messages.iter().filter(|m| m.role == Role::User).count();
            if user_msg_count == 0 {
                let mut title: String = draft.chars().take(48).collect();
                if draft.chars().count() > 48 {
                    title.push_str("...");
                }
                conv.title = title;
            }
            conv.touch();
            conv.messages.push(Message::user(draft.clone()));
            let mut payload: Vec<Message> = Vec::new();
            if !system_prompt.trim().is_empty() {
                payload.push(Message::system(system_prompt.clone()));
            }
            payload.extend(conv.messages.iter().cloned());
            payload
        };

        // Marcador de posición para la respuesta del asistente.
        if let Some(conv) = self.conversation_mut(&conv_id) {
            conv.messages.push(Message::assistant(String::new()));
        }

        let settings = self.settings.clone();
        let (tx, rx) = mpsc::unbounded_channel();
        self.rx = Some(rx);
        self.stream_conv_id = Some(conv_id);
        self.streaming = true;
        self.draft.clear();
        self.input_focus = true;
        self.needs_save = true;

        self.join = Some(self.runtime.spawn(async move {
            api::stream_chat(&settings, &messages_to_send, tx).await;
        }));
    }

    fn stop(&mut self) {
        if let Some(join) = self.join.take() {
            join.abort();
        }
        self.streaming = false;
        self.rx = None;
        self.stream_conv_id = None;
        self.needs_save = true;
    }

    fn poll_stream(&mut self) {
        let Some(mut rx) = self.rx.take() else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(StreamEvent::Reasoning(text)) => {
                    if let Some(conv) = self
                        .stream_conv_id
                        .clone()
                        .and_then(|id| self.conversation_mut(&id))
                    {
                        if let Some(last) = conv.messages.last_mut() {
                            last.reasoning.push_str(&text);
                        }
                    }
                }
                Ok(StreamEvent::Chunk(text)) => {
                    if let Some(conv) = self
                        .stream_conv_id
                        .clone()
                        .and_then(|id| self.conversation_mut(&id))
                    {
                        if let Some(last) = conv.messages.last_mut() {
                            last.content.push_str(&text);
                        }
                    }
                }
                Ok(StreamEvent::Done) => {
                    self.streaming = false;
                    self.rx = None;
                    self.join = None;
                    self.stream_conv_id = None;
                    self.needs_save = true;
                    break;
                }
                Ok(StreamEvent::Error(message)) => {
                    self.status = Some(message);
                    self.streaming = false;
                    self.rx = None;
                    self.join = None;
                    self.stream_conv_id = None;
                    self.needs_save = true;
                    break;
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    self.rx = Some(rx);
                    break;
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    self.streaming = false;
                    self.rx = None;
                    self.join = None;
                    self.stream_conv_id = None;
                    self.needs_save = true;
                    break;
                }
            }
        }
    }

    fn save_if_needed(&mut self) {
        if self.needs_save && !self.streaming {
            if let Err(e) = config::save_settings(&self.settings) {
                self.status = Some(format!("No se pudo guardar la configuración: {e}"));
            }
            if let Err(e) = config::save_conversations(&self.conversations) {
                self.status = Some(format!("No se pudieron guardar las conversaciones: {e}"));
            }
            self.needs_save = false;
        }
    }

    /// Lanza la comprobación de actualizaciones en segundo plano.
    fn trigger_update_check(&mut self) {
        if matches!(
            self.update_status,
            UpdateStatus::Checking | UpdateStatus::Downloading
        ) {
            return;
        }
        self.update_status = UpdateStatus::Checking;
        let (tx, rx) = mpsc::unbounded_channel();
        self.update_rx = Some(rx);
        let handle = self.runtime.clone();
        handle.spawn(async move {
            let _ = tx.send(UpdateEvent::Checked(update::check_latest_release().await));
        });
    }

    /// Descarga e instala la actualización disponible en segundo plano.
    fn start_update_install(&mut self) {
        let UpdateStatus::Available(info) = &self.update_status else {
            return;
        };
        let info = info.clone();
        self.update_status = UpdateStatus::Downloading;
        let (tx, rx) = mpsc::unbounded_channel();
        self.update_rx = Some(rx);
        let handle = self.runtime.clone();
        handle.spawn(async move {
            let result = match update::download_asset(&info.download_url, &info.asset_name).await {
                Ok(path) => update::install_and_restart(&path),
                Err(e) => Err(e),
            };
            let _ = tx.send(UpdateEvent::Install(result));
        });
    }

    /// Recoge los resultados de las tareas de actualización.
    fn poll_update(&mut self, ctx: &egui::Context) {
        let Some(mut rx) = self.update_rx.take() else {
            return;
        };
        let mut disconnected = false;
        let mut events: Vec<UpdateEvent> = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(ev) => events.push(ev),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if !disconnected {
            self.update_rx = Some(rx);
        }
        for ev in events {
            match ev {
                UpdateEvent::Checked(result) => match result {
                    Ok(Some(info)) => self.update_status = UpdateStatus::Available(info),
                    Ok(None) => self.update_status = UpdateStatus::UpToDate,
                    Err(e) => self.update_status = UpdateStatus::Error(e),
                },
                UpdateEvent::Install(result) => match result {
                    Ok(()) => {
                        // La instalación ya está lanzada: guarda datos pendientes
                        // y sale para que el reemplazo del binario pueda completarse.
                        self.save_if_needed();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        std::process::exit(0);
                    }
                    Err(e) => self.update_status = UpdateStatus::Error(e),
                },
            }
        }
        ctx.request_repaint();
    }

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_stream();
        self.poll_models();
        self.poll_imports(ctx);
        self.poll_update(ctx);

        // ---------- Tamaño de letra global ----------
        // Reescala los estilos de texto (Body, Small, Monospace, botones, etc.)
        // solo cuando cambia el factor elegido por el usuario.
        let scale = self.settings.font_scale;
        if self.applied_font_scale != scale {
            let mut style = self.base_style.clone();
            for font_id in style.text_styles.values_mut() {
                font_id.size *= scale;
            }
            ctx.set_style(style);
            self.applied_font_scale = scale;
        }

        // Escape cancela un renombrado en curso.
        if self.renaming_id.is_some()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.cancel_rename();
        }

        // ---------- Barra lateral: chats recientes ----------
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .exact_width(280.0)
            .frame(
                egui::Frame::new()
                    .fill(SIDEBAR_BG)
                    .inner_margin(Margin::symmetric(10, 12)),
            )
            .show(ctx, |ui| {
                // Marca / logo
                ui.horizontal(|ui| {
                    avatar_circle(ui, "L", ACCENT, scale);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.label(
                            egui::RichText::new("LLMchat")
                                .size(fs(16.5, scale))
                                .strong()
                                .color(TEXT_MAIN),
                        );
                        ui.label(
                            egui::RichText::new("tus conversaciones")
                                .size(fs(10.5, scale))
                                .color(TEXT_DIM),
                        );
                    });
                });
                ui.add_space(10.0);

                // Botón de nueva conversación
                let new_btn = egui::Button::new(
                    egui::RichText::new("＋  Nueva conversación")
                        .size(fs(13.0, scale))
                        .color(Color32::WHITE),
                )
                .fill(ACCENT)
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(8))
                .min_size(egui::vec2(ui.available_width(), 36.0));
                if ui
                    .add(new_btn)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.new_conversation();
                }

                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("RECIENTES")
                        .size(fs(10.0, scale))
                        .strong()
                        .color(TEXT_FAINT),
                );
                ui.add_space(4.0);

                // Lista de conversaciones recientes (ordenadas por actividad)
                let items: Vec<(String, String, String, String, bool)> = {
                    let mut list: Vec<&Conversation> = self.conversations.iter().collect();
                    list.sort_by(|a, b| b.updated.cmp(&a.updated));
                    list.into_iter()
                        .map(|c| {
                            let preview = c
                                .messages
                                .iter()
                                .rev()
                                .find(|m| {
                                    !m.content.trim().is_empty() || !m.reasoning.trim().is_empty()
                                })
                                .map(message_preview)
                                .unwrap_or_else(|| "Sin mensajes todavía".to_string());
                            let selected = self.current_id.as_deref() == Some(c.id.as_str());
                            (
                                c.id.clone(),
                                truncate(&c.title, 40),
                                preview,
                                relative_time(c.updated),
                                selected,
                            )
                        })
                        .collect()
                };
                let mut actions: Vec<SidebarAction> = Vec::new();
                let scroll_h = (ui.available_height() - 56.0).max(80.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(scroll_h)
                    .show(ui, |ui| {
                        for (id, title, preview, rel, selected) in &items {
                            let editing = self.renaming_id.as_deref() == Some(id.as_str());
                            if let Some(action) = sidebar_card(
                                ui,
                                id,
                                title,
                                preview,
                                rel,
                                *selected,
                                editing,
                                &mut self.rename_draft,
                                &mut self.rename_focus_pending,
                                scale,
                            ) {
                                actions.push(action);
                            }
                        }
                        if items.is_empty() {
                            ui.label(
                                egui::RichText::new("Aún no hay conversaciones.")
                                    .size(fs(11.5, scale))
                                    .color(TEXT_DIM),
                            );
                        }
                    });
                for action in actions {
                    match action {
                        SidebarAction::Select(id) => {
                            self.current_id = Some(id);
                            self.input_focus = true;
                        }
                        SidebarAction::Delete(id) => self.delete_conversation(&id),
                        SidebarAction::Rename(id) => self.start_rename(id),
                        SidebarAction::CommitRename(id, title) => self.commit_rename(&id, &title),
                    }
                }

                ui.add_space(6.0);
                ui.separator();
                let settings_btn = egui::Button::new(
                    egui::RichText::new("⚙  Ajustes")
                        .size(fs(13.0, scale))
                        .color(TEXT_MAIN),
                )
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::NONE)
                .corner_radius(CornerRadius::same(8))
                .min_size(egui::vec2(ui.available_width(), 34.0));
                if ui
                    .add(settings_btn)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    self.show_settings = true;
                }
            });

        // ---------- Cabecera del chat (sobre la zona central) ----------
        egui::TopBottomPanel::top("chat_header").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let title = self
                    .current_id
                    .clone()
                    .and_then(|id| self.conversation(&id))
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(&title)
                                    .size(fs(15.0, scale))
                                    .strong()
                                    .color(TEXT_MAIN),
                            );
                            if let Some(status) = self.status.clone() {
                                ui.label(
                                    egui::RichText::new(status)
                                        .size(fs(10.5, scale))
                                        .color(TEXT_DIM),
                                );
                            }
                        });
                        if !title.is_empty() {
                            let ren_btn = egui::Button::new(
                                egui::RichText::new("✎").size(fs(13.0, scale)).color(TEXT_DIM),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(6));
                            if ui
                                .add(ren_btn)
                                .on_hover_text("Renombrar conversación")
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                if let Some(id) = self.current_id.clone() {
                                    self.start_rename(id);
                                }
                            }
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut model = self.settings.model.clone();
                    egui::ComboBox::from_id_salt("modelo")
                        .selected_text(model.clone())
                        .width(240.0)
                        // No cerrar el popup al hacer clic dentro (p. ej. en el buscador).
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            ui.set_min_width(240.0);
                            // Campo de búsqueda para filtrar los modelos.
                            ui.add(
                                egui::TextEdit::singleline(&mut self.model_query)
                                    .hint_text("Buscar modelo...")
                                    .desired_width(f32::INFINITY),
                            );
                            ui.separator();
                            let query = self.model_query.trim().to_lowercase();
                            let total = self.settings.models.len();
                            let mut shown = 0u32;
                            let mut last_family: Option<&'static str> = None;
                            for m in &self.settings.models {
                                if query.is_empty() || m.to_lowercase().contains(&query) {
                                    shown += 1;
                                    let fam = model_family(m);
                                    // Muestra un encabezado de grupo cuando cambia
                                    // la familia (y sólo si el modelo es visible).
                                    if fam != last_family {
                                        last_family = fam;
                                        ui.add_space(6.0);
                                        ui.label(
                                            egui::RichText::new(fam.unwrap_or("Otros"))
                                                .size(fs(10.5, scale))
                                                .strong()
                                                .color(TEXT_DIM),
                                        );
                                        ui.separator();
                                    }
                                    if ui
                                        .selectable_value(&mut model, m.clone(), m.as_str())
                                        .clicked()
                                    {
                                        // Cerrar el popup tras elegir un modelo
                                        egui::Popup::close_all(ui.ctx());
                                    }
                                }
                            }
                            ui.add_space(2.0);
                            if shown == 0 {
                                ui.colored_label(
                                    Color32::from_rgb(0xf0, 0x7a, 0x7a),
                                    if total == 0 {
                                        "Sin modelos. Pulsa Cargar modelos en Ajustes."
                                    } else {
                                        "Sin coincidencias."
                                    },
                                );
                            } else {
                                ui.label(format!("{shown}/{total} modelos"));
                            }
                        });
                    if model != self.settings.model {
                        self.settings.model = model;
                        self.model_query.clear();
                        self.needs_save = true;
                    }
                });
            });
            ui.add_space(8.0);
        });


        // ---------- Entrada de mensaje ----------
        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(INPUT_BG)
                .corner_radius(CornerRadius::same(14))
                .stroke(Stroke::new(1.0_f32, BUBBLE_BORDER))
                .inner_margin(Margin::symmetric(8, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new("📎")
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE)
                                    .corner_radius(CornerRadius::same(10))
                                    .min_size(egui::vec2(36.0, 42.0)),
                            )
                            .on_hover_text("Adjuntar archivos (imágenes, PDF, EPUB, texto...)")
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.pick_files_to_import();
                        }
                        let input = egui::TextEdit::multiline(&mut self.draft)
                            .hint_text(
                                "Escribe un mensaje... (Enter envía, Shift+Enter salto de línea)",
                            )
                            .desired_rows(2)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Body);
                        let response = ui.add(input);
                        if self.input_focus {
                            response.request_focus();
                            self.input_focus = false;
                        }
                        if response.has_focus() {
                            let enter = ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                            });
                            if enter {
                                self.send();
                            }
                        }
                        let label = if self.streaming { "Detener" } else { "Enviar" };
                        let send_btn = egui::Button::new(
                            egui::RichText::new(label).size(fs(13.0, scale)).color(Color32::WHITE),
                        )
                        .fill(if self.streaming {
                            Color32::from_rgb(198, 72, 72)
                        } else {
                            ACCENT
                        })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(10))
                        .min_size(egui::vec2(88.0, 42.0));
                        if ui
                            .add(send_btn)
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            if self.streaming {
                                self.stop();
                            } else {
                                self.send();
                            }
                        }
                    });
                });
            if self.streaming {
                ui.horizontal(|ui| {
                    ui.add_space(2.0);
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(
                        egui::RichText::new("Generando respuesta...")
                            .size(fs(11.0, scale))
                            .color(TEXT_DIM),
                    );
                });
            }
            ui.add_space(6.0);
        });

        // Pre-carga texturas de imágenes persistentes (p. ej. tras reiniciar).
        if let Some(id) = self.current_id.clone() {
            let to_load: Vec<String> = self
                .conversation(&id)
                .map(|c| {
                    c.messages
                        .iter()
                        .flat_map(|m| &m.attachments)
                        .filter_map(|a| a.image_data.clone())
                        .collect()
                })
                .unwrap_or_default();
            for data_uri in to_load {
                self.ensure_image_loaded(ctx, &data_uri);
            }
        }

        // ---------- Zona central: mensajes ----------
        egui::CentralPanel::default().show(ctx, |ui| {
            let conv = match self.current_id.clone().and_then(|id| {
                self.conversations.iter().find(|c| c.id == id).cloned()
            }) {
                Some(c) => c,
                None => {
                    ui.centered_and_justified(|ui| {
                        ui.label("Crea o selecciona una conversación para empezar.");
                    });
                    return;
                }
            };

            if conv.messages.is_empty() {
                // Pantalla de bienvenida
                ui.centered_and_justified(|ui| {
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("💬").size(fs(52.0, scale)));
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("¿En qué puedo ayudarte?")
                                .size(fs(26.0, scale))
                                .strong()
                                .color(TEXT_MAIN),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(
                                "Escribe un mensaje abajo o adjunta un archivo para empezar.",
                            )
                            .size(fs(13.0, scale))
                            .color(TEXT_DIM),
                        );
                    });
                });
                return;
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add_space(14.0);
                    let avail = ui.available_width();
                    let col = (avail - 16.0).min(900.0).max(200.0);
                    let left = ((avail - col) * 0.5).max(0.0);
                    ui.horizontal(|ui| {
                        ui.add_space(left);
                        ui.vertical(|ui| {
                            ui.set_width(col);
                            for (idx, message) in conv.messages.iter().enumerate() {
                                match message.role {
                                    Role::User => render_user_bubble(
                                        ui,
                                        &message.content,
                                        &message.attachments,
                                        &self.images,
                                        scale,
                                    ),
                                    Role::Assistant => {
                                        render_assistant_thinking(
                                            ui,
                                            &message.reasoning,
                                            idx,
                                            scale,
                                        );
                                        render_assistant_bubble(ui, &message.content, scale);
                                    }
                                    Role::System => {}
                                }
                            }
                            if self.streaming {
                                // Durante la fase de "thinking" no mostramos el
                                // "escribiendo...": ya se ve el razonamiento.
                                let thinking_phase = conv.messages.last().map_or(false, |m| {
                                    m.role == Role::Assistant
                                        && !m.reasoning.trim().is_empty()
                                        && m.content.trim().is_empty()
                                });
                                if !thinking_phase {
                                    render_typing_indicator(ui, scale);
                                }
                            }
                        });
                    });
                    ui.add_space(20.0);
                });
        });

        // ---------- Ajustes ----------
        if self.show_settings {
            egui::Window::new("Ajustes")
                .resizable(false)
                .default_width(480.0)
                .show(ctx, |ui| {
                    ui.add_space(4.0);
                    ui.label("URL base (OpenAI-compatible)");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.base_url)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(4.0);
                    ui.label("API key");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.api_key)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Modelo");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.fetching_models {
                                ui.add(egui::Spinner::new());
                            }
                            if ui.button("Cargar modelos").clicked() {
                                self.fetch_models();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.settings.model)
                                .desired_width(ui.available_width() - 90.0),
                        );
                        if ui.button("Añadir").clicked() {
                            let m = self.settings.model.trim().to_string();
                            if !m.is_empty() && !self.settings.models.contains(&m) {
                                self.settings.models.push(m);
                                self.needs_save = true;
                            }
                        }
                    });
                    if !self.settings.models.is_empty() {
                        ui.add_space(2.0);
                        let mut selected = self.settings.model.clone();
                        egui::ComboBox::from_id_salt("modelos_settings")
                            .selected_text(selected.clone())
                            .width(ui.available_width())
                            .show_ui(ui, |ui| {
                                for m in &self.settings.models {
                                    ui.selectable_value(&mut selected, m.clone(), m.as_str());
                                }
                            });
                        if selected != self.settings.model {
                            self.settings.model = selected;
                            self.needs_save = true;
                        }
                        ui.add_space(2.0);
                    }
                    ui.add_space(4.0);
                    ui.label("Temperatura");
                    ui.add(
                        egui::Slider::new(&mut self.settings.temperature, 0.0..=2.0).text("t"),
                    );
                    ui.add_space(4.0);
                    ui.label("Tamaño de letra");
                    let font_before = self.settings.font_scale;
                    ui.add(
                        egui::Slider::new(&mut self.settings.font_scale, 0.75..=1.5)
                            .text("×")
                            .fixed_decimals(2),
                    );
                    if self.settings.font_scale != font_before {
                        self.needs_save = true;
                    }
                    ui.add_space(4.0);
                    ui.label("Prompt de sistema (opcional)");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.settings.system_prompt)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label("Actualizaciones");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                egui::RichText::new(format!("v{}", update::current_version()))
                                    .color(TEXT_DIM),
                            );
                        });
                    });
                    match &self.update_status {
                        UpdateStatus::Idle | UpdateStatus::Checking => {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new());
                                ui.label("Buscando actualizaciones...");
                            });
                        }
                        UpdateStatus::UpToDate => {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Estás al día (v{}).",
                                    update::current_version()
                                ))
                                .color(TEXT_DIM),
                            );
                        }
                        UpdateStatus::Available(info) => {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Actualización disponible: {}",
                                    info.version
                                ))
                                .color(Color32::from_rgb(126, 231, 135)),
                            );
                        }
                        UpdateStatus::Downloading => {
                            ui.horizontal(|ui| {
                                ui.add(egui::Spinner::new());
                                ui.label("Descargando e instalando...");
                            });
                        }
                        UpdateStatus::Error(e) => {
                            ui.label(
                                egui::RichText::new(format!("Error: {e}"))
                                    .color(Color32::from_rgb(230, 100, 100)),
                            );
                        }
                    }
                    ui.horizontal(|ui| {
                        if matches!(&self.update_status, UpdateStatus::Available(_))
                            && ui.button("⬇  Descargar e instalar").clicked()
                        {
                            self.start_update_install();
                        }
                        let busy = matches!(
                            &self.update_status,
                            UpdateStatus::Checking | UpdateStatus::Downloading
                        );
                        if !busy && ui.button("🔍 Buscar actualizaciones").clicked() {
                            self.trigger_update_check();
                        }
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button("Guardar").clicked() {
                            self.needs_save = true;
                            self.show_settings = false;
                        }
                        if ui.button("Cancelar").clicked() {
                            self.show_settings = false;
                        }
                    });
                });
        }

        if self.streaming {
            ctx.request_repaint();
        }
        self.save_if_needed();
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("...");
    out
}

/// Decodifica un data URI base64 de imagen en un `egui::ColorImage` para vista previa.
fn decode_data_uri(data_uri: &str) -> Option<egui::ColorImage> {
    let comma = data_uri.find(',')?;
    let b64 = &data_uri[comma + 1..];
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
    let img = if img.width() > 800 || img.height() > 800 {
        img.thumbnail(800, 800)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
}

/// Dibuja un círculo con iniciales (avatar) en la UI.
fn avatar_circle(ui: &mut egui::Ui, glyph: &str, color: Color32, scale: f32) -> egui::Response {
    let size = 26.0;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), size * 0.5, color);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(fs(12.0, scale)),
        Color32::WHITE,
    );
    response
}

/// Resumen legible del último mensaje de una conversación.
fn message_preview(m: &Message) -> String {
    let prefix = match m.role {
        Role::User => "Tú: ",
        Role::Assistant => "✦ ",
        Role::System => "",
    };
    let text = if m.content.trim().is_empty() && !m.reasoning.trim().is_empty() {
        "razonando...".to_string()
    } else {
        m.content.clone()
    };
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat: String = flat.chars().take(64).collect();
    if flat.chars().count() >= 64 {
        format!("{prefix}{flat}…")
    } else {
        format!("{prefix}{flat}")
    }
}

/// Tiempo relativo corto ("hace 5 min", "hace 2 d"...) para el historial.
fn relative_time(ms: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let secs = now.saturating_sub(ms) / 1000;
    if secs < 60 {
        "ahora".to_string()
    } else if secs < 3600 {
        format!("hace {} min", secs / 60)
    } else if secs < 86_400 {
        format!("hace {} h", secs / 3600)
    } else if secs < 604_800 {
        format!("hace {} d", secs / 86_400)
    } else {
        format!("hace {} sem", secs / 604_800)
    }
}

/// Tarjeta de conversación de la barra lateral. Devuelve la acción si el
/// usuario hace clic en la tarjeta, en su botón de renombrar o en el de
/// eliminar. Cuando `editing` es `true` muestra un campo de texto para
/// renombrar el título en línea (Enter o clic fuera confirman).
#[allow(clippy::too_many_arguments)]
fn sidebar_card(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    preview: &str,
    rel: &str,
    selected: bool,
    editing: bool,
    draft: &mut String,
    focus_pending: &mut bool,
    scale: f32,
) -> Option<SidebarAction> {
    let h = if editing { 64.0 } else { 58.0 };
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), h),
        egui::Sense::click(),
    );
    let hovered = response.hovered();
    let bg = if selected {
        CARD_SELECTED
    } else if hovered {
        CARD_HOVER
    } else {
        CARD_BG
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), bg);
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(1.2_f32, ACCENT),
            StrokeKind::Inside,
        );
    }
    let inner = rect.shrink2(egui::vec2(10.0, 6.0));
    let mut action: Option<SidebarAction> = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
        if editing {
            // ---------- Renombrado en línea del título ----------
            let resp = ui.add(
                egui::TextEdit::singleline(draft)
                    .id(egui::Id::new(("rename_edit", id)))
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Small),
            );
            if *focus_pending {
                resp.request_focus();
                // Selecciona todo el título al empezar a renombrar (egui 0.32
                // no expone `select_all`, así que se ajusta el rango de cursor).
                let n = draft.chars().count();
                let range = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(n),
                );
                if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), resp.id) {
                    state.cursor.set_char_range(Some(range));
                    egui::TextEdit::store_state(ui.ctx(), resp.id, state);
                }
                *focus_pending = false;
            }
            // Enter o clic fuera confirma el cambio (Escape se maneja en update).
            if resp.lost_focus() {
                action = Some(SidebarAction::CommitRename(
                    id.to_string(),
                    draft.trim().to_string(),
                ));
            }
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(preview).size(fs(10.5, scale)).color(TEXT_DIM),
                    )
                    .truncate(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(rel).size(fs(9.5, scale)).color(TEXT_FAINT));
                });
            });
        } else {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(title)
                                .size(fs(12.5, scale))
                                .strong()
                                .color(if selected { Color32::WHITE } else { TEXT_MAIN }),
                        )
                        .truncate(),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(preview).size(fs(10.5, scale)).color(TEXT_DIM),
                        )
                        .truncate(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if hovered {
                        let del = egui::Button::new("✕")
                            .small()
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(5));
                        if ui
                            .add(del)
                            .on_hover_text("Eliminar conversación")
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            action = Some(SidebarAction::Delete(id.to_string()));
                        }
                        let ren = egui::Button::new("✎")
                            .small()
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(CornerRadius::same(5));
                        if ui
                            .add(ren)
                            .on_hover_text("Renombrar conversación")
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            action = Some(SidebarAction::Rename(id.to_string()));
                        }
                    }
                    ui.label(egui::RichText::new(rel).size(fs(9.5, scale)).color(TEXT_FAINT));
                });
            });
        }
    });
    if action.is_none() && response.clicked() {
        action = Some(SidebarAction::Select(id.to_string()));
    }
    ui.add_space(4.0);
    action
}

fn render_user_bubble(
    ui: &mut egui::Ui,
    text: &str,
    attachments: &[Attachment],
    images: &HashMap<String, egui::TextureHandle>,
    scale: f32,
) {
    let bubble_max = (ui.available_width() * 0.86).clamp(160.0, 720.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.vertical(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.label(
                    egui::RichText::new("Tú")
                        .size(fs(10.5, scale))
                        .strong()
                        .color(TEXT_DIM),
                );
            });
            ui.add_space(3.0);
            egui::Frame::new()
                .fill(BUBBLE_USER)
                .corner_radius(CornerRadius::same(14))
                .inner_margin(Margin::symmetric(14, 9))
                .show(ui, |ui| {
                    ui.set_max_width(bubble_max);
                    ui.label(egui::RichText::new(text).color(Color32::WHITE));
                    // Vista previa de imágenes adjuntas.
                    for attach in attachments {
                        if let Some(data_uri) = &attach.image_data {
                            if let Some(tex) = images.get(data_uri) {
                                ui.add_space(6.0);
                                egui::Frame::new()
                                    .corner_radius(CornerRadius::same(8))
                                    .show(ui, |ui| {
                                        let max = ui.available_width().min(320.0).max(40.0);
                                        ui.add(egui::Image::new(
                                            egui::load::SizedTexture::from_handle(tex),
                                        )
                                        .max_size(egui::vec2(max, 280.0)));
                                    });
                            }
                        }
                    }
                });
        });
    });
    ui.add_space(12.0);
}

/// Bloque colapsable con el razonamiento ("thinking") del asistente.
/// Solo aparece cuando el modelo emite `reasoning_content`.
fn render_assistant_thinking(ui: &mut egui::Ui, reasoning: &str, index: usize, scale: f32) {
    if reasoning.trim().is_empty() {
        return;
    }
    let bubble_max = (ui.available_width() * 0.86).clamp(160.0, 720.0);
    ui.horizontal_top(|ui| {
        avatar_circle(ui, "M", ACCENT, scale);
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(Color32::from_rgb(24, 24, 34))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(14, 10))
            .stroke(Stroke::new(1.0_f32, Color32::from_rgb(40, 40, 52)))
            .show(ui, |ui| {
                ui.set_max_width(bubble_max);
                egui::CollapsingHeader::new(
                    egui::RichText::new("🧠 Razonamiento")
                        .size(fs(12.0, scale))
                        .strong()
                        .color(Color32::from_rgb(146, 126, 255)),
                )
                .id_salt(("thinking", index))
                .default_open(false)
                .show(ui, |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(reasoning.trim())
                            .size(fs(12.5, scale))
                            .italics()
                            .color(TEXT_DIM),
                    );
                });
            });
    });
    ui.add_space(8.0);
}


fn render_assistant_bubble(ui: &mut egui::Ui, text: &str, scale: f32) {
    if text.trim().is_empty() {
        return;
    }
    let bubble_max = (ui.available_width() * 0.86).clamp(160.0, 720.0);
    ui.horizontal_top(|ui| {
        avatar_circle(ui, "M", ACCENT, scale);
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(BUBBLE_ASSISTANT)
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(14, 10))
            .stroke(Stroke::new(1.0_f32, BUBBLE_BORDER))
            .show(ui, |ui| {
                ui.set_max_width(bubble_max);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("✦ LLMchat")
                            .size(fs(10.5, scale))
                            .strong()
                            .color(Color32::from_rgb(146, 126, 255)),
                    );
                    if ui
                        .small_button("⧉")
                        .on_hover_text("Copiar")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        ui.ctx().copy_text(text.to_string());
                    }
                });
                ui.add_space(2.0);
                markdown::render(ui, text, scale);
            });
    });
    ui.add_space(12.0);
}

/// Indicador "escribiendo..." mientras llegan tokens del modelo.
fn render_typing_indicator(ui: &mut egui::Ui, scale: f32) {
    ui.horizontal_top(|ui| {
        avatar_circle(ui, "M", ACCENT, scale);
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(BUBBLE_ASSISTANT)
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::symmetric(14, 10))
            .stroke(Stroke::new(1.0_f32, BUBBLE_BORDER))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.label(
                        egui::RichText::new("escribiendo...")
                            .size(fs(12.0, scale))
                            .color(TEXT_DIM),
                    );
                });
            });
    });
    ui.add_space(12.0);
}

// ---------------------------------------------------------------------------
// Agrupación de modelos por proveedor/familia
// ---------------------------------------------------------------------------

/// Devuelve la familia a la que pertenece un id de modelo según su nombre.
/// Los proveedores agregan un prefijo `proveedor/modelo` (por ejemplo
/// `deepseek/deepseek-v4`, `openai/gpt-5`, `anthropic/claude-opus-5`,
/// `qwen/qwen3`, `~x-ai/grok-latest`, etc.); aquí detectamos las familias más
/// comunes para agruparlas en el selector. Devuelve `None` para modelos que no
/// encajan en ninguna familia reconocida (se mostrarán en un grupo "Otros").
fn model_family(id: &str) -> Option<&'static str> {
    let name = id.to_lowercase();

    // Pares (fragmento, etiqueta). Se comprueban de arriba a abajo y gana la
    // primera coincidencia: las familias con prefijos más específicos
    // (gpt-oss, gpt-image, dall-e) van antes que "gpt" y que "o3-…".
    let families: &[(&str, &str)] = &[
        ("claude", "Anthropic Claude"),
        ("deepseek", "DeepSeek"),
        ("grok", "xAI Grok"),
        ("gemini", "Google Gemini"),
        ("gemma", "Google Gemma"),
        ("gpt-oss", "OpenAI GPT-oss"),
        ("gpt-image", "OpenAI Imagen"),
        ("dall-e", "OpenAI DALL·E"),
        ("gpt", "ChatGPT / OpenAI"),
        ("openai/o", "ChatGPT / OpenAI"),
        ("qwen", "Alibaba Qwen"),
        ("kimi", "Moonshot Kimi"),
        ("glm", "Zhipu GLM"),
        ("mistral", "Mistral"),
        ("mixtral", "Mistral"),
        ("llama", "Meta Llama"),
        ("command", "Cohere Command"),
    ];
    for (frag, label) in families {
        if name.contains(frag) {
            return Some(label);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::model_family;

    #[test]
    fn groups_common_providers() {
        assert_eq!(model_family("~deepseek/deepseek-v4-flash-latest"), Some("DeepSeek"));
        assert_eq!(model_family("deepseek/deepseek-r1"), Some("DeepSeek"));
        assert_eq!(model_family("anthropic/claude-opus-5"), Some("Anthropic Claude"));
        assert_eq!(model_family("openai/gpt-5.6-pro"), Some("ChatGPT / OpenAI"));
        assert_eq!(model_family("openai/o3"), Some("ChatGPT / OpenAI"));
        assert_eq!(model_family("x-ai/grok-4.5"), Some("xAI Grok"));
        assert_eq!(model_family("google/gemini-3.5-flash"), Some("Google Gemini"));
        assert_eq!(model_family("qwen/qwen3.8-27b"), Some("Alibaba Qwen"));
        assert_eq!(model_family("moonshotai/kimi-k3"), Some("Moonshot Kimi"));
        assert_eq!(model_family("z-ai/glm-5.2"), Some("Zhipu GLM"));
    }

    #[test]
    fn leaves_unknown_models_unclassified() {
        assert_eq!(model_family("some-custom/weird"), None);
        assert_eq!(model_family("private/forbidden-model"), None);
        assert_eq!(model_family(""), None);
    }
}
