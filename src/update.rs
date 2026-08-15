//! Autoactualización: consulta la última release de GitHub, descarga el
//! binario de la plataforma actual y lo instala reemplazando el ejecutable
//! en uso, relanzando la app con la versión nueva.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/miguepollo/llmchat/releases/latest";
const USER_AGENT: &str = "llmchat-updater";

/// Información de una actualización lista para descargar.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// Etiqueta de la release, p. ej. `v0.2.0`.
    pub version: String,
    /// Nombre del binario para esta plataforma (el del CI).
    pub asset_name: String,
    /// URL directa de descarga del binario.
    pub download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Versión compilada en este binario (la de `Cargo.toml`).
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Nombre del binario de actualización para la plataforma actual. Coincide
/// con los nombres que publica el workflow en cada release.
pub fn asset_name() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "llmchat-windows-x86_64.exe"
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "llmchat-linux-x86_64-musl"
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "llmchat-macos-aarch64"
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "llmchat-macos-x86_64"
    }

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        "llmchat-unsupported-platform"
    }
}

/// Compara versiones `X.Y.Z` (admite prefijo `v`, sufijos tipo `-beta` y
/// etiquetas no numéricas como `continuous`). Devuelve -1/0/1.
pub fn compare_versions(a: &str, b: &str) -> i32 {
    let pa = parse_version(a);
    let pb = parse_version(b);
    for i in 0..3 {
        match pa[i].cmp(&pb[i]) {
            std::cmp::Ordering::Less => return -1,
            std::cmp::Ordering::Greater => return 1,
            std::cmp::Ordering::Equal => {}
        }
    }
    0
}

fn parse_version(s: &str) -> [u32; 3] {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let mut out = [0u32; 3];
    for (i, part) in s.split('.').enumerate().take(3) {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(n) = digits.parse() {
            out[i] = n;
        }
    }
    out
}

fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .map_err(|e| format!("No se pudo crear el cliente HTTP: {e}"))
}

/// Consulta la última release estable en GitHub y devuelve `Some` si existe
/// un binario para esta plataforma **más nuevo** que la versión instalada.
/// Devuelve `None` si no hay release, no hay binario o ya estás al día.
pub async fn check_latest_release() -> Result<Option<ReleaseInfo>, String> {
    let client = client(Duration::from_secs(20))?;
    let response = client
        .get(LATEST_RELEASE_API)
        .send()
        .await
        .map_err(|e| format!("No se pudo consultar GitHub: {e}"))?;

    // Sin release estable aún: no hay nada que actualizar.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!("GitHub respondió HTTP {}", response.status()));
    }

    let release: GitHubRelease = response
        .json()
        .await
        .map_err(|e| format!("Respuesta de GitHub inválida: {e}"))?;

    let Some(asset) = release
        .assets
        .iter()
        .find(|a| a.name == asset_name())
        .cloned()
    else {
        return Ok(None);
    };

    if compare_versions(&release.tag_name, current_version()) <= 0 {
        return Ok(None);
    }

    Ok(Some(ReleaseInfo {
        version: release.tag_name,
        asset_name: asset.name,
        download_url: asset.browser_download_url,
    }))
}

/// Descarga el binario de actualización a un archivo temporal.
pub async fn download_asset(url: &str, file_name: &str) -> Result<PathBuf, String> {
    let client = client(Duration::from_secs(600))?;
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("No se pudo descargar: {e}"))?
        .error_for_status()
        .map_err(|e| format!("La descarga falló: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Error leyendo la descarga: {e}"))?;

    let dest = std::env::temp_dir().join(format!("llmchat-update-{file_name}"));
    let _ = std::fs::remove_file(&dest);
    std::fs::write(&dest, &bytes).map_err(|e| format!("No se pudo escribir el archivo: {e}"))?;
    Ok(dest)
}

/// Instala el binario descargado (reemplaza el actual) y relanza la app.
/// Devuelve `Ok` cuando el proceso de instalación queda lanzado; la app debe
/// salir inmediatamente después para que el reemplazo pueda completarse.
pub fn install_and_restart(new_bin: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        install_windows(new_bin)
    }
    #[cfg(not(windows))]
    {
        install_unix(new_bin)
    }
}

#[cfg(windows)]
fn install_windows(new_bin: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let current = std::env::current_exe().map_err(|e| format!("No se pudo localizar el ejecutable actual: {e}"))?;
    let new_bin = new_bin
        .canonicalize()
        .map_err(|e| format!("No se pudo localizar el binario descargado: {e}"))?;

    let script_path = std::env::temp_dir().join("llmchat-update.ps1");
    let script = format!(
        r#"$ErrorActionPreference = "Stop"
$target = '{cur}'
$new = '{new}'
$oldPid = {pid}
# Espera a que la app salga y, después, sustituye y relanza.
while ($null -ne (Get-Process -Id $oldPid -ErrorAction SilentlyContinue)) {{ Start-Sleep -Milliseconds 300 }}
Start-Sleep -Milliseconds 300
Copy-Item -Force -Path $new -Destination $target
Remove-Item -Force -Path $new -ErrorAction SilentlyContinue
Start-Process -FilePath $target
"#,
        cur = ps_escape(&current.to_string_lossy()),
        new = ps_escape(&new_bin.to_string_lossy()),
        pid = std::process::id(),
    );
    std::fs::write(&script_path, script).map_err(|e| format!("No se pudo escribir el instalador: {e}"))?;

    Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script_path)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("No se pudo lanzar el instalador: {e}"))?;
    Ok(())
}

/// Escapa una ruta para meterla en un string entre comillas simples de PowerShell.
#[cfg(windows)]
fn ps_escape(s: &str) -> String {
    s.replace('\'', "''")
}

#[cfg(not(windows))]
fn install_unix(new_bin: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let current = std::env::current_exe().map_err(|e| format!("No se pudo localizar el ejecutable actual: {e}"))?;
    let mut perms = std::fs::metadata(new_bin)
        .map_err(|e| format!("No se pudo leer el binario descargado: {e}"))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(new_bin, perms).map_err(|e| e.to_string())?;

    // En Unix se puede renombrar sobre el binario en ejecución sin problema.
    std::fs::rename(new_bin, &current).map_err(|e| format!("No se pudo reemplazar el binario: {e}"))?;

    // Relanza la app nueva.
    let _ = std::process::Command::new(&current).spawn();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert_eq!(compare_versions("v0.1.0", "v0.1.0"), 0);
        assert_eq!(compare_versions("0.1.0", "v0.1.1"), -1);
        assert_eq!(compare_versions("v0.2.0", "v0.1.9"), 1);
        assert_eq!(compare_versions("v0.1.10", "v0.1.9"), 1);
        assert_eq!(compare_versions("v1.0.0", "v0.9.9"), 1);
        assert_eq!(compare_versions("continuous", "v0.1.0"), -1);
        assert_eq!(compare_versions("v0.2.0-beta", "v0.2.0"), 0);
        assert_eq!(compare_versions("v0.2.1", "v0.2.0-beta"), 1);
    }

    #[test]
    fn parses_current_version() {
        // La versión compilada siempre debe tener formato X.Y.Z.
        let v = parse_version(current_version());
        assert!(v[0] > 0 || v[1] > 0 || v[2] > 0);
    }

    #[test]
    fn knows_platform_asset() {
        let name = asset_name();
        assert!(name.starts_with("llmchat-"));
        assert!(!name.contains("unsupported"));
    }
}

