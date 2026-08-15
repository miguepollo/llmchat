use crate::types::{Attachment, AttachmentKind};
use std::io::Read;
use std::path::Path;

/// Límite de caracteres extraídos para no inundar el historial ni la API.
pub const MAX_TEXT: usize = 150_000;

/// Resultado de importar un archivo.
pub struct Imported {
    /// Adjuntos a añadir al mensaje. Para una imagen o texto suele ser uno solo;
    /// para un PDF puede ser el propio adjunto más las imágenes embebidas.
    pub attachments: Vec<Attachment>,
    /// Texto extraído (PDF/EPUB/texto) que se añadirá al contenido del mensaje.
    pub extracted_text: Option<String>,
}

/// Clasifica un archivo por su extensión y lo importa (extrae texto o decodifica imagen).
pub fn import_file(path: &Path) -> Result<Imported, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let kind = match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" => {
            AttachmentKind::Image
        }
        "pdf" => AttachmentKind::Pdf,
        "epub" => AttachmentKind::Epub,
        "txt" | "md" | "markdown" | "json" | "csv" | "log" | "toml" | "yaml" | "yml" => {
            AttachmentKind::Text
        }
        _ => AttachmentKind::Other,
    };

    let mut attachments = Vec::new();
    let mut extracted_text = None;

    match kind {
        AttachmentKind::Image => {
            let image_data =
                image_data_uri(path).map_err(|e| format!("No se pudo leer la imagen: {e}"))?;
            attachments.push(Attachment {
                kind,
                name: name.clone(),
                summary: format!("{}: {}", kind.label(), name),
                image_file: Some(path.to_string_lossy().to_string()),
                image_data: Some(image_data),
            });
        }
        AttachmentKind::Pdf => {
            let text = pdf_text(path).map_err(|e| format!("No se pudo leer el PDF: {e}"))?;
            attachments.push(Attachment {
                kind,
                name: name.clone(),
                summary: format!("{}: {}", kind.label(), name),
                image_file: None,
                image_data: None,
            });
            // Imágenes embebidas en el PDF (multimodal).
            if let Ok(images) = pdf_images(path) {
                for (i, img) in images.into_iter().enumerate() {
                    attachments.push(Attachment {
                        kind: AttachmentKind::Image,
                        name: format!("{name} - imagen {}", i + 1),
                        summary: format!("Imagen: {} ({})", name, i + 1),
                        image_file: None,
                        image_data: Some(img),
                    });
                }
            }
            extracted_text = Some(text);
        }
        AttachmentKind::Epub => {
            let text = epub_text(path).map_err(|e| format!("No se pudo leer el EPUB: {e}"))?;
            attachments.push(Attachment {
                kind,
                name: name.clone(),
                summary: format!("{}: {}", kind.label(), name),
                image_file: None,
                image_data: None,
            });
            extracted_text = Some(text);
        }
        AttachmentKind::Text => {
            let text = read_text(path).map_err(|e| format!("No se pudo leer el archivo: {e}"))?;
            attachments.push(Attachment {
                kind,
                name: name.clone(),
                summary: format!("{}: {}", kind.label(), name),
                image_file: None,
                image_data: None,
            });
            extracted_text = Some(text);
        }
        AttachmentKind::Other => {
            attachments.push(Attachment {
                kind,
                name: name.clone(),
                summary: format!("{}: {}", kind.label(), name),
                image_file: None,
                image_data: None,
            });
        }
    }

    Ok(Imported {
        attachments,
        extracted_text: extracted_text.map(|t| truncate(&t)),
    })
}

