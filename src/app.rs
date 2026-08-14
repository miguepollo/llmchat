use crate::api::{self, StreamEvent};
use crate::config::{self, Settings};
use crate::markdown;
use crate::types::{Conversation, Message, Role};
use eframe::egui;
use tokio::sync::mpsc;

pub struct App {
    settings: Settings,
    conversations: Vec<Conversation>,
    current_id: Option<String>,
    draft: String,
    streaming: bool,
    stream_conv_id: Option<String>,
    rx: Option<mpsc::UnboundedReceiver<StreamEvent>>,
    join: Option<tokio::task::JoinHandle<()>>,
    show_settings: bool,
    status: Option<String>,
    input_focus: bool,
    needs_save: bool,
    runtime: tokio::runtime::Handle,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, runtime: tokio::runtime::Handle) -> Self {
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let mut conversations = config::load_conversations();
        if conversations.is_empty() {
            conversations.push(Conversation::new());
        }
        let current_id = conversations.first().map(|c| c.id.clone());
        Self {
            settings: config::load_settings(),
            conversations,
            current_id,
            draft: String::new(),
            streaming: false,
            stream_conv_id: None,
            rx: None,
            join: None,
            show_settings: false,
            status: None,
            input_focus: true,
            needs_save: false,
            runtime,
        }
    }

    fn conversation(&self, id: &str) -> Option<&Conversation> {
        self.conversations.iter().find(|c| c.id == id)
    }

    fn conversation_mut(&mut self, id: &str) -> Option<&mut Conversation> {
        self.conversations.iter_mut().find(|c| c.id == id)
    }

    fn new_conversation(&mut self) {
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
        if self.current_id.as_deref() == Some(id) {
            self.current_id = self.conversations.first().map(|c| c.id.clone());
        }
        if self.conversations.is_empty() {
            self.conversations.push(Conversation::new());
            self.current_id = self.conversations.first().map(|c| c.id.clone());
        }
        self.needs_save = true;
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

}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_stream();

        // ---------- Cabecera ----------
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("Msty Studio");
                ui.add_space(12.0);
                if let Some(status) = self.status.clone() {
                    ui.colored_label(egui::Color32::from_rgb(0xf0, 0x7a, 0x7a), status);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Ajustes").clicked() {
                        self.show_settings = true;
                    }
                    let mut model = self.settings.model.clone();
                    egui::ComboBox::from_id_salt("modelo")
                        .selected_text(model.clone())
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            for m in &self.settings.models {
                                ui.selectable_value(&mut model, m.clone(), m.as_str());
                            }
                        });
                    if model != self.settings.model {
                        self.settings.model = model;
                        self.needs_save = true;
                    }
                });
            });
            ui.add_space(6.0);
        });

        // ---------- Barra lateral ----------
        egui::SidePanel::left("sidebar")
            .resizable(false)
            .default_width(230.0)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Nueva").clicked() {
                        self.new_conversation();
                    }
                    if ui.button("Borrar actual").clicked() {
                        if let Some(id) = self.current_id.clone() {
                            self.delete_conversation(&id);
                        }
                    }
                });
                ui.add_space(4.0);
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let ids: Vec<String> =
                            self.conversations.iter().map(|c| c.id.clone()).collect();
                        for id in ids {
                            let selected = self.current_id.as_deref() == Some(id.as_str());
                            let title = self
                                .conversation(&id)
                                .map(|c| c.title.clone())
                                .unwrap_or_default();
                            if ui
                                .selectable_label(selected, truncate(&title, 28))
                                .clicked()
                            {
                                self.current_id = Some(id.clone());
                                self.input_focus = true;
                            }
                        }
                    });
            });

        // ---------- Entrada de mensaje ----------
        egui::TopBottomPanel::bottom("input").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let input = egui::TextEdit::multiline(&mut self.draft)
                    .hint_text("Escribe un mensaje...  (Enter envía, Shift+Enter salto de línea)")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Body);
                let response = ui.add(input);
                if self.input_focus {
                    response.request_focus();
                    self.input_focus = false;
                }
                if response.has_focus() {
                    let enter =
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    if enter {
                        self.send();
                    }
                }
                let label = if self.streaming { "Detener" } else { "Enviar" };
                let button = egui::Button::new(label).min_size(egui::vec2(80.0, 60.0));
                if ui.add(button).clicked() {
                    if self.streaming {
                        self.stop();
                    } else {
                        self.send();
                    }
                }
            });
            ui.add_space(4.0);
            if self.streaming {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.weak("Generando respuesta...");
                });
            }
            ui.add_space(2.0);
        });

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

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for message in &conv.messages {
                        match message.role {
                            Role::User => render_user_bubble(ui, &message.content),
                            Role::Assistant => render_assistant_bubble(ui, &message.content),
                            Role::System => {}
                        }
                    }
                    if self.streaming {
                        ui.add(egui::Spinner::new());
                    }
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
                    ui.label("Modelo");
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
                    ui.add_space(4.0);
                    ui.label("Temperatura");
                    ui.add(
                        egui::Slider::new(&mut self.settings.temperature, 0.0..=2.0).text("t"),
                    );
                    ui.add_space(4.0);
                    ui.label("Prompt de sistema (opcional)");
                    ui.add(
                        egui::TextEdit::multiline(&mut self.settings.system_prompt)
                            .desired_rows(3)
                            .desired_width(f32::INFINITY),
                    );
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

fn render_user_bubble(ui: &mut egui::Ui, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        egui::Frame::new()
            .fill(ui.visuals().selection.bg_fill)
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(text).color(egui::Color32::WHITE));
            });
    });
    ui.add_space(8.0);
}

fn render_assistant_bubble(ui: &mut egui::Ui, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            markdown::render(ui, text);
        });
    ui.horizontal(|ui| {
        if ui.small_button("Copiar").clicked() {
            ui.ctx().copy_text(text.to_string());
        }
    });
    ui.add_space(8.0);
}
