# LLMchat

Cliente de chat **ligero y nativo** para APIs **OpenAI-compatible**, escrito en Rust con
[egui/eframe](https://github.com/emilk/egui). Un solo binario, sin Electron, sin dependencias
pesadas. Sin MCPs ni modelos locales: solo se necesita un endpoint OpenAI-compatible.

## Características

- 💬 Chat con **streaming** token a token (respuesta fluida, con botón de detener)
- ⚙️ Configurable para cualquier API OpenAI-compatible (OpenAI, proveedores de terceros, proxies, etc.)
- 📝 Renderizado de **Markdown** (negritas, código con botón copiar, listas, citas, encabezados)
- 🗂️ Historial de conversaciones persistido en disco
- ✏️ Renombra tus conversaciones (botón ✎ en la tarjeta o en la cabecera)
- 📎 Adjunta archivos al chat: imágenes (con vista previa) + extracción de texto
  de **PDF**, **EPUB** y archivos de texto (TXT/MD/JSON/CSV…)
- 🖥️ Ventana nativa, ligera y rápida

## Requisitos

- [Rust](https://rustup.rs/) (toolchain stable con MSVC en Windows)

## Compilar y ejecutar

```sh
# en modo desarrollo (rápido de compilar, más lento en ejecución)
cargo run

# binario optimizado y pequeño
cargo build --release
./target/release/llmchat.exe   # Windows
./target/release/llmchat       # Linux / macOS
```

### Compilar en Linux

En Linux (o Windows con WSL2) se compila de forma **nativa**:

```sh
# Debian / Ubuntu: dependencias de sistema para la ventana (egui/winit)
sudo apt-get update
sudo apt-get install -y \
  libx11-dev libxrandr-dev libxkbcommon-dev libxkbcommon-x11-dev \
  libwayland-dev libegl1-mesa-dev libgl1-mesa-dev

cargo build --release
```

> El binario de Windows no se puede "convertir" a Linux: hay que compilar en
> Linux o usando el CI (abajo). WSL2 no está activado por defecto en Windows;
> habilitarlo requiere la "Plataforma de máquina virtual" y un reinicio.

### Binarios automáticos (GitHub Actions)

El repositorio incluye un workflow (`.github/workflows/build.yml`) que compila
**de forma nativa** y publica binarios para:

- Linux `x86_64` (glibc) y `x86_64` **estático (musl)**
- Windows `x86_64`
- macOS `x86_64` y Apple Silicon (`aarch64`)

Se ejecuta al hacer push, en pull requests y al crear etiquetas `v*`:

- **Cada push a `main`**: los binarios se suben como artefactos y se publica /
  actualiza automáticamente un Release continuo llamado **"Continuous build"**
  en la pestaña *Releases*.
- **Etiqueta `v*`**: se publica un **Release estable** con los binarios
  descargables. Para crearlo:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Los artefactos de cada build también quedan en la página **Actions** del
repositorio sin necesidad de crear una etiqueta.

## Autoactualización

La app comprueba al arrancar (y desde **Ajustes → Actualizaciones**) si la
última **release estable** de GitHub es más nueva que la versión instalada.
Si hay una versión nueva, pulsa **"Descargar e instalar"**: la app descarga el
binario de tu plataforma, se cierra, lo reemplaza y se relanza con la versión
actualizada.

Para publicar una actualización que la app detecte, sube una etiqueta `v*` más
alta que la versión actual de `Cargo.toml` (p. ej. `git tag v0.2.0`).

## Configuración

En la ventana pulsa **Ajustes** y rellena:

| Campo | Descripción | Ejemplo |
|---|---|---|
| URL base | Endpoint base OpenAI-compatible | `https://api.openai.com/v1` |
| API key | Tu clave de API (se guarda en el archivo de configuración local) | `sk-...` |
| Modelo | Nombre del modelo a usar | `gpt-4o-mini` |
| Temperatura | 0.0 (preciso) – 1.0 (creativo) – 2.0 (máximo) | `0.7` |
| Tamaño de letra | Tamaño de la fuente de toda la interfaz (0.75× – 1.5×) | `1.0` |
| Prompt de sistema | Instrucciones globales opcionales | `Eres un asistente útil` |

La app añade `/chat/completions` a la URL base automáticamente (si la URL no lo incluye ya).
Envíos con **Enter**; **Shift+Enter** para un salto de línea.

## Adjuntar archivos

Pulsa el botón **📎** junto al campo del mensaje y selecciona uno o varios archivos:

- **Imágenes** (PNG, JPG, GIF, WebP, BMP, TIFF): se muestran como **vista previa**
  y se **envían al modelo** en base64 (formato OpenAI multimodal:
  `content: [ {type:text}, {type:image_url, url:data:image/...;base64,...} ]`).
- **PDF**: se **extrae el texto** y además se extraen las **imágenes embebidas**
  (JPEG/DCT y RGB/Gris/CMYK con FlateDecode), que también se envían al modelo.
- **EPUB**: se extrae el texto y se adjunta como contexto para el modelo.
- **Texto** (TXT, MD, JSON, CSV, LOG, TOML, YAML): se envía su contenido.
- El texto extraído se añade automáticamente a ese mensaje del usuario; está
  limitado a los primeros **150 000 caracteres** para no saturar el historial.

> **Multimodal**: la app usa el estándar de OpenAI para visión. Necesitarás un
> modelo que soporte imágenes (por ejemplo `gpt-4o`, `gpt-4o-mini`, `claude-*`,
> `llama-3.2-vision`, etc.). Si el modelo no es multimodal, suele responder que
> no puede ver las imágenes; el texto extraído de PDF/EPUB siempre se envía.
> Las imágenes se guardan en el historial como data URI base64.

> Nota: la API key se guarda en texto plano en el archivo de configuración local
> (`%APPDATA%\llmchat\config.json`). No compartas ese archivo.

## Estructura

```
src/
├── main.rs      Punto de entrada (runtime async + eframe)
├── app.rs       Interfaz y estado de la app (egui)
├── api.rs       Cliente HTTP OpenAI-compatible (streaming SSE)
├── markdown.rs  Renderizado de Markdown en egui
├── config.rs    Carga/guardado de configuración y conversaciones
└── types.rs     Tipos de datos (mensajes, conversaciones)
```
