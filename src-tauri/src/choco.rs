//! Chocolatey, el otro gestor de paquetes de Windows.
//!
//! La búsqueda va por la API pública del repositorio de la comunidad, no por
//! `choco.exe`: así contesta también en un equipo que no tiene Chocolatey
//! puesto, que son casi todos la primera vez. Enseñar lo que hay es
//! independiente de poder instalarlo.
//!
//! Instalar sí necesita su programa, y de eso se ocupa `ensure_available`: la
//! primera vez instala Chocolatey y después el paquete. Es la única parte que
//! toca el equipo, y por eso va aparte y se anuncia como un paso propio.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use crate::download::DownloadFlags;

/// El repositorio de la comunidad. Es el único que la tienda consulta: los
/// privados de una empresa se configuran en el propio Chocolatey y no se
/// buscan desde aquí.
const COMMUNITY_API: &str = "https://community.chocolatey.org/api/v2";

/// El instalador oficial, el mismo que documenta chocolatey.org.
const BOOTSTRAP_SCRIPT: &str = "https://community.chocolatey.org/install.ps1";

/// Cuántos paquetes se piden. La sección enseña unos pocos y ofrece abrir el
/// resto en el navegador, así que traer el catálogo entero no serviría de nada.
const SEARCH_LIMIT: usize = 24;

/// Lo que tarda como mucho una búsqueda antes de darse por perdida. La barra de
/// arriba escribe a cada tecla y una consulta colgada no puede quedarse
/// bloqueando la sección.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(20);

/// Un paquete del repositorio, con lo justo para pintar su tarjeta.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChocoPackage {
    /// El identificador con el que se instala: `choco install <id>`.
    pub id: String,
    /// El nombre legible que publica el paquete. Cae al identificador cuando no
    /// trae ninguno.
    pub title: String,
    pub version: String,
    pub summary: String,
    pub author: String,
    pub icon_url: Option<String>,
    /// Su ficha en community.chocolatey.org, para quien quiera leerla entera.
    pub page_url: String,
    pub downloads: u64,
    /// Chocolatey modera los paquetes de la comunidad y publica el veredicto.
    /// Uno sin aprobar se enseña igual, pero diciéndolo.
    pub approved: bool,
}

/// Dónde está `choco.exe`, si está.
///
/// El instalador escribe `ChocolateyInstall` en el entorno de la máquina, pero
/// un proceso que ya estaba abierto cuando se instaló no ve esa variable: por
/// eso se mira también la ruta de siempre.
pub fn executable() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(root) = std::env::var("ChocolateyInstall") {
        let root = root.trim();
        if !root.is_empty() {
            candidates.push(PathBuf::from(root).join("bin").join("choco.exe"));
        }
    }
    let program_data =
        std::env::var("ProgramData").unwrap_or_else(|_| String::from(r"C:\ProgramData"));
    candidates.push(
        PathBuf::from(program_data)
            .join("chocolatey")
            .join("bin")
            .join("choco.exe"),
    );
    candidates.into_iter().find(|path| path.is_file())
}