fn truncate(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= MAX_TEXT {
        text.to_string()
    } else {
        let mut out: String = chars.into_iter().take(MAX_TEXT).collect();
        out.push_str("\n\n[... extracto truncado ...]");
        out
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Lee un archivo de imagen y devuelve su data URI base64, redimensionando si es enorme.
fn image_data_uri(path: &Path) -> Result<String, String> {
    let img = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| e.to_string())?;
    Ok(encode_resized_data_uri(&img))
}

/// Codifica una imagen a data URI, limitando su lado mayor a 2048 px (la mayoría
/// de APIs de visión no necesitan más y evita payloads gigantes).
fn encode_resized_data_uri(img: &image::DynamicImage) -> String {
    let img = if img.width() > 2048 || img.height() > 2048 {
        img.thumbnail(2048, 2048)
    } else {
        img.clone()
    };
    let rgba = img.to_rgba8();
    let mut bytes: Vec<u8> = Vec::new();
    if image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .is_ok()
    {
        to_data_uri("image/png", &bytes)
    } else {
        // Fallback: PNG transparente 1x1 para no bloquear la petición.
        let transparent = [0u8; 4];
        to_data_uri("image/png", &transparent)
    }
}

fn to_data_uri(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

// ---------------------------------------------------------------------------
// PDF (lopdf): extrae el texto de los "content streams" de cada página.
// ---------------------------------------------------------------------------

fn pdf_text(path: &Path) -> Result<String, String> {
    let doc = lopdf::Document::load(path).map_err(|e| e.to_string())?;
    let mut out = String::new();
    for (_, page_id) in doc.get_pages() {
        let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
        match page.get(b"Contents") {
            Ok(obj) => {
                if let Ok(id) = obj.as_reference() {
                    append_pdf_stream(&mut out, &doc, id)?;
                } else if let Ok(arr) = obj.as_array() {
                    for item in arr {
                        if let Ok(id) = item.as_reference() {
                            append_pdf_stream(&mut out, &doc, id)?;
                        }
                    }
                }
            }
            Err(_) => continue,
        }
        out.push('\n');
    }
    Ok(out)
}

fn append_pdf_stream(
    out: &mut String,
    doc: &lopdf::Document,
    id: lopdf::ObjectId,
) -> Result<(), String> {
    let obj = doc.get_object(id).map_err(|e| e.to_string())?;
    let stream = obj.as_stream().map_err(|e| e.to_string())?;
    let bytes = stream.decompressed_content().map_err(|e| e.to_string())?;
    let content = String::from_utf8_lossy(&bytes);
    push_pdf_text(out, &content);
    Ok(())
}

/// Barrido mínimo de operadores de texto PDF: lee las cadenas entre paréntesis
/// que corresponden a `(...) Tj` / `[...] TJ` / `(...) '` etc.
fn push_pdf_text(out: &mut String, content: &str) {
    let b = content.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'(' => {
                i += 1;
                let mut buf: Vec<u8> = Vec::new();
                while i < b.len() {
                    match b[i] {
                        b'\\' => {
                            if i + 1 < b.len() {
                                buf.push(b[i + 1]);
                            }
                            i += 2;
                        }
                        b')' => break,
                        c => {
                            buf.push(c);
                            i += 1;
                        }
                    }
                }
                let piece = String::from_utf8_lossy(&buf).trim().to_string();
                if piece.is_empty() {
                    continue;
                }
                if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(&piece);
            }
            _ => i += 1,
        }
    }
}

// ---------------------------------------------------------------------------
// PDF imágenes: extrae los XObject de imagen y los devuelve como data URIs PNG.
// ---------------------------------------------------------------------------

