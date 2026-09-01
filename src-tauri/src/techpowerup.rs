//! El canal de descarga de TechPowerUp.
//!
//! TechPowerUp no publica una dirección directa y estable. La ficha del
//! producto lista las versiones, la versión se canjea por una lista de espejos
//! y cada espejo por una dirección firmada que caduca en minutos. Son tres
//! pasos, y el último hay que darlo justo antes de descargar.
//!
//! Se dan aquí en lugar de abrir la web porque la web es exactamente esos tres
//! clics, y porque un espejo que falla no tiene por qué ser el final: quedan
//! los demás.

use std::sync::OnceLock;
use std::time::Duration;

const USER_AGENT: &str = "CenterAppStore/5.0";

/// Uno de los servidores desde los que TechPowerUp sirve el archivo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mirror {
    pub id: String,
    pub name: String,
}

/// Todo lo que hace falta para descargar la versión publicada de un producto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub page_url: String,
    /// El identificador del archivo dentro de la ficha, que es lo que se canjea
    /// por la lista de espejos.
    pub file_id: String,
    pub file_name: String,
    pub version: Option<String>,
    /// La huella que publica la ficha. Un espejo puede entregar el archivo a
    /// medias sin decirlo, y sin esto no habría con qué notarlo.
    pub sha256: Option<String>,
    pub mirrors: Vec<Mirror>,
}

/// La dirección de la ficha de un producto.
pub fn page_url(slug: &str) -> String {
    format!(
        "https://www.techpowerup.com/download/{}/",
        slug.trim().trim_matches('/')
    )
}

/// Un cliente propio porque aquí no se quiere seguir la redirección: la
/// dirección firmada es justo lo que se busca, y seguirla abriría la descarga
/// por una conexión que después se tira.
fn client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// Qué publica ahora mismo TechPowerUp para este producto y desde dónde puede
/// bajarse.
pub async fn plan(slug: &str) -> Result<Plan, String> {
    let url = page_url(slug);
    let page = get(&url).await?;
    let mut plan = parse_plan(&page, &url)?;

    let mirrors = post(&url, &[("id", plan.file_id.as_str())]).await?;
    plan.mirrors = parse_mirrors(&mirrors);
    if plan.mirrors.is_empty() {
        return Err(format!(
            "TechPowerUp no ofreció ningún servidor para {}.",
            plan.file_name
        ));
    }
    Ok(plan)
}

/// Canjea un espejo por la dirección firmada desde la que descargar.
///
/// Caduca en minutos, así que se pide para el espejo que toca y en el momento
/// en que va a usarse, no para todos por adelantado.
pub async fn mirror_url(plan: &Plan, mirror: &Mirror) -> Result<String, String> {
    let response = client()?
        .post(&plan.page_url)
        .header(reqwest::header::REFERER, &plan.page_url)
        .form(&[
            ("id", plan.file_id.as_str()),
            ("server_id", mirror.id.as_str()),
        ])
        .send()
        .await
        .map_err(|error| format!("No se pudo hablar con {}: {error}", mirror.name))?;

    let status = response.status();
    response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| value.starts_with("http"))
        .map(str::to_string)
        .ok_or_else(|| {
            format!("{} no devolvió un enlace de descarga (respondió {status}).", mirror.name)
        })
}

async fn get(url: &str) -> Result<String, String> {
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|error| format!("No se pudo abrir la ficha de TechPowerUp: {error}"))?;
    read_body(response, url).await
}

async fn post(url: &str, form: &[(&str, &str)]) -> Result<String, String> {
    let response = client()?
        .post(url)
        .header(reqwest::header::REFERER, url)
        .form(form)
        .send()
        .await
        .map_err(|error| format!("No se pudo consultar TechPowerUp: {error}"))?;
    read_body(response, url).await
}

async fn read_body(response: reqwest::Response, url: &str) -> Result<String, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("TechPowerUp respondió {status} a {url}."));
    }
    response
        .text()
        .await
        .map_err(|error| format!("Respuesta ilegible de TechPowerUp: {error}"))
}