/// La versión que dice tener, cuando lo hay. Sirve para el aviso de la sección,
/// que distingue "no está" de "está y es esta".
pub fn version() -> Option<String> {
    let path = executable()?;
    let output = crate::process::hidden_output_timeout(
        &path.to_string_lossy(),
        &["--version"],
        Duration::from_secs(15),
    )
    .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

/// La dirección de la búsqueda.
///
/// El servicio es OData: el término va entre comillas simples dentro del valor
/// del parámetro, y es `Url` quien se encarga de escaparlo. Una comilla dentro
/// de lo escrito cerraría la cadena, así que se dobla antes, que es como OData
/// escribe una comilla literal.
fn search_url(query: &str) -> Result<String, String> {
    let term = query.trim().replace('\'', "''");
    let mut url = url::Url::parse(&format!("{COMMUNITY_API}/Search()"))
        .map_err(|error| format!("Dirección de búsqueda no válida: {error}"))?;
    url.query_pairs_mut()
        .append_pair("$filter", "IsLatestVersion")
        .append_pair("$top", &SEARCH_LIMIT.to_string())
        .append_pair("searchTerm", &format!("'{term}'"))
        .append_pair("targetFramework", "''")
        .append_pair("includePrerelease", "false");
    Ok(url.to_string())
}

/// El texto de una propiedad OData del paquete, si dice algo.
fn property(entry: &crate::msstore::xml::Node, name: &str) -> Option<String> {
    let properties = entry.child("properties")?;
    let text = properties.child(name)?.inner_text();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Recorta una descripción larga a su primera idea.
///
/// La ficha de un paquete trae el README entero en Markdown —títulos, listas,
/// enlaces— y una tarjeta no es sitio para eso. Se queda con el primer párrafo
/// de texto corriente, que es lo que el autor escribió como resumen.
fn shorten(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('-'))
        .unwrap_or("")
        .trim();
    let mut clean = String::with_capacity(first.len());
    let mut chars = first.chars().peekable();
    // Lo justo de Markdown para que no queden asteriscos y corchetes sueltos en
    // pantalla: el énfasis se cae y de un enlace se queda su texto.
    while let Some(ch) = chars.next() {
        match ch {
            '*' | '_' | '`' => {}
            '[' => {}
            ']' => {
                if chars.peek() == Some(&'(') {
                    for skipped in chars.by_ref() {
                        if skipped == ')' {
                            break;
                        }
                    }
                }
            }
            other => clean.push(other),
        }
    }
    let clean = clean.trim();
    if clean.chars().count() <= 220 {
        return clean.to_string();
    }
    let cut: String = clean.chars().take(220).collect();
    let cut = cut.rsplit_once(' ').map(|(head, _)| head).unwrap_or(&cut);
    format!("{cut}…")
}

fn package_from(entry: &crate::msstore::xml::Node) -> Option<ChocoPackage> {
    let id = entry.text_of("title")?;
    let version = property(entry, "Version").unwrap_or_default();
    let title = property(entry, "Title").unwrap_or_else(|| id.clone());
    let summary = entry
        .text_of("summary")
        .or_else(|| property(entry, "Description"))
        .map(|text| shorten(&text))
        .unwrap_or_default();
    let author = entry
        .child("author")
        .and_then(|author| author.text_of("name"))
        .unwrap_or_default();
    let page_url = property(entry, "GalleryDetailsUrl")
        .unwrap_or_else(|| format!("https://community.chocolatey.org/packages/{id}"));
    Some(ChocoPackage {
        id,
        title,
        version,
        summary,
        author,
        icon_url: property(entry, "IconUrl"),
        page_url,
        downloads: property(entry, "DownloadCount")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        approved: property(entry, "PackageStatus")
            .map(|status| status.eq_ignore_ascii_case("Approved"))
            .unwrap_or(false),
    })
}

/// Lo que el repositorio de la comunidad ofrece para lo que se ha escrito.
///
/// Los paquetes sin moderar quedan al final: el orden del servicio es por
/// relevancia y descargas, y un paquete aprobado por Chocolatey es una
/// respuesta mejor que uno recién subido que nadie ha mirado.
pub async fn search(query: &str) -> Result<Vec<ChocoPackage>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let client = crate::download::http_client()?;
    let response = client
        .get(search_url(query)?)
        .timeout(SEARCH_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("No se pudo consultar Chocolatey: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "Chocolatey respondió {} al buscar «{query}».",
            response.status()
        ));
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("Respuesta de Chocolatey ilegible: {error}"))?;

    let document = crate::msstore::xml::parse(&body)?;
    let mut entries = Vec::new();
    document.find_all("entry", &mut entries);
    let mut packages: Vec<ChocoPackage> = entries
        .iter()
        .filter_map(|entry| package_from(entry))
        .filter(|package| !package.id.is_empty())
        .collect();
    packages.sort_by_key(|package| !package.approved);
    Ok(packages)
}

/// Chocolatey escribe en `C:\ProgramData` y en el entorno de la máquina, y sus
/// paquetes instalan programas: todo lo suyo necesita el token de
/// administrador. La tienda lo tiene siempre —su manifiesto pide
/// `requireAdministrator`—, así que esto sólo salta ejecutando desde `cargo`,
/// y salta con lo que hay que hacer en vez de dejar que Chocolatey falle con un
/// error de permisos a medio instalar.
fn require_elevation(what: &str) -> Result<(), String> {
    if crate::process::is_elevated() {
        return Ok(());
    }
    crate::logger::warn(
        "choco",
        format!("{what} necesita permisos de administrador y la tienda no los tiene."),
    );
    Err(format!(
        "{what} necesita permisos de administrador. Cierra WinSlimCenter y vuelve a abrirlo \
         como administrador."
    ))
}