/// Extrae las imágenes embebidas de un PDF y las devuelve como data URIs PNG.
fn pdf_images(path: &Path) -> Result<Vec<String>, String> {
    let doc = lopdf::Document::load(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (_, obj) in doc.objects.iter() {
        // Sólo nos interesan los streams de imagen (XObject /Subtype /Image).
        let stream = match obj.as_stream() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let is_image = stream
            .dict
            .get(b"Subtype")
            .ok()
            .and_then(|o| o.as_name_str().ok())
            == Some("Image");
        if !is_image {
            continue;
        }
        if let Ok(data_uri) = pdf_stream_to_data_uri(stream) {
            out.push(data_uri);
            // Evita saturar la API con decenas de mini-imágenes.
            if out.len() >= 20 {
                break;
            }
        }
    }
    Ok(out)
}

/// Convierte un stream de imagen PDF a una data URI PNG (o JPEG si viene así).
fn pdf_stream_to_data_uri(stream: &lopdf::Stream) -> Result<String, String> {
    let filters = stream.filters().unwrap_or_default();
    // DCTDecode == JPEG ya comprimido: se puede usar tal cual (con re-encode a PNG
    // para normalizar y redimensionar si es demasiado grande).
    if filters.iter().any(|f| f == "DCTDecode") {
        let width = stream
            .dict
            .get(b"Width")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0) as u32;
        let height = stream
            .dict
            .get(b"Height")
            .ok()
            .and_then(|o| o.as_i64().ok())
            .unwrap_or(0) as u32;
        let max_side = width.max(height);
        if max_side > 2048 {
            // Decodifica con `image`, redimensiona y re-codifica.
            if let Ok(img) = image::load_from_memory(&stream.content) {
                return Ok(encode_resized_data_uri(&img));
            }
        }
        return Ok(to_data_uri("image/jpeg", &stream.content));
    }
    // JPEG2000 no soportado por nuestra cadena de decodificación.
    if filters.iter().any(|f| f == "JPXDecode" || f == "JPEG2000") {
        return Err("JPEG2000 no soportado".into());
    }

    let width = stream
        .dict
        .get(b"Width")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0) as u32;
    let height = stream
        .dict
        .get(b"Height")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0) as u32;
    if width == 0 || height == 0 || width > 8000 || height > 8000 {
        return Err("dimensiones no válidas".into());
    }

    let bits = stream
        .dict
        .get(b"BitsPerComponent")
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(8) as u32;

    // Nombre del espacio de color (DeviceRGB / DeviceGray / DeviceCMYK ...).
    let colorspace = stream
        .dict
        .get(b"ColorSpace")
        .ok()
        .and_then(|o| o.as_name_str().ok())
        .unwrap_or("/DeviceRGB");
    let channels = match colorspace {
        "/DeviceGray" => 1,
        "/DeviceRGB" => 3,
        "/DeviceCMYK" | "/DeviceCMYK16" => 4,
        _ => {
            return Err("espacio de color no soportado".into());
        }
    };

    let decode_parms = stream
        .dict
        .get(b"DecodeParms")
        .ok()
        .and_then(|o| o.as_dict().ok());
    let predictor = decode_parms
        .and_then(|p| p.get(b"Predictor").ok().and_then(|o| o.as_i64().ok()))
        .unwrap_or(1) as u32;

    // Descomprime el contenido (FlateDecode / LZW / ASCII85 / sin filtro).
    let raw = match stream.get_plain_content() {
        Ok(bytes) => bytes,
        Err(_) => return Err("no se pudo descomprimir la imagen".into()),
    };

    // Para 8 bits por componente y predicción simple (1 o 2 = TIFF) es directo.
    if bits == 8 && (predictor == 1 || predictor == 2) {
        let mut data: Vec<u8> = raw;
        // Predictor 2 (TIFF): diferencia acumulada por componente.
        if predictor == 2 {
            let row_bytes = (width as usize) * channels;
            for row in 0..height as usize {
                let start = row * row_bytes;
                if start + row_bytes > data.len() {
                    continue;
                }
                for c in channels..row_bytes {
                    let idx = start + c;
                    data[idx] = data[idx].wrapping_add(data[idx - channels]);
                }
            }
        }
        return encode_rgba_png(width, height, &data, channels);
    }

    Err("modo de imagen no soportado".into())
}

/// Convierte muestras planas (channels por píxel, 8 bits) a una data URI PNG.
fn encode_rgba_png(width: u32, height: u32, data: &[u8], channels: usize) -> Result<String, String> {
    let expected = (width as usize) * (height as usize) * channels;
    if data.len() < expected {
        return Err(format!("datos insuficientes: {} < {expected}", data.len()));
    }
    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for px in data.chunks(channels) {
        match channels {
            1 => {
                let g = px[0];
                rgba.extend_from_slice(&[g, g, g, 255]);
            }
            3 => {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            // CMYK -> RGB aproximado.
            4 => {
                let c = px[0] as f32 / 255.0;
                let m = px[1] as f32 / 255.0;
                let y = px[2] as f32 / 255.0;
                let k = px[3] as f32 / 255.0;
                let r = (255.0 * (1.0 - c) * (1.0 - k)).round() as u8;
                let g = (255.0 * (1.0 - m) * (1.0 - k)).round() as u8;
                let b = (255.0 * (1.0 - y) * (1.0 - k)).round() as u8;
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
            _ => return Err("canales inesperados".into()),
        }
    }
    let img = image::RgbaImage::from_raw(width, height, rgba)
        .ok_or("no se pudo construir la imagen")?;
    let mut bytes: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(to_data_uri("image/png", &bytes))
}

// ---------------------------------------------------------------------------
// EPUB (zip + XHTML): descomprime, coge los XHTML y los pasa a texto plano.
// ---------------------------------------------------------------------------

fn epub_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut out = String::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_ascii_lowercase();
        if name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm") {
            let mut raw = String::new();
            entry.read_to_string(&mut raw).map_err(|e| e.to_string())?;
            out.push_str(&strip_html(&raw));
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        return Err("no se encontró texto en el EPUB (XHTML)".to_string());
    }
    Ok(out)
}