/// Lee de la ficha la versión publicada: cómo se llama el archivo, qué huella
/// tiene y con qué identificador se pide.
///
/// La página lista todas las versiones, de la más nueva a la más vieja. Se
/// acota el texto a la primera antes de leer nada: buscar en la página entera
/// mezclaría el nombre de una versión con el identificador de otra.
pub fn parse_plan(html: &str, page_url: &str) -> Result<Plan, String> {
    let start = html
        .find("class=\"filename\"")
        .ok_or("La ficha de TechPowerUp no lista ningún archivo.")?;
    let end = html[start..]
        .find("</form>")
        .map(|offset| start + offset)
        .unwrap_or(html.len());
    let entry = &html[start..end];

    let file_name = between(entry, "title=\"File Name\">", "<")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("La ficha de TechPowerUp no dice cómo se llama el archivo.")?
        .to_string();
    let file_id = between(entry, "name=\"id\" value=\"", "\"")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or("La ficha de TechPowerUp no identifica la versión publicada.")?
        .to_string();

    let sha256 = entry
        .find("SHA256:")
        .and_then(|position| between(&entry[position..], "hash-value\">", "<"))
        .map(|value| value.trim().to_lowercase())
        .filter(|value| value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit()));

    Ok(Plan {
        page_url: page_url.to_string(),
        file_id,
        version: version_in(&file_name),
        file_name,
        sha256,
        mirrors: Vec::new(),
    })
}

/// Los espejos que ofrece la respuesta, en el orden en que los ofrece.
///
/// Ese orden no es casual: TechPowerUp pone delante el más cercano a quien
/// pregunta, así que probar por orden es empezar por el que mejor debería ir.
pub fn parse_mirrors(html: &str) -> Vec<Mirror> {
    const MARK: &str = "name=\"server_id\" value=\"";
    let mut mirrors = Vec::new();
    let mut rest = html;
    while let Some(position) = rest.find(MARK) {
        let after = &rest[position + MARK.len()..];
        let id = after.split('"').next().unwrap_or_default().trim();
        // El nombre vive dentro del mismo botón: se busca sin salir de él, o
        // cada espejo se quedaría con el nombre del siguiente.
        let button = &after[..after.find(MARK).unwrap_or(after.len())];
        let name = between(button, "class=\"server-name\">", "<")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(id);
        if !id.is_empty() {
            mirrors.push(Mirror {
                id: id.to_string(),
                name: name.to_string(),
            });
        }
        rest = after;
    }
    mirrors
}

/// La versión que lleva dentro el nombre del archivo, cuando lo lleva.
fn version_in(file_name: &str) -> Option<String> {
    let stem = file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(file_name);
    let candidate = stem.rsplit(['_', '-']).next()?;
    let looks_like_a_version = candidate.starts_with(|c: char| c.is_ascii_digit())
        && candidate.chars().all(|c| c.is_ascii_digit() || c == '.');
    looks_like_a_version.then(|| candidate.to_string())
}

fn between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = haystack.find(start)? + start.len();
    let rest = &haystack[from..];
    let to = rest.find(end)?;
    Some(&rest[..to])
}

#[cfg(test)]
mod tests {
    use super::*;

    const FICHA: &str = r#"
<div class="version">
  <h3 class="title">NVCleanstall v1.19.0</h3>
  <ul class="files"><li class="file clearfix expanded">
    <div class="filesize">3.8 MB</div>
    <div class="filename" title="File Name">NVCleanstall_1.19.0.exe</div>
    <div class="hashes">
      <div class="hash-entry"><div class="hash-name">MD5:</div><div class="hash-value">B2C6</div></div>
      <div class="hash-entry"><div class="hash-name">SHA256:</div>
        <div class="hash-value">9DD36EF956AF927CF41FA441F91B329A7973E13965E4E7D70E6FA9C1DF1CADE6</div></div>
    </div>
    <form action="/download/techpowerup-nvcleanstall/" method="POST" class="download-version-form">
      <input type="hidden" name="id" value="2849" />
    </form>
  </li></ul>
</div>
<div class="version hidden">
  <h3 class="title">NVCleanstall v1.18.0</h3>
  <div class="filename" title="File Name">NVCleanstall_1.18.0.exe</div>
  <form class="download-version-form"><input type="hidden" name="id" value="2701" /></form>
</div>"#;