/// Deja Chocolatey listo, instalándolo si hace falta.
///
/// El script es el que documenta chocolatey.org y se ejecuta tal cual, heredando
/// el token de la tienda: no hay un segundo aviso de UAC que dar, porque la
/// tienda ya arrancó elevada.
pub async fn ensure_available(
    on_progress: &mut impl FnMut(u32, String, bool),
) -> Result<PathBuf, String> {
    if let Some(path) = executable() {
        return Ok(path);
    }
    require_elevation("Instalar Chocolatey")?;
    crate::logger::info(
        "choco",
        "Chocolatey no está en el equipo: se instalará antes que el paquete.",
    );
    on_progress(5, "Instalando Chocolatey...".into(), false);

    let script = format!(
        "Set-ExecutionPolicy Bypass -Scope Process -Force; \
         [System.Net.ServicePointManager]::SecurityProtocol = \
         [System.Net.ServicePointManager]::SecurityProtocol -bor 3072; \
         iex ((New-Object System.Net.WebClient).DownloadString('{BOOTSTRAP_SCRIPT}'))"
    );
    let output = tokio::task::spawn_blocking(move || {
        crate::process::hidden_output(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                script.as_str(),
            ],
        )
    })
    .await
    .map_err(|error| format!("No se pudo ejecutar el instalador de Chocolatey: {error}"))?
    .map_err(|error| format!("No se pudo ejecutar el instalador de Chocolatey: {error}"))?;

    if !output.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(format!(
            "No se pudo instalar Chocolatey{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    // El instalador escribe `ChocolateyInstall` en el entorno de la máquina, y
    // este proceso conserva el suyo de cuando arrancó: no la va a ver hasta que
    // la tienda se reinicie. No hace falta, porque `executable()` mira también
    // la ruta de siempre, que es donde el instalador oficial lo deja.
    executable().ok_or_else(|| {
        "El instalador de Chocolatey terminó, pero choco.exe no aparece en el equipo.".to_string()
    })
}

/// Lo que Chocolatey deja escrito cuando el paquete ya estaba puesto y al día.
fn already_current(output: &str, package_id: &str) -> bool {
    let prefix = format!("{} ", package_id.to_lowercase());
    output.lines().any(|line| {
        let lowered = line.trim().to_lowercase();
        lowered.starts_with(&prefix)
            && (lowered.contains("already installed")
                || lowered.contains("ya está instalado")
                || lowered.contains("is the latest version available"))
    })
}

fn install_arguments(package_id: &str, update: bool, reinstall: bool) -> Vec<String> {
    let mut args = vec![
        if update { "upgrade" } else { "install" }.to_string(),
        package_id.to_string(),
        "--yes".into(),
        "--no-progress".into(),
        "--accept-license".into(),
    ];
    if reinstall {
        args.push("--force".into());
    }
    args
}

/// Por qué falló, en una frase que se pueda leer en un diálogo.
///
/// Chocolatey cuenta lo ocurrido en varias decenas de líneas y termina con
/// «See the log for details», que es lo que quedaba al coger la última: el
/// diálogo decía que algo había fallado sin decir qué, teniendo el motivo
/// cuatro líneas más arriba. Se busca por orden lo que de verdad lo explica —la
/// línea `ERROR:`, la del bloque `Failures`, la del script del paquete— y sólo
/// si no hay nada de eso se recurre al final del volcado.
fn failure_reason(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    // `ERROR:` es lo que Chocolatey escribe cuando sabe qué salió mal, y es la
    // línea que nombra el servidor caído, el 404 o el certificado inválido.
    if let Some(error) = lines
        .iter()
        .find_map(|line| line.strip_prefix("ERROR: ").or(line.strip_prefix("ERROR:")))
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        return error.to_string();
    }
    // El resumen final: `- paquete (exited 404) - lo que pasó`.
    if let Some(failure) = lines
        .iter()
        .find(|line| line.starts_with("- ") && line.contains("exited"))
    {
        return failure.trim_start_matches("- ").to_string();
    }
    // Un paquete cuyo script se rompió sin decir más.
    if let Some(script) = lines
        .iter()
        .find(|line| line.starts_with("Error while running"))
    {
        return script.to_string();
    }
    lines.last().unwrap_or(&"").to_string()
}