/// Elimina scripts/estilos, etiquetas HTML y decodifica las entidades básicas.
fn strip_html(raw: &str) -> String {
    let mut s = raw.to_string();
    s = remove_region(&s, "<script", "</script>");
    s = remove_region(&s, "<style", "</style>");
    s = remove_region(&s, "<!--", "-->");

    // Quitar etiquetas fuera de esas regiones.
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    decode_entities(&out)
}

fn remove_region(text: &str, start: &str, end: &str) -> String {
    let mut out = text.to_string();
    loop {
        let s = out.to_lowercase();
        let Some(istart) = s.find(start) else { break };
        let Some(irel) = s[istart..].find(end) else {
            out.replace_range(istart.., "");
            break;
        };
        let iend = istart + irel + end.len();
        out.replace_range(istart..iend, "");
    }
    out
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes.get(i) == Some(&b'&') {
            let rest = &text[i..];
            let (ch, skip) = if let Some(_) = rest.strip_prefix("&amp;") {
                ('&', 5)
            } else if let Some(_) = rest.strip_prefix("&lt;") {
                ('<', 4)
            } else if let Some(_) = rest.strip_prefix("&gt;") {
                ('>', 4)
            } else if let Some(_) = rest.strip_prefix("&quot;") {
                ('"', 6)
            } else if let Some(_) = rest.strip_prefix("&#39;") {
                ('\'', 5)
            } else if let Some(_) = rest.strip_prefix("&nbsp;") {
                (' ', 6)
            } else if let Some(_) = rest.strip_prefix("&#160;") {
                (' ', 6)
            } else {
                ('&', 1)
            };
            out.push(ch);
            i += skip;
        } else {
            let ch_len = utf8_len(bytes[i]);
            let end = (i + ch_len).min(bytes.len());
            out.push_str(&text[i..end]);
            i = end;
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0..=127 => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_html_tags_and_entities() {
        assert_eq!(
            strip_html("<p>Hola <b>mundo</b> &amp; adiós</p>"),
            "Hola mundo & adiós"
        );
        assert_eq!(strip_html("a &lt; b &gt; c"), "a < b > c");
        assert_eq!(strip_html("<script>evil();</script>ok<!--x-->"), "ok");
    }

    #[test]
    fn extracts_text_operators_from_pdf_content() {
        let mut out = String::new();
        push_pdf_text(&mut out, "BT /F1 12 Tf (Hola ) Tj (mundo) Tj ET");
        assert_eq!(out, "Hola mundo");
    }

    #[test]
    fn handles_escaped_parentheses_in_pdf() {
        let mut out = String::new();
        push_pdf_text(&mut out, r"(a\)b) Tj");
        assert_eq!(out, "a)b");
    }

    #[test]
    fn truncates_long_text() {
        let s = "a".repeat(MAX_TEXT + 5000);
        let t = truncate(&s);
        assert!(t.starts_with(&"a".repeat(MAX_TEXT)));
        assert!(t.contains("truncado"));
    }

    #[test]
    fn builds_data_uri() {
        let uri = to_data_uri("image/png", &[1, 2, 3]);
        assert!(uri.starts_with("data:image/png;base64,"));
        use base64::Engine as _;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(uri.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        assert_eq!(decoded, vec![1, 2, 3]);
    }

    #[test]
    fn encodes_gray_samples_to_png_uri() {
        // 2x2 imagen gris (1 canal), valor 255 en todos los píxeles.
        let uri = encode_rgba_png(2, 2, &[255, 255, 255, 255], 1).unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        // Se puede volver a decodificar a un PNG válido.
        let b64 = uri.split(',').nth(1).unwrap();
        use base64::Engine as _;
        let bytes =
            base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let img = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 2);
        assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn rejects_missing_data() {
        assert!(encode_rgba_png(10, 10, &[0, 0], 3).is_err());
    }
}