    const ESPEJOS: &str = r#"
<form method="POST" class="clearfix">
  <input type="hidden" name="id" value="2849" />
  <div class="mirrorlist">
    <button type="submit" name="server_id" value="5">
      <span class="closest">(closest to you)</span><span class="server-name">TechPowerUp UK-1</span>
      <span class="server-load low">Server load: 3%</span></button>
    <button type="submit" name="server_id" value="28">
      <span class="server-name">TechPowerUp UK-3</span>
      <span class="server-load low">Server load: 1%</span></button>
    <button type="submit" name="server_id" value="25">
      <span class="server-name">TechPowerUp DE</span></button>
  </div>
</form>"#;

    #[test]
    fn de_la_ficha_sale_la_version_de_arriba_y_no_las_de_abajo() {
        let plan = parse_plan(FICHA, "https://www.techpowerup.com/download/x/").unwrap();
        assert_eq!(plan.file_name, "NVCleanstall_1.19.0.exe");
        // La ficha lista todas las versiones publicadas: coger el identificador
        // de la segunda descargaría una anterior sin que nada lo dijera.
        assert_eq!(plan.file_id, "2849");
        assert_eq!(plan.version.as_deref(), Some("1.19.0"));
        assert_eq!(
            plan.sha256.as_deref(),
            Some("9dd36ef956af927cf41fa441f91b329a7973e13965e4e7d70e6fa9c1df1cade6")
        );
    }

    #[test]
    fn los_espejos_se_leen_en_el_orden_en_que_se_ofrecen() {
        let mirrors = parse_mirrors(ESPEJOS);
        let leidos: Vec<(&str, &str)> = mirrors
            .iter()
            .map(|mirror| (mirror.id.as_str(), mirror.name.as_str()))
            .collect();
        assert_eq!(
            leidos,
            vec![
                ("5", "TechPowerUp UK-1"),
                ("28", "TechPowerUp UK-3"),
                // El último no hereda el nombre de nadie: su bloque termina
                // donde termina la lista.
                ("25", "TechPowerUp DE"),
            ]
        );
    }

    #[test]
    fn una_respuesta_sin_espejos_no_inventa_ninguno() {
        assert!(parse_mirrors("<form></form>").is_empty());
        assert!(parse_plan("<html></html>", "https://x/").is_err());
    }

    #[test]
    fn la_version_solo_se_lee_cuando_el_nombre_la_lleva() {
        assert_eq!(
            version_in("NVCleanstall_1.19.0.exe").as_deref(),
            Some("1.19.0")
        );
        assert_eq!(version_in("ThrottleStop-9.6.exe").as_deref(), Some("9.6"));
        assert_eq!(version_in("instalador.exe"), None);
    }

    /// Habla de verdad con TechPowerUp.
    ///
    /// Fuera de la suite porque necesita red, pero se conserva porque comprueba
    /// lo único que ninguna prueba con datos fijos puede: que la ficha sigue
    /// escribiéndose como estaba escrita cuando se leyó por primera vez.
    ///
    /// `cargo test --release --lib -- --ignored --nocapture sonda_techpowerup`
    #[test]
    #[ignore]
    fn sonda_techpowerup() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let plan = match plan("techpowerup-nvcleanstall").await {
                Ok(plan) => plan,
                Err(error) => panic!("la ficha falló: {error}"),
            };
            println!(
                "  [ficha] archivo={} versión={:?} huella={:?}",
                plan.file_name, plan.version, plan.sha256
            );
            println!("  [espejos] {}", plan.mirrors.len());
            for mirror in plan.mirrors.iter().take(3) {
                println!("      {} (id={})", mirror.name, mirror.id);
            }
            assert!(!plan.mirrors.is_empty());

            let first = &plan.mirrors[0];
            match mirror_url(&plan, first).await {
                Ok(url) => {
                    println!("  [enlace] {} -> {url}", first.name);
                    assert!(url.ends_with(&plan.file_name));
                }
                Err(error) => panic!("el espejo {} falló: {error}", first.name),
            }
        });
    }

    #[test]
    fn la_direccion_de_la_ficha_se_compone_igual_con_barra_y_sin_ella() {
        assert_eq!(
            page_url("techpowerup-nvcleanstall"),
            "https://www.techpowerup.com/download/techpowerup-nvcleanstall/"
        );
        assert_eq!(
            page_url("/techpowerup-nvcleanstall/"),
            "https://www.techpowerup.com/download/techpowerup-nvcleanstall/"
        );
    }
}