/// Instala —o actualiza— un paquete con Chocolatey.
///
/// `upgrade` sobre algo que no está puesto lo instala, así que el verbo se
/// elige por lo que pidió la tienda y no hay que comprobar nada antes.
pub async fn install(
    package_id: &str,
    force_update: bool,
    reinstall: bool,
    flags: &Arc<DownloadFlags>,
    on_progress: &mut impl FnMut(u32, String, bool),
) -> Result<bool, String> {
    let package_id = package_id.trim();
    if package_id.is_empty() {
        return Err("Falta choco_id".into());
    }
    // Antes de nada: un paquete de Chocolatey instala programas en la máquina y
    // escribe en `C:\ProgramData\chocolatey`. Sin el token de administrador se
    // queda a medias con un error que no dice de qué va.
    require_elevation("Instalar paquetes de Chocolatey")?;
    let choco = ensure_available(on_progress).await?;
    if flags.cancel.load(Ordering::SeqCst) {
        return Err(crate::installer::CANCELLED_MARKER.into());
    }

    on_progress(15, "Trabajando en segundo plano...".into(), false);
    let program = choco.to_string_lossy().to_string();
    let args = install_arguments(package_id, force_update, reinstall);
    let cancel_flags = flags.clone();
    let output = tokio::task::spawn_blocking(move || {
        crate::process::hidden_output_cancelable(
            &program,
            &args.iter().map(String::as_str).collect::<Vec<_>>(),
            &cancel_flags.cancel,
        )
    })
    .await
    .map_err(|error| format!("No se pudo ejecutar Chocolatey: {error}"))?
    .map_err(|error| format!("Chocolatey no está disponible: {error}"))?;

    if flags.cancel.load(Ordering::SeqCst) {
        return Err(crate::installer::CANCELLED_MARKER.into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.success() {
        let combined = format!("{stdout}\n{stderr}");
        // El volcado entero va al diario; en pantalla, sólo el motivo.
        crate::logger::warn(
            "choco",
            format!("Chocolatey falló al instalar {package_id}:\n{combined}"),
        );
        let detail = failure_reason(&combined);
        return Err(format!(
            "Chocolatey no pudo instalar {package_id}{}",
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }

    if already_current(&stdout, package_id) && !reinstall {
        on_progress(
            100,
            "La aplicación ya está en su última versión".into(),
            false,
        );
        return Ok(false);
    }
    on_progress(100, "Comprobando la instalación...".into(), false);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_is_specific_to_the_requested_package() {
        assert!(already_current(
            "cheatengine v7.7.0 already installed.\nChocolatey installed 0/1 packages.",
            "cheatengine",
        ));
        assert!(already_current(
            "cheatengine v7.7.0 is the latest version available based on your source(s).",
            "cheatengine",
        ));
        assert!(!already_current(
            "chocolatey-core.extension v1.4.0 already installed.\nThe install of cheatengine was successful.",
            "cheatengine",
        ));
    }

    #[test]
    fn repair_only_forces_the_requested_package() {
        let normal = install_arguments("cheatengine", false, false);
        assert!(!normal.iter().any(|arg| arg == "--force"));
        let repair = install_arguments("cheatengine", false, true);
        assert_eq!(&repair[..2], &["install", "cheatengine"]);
        assert!(repair.iter().any(|arg| arg == "--force"));
        assert!(!repair.iter().any(|arg| arg == "--force-dependencies"));
        assert!(!repair.iter().any(|arg| arg.starts_with("--ignore-checksum")));
    }

    #[test]
    fn the_search_term_travels_quoted_and_escaped() {
        let url = search_url("visual studio").unwrap();
        assert!(url.contains("searchTerm=%27visual+studio%27"), "{url}");
        assert!(url.contains("%24filter=IsLatestVersion"), "{url}");
    }

    #[test]
    fn a_quote_in_the_query_does_not_close_the_odata_string() {
        let url = search_url("o'brien").unwrap();
        assert!(url.contains("o%27%27brien"), "{url}");
    }

    #[test]
    fn a_readme_is_cut_down_to_its_first_sentence() {
        let text =
            "# 7-Zip\n\n7-Zip is a **file archiver** with a high ratio.\n\n## Features\n- one";
        assert_eq!(shorten(text), "7-Zip is a file archiver with a high ratio.");
    }

    #[test]
    fn a_markdown_link_keeps_only_its_text() {
        assert_eq!(
            shorten("See [the site](http://x.y) for more."),
            "See the site for more."
        );
    }

    #[test]
    fn an_entry_becomes_a_package() {
        let feed = r#"<feed xmlns="http://www.w3.org/2005/Atom"
            xmlns:d="http://schemas.microsoft.com/ado/2007/08/dataservices"
            xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">
          <entry>
            <title type="text">7zip</title>
            <summary type="text">7-Zip is a file archiver.</summary>
            <author><name>Igor Pavlov</name></author>
            <m:properties>
              <d:Version>26.3.0</d:Version>
              <d:Title>7-Zip</d:Title>
              <d:DownloadCount>35538182</d:DownloadCount>
              <d:IconUrl>https://cdn.example/7zip.png</d:IconUrl>
              <d:GalleryDetailsUrl>https://community.chocolatey.org/packages/7zip/26.3.0</d:GalleryDetailsUrl>
              <d:PackageStatus>Approved</d:PackageStatus>
            </m:properties>
          </entry>
        </feed>"#;
        let document = crate::msstore::xml::parse(feed).unwrap();
        let mut entries = Vec::new();
        document.find_all("entry", &mut entries);
        let package = package_from(entries[0]).unwrap();
        assert_eq!(package.id, "7zip");
        assert_eq!(package.title, "7-Zip");
        assert_eq!(package.version, "26.3.0");
        assert_eq!(package.author, "Igor Pavlov");
        assert_eq!(package.downloads, 35_538_182);
        assert!(package.approved);
    }

    /// El final de la salida real que dejó `choco install ntlite-free`, que es
    /// el fallo que descubrió que se estaba enseñando la línea equivocada.
    const CHOCO_FAILED_NTLITE: &str = concat!(
        "ntlite-free package files install completed. Performing other installation steps.\n",
        "Attempt to get headers for https://downloads.ntlite.com/files/NTLite_setup_x64.exe failed.\n",
        "ERROR: The remote file either doesn't exist, is unauthorized, or is forbidden for url ",
        "'https://downloads.ntlite.com/files/NTLite_setup_x64.exe'.\n",
        "The install of ntlite-free was NOT successful.\n",
        "Error while running 'C:\\ProgramData\\chocolatey\\lib\\ntlite-free\\tools\\chocolateyinstall.ps1'.\n",
        " See log for details.\n",
        "\n",
        "Chocolatey installed 0/1 packages. 1 packages failed.\n",
        " See the log for details (C:\\ProgramData\\chocolatey\\logs\\chocolatey.log).\n",
        "\n",
        "Failures\n",
        " - ntlite-free (exited 404) - Error while running 'chocolateyinstall.ps1'.\n",
        " See log for details.\n",
    );

    #[test]
    fn the_reason_shown_is_the_error_line_and_not_the_last_one() {
        let reason = failure_reason(CHOCO_FAILED_NTLITE);
        assert!(
            reason.starts_with("The remote file either doesn't exist"),
            "se esperaba el motivo real, salió: {reason}"
        );
        assert!(!reason.contains("See the log for details"));
    }

    #[test]
    fn without_an_error_line_the_failures_summary_explains_it() {
        let output = concat!(
            "Chocolatey v2.7.4\n",
            "Chocolatey installed 0/1 packages. 1 packages failed.\n",
            "Failures\n",
            " - algo (exited 1) - el instalador devolvió 1.\n",
        );
        assert_eq!(
            failure_reason(output),
            "algo (exited 1) - el instalador devolvió 1."
        );
    }

    #[test]
    fn with_nothing_recognizable_the_last_line_is_still_better_than_silence() {
        assert_eq!(failure_reason("una cosa\notra cosa\n"), "otra cosa");
        assert_eq!(failure_reason(""), "");
    }

    #[test]
    fn an_unmoderated_package_is_listed_after_the_approved_ones() {
        let mut packages = vec![
            ChocoPackage {
                id: "b".into(),
                title: "b".into(),
                version: "1".into(),
                summary: String::new(),
                author: String::new(),
                icon_url: None,
                page_url: String::new(),
                downloads: 0,
                approved: false,
            },
            ChocoPackage {
                id: "a".into(),
                title: "a".into(),
                version: "1".into(),
                summary: String::new(),
                author: String::new(),
                icon_url: None,
                page_url: String::new(),
                downloads: 0,
                approved: true,
            },
        ];
        packages.sort_by_key(|package| !package.approved);
        assert_eq!(packages[0].id, "a");
    }
}
