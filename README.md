# Msty Studio

Cliente de chat **ligero y nativo** para APIs **OpenAI-compatible**, escrito en Rust con
[egui/eframe](https://github.com/emilk/egui). Un solo binario, sin Electron, sin dependencias
pesadas. Sin MCPs ni modelos locales: solo se necesita un endpoint OpenAI-compatible.

## Características

- 💬 Chat con **streaming** token a token (respuesta fluida, con botón de detener)
- ⚙️ Configurable para cualquier API OpenAI-compatible (OpenAI, proveedores de terceros, proxies, etc.)
- 📝 Renderizado de **Markdown** (negritas, código con botón copiar, listas, citas, encabezados)
- 🗂️ Historial de conversaciones persistido en disco
- 🖥️ Ventana nativa, ligera y rápida

## Requisitos

- [Rust](https://rustup.rs/) (toolchain stable con MSVC en Windows)

## Compilar y ejecutar

```sh
# en modo desarrollo (rápido de compilar, más lento en ejecución)
cargo run

# binario optimizado y pequeño
cargo build --release
./target/release/msty_studio.exe
```

## Configuración

En la ventana pulsa **Ajustes** y rellena:

| Campo | Descripción | Ejemplo |
|---|---|---|
| URL base | Endpoint base OpenAI-compatible | `https://api.openai.com/v1` |
| API key | Tu clave de API (se guarda en el archivo de configuración local) | `sk-...` |
| Modelo | Nombre del modelo a usar | `gpt-4o-mini` |
| Temperatura | 0.0 (preciso) – 1.0 (creativo) – 2.0 (máximo) | `0.7` |
| Prompt de sistema | Instrucciones globales opcionales | `Eres un asistente útil` |

La app añade `/chat/completions` a la URL base automáticamente (si la URL no lo incluye ya).
Envíos con **Enter**; **Shift+Enter** para un salto de línea.

> Nota: la API key se guarda en texto plano en el archivo de configuración local
> (`%APPDATA%\msty_studio\config.json`). No compartas ese archivo.

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
