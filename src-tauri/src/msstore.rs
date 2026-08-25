//! La Microsoft Store, dentro de la tienda.
//!
//! El catálogo propio de WinSlimCenter cubre programas que se descargan de la
//! web de su autor. Lo que Microsoft publica en su tienda no está ahí, y hasta
//! ahora la única forma de instalarlo era abrir otra aplicación. Este módulo
//! trae ese catálogo dentro: busca en el mismo índice que `apps.microsoft.com`,
//! resuelve los paquetes reales que hay detrás de un producto y los descarga
//! por el canal por el que los sirve Windows Update.
//!
//! Un producto de la tienda es de una de dos clases, y se distinguen por cómo
//! empieza su identificador:
//!
//! * `9…` — una aplicación empaquetada (UWP/MSIX). No hay un enlace directo: se
//!   pide una cookie al servicio de entrega, se traduce el producto a una
//!   categoría de Windows Update, se le pregunta por sus paquetes y cada uno se
//!   canjea por una URL firmada y caducable. Las dependencias (frameworks) se
//!   instalan antes que la aplicación que las necesita.
//! * `XP…` — un instalador clásico (exe/msi). El propio producto suele traer la
//!   URL, y cuando no la trae se lee del manifiesto de paquetes.
//!
//! El canal (`ring`) elige qué versión publica Windows Update: la de tienda, la
//! de Release Preview o las de los anillos Insider. La arquitectura se detecta
//! por defecto de la máquina, porque un paquete ARM64 en un equipo x64 no falla
//! al descargarse, sino al final.

use crate::download::{self, DownloadFlags};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MARKET: &str = "US";
const LOCALE: &str = "en-us";
const DEVICE_FAMILY: &str = "Windows.Desktop";

/// Con lo que se presenta la tienda ante los servicios de Microsoft. No es un
/// adorno: el servicio de entrega contesta de otra manera —o no contesta— a un
/// agente que no reconoce.
const MICROSOFT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; rv:107.0) Gecko/20100101 Firefox/107.0";

const API_TIMEOUT: Duration = Duration::from_secs(30);
const DETAILS_TTL: Duration = Duration::from_secs(600);
/// Los paquetes UWP se piden a través de una cookie con caducidad propia y las
/// URL firmadas duran poco, así que la lista se recuerda lo justo para que
/// abrir la ficha y pulsar «Instalar» no repita toda la conversación.
const PACKAGES_TTL: Duration = Duration::from_secs(300);

/// El prefijo con el que se nombran las tareas de descarga de la Microsoft
/// Store, para que no puedan chocar nunca con el `id` de una aplicación del
/// catálogo propio.
pub const TASK_PREFIX: &str = "msstore:";

/// El identificador de tarea que le corresponde a un producto.
pub fn task_id(product_id: &str) -> String {
    format!("{TASK_PREFIX}{}", product_id.to_uppercase())
}

// ---------------------------------------------------------------------------
// Canales, arquitecturas y clases de producto
// ---------------------------------------------------------------------------

/// Los canales de publicación, en el orden en el que se ofrecen.
pub const RINGS: [(&str, &str); 4] = [
    ("Retail", "Retail (Base)"),
    ("RP", "Release Preview"),
    ("WIS", "Insider Slow"),
    ("WIF", "Insider Fast"),
];

/// El canal por defecto. Release Preview publica lo mismo que la tienda salvo
/// cuando hay una corrección en camino, que es justo el caso en el que
/// interesa tenerla.
pub const DEFAULT_RING: &str = "RP";

pub fn normalize_ring(ring: &str) -> &'static str {
    match ring.trim().to_ascii_uppercase().as_str() {
        "RETAIL" => "Retail",
        "WIS" | "INSIDERSLOW" => "WIS",
        "WIF" | "INSIDERFAST" => "WIF",
        _ => "RP",
    }
}

pub fn ring_label(ring: &str) -> &'static str {
    let value = normalize_ring(ring);
    RINGS
        .iter()
        .find(|(id, _)| *id == value)
        .map(|(_, label)| *label)
        .unwrap_or("Release Preview")
}

/// La arquitectura de la máquina, tal y como la nombran los paquetes.
pub fn host_arch() -> &'static str {
    let reported = std::env::var("PROCESSOR_ARCHITEW6432")
        .or_else(|_| std::env::var("PROCESSOR_ARCHITECTURE"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    match reported.as_str() {
        "arm64" => "arm64",
        _ => "x64",
    }
}

/// Traduce la elección del usuario a la arquitectura con la que se filtra.
/// `auto` es la de la máquina y `all` no filtra nada.
pub fn resolve_arch(arch: &str) -> String {
    match arch.trim().to_ascii_lowercase().as_str() {
        "" | "auto" => host_arch().to_string(),
        other => other.to_string(),
    }
}

/// Las dos clases de producto que la tienda sabe instalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProductKind {
    Uwp,
    Win32,
}

impl ProductKind {
    pub fn from_product_id(product_id: &str) -> Option<Self> {
        let id = product_id.trim().to_uppercase();
        if id.starts_with('9') {
            Some(Self::Uwp)
        } else if id.starts_with("XP") {
            Some(Self::Win32)
        } else {
            None
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uwp => "uwp",
            Self::Win32 => "win32",
        }
    }
}

/// Deja un identificador tal y como lo esperan las APIs.
///
/// Lo que llega puede venir de un campo de texto, de una dirección de la tienda
/// o de una lista separada por comas; de todo eso sólo el primer identificador,
/// sin puntuación, significa algo.
pub fn normalize_product_id(raw: &str) -> Result<String, String> {
    let cleaned: String = raw
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == ',')
        .collect();
    let first = cleaned
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .to_uppercase();
    if first.is_empty() {
        return Err("No se indicó ningún producto de la Microsoft Store.".into());
    }
    if ProductKind::from_product_id(&first).is_none() {
        return Err(format!(
            "'{first}' no parece un identificador de producto de la Microsoft Store."
        ));
    }
    Ok(first)
}

/// El identificador que esconde una búsqueda, cuando lo hay.
///
/// Se admite pegar la dirección de la ficha en la web de Microsoft además del
/// identificador suelto: es lo que un usuario tiene a mano cuando llega desde
/// el navegador.
pub fn product_id_in_query(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.to_lowercase().starts_with("http") {
        let parsed = url::Url::parse(trimmed).ok()?;
        if !parsed.host_str()?.contains("microsoft.com") {
            return None;
        }
        let last = parsed
            .path_segments()?
            .filter(|segment| !segment.is_empty())
            .next_back()?;
        return normalize_product_id(last).ok();
    }
    if trimmed.contains(char::is_whitespace) {
        return None;
    }
    normalize_product_id(trimmed).ok()
}

// ---------------------------------------------------------------------------
// Un árbol XML mínimo
// ---------------------------------------------------------------------------

/// Lo justo de XML para leer las respuestas del servicio de entrega.
///
/// Se construye un árbol en lugar de consumir el flujo de eventos porque las
/// respuestas traen dos listas que hay que cruzar por identificador —la de
/// archivos y la de identidades—, y recorrer el documento dos veces con un
/// lector de eventos sería más código y menos claro que esto.
pub mod xml {
    use quick_xml::events::{BytesStart, Event};

    #[derive(Debug, Default, Clone)]
    pub struct Node {
        pub name: String,
        pub attributes: Vec<(String, String)>,
        pub children: Vec<Node>,
        pub text: String,
    }

    impl Node {
        pub fn attr(&self, name: &str) -> Option<&str> {
            self.attributes
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        }

        /// El primer hijo directo con ese nombre.
        pub fn child(&self, name: &str) -> Option<&Node> {
            self.children.iter().find(|child| child.name == name)
        }

        /// Todos los hijos directos con ese nombre.
        pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> {
            self.children.iter().filter(move |child| child.name == name)
        }

        /// Desciende por una cadena de hijos directos.
        pub fn path(&self, names: &[&str]) -> Option<&Node> {
            let mut current = self;
            for name in names {
                current = current.child(name)?;
            }
            Some(current)
        }

        /// El primer descendiente con ese nombre, a cualquier profundidad.
        pub fn find(&self, name: &str) -> Option<&Node> {
            if self.name == name {
                return Some(self);
            }
            self.children.iter().find_map(|child| child.find(name))
        }

        /// Todos los descendientes con ese nombre, a cualquier profundidad.
        pub fn find_all<'a>(&'a self, name: &str, found: &mut Vec<&'a Node>) {
            if self.name == name {
                found.push(self);
            }
            for child in &self.children {
                child.find_all(name, found);
            }
        }

        /// El texto del nodo y el de todo lo que cuelga de él.
        pub fn inner_text(&self) -> String {
            let mut text = self.text.clone();
            for child in &self.children {
                text.push_str(&child.inner_text());
            }
            text
        }

        /// El texto de un hijo directo, si existe y dice algo.
        pub fn text_of(&self, name: &str) -> Option<String> {
            let text = self.child(name)?.inner_text();
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    }

    /// El nombre sin su prefijo de espacio de nombres: `s:Envelope` es
    /// `Envelope`. Las respuestas usan varios prefijos para el mismo espacio y
    /// ninguno de ellos aporta nada a lo que se busca aquí.
    fn local_name(name: &str) -> &str {
        name.rsplit(':').next().unwrap_or(name)
    }

    fn resolve_entity(entity: &str) -> Option<String> {
        if let Some(number) = entity
            .strip_prefix("#x")
            .or_else(|| entity.strip_prefix("#X"))
        {
            return u32::from_str_radix(number, 16)
                .ok()
                .and_then(char::from_u32)
                .map(String::from);
        }
        if let Some(number) = entity.strip_prefix('#') {
            return number
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(String::from);
        }
        quick_xml::escape::resolve_xml_entity(entity).map(String::from)
    }

    fn node_from(start: &BytesStart) -> Result<Node, String> {
        let raw_name = String::from_utf8_lossy(start.name().as_ref()).into_owned();
        let mut attributes = Vec::new();
        for attribute in start.attributes() {
            let attribute = attribute.map_err(|error| format!("Atributo XML ilegible: {error}"))?;
            let key = String::from_utf8_lossy(attribute.key.as_ref()).into_owned();
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|error| format!("Valor de atributo XML ilegible: {error}"))?
                .into_owned();
            attributes.push((local_name(&key).to_string(), value));
        }
        Ok(Node {
            name: local_name(&raw_name).to_string(),
            attributes,
            children: Vec::new(),
            text: String::new(),
        })
    }

    fn close(open: &mut Vec<Node>, root: &mut Node, node: Node) {
        match open.last_mut() {
            Some(parent) => parent.children.push(node),
            None => root.children.push(node),
        }
    }

    fn append_text(open: &mut [Node], text: &str) {
        if let Some(node) = open.last_mut() {
            node.text.push_str(text);
        }
    }

    /// Construye el árbol de un documento.
    ///
    /// El lector va con las comprobaciones relajadas a propósito: lo que se le
    /// da es un fragmento reensamblado a partir de una respuesta SOAP, y tirar
    /// el documento entero por una etiqueta mal cerrada dejaría sin instalar
    /// una aplicación que se descarga perfectamente.
    pub fn parse(source: &str) -> Result<Node, String> {
        let mut reader = quick_xml::Reader::from_str(source);
        let config = reader.config_mut();
        config.check_end_names = false;
        config.allow_unmatched_ends = true;
        config.allow_dangling_amp = true;

        let mut root = Node {
            name: "#document".into(),
            ..Node::default()
        };
        let mut open: Vec<Node> = Vec::new();

        loop {
            match reader
                .read_event()
                .map_err(|error| format!("Respuesta XML ilegible: {error}"))?
            {
                Event::Eof => break,
                Event::Start(start) => open.push(node_from(&start)?),
                Event::Empty(start) => {
                    let node = node_from(&start)?;
                    close(&mut open, &mut root, node);
                }
                Event::End(_) => {
                    if let Some(node) = open.pop() {
                        close(&mut open, &mut root, node);
                    }
                }
                Event::Text(text) => {
                    let decoded = text
                        .decode()
                        .map_err(|error| format!("Texto XML ilegible: {error}"))?;
                    append_text(&mut open, &decoded);
                }
                Event::CData(data) => {
                    let decoded = data
                        .decode()
                        .map_err(|error| format!("Sección CDATA ilegible: {error}"))?;
                    append_text(&mut open, &decoded);
                }
                Event::GeneralRef(entity) => {
                    let name = entity
                        .decode()
                        .map_err(|error| format!("Entidad XML ilegible: {error}"))?;
                    if let Some(resolved) = resolve_entity(&name) {
                        append_text(&mut open, &resolved);
                    }
                }
                _ => {}
            }
        }

        // Lo que quedara abierto al terminar se cierra por orden, para que un
        // documento truncado conserve todo lo que sí llegó.
        while let Some(node) = open.pop() {
            close(&mut open, &mut root, node);
        }
        Ok(root)
    }
}

// ---------------------------------------------------------------------------
// Huellas
// ---------------------------------------------------------------------------

/// SHA-1, escrito aquí porque es lo único que queda del algoritmo en el
/// proyecto: los paquetes UWP siguen publicando su `Digest` con él, aunque
/// desde hace años lo acompañen de un SHA-256 en `AdditionalDigest`. Se
/// prefiere el segundo siempre que esté; esto existe para no dejar sin
/// comprobar los paquetes que sólo traen el primero.
fn sha1_bytes(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut message = data.to_vec();
    let bit_length = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    for block in message.chunks_exact(64) {
        let mut words = [0u32; 80];
        for (index, word) in block.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in words.iter().enumerate() {
            let (mixed, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999_u32),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(mixed)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }

    let mut digest = [0u8; 20];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_hex_digest(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn decode_expected_digest(expected: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let trimmed = expected.trim();
    [
        &base64::engine::general_purpose::STANDARD,
        &base64::engine::general_purpose::STANDARD_NO_PAD,
        &base64::engine::general_purpose::URL_SAFE,
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
    ]
    .iter()
    .find_map(|engine| engine.decode(trimmed).ok())
}

/// Comprueba que un archivo descargado es el que la tienda dijo que era.
///
/// Los dos formatos en los que llega la huella conviven: el catálogo Win32 la
/// da ya en hexadecimal y el de paquetes UWP en base64, así que se acepta la
/// que venga y se compara por bytes.
pub fn verify_digest(path: &Path, expected: &str, algorithm: &str) -> Result<bool, String> {
    let expected = expected.trim();
    if expected.is_empty() {
        return Ok(false);
    }
    let data = std::fs::read(path).map_err(|error| {
        format!(
            "No se pudo leer {} para comprobarlo: {error}",
            path.display()
        )
    })?;

    let normalized = algorithm.trim().to_ascii_uppercase().replace('-', "");
    let actual: Vec<u8> = match normalized.as_str() {
        "SHA1" => sha1_bytes(&data).to_vec(),
        "SHA256" => {
            use sha2::Digest;
            sha2::Sha256::digest(&data).to_vec()
        }
        _ => return Ok(false),
    };

    if is_hex_digest(expected) {
        return Ok(hex(&actual).eq_ignore_ascii_case(expected));
    }
    Ok(decode_expected_digest(expected).is_some_and(|bytes| bytes == actual))
}

// ---------------------------------------------------------------------------
// Direcciones
// ---------------------------------------------------------------------------

fn search_url(query: &str) -> String {
    format!(
        "https://apps.microsoft.com/api/products/search?gl={MARKET}&hl={LOCALE}&query={}\
&mediaType=all&age=all&price=all&category=all&subscription=all",
        urlencoding(query)
    )
}

fn details_url(product_id: &str) -> String {
    format!(
        "https://apps.microsoft.com/api/ProductsDetails/GetProductDetailsById/{product_id}\
?gl={MARKET}&hl={LOCALE}"
    )
}

fn category_url(product_id: &str) -> String {
    format!(
        "https://storeedgefd.dsx.mp.microsoft.com/v9.0/products/{product_id}\
?market={MARKET}&locale={LOCALE}&deviceFamily={DEVICE_FAMILY}"
    )
}

fn manifest_url(product_id: &str) -> String {
    format!(
        "https://storeedgefd.dsx.mp.microsoft.com/v9.0/packageManifests/{product_id}?Market={MARKET}"
    )
}

fn fe3_url(secured: bool) -> &'static str {
    if secured {
        "https://fe3.delivery.mp.microsoft.com/ClientWebService/client.asmx/secured"
    } else {
        "https://fe3.delivery.mp.microsoft.com/ClientWebService/client.asmx"
    }
}

/// Codifica un término de búsqueda para llevarlo en la dirección. Sólo se
/// necesita para esto, y traer un codificador entero por una consulta de texto
/// no compensa.
fn urlencoding(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            b' ' => encoded.push_str("%20"),
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

// ---------------------------------------------------------------------------
// Conversación con los servicios
// ---------------------------------------------------------------------------

async fn get_json(url: &str, what: &str) -> Result<Value, String> {
    let client = download::http_client()?;
    let request = client
        .get(url)
        .header("accept", "application/json")
        .header("user-agent", MICROSOFT_USER_AGENT)
        .send();
    let response = tokio::time::timeout(API_TIMEOUT, request)
        .await
        .map_err(|_| format!("La Microsoft Store no contestó a tiempo al {what}."))?
        .map_err(|error| format!("No se pudo {what}: {error}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Respuesta ilegible al {what}: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "La Microsoft Store respondió {} al {what}.",
            status.as_u16()
        ));
    }
    serde_json::from_str(&body).map_err(|error| format!("Respuesta inesperada al {what}: {error}"))
}

async fn post_soap(url: &str, body: String, what: &str) -> Result<String, String> {
    let client = download::http_client()?;
    let request = client
        .post(url)
        .header("user-agent", MICROSOFT_USER_AGENT)
        .header("accept", "*/*")
        .header("content-type", "application/soap+xml")
        .body(body)
        .send();
    let response = tokio::time::timeout(API_TIMEOUT, request)
        .await
        .map_err(|_| format!("El servicio de entrega no contestó a tiempo al {what}."))?
        .map_err(|error| format!("No se pudo {what}: {error}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| format!("Respuesta ilegible al {what}: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "El servicio de entrega respondió {} al {what}.",
            status.as_u16()
        ));
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// Búsqueda y ficha
// ---------------------------------------------------------------------------

/// Un resultado de búsqueda, ya reducido a lo que la tienda pinta en una
/// tarjeta.
#[derive(Debug, Clone, Serialize)]
pub struct StoreProduct {
    pub product_id: String,
    pub kind: &'static str,
    pub title: String,
    pub description: String,
    pub publisher: String,
    pub icon_url: Option<String>,
    pub price: String,
    pub family: Option<String>,
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Los iconos llegan a veces como una ruta sin esquema, y a veces sin ella pero
/// con una imagen equivalente en `previews`.
fn icon_of(value: &Value) -> Option<String> {
    let direct = text_field(value, "iconUrl").or_else(|| text_field(value, "pdpImageUrl"));
    let candidate = direct.or_else(|| {
        value
            .get("previews")
            .and_then(Value::as_array)
            .and_then(|previews| previews.iter().find_map(|preview| text_field(preview, "url")))
    })?;
    Some(if candidate.starts_with("//") {
        format!("https:{candidate}")
    } else {
        candidate
    })
}

fn product_from(value: &Value) -> Option<StoreProduct> {
    let product_id = text_field(value, "productId")?.to_uppercase();
    let kind = ProductKind::from_product_id(&product_id)?;
    Some(StoreProduct {
        product_id,
        kind: kind.as_str(),
        title: text_field(value, "title").unwrap_or_else(|| "Sin título".into()),
        description: text_field(value, "description").unwrap_or_default(),
        publisher: text_field(value, "publisherName").unwrap_or_default(),
        icon_url: icon_of(value),
        price: text_field(value, "displayPrice").unwrap_or_else(|| "Free".into()),
        family: text_field(value, "productFamilyName"),
    })
}

/// `true` cuando el producto puede instalarse sin pasar por caja.
///
/// La tienda no sabe comprar nada, así que un resultado de pago sólo sería una
/// tarjeta que falla al pulsarla.
fn is_free(product: &StoreProduct) -> bool {
    let price = product.price.trim();
    price.eq_ignore_ascii_case("free") || price.eq_ignore_ascii_case("gratis") || price.is_empty()
}

/// Busca en el catálogo de Microsoft y devuelve sólo lo que se puede instalar.
pub async fn search(query: &str) -> Result<Vec<StoreProduct>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let response = get_json(&search_url(query), "buscar en la Microsoft Store").await?;

    let mut products = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for key in ["highlightedList", "productsList"] {
        let Some(list) = response.get(key).and_then(Value::as_array) else {
            continue;
        };
        for entry in list {
            let Some(product) = product_from(entry) else {
                continue;
            };
            if !is_free(&product) || !seen.insert(product.product_id.clone()) {
                continue;
            }
            products.push(product);
        }
    }
    Ok(products)
}

static DETAILS_CACHE: Mutex<Option<HashMap<String, (Instant, Value)>>> = Mutex::new(None);

fn cached_details(product_id: &str) -> Option<Value> {
    let mut cache = DETAILS_CACHE.lock();
    let entries = cache.get_or_insert_with(HashMap::new);
    entries.retain(|_, (stored, _)| stored.elapsed() < DETAILS_TTL);
    entries.get(product_id).map(|(_, value)| value.clone())
}

fn remember_details(product_id: &str, details: &Value) {
    let mut cache = DETAILS_CACHE.lock();
    cache
        .get_or_insert_with(HashMap::new)
        .insert(product_id.to_string(), (Instant::now(), details.clone()));
}

/// La ficha completa de un producto, tal y como la sirve Microsoft.
///
/// Se devuelve el JSON entero: la interfaz elige qué enseñar y así un campo
/// nuevo no obliga a tocar el backend.
pub async fn details(product_id: &str) -> Result<Value, String> {
    let product_id = normalize_product_id(product_id)?;
    if let Some(cached) = cached_details(&product_id) {
        return Ok(cached);
    }
    let details = get_json(
        &details_url(&product_id),
        "consultar la ficha del producto",
    )
    .await?;
    remember_details(&product_id, &details);
    Ok(details)
}

/// La misma tarjeta que devuelve una búsqueda, para un producto que ya se
/// conoce por su identificador.
///
/// Es lo que necesita quien llega pegando la dirección de la ficha en lugar de
/// buscando un nombre: la tienda le enseña el mismo resultado que si lo hubiera
/// encontrado por su cuenta.
pub async fn product_summary(product_id: &str) -> Result<StoreProduct, String> {
    let product_id = normalize_product_id(product_id)?;
    let details = details(&product_id).await?;
    product_from(&details)
        .or_else(|| {
            // La ficha no siempre repite el identificador con el que se pidió.
            let mut with_id = details.clone();
            if let Some(object) = with_id.as_object_mut() {
                object.insert("productId".into(), Value::String(product_id.clone()));
            }
            product_from(&with_id)
        })
        .ok_or_else(|| format!("La Microsoft Store no tiene una ficha de {product_id}."))
}

// ---------------------------------------------------------------------------
// Paquetes
// ---------------------------------------------------------------------------

/// Un archivo concreto que hay que descargar e instalar.
#[derive(Debug, Clone, Serialize)]
pub struct StorePackage {
    pub product_id: String,
    pub kind: &'static str,
    /// Un framework del que depende la aplicación. Se instala antes que ella.
    pub is_dependency: bool,
    /// La dirección, cuando el catálogo ya la trae (Win32).
    pub uri: String,
    pub arch: String,
    pub file_name: String,
    pub file_type: String,
    pub size: u64,
    pub digest: Option<String>,
    pub digest_algorithm: Option<String>,
    /// La identidad con la que se canjea la dirección de descarga (UWP).
    pub update_id: Option<String>,
    pub revision_number: Option<String>,
    /// Los modificadores con los que el instalador clásico se ejecuta callado.
    pub silent_args: Option<String>,
}

impl StorePackage {
    /// El nombre con el que se guarda en disco, con su extensión.
    pub fn stored_name(&self) -> String {
        let name = if self.file_name.trim().is_empty() {
            format!("package_{}", self.product_id)
        } else {
            self.file_name.trim().to_string()
        };
        if name.to_lowercase().ends_with(&format!(".{}", self.file_type.to_lowercase())) {
            name
        } else {
            format!("{name}.{}", self.file_type)
        }
    }

    pub fn has_digest(&self) -> bool {
        self.digest.as_deref().is_some_and(|d| !d.trim().is_empty())
            && self
                .digest_algorithm
                .as_deref()
                .is_some_and(|a| !a.trim().is_empty())
    }
}

/// Las arquitecturas conocidas, en el orden en el que se buscan dentro del
/// nombre de un paquete. `arm64` va antes que `arm` porque el segundo es
/// prefijo del primero.
const KNOWN_ARCHITECTURES: [&str; 5] = ["x86", "x64", "arm64", "arm", "neutral"];

/// La arquitectura que anuncia el nombre de un paquete.
pub fn architecture_in(name: &str) -> &'static str {
    let lowered = name.to_lowercase();
    for arch in KNOWN_ARCHITECTURES {
        if lowered.contains(arch) {
            return arch;
        }
    }
    "neutral"
}

/// Se queda con los paquetes que este equipo puede instalar.
///
/// Un framework de 32 bits acompaña legítimamente a una aplicación de 64: no es
/// la arquitectura del equipo, pero sin él la aplicación no arranca.
pub fn filter_by_arch(packages: Vec<StorePackage>, arch: &str) -> Vec<StorePackage> {
    if arch.eq_ignore_ascii_case("all") {
        return packages;
    }
    let wanted = resolve_arch(arch);
    packages
        .into_iter()
        .filter(|package| {
            let package_arch = package.arch.to_lowercase();
            if package_arch == "neutral" || package_arch == wanted {
                return true;
            }
            package.is_dependency
                && ((wanted == "x64" && package_arch == "x86")
                    || (wanted == "arm64" && package_arch == "arm"))
        })
        .collect()
}

/// De los instaladores clásicos que valen para este equipo, uno.
///
/// Un producto UWP se instala con todos sus paquetes —la aplicación y los
/// frameworks de los que depende—, pero un producto Win32 es un instalador: si
/// el manifiesto publica dos que encajan, ejecutarlos ambos son dos asistentes
/// seguidos para instalar una sola aplicación. Se prefiere el de la
/// arquitectura pedida, y el que sirve para cualquiera como recambio.
pub fn best_win32(packages: Vec<StorePackage>, wanted: &str) -> Vec<StorePackage> {
    if packages.len() <= 1 {
        return packages;
    }
    let chosen = packages
        .iter()
        .find(|package| package.arch.eq_ignore_ascii_case(wanted))
        .or_else(|| packages.iter().find(|package| package.arch == "neutral"))
        .cloned();
    match chosen {
        Some(package) => vec![package],
        None => packages.into_iter().take(1).collect(),
    }
}

/// Ordena la instalación: primero las dependencias, después la aplicación.
pub fn install_order(packages: &[StorePackage]) -> Vec<StorePackage> {
    let mut ordered: Vec<StorePackage> = packages
        .iter()
        .filter(|package| package.is_dependency)
        .cloned()
        .collect();
    ordered.extend(
        packages
            .iter()
            .filter(|package| !package.is_dependency)
            .cloned(),
    );
    ordered
}

static COOKIE: Mutex<Option<String>> = Mutex::new(None);
static PACKAGES_CACHE: Mutex<Option<HashMap<String, (Instant, Vec<StorePackage>)>>> =
    Mutex::new(None);

fn packages_key(product_id: &str, ring: &str) -> String {
    format!("{product_id}-{ring}")
}

fn cached_packages(key: &str) -> Option<Vec<StorePackage>> {
    let mut cache = PACKAGES_CACHE.lock();
    let entries = cache.get_or_insert_with(HashMap::new);
    entries.retain(|_, (stored, _)| stored.elapsed() < PACKAGES_TTL);
    entries.get(key).map(|(_, packages)| packages.clone())
}

fn remember_packages(key: &str, packages: &[StorePackage]) {
    PACKAGES_CACHE
        .lock()
        .get_or_insert_with(HashMap::new)
        .insert(key.to_string(), (Instant::now(), packages.to_vec()));
}

/// Olvida la sesión con el servicio de entrega.
///
/// La cookie caduca sin avisar y el síntoma es una respuesta vacía, no un
/// error: cuando eso ocurre se tira y se pide otra antes de reintentar.
pub fn forget_session() {
    *COOKIE.lock() = None;
    *PACKAGES_CACHE.lock() = None;
}

async fn cookie() -> Result<String, String> {
    if let Some(cookie) = COOKIE.lock().clone() {
        return Ok(cookie);
    }
    let response = post_soap(
        fe3_url(false),
        crate::msstore_soap::COOKIE.to_string(),
        "abrir sesión con el servicio de entrega",
    )
    .await?;
    let document = xml::parse(&response)?;
    let encrypted = document
        .find("EncryptedData")
        .map(|node| node.inner_text().trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or("El servicio de entrega no devolvió una sesión utilizable.")?;
    *COOKIE.lock() = Some(encrypted.clone());
    Ok(encrypted)
}

/// La categoría de Windows Update que corresponde a un producto de la tienda.
async fn category_of(product_id: &str) -> Result<String, String> {
    let response = get_json(
        &category_url(product_id),
        "identificar el paquete del producto",
    )
    .await?;

    let skus = response
        .get("Payload")
        .and_then(|payload| payload.get("Skus"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for sku in skus {
        let is_full = sku
            .get("SkuType")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind.eq_ignore_ascii_case("full"));
        if !is_full {
            continue;
        }
        // `FulfillmentData` viaja como un JSON dentro de otro JSON, en texto.
        let Some(raw) = sku.get("FulfillmentData") else {
            continue;
        };
        let fulfillment: Value = match raw {
            Value::String(text) => serde_json::from_str(text).unwrap_or(Value::Null),
            other => other.clone(),
        };
        if let Some(category) = fulfillment
            .get("WuCategoryId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            return Ok(category.to_string());
        }
    }

    Err(format!(
        "El producto {product_id} no publica un paquete instalable para Windows."
    ))
}

/// La huella preferida de un archivo: SHA-256 cuando lo hay, y el SHA-1
/// histórico cuando es lo único publicado.
fn preferred_digest(file: &xml::Node) -> (Option<String>, Option<String>) {
    let mut fallback: Option<(String, String)> = None;
    for additional in file.children_named("AdditionalDigest") {
        let algorithm = additional.attr("Algorithm").unwrap_or_default().to_string();
        let value = additional.inner_text().trim().to_string();
        if value.is_empty() {
            continue;
        }
        if algorithm.to_uppercase().replace('-', "").contains("SHA256") {
            return (Some(value), Some(algorithm));
        }
        fallback.get_or_insert((value, algorithm));
    }
    if let Some((value, algorithm)) = fallback {
        return (Some(value), Some(algorithm));
    }
    (
        file.attr("Digest").map(str::to_string),
        file.attr("DigestAlgorithm").map(str::to_string),
    )
}

/// Un paquete tal y como sale de la respuesta, antes de quedarse sólo con el
/// más reciente de cada identidad.
struct UwpCandidate {
    package: StorePackage,
    identity_name: Option<String>,
    modified: Option<String>,
}

/// Lee la lista de paquetes de una respuesta de `SyncUpdates`.
///
/// La respuesta trae la información partida en dos: `ExtendedUpdateInfo` sabe
/// qué archivos hay y `NewUpdates` sabe con qué identidad se piden y para qué
/// arquitectura son. Se cruzan por el identificador de actualización.
pub fn parse_package_list(xml_text: &str, product_id: &str) -> Result<Vec<StorePackage>, String> {
    // El XML interesante viaja escapado dentro del sobre SOAP.
    let unescaped = xml_text.replace("&lt;", "<").replace("&gt;", ">");
    let document = xml::parse(&unescaped)?;
    let Some(result) = document.find("SyncUpdatesResult") else {
        return Ok(Vec::new());
    };

    let mut candidates: HashMap<String, UwpCandidate> = HashMap::new();

    if let Some(updates) = result.path(&["ExtendedUpdateInfo", "Updates"]) {
        for update in updates.children_named("Update") {
            let Some(id) = update.text_of("ID") else {
                continue;
            };
            let Some(payload) = update.child("Xml") else {
                continue;
            };
            let Some(files) = payload.child("Files") else {
                continue;
            };

            let properties = payload.child("ExtendedProperties");
            let is_framework = properties
                .and_then(|node| node.attr("IsAppxFramework"))
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            let identity_name = properties
                .and_then(|node| node.attr("PackageIdentityName"))
                .map(str::to_string);

            for file in files.children_named("File") {
                let Some(file_name) = file.attr("FileName") else {
                    continue;
                };
                let Some(package_full_name) = file.attr("InstallerSpecificIdentifier") else {
                    continue;
                };
                let file_type = file_name.rsplit('.').next().unwrap_or("appx").to_string();
                // Los paquetes cifrados (`.eappx`, `.emsix`) no se pueden
                // instalar fuera de la tienda de Windows: descargarlos sería
                // gastar el ancho de banda para nada.
                if file_type.to_lowercase().starts_with('e') {
                    continue;
                }

                let (digest, digest_algorithm) = preferred_digest(file);
                candidates.insert(
                    id.clone(),
                    UwpCandidate {
                        package: StorePackage {
                            product_id: product_id.to_string(),
                            kind: ProductKind::Uwp.as_str(),
                            is_dependency: is_framework,
                            uri: String::new(),
                            arch: "neutral".into(),
                            file_name: package_full_name.to_string(),
                            file_type,
                            size: file
                                .attr("Size")
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or(0),
                            digest,
                            digest_algorithm,
                            update_id: None,
                            revision_number: None,
                            silent_args: None,
                        },
                        identity_name: identity_name.clone(),
                        modified: file.attr("Modified").map(str::to_string),
                    },
                );
                break;
            }
        }
    }

    if let Some(new_updates) = result.child("NewUpdates") {
        for info in new_updates.children_named("UpdateInfo") {
            let Some(id) = info.text_of("ID") else {
                continue;
            };
            if !candidates.contains_key(&id) {
                continue;
            }
            let Some(payload) = info.child("Xml") else {
                continue;
            };
            let Some(identity) = payload.child("UpdateIdentity") else {
                continue;
            };

            let moniker = payload
                .path(&[
                    "ApplicabilityRules",
                    "Metadata",
                    "AppxPackageMetadata",
                    "AppxMetadata",
                ])
                .and_then(|node| node.attr("PackageMoniker"))
                .map(str::to_string);

            // La publicidad viaja en el mismo paquete que la aplicación y no es
            // parte de ella.
            if moniker
                .as_deref()
                .is_some_and(|value| value.starts_with("Microsoft.Advertising"))
            {
                candidates.remove(&id);
                continue;
            }

            let Some(candidate) = candidates.get_mut(&id) else {
                continue;
            };
            candidate.package.update_id =
                identity.attr("UpdateID").map(str::to_string);
            candidate.package.revision_number =
                identity.attr("RevisionNumber").map(str::to_string);
            if let Some(moniker) = moniker.as_deref() {
                candidate.package.arch = architecture_in(moniker).to_string();
                candidate.package.file_name = moniker.to_string();
            }
        }
    }

    Ok(latest_of_each(candidates))
}

/// De cada aplicación y arquitectura, la versión publicada más recientemente.
///
/// La respuesta arrastra versiones antiguas del mismo paquete. Instalar la que
/// llegara primero sería instalar cualquiera de ellas.
fn latest_of_each(candidates: HashMap<String, UwpCandidate>) -> Vec<StorePackage> {
    let mut latest: HashMap<String, UwpCandidate> = HashMap::new();
    for candidate in candidates.into_values() {
        // Sin identidad ni dirección de descarga no hay nada que instalar.
        if candidate.package.update_id.is_none() {
            continue;
        }
        let Some(identity_name) = candidate.identity_name.clone() else {
            continue;
        };
        let key = format!("{identity_name}-{}", candidate.package.arch);
        match latest.get(&key) {
            None => {
                latest.insert(key, candidate);
            }
            Some(existing) => {
                if is_newer(candidate.modified.as_deref(), existing.modified.as_deref()) {
                    latest.insert(key, candidate);
                }
            }
        }
    }
    let mut packages: Vec<StorePackage> = latest
        .into_values()
        .map(|candidate| candidate.package)
        .collect();
    packages.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    packages
}

/// Compara dos marcas de publicación. Vienen en ISO-8601 y con el mismo
/// formato, así que el orden alfabético es el cronológico; comparar el texto
/// evita traer un analizador de fechas para una decisión de desempate.
fn is_newer(candidate: Option<&str>, existing: Option<&str>) -> bool {
    match (candidate, existing) {
        (Some(candidate), Some(existing)) => candidate > existing,
        (Some(_), None) => true,
        _ => false,
    }
}

async fn uwp_packages(product_id: &str, ring: &str) -> Result<Vec<StorePackage>, String> {
    let category = category_of(product_id).await?;

    for attempt in 0..2 {
        let body = crate::msstore_soap::SYNC_UPDATES
            .replace("{1}", &cookie().await?)
            .replace("{2}", &category)
            .replace("{3}", ring);
        let response = post_soap(
            fe3_url(false),
            body,
            "consultar los paquetes del producto",
        )
        .await?;
        let packages = parse_package_list(&response, product_id)?;
        if !packages.is_empty() {
            return Ok(packages);
        }
        // Una sesión caducada se contesta con una lista vacía, no con un error.
        if attempt == 0 {
            *COOKIE.lock() = None;
        }
    }

    Err(format!(
        "El servicio de entrega no publicó paquetes de {product_id} en el canal {}.",
        ring_label(ring)
    ))
}

fn win32_packages_from_details(product_id: &str, details: &Value) -> Vec<StorePackage> {
    let Some(architectures) = details
        .get("installer")
        .and_then(|installer| installer.get("architectures"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };

    architectures
        .iter()
        .filter_map(|(arch, entry)| {
            let url = text_field(entry, "sourceUri").or_else(|| text_field(entry, "cdnUri"))?;
            let file_name = url.rsplit('/').next().unwrap_or("installer.exe").to_string();
            let file_type = file_name
                .rsplit_once('.')
                .map(|(_, extension)| extension.to_lowercase())
                .unwrap_or_else(|| "exe".into());
            let hash = text_field(entry, "hash").map(|value| value.to_lowercase());
            Some(StorePackage {
                product_id: product_id.to_string(),
                kind: ProductKind::Win32.as_str(),
                is_dependency: false,
                uri: url,
                arch: arch.to_lowercase(),
                file_name,
                file_type,
                size: 0,
                digest_algorithm: hash.as_ref().map(|_| "SHA256".to_string()),
                digest: hash,
                update_id: None,
                revision_number: None,
                silent_args: text_field(entry, "args").map(|args| args.replace('"', "")),
            })
        })
        .collect()
}

async fn win32_packages_from_manifest(product_id: &str) -> Result<Vec<StorePackage>, String> {
    let manifest = get_json(
        &manifest_url(product_id),
        "consultar el manifiesto del producto",
    )
    .await?;

    let versions = manifest
        .get("Data")
        .and_then(|data| data.get("Versions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut packages = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for version in versions {
        let installers = version
            .get("Installers")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for installer in installers {
            let Some(url) = text_field(&installer, "InstallerUrl") else {
                continue;
            };
            if !seen.insert(url.clone()) {
                continue;
            }
            let file_type = text_field(&installer, "InstallerType")
                .unwrap_or_else(|| {
                    url.rsplit_once('.')
                        .map(|(_, extension)| extension.to_string())
                        .unwrap_or_else(|| "exe".into())
                })
                .to_lowercase();
            if !matches!(file_type.as_str(), "exe" | "msi") {
                continue;
            }
            let url_file_name = url.rsplit('/').next().unwrap_or("installer").to_string();
            let file_name = match text_field(&installer, "InstallerLocale") {
                Some(locale) => format!("{locale}-{url_file_name}"),
                None => url_file_name,
            };
            let hash = text_field(&installer, "InstallerSha256").map(|value| value.to_lowercase());
            packages.push(StorePackage {
                product_id: product_id.to_string(),
                kind: ProductKind::Win32.as_str(),
                is_dependency: false,
                arch: text_field(&installer, "Architecture")
                    .map(|value| value.to_lowercase())
                    .unwrap_or_else(|| architecture_in(&url).to_string()),
                file_name,
                file_type,
                size: 0,
                digest_algorithm: hash.as_ref().map(|_| "SHA256".to_string()),
                digest: hash,
                update_id: None,
                revision_number: None,
                silent_args: installer
                    .get("InstallerSwitches")
                    .and_then(|switches| switches.get("Silent"))
                    .and_then(Value::as_str)
                    .map(|args| args.replace('"', "")),
                uri: url,
            });
        }
    }
    Ok(packages)
}

/// Los paquetes que hay que descargar para instalar un producto en este equipo.
pub async fn packages(
    product_id: &str,
    ring: &str,
    arch: &str,
) -> Result<Vec<StorePackage>, String> {
    let product_id = normalize_product_id(product_id)?;
    let ring = normalize_ring(ring);
    let kind = ProductKind::from_product_id(&product_id)
        .ok_or("Producto de la Microsoft Store no reconocido.")?;

    let key = packages_key(&product_id, ring);
    let all = match cached_packages(&key) {
        Some(cached) => cached,
        None => {
            let resolved = match kind {
                ProductKind::Uwp => uwp_packages(&product_id, ring).await?,
                ProductKind::Win32 => {
                    let from_details = details(&product_id)
                        .await
                        .map(|details| win32_packages_from_details(&product_id, &details))
                        .unwrap_or_default();
                    if from_details.is_empty() {
                        win32_packages_from_manifest(&product_id).await?
                    } else {
                        from_details
                    }
                }
            };
            remember_packages(&key, &resolved);
            resolved
        }
    };

    let mut filtered = filter_by_arch(all, arch);
    // Pedir «todas» las arquitecturas es pedirlas a propósito; en cualquier
    // otro caso un instalador clásico se queda en uno.
    if kind == ProductKind::Win32 && !arch.trim().eq_ignore_ascii_case("all") {
        filtered = best_win32(filtered, &resolve_arch(arch));
    }
    if filtered.is_empty() {
        return Err(format!(
            "No hay paquetes de {product_id} para {} en el canal {}.",
            resolve_arch(arch),
            ring_label(ring)
        ));
    }
    Ok(filtered)
}

/// Canjea la identidad de un paquete UWP por su dirección de descarga.
///
/// La dirección va firmada y caduca en minutos, así que se pide justo antes de
/// usarla y no se guarda en ninguna parte.
pub async fn download_url(package: &StorePackage, ring: &str) -> Result<String, String> {
    if !package.uri.trim().is_empty() {
        return Ok(package.uri.clone());
    }
    let (Some(update_id), Some(revision)) =
        (package.update_id.as_deref(), package.revision_number.as_deref())
    else {
        return Err(format!(
            "El paquete {} no indica de dónde descargarse.",
            package.file_name
        ));
    };

    let body = crate::msstore_soap::DOWNLOAD_URL
        .replace("{1}", update_id)
        .replace("{2}", revision)
        .replace("{3}", normalize_ring(ring));
    let response = post_soap(
        fe3_url(true),
        body,
        "obtener el enlace de descarga del paquete",
    )
    .await?;
    parse_download_url(&response, package.digest.as_deref())
}

/// La dirección de descarga dentro de la respuesta.
///
/// Cuando la respuesta trae varias, la que corresponde es la del archivo cuya
/// huella coincide con la del paquete que se está descargando.
pub fn parse_download_url(xml_text: &str, digest: Option<&str>) -> Result<String, String> {
    let document = xml::parse(xml_text)?;

    if let Some(digest) = digest.map(str::trim).filter(|value| !value.is_empty()) {
        let mut locations = Vec::new();
        document.find_all("FileLocation", &mut locations);
        for location in locations {
            if location
                .text_of("FileDigest")
                .is_some_and(|value| value == digest)
            {
                if let Some(url) = location.text_of("Url") {
                    return Ok(url);
                }
            }
        }
    }

    document
        .find("Url")
        .map(|node| node.inner_text().trim().to_string())
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "El servicio de entrega no devolvió un enlace de descarga.".into())
}

// ---------------------------------------------------------------------------
// Descarga e instalación
// ---------------------------------------------------------------------------

/// Dónde se guardan los paquetes de un producto mientras se instalan.
///
/// Cuelga de la carpeta de descargas de la tienda, que se vacía en cada
/// arranque: un paquete a medio bajar no sobrevive a la sesión.
pub fn download_dir(product_id: &str, ring: &str) -> PathBuf {
    crate::paths::package_download_dir(&task_id(product_id)).join(normalize_ring(ring))
}

/// La ruta que le corresponde a un paquete dentro de esa carpeta.
pub fn package_path(directory: &Path, package: &StorePackage) -> PathBuf {
    if package.is_dependency {
        directory.join("Dependencies").join(package.stored_name())
    } else {
        directory.join(package.stored_name())
    }
}

/// Descarga un paquete, reutilizando el que ya estuviera en disco cuando se
/// puede demostrar que es el mismo.
pub async fn fetch_package(
    package: &StorePackage,
    ring: &str,
    destination: &Path,
    flags: &DownloadFlags,
    on_progress: impl FnMut(u32, String, bool),
) -> Result<(), String> {
    if destination.exists() {
        let reusable = if package.has_digest() {
            verify_digest(
                destination,
                package.digest.as_deref().unwrap_or_default(),
                package.digest_algorithm.as_deref().unwrap_or_default(),
            )
            .unwrap_or(false)
        } else {
            package.size > 0
                && std::fs::metadata(destination)
                    .map(|meta| meta.len() == package.size)
                    .unwrap_or(false)
        };
        if reusable {
            crate::logger::info(
                "msstore",
                format!("Se reutiliza el paquete ya descargado {}", package.file_name),
            );
            return Ok(());
        }
        let _ = std::fs::remove_file(destination);
    }

    let url = download_url(package, ring).await?;
    download::download_url(&url, destination, flags, on_progress).await?;

    if package.has_digest() {
        let verified = verify_digest(
            destination,
            package.digest.as_deref().unwrap_or_default(),
            package.digest_algorithm.as_deref().unwrap_or_default(),
        )?;
        if !verified {
            let _ = std::fs::remove_file(destination);
            return Err(format!(
                "El paquete {} no coincide con la huella publicada por Microsoft y se ha descartado.",
                package.file_name
            ));
        }
    } else {
        crate::logger::warn(
            "msstore",
            format!(
                "Microsoft no publicó huella para {}; se instala sin comprobarla.",
                package.file_name
            ),
        );
    }
    Ok(())
}

/// Instala un paquete ya descargado.
///
/// Un paquete empaquetado lo registra Windows; un instalador clásico se ejecuta
/// con los modificadores silenciosos que publica su manifiesto, y sin ellos se
/// deja que enseñe su asistente, que es lo único que puede hacer.
pub fn install_package(package: &StorePackage, path: &Path) -> Result<(), String> {
    match ProductKind::from_product_id(&package.product_id) {
        Some(ProductKind::Uwp) => install_appx(path),
        _ => install_win32(package, path),
    }
}

fn install_appx(path: &Path) -> Result<(), String> {
    let escaped = path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; try {{ Add-AppxPackage -Path '{escaped}' \
-ForceApplicationShutdown }} catch {{ [Console]::Error.WriteLine($_.Exception.Message); exit 1 }}"
    );
    let output = crate::process::hidden_output(
        "powershell.exe",
        &[
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script.as_str(),
        ],
    )
    .map_err(|error| format!("Windows no pudo registrar el paquete: {error}"))?;

    if output.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if detail.is_empty() {
        format!(
            "Windows rechazó el paquete {} sin dar explicaciones.",
            path.display()
        )
    } else {
        format!("Windows rechazó el paquete: {detail}")
    })
}

fn install_win32(package: &StorePackage, path: &Path) -> Result<(), String> {
    let arguments: Vec<String> = package
        .silent_args
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect();

    let is_msi = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("msi"));

    let (program, arguments) = if is_msi {
        let mut parts = vec!["/i".to_string(), path.to_string_lossy().to_string()];
        if arguments.is_empty() {
            parts.push("/qn".into());
        } else {
            parts.extend(arguments);
        }
        ("msiexec.exe".to_string(), parts)
    } else {
        (path.to_string_lossy().to_string(), arguments)
    };

    let mut command = std::process::Command::new(&program);
    // Un instalador que no sabe ir callado necesita enseñar su asistente; el
    // resto no tiene por qué abrir una consola.
    if is_msi || !arguments.is_empty() {
        crate::process::background(&mut command);
    }
    command.args(&arguments);
    if let Some(parent) = path.parent() {
        command.current_dir(parent);
    }

    let status = command
        .status()
        .map_err(|error| format!("No se pudo ejecutar el instalador: {error}"))?;
    match status.code() {
        Some(0) => Ok(()),
        // 1641 y 3010 son instalaciones correctas que piden reiniciar.
        Some(1641) | Some(3010) => Ok(()),
        Some(1602) => Err("La instalación fue cancelada.".into()),
        Some(code) => Err(format!("El instalador terminó con el código {code}.")),
        None => Err("El instalador se cerró antes de terminar.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_identificador_dice_de_que_clase_es_el_producto() {
        assert_eq!(
            ProductKind::from_product_id("9WZDNCRFJ3PZ"),
            Some(ProductKind::Uwp)
        );
        assert_eq!(
            ProductKind::from_product_id("XPDC2RH70K22MN"),
            Some(ProductKind::Win32)
        );
        assert_eq!(ProductKind::from_product_id("firefox"), None);
    }

    #[test]
    fn un_identificador_se_limpia_antes_de_usarlo() {
        assert_eq!(normalize_product_id(" 9wzdncrfj3pz ").unwrap(), "9WZDNCRFJ3PZ");
        // Puntuación y sobrantes tras la primera coma.
        assert_eq!(
            normalize_product_id("9WZDNCRFJ3PZ, 9NBLGGH4NNS1").unwrap(),
            "9WZDNCRFJ3PZ"
        );
        assert!(normalize_product_id("").is_err());
        assert!(normalize_product_id("notepad").is_err());
    }

    #[test]
    fn una_direccion_de_la_tienda_lleva_dentro_el_producto() {
        assert_eq!(
            product_id_in_query("https://apps.microsoft.com/detail/9NBLGGH4NNS1?hl=es-es").as_deref(),
            Some("9NBLGGH4NNS1")
        );
        assert_eq!(
            product_id_in_query("XPDC2RH70K22MN").as_deref(),
            Some("XPDC2RH70K22MN")
        );
        // Una búsqueda normal se queda como búsqueda.
        assert_eq!(product_id_in_query("visual studio code"), None);
        assert_eq!(product_id_in_query("https://example.com/9NBLGGH4NNS1"), None);
    }

    #[test]
    fn el_canal_se_normaliza_y_tiene_nombre_visible() {
        assert_eq!(normalize_ring("retail"), "Retail");
        assert_eq!(normalize_ring("RP"), "RP");
        assert_eq!(normalize_ring("wif"), "WIF");
        // Cualquier cosa que no se reconozca cae en el canal por defecto.
        assert_eq!(normalize_ring("otro"), DEFAULT_RING);
        assert_eq!(ring_label("wis"), "Insider Slow");
    }

    #[test]
    fn la_arquitectura_se_lee_del_nombre_del_paquete() {
        assert_eq!(architecture_in("Microsoft.App_1.0_x64__8wekyb3d8bbwe"), "x64");
        assert_eq!(architecture_in("Microsoft.App_1.0_arm64__8wekyb3d8bbwe"), "arm64");
        assert_eq!(architecture_in("Microsoft.App_1.0_x86__8wekyb3d8bbwe"), "x86");
        assert_eq!(architecture_in("Microsoft.App_1.0_neutral"), "neutral");
        // Sin ninguna pista, se supone que sirve para cualquier equipo.
        assert_eq!(architecture_in("Microsoft.App_1.0"), "neutral");
    }

    fn package(arch: &str, is_dependency: bool) -> StorePackage {
        StorePackage {
            product_id: "9TEST".into(),
            kind: ProductKind::Uwp.as_str(),
            is_dependency,
            uri: String::new(),
            arch: arch.into(),
            file_name: format!("paquete_{arch}"),
            file_type: "msix".into(),
            size: 0,
            digest: None,
            digest_algorithm: None,
            update_id: Some("id".into()),
            revision_number: Some("1".into()),
            silent_args: None,
        }
    }

    #[test]
    fn se_descartan_los_paquetes_que_este_equipo_no_puede_instalar() {
        let all = vec![
            package("x64", false),
            package("arm64", false),
            package("neutral", false),
            package("x86", true),
            package("x86", false),
        ];
        let kept: Vec<String> = filter_by_arch(all.clone(), "x64")
            .into_iter()
            .map(|package| format!("{}{}", package.arch, if package.is_dependency { "-dep" } else { "" }))
            .collect();
        // El framework de 32 bits acompaña a la aplicación de 64; el programa
        // de 32 bits suelto, no.
        assert_eq!(kept, vec!["x64", "neutral", "x86-dep"]);
        assert_eq!(filter_by_arch(all, "all").len(), 5);
    }

    #[test]
    fn de_varios_instaladores_clasicos_se_ejecuta_uno_solo() {
        let win32 = |arch: &str| StorePackage {
            kind: ProductKind::Win32.as_str(),
            product_id: "XPTEST".into(),
            file_type: "exe".into(),
            ..package(arch, false)
        };
        // El de la arquitectura pedida gana.
        let chosen = best_win32(vec![win32("neutral"), win32("x64"), win32("x86")], "x64");
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].arch, "x64");
        // Sin uno exacto, el que sirve para cualquier equipo.
        let chosen = best_win32(vec![win32("x86"), win32("neutral")], "x64");
        assert_eq!(chosen[0].arch, "neutral");
        // Y sin ninguno de los dos, no se descarta lo único que hay.
        let chosen = best_win32(vec![win32("x86"), win32("arm")], "x64");
        assert_eq!(chosen.len(), 1);
        assert_eq!(chosen[0].arch, "x86");
    }

    #[test]
    fn las_dependencias_se_instalan_antes_que_la_aplicacion() {
        let packages = vec![
            package("x64", false),
            package("x86", true),
            package("x64", true),
        ];
        let ordered: Vec<bool> = install_order(&packages)
            .into_iter()
            .map(|package| package.is_dependency)
            .collect();
        assert_eq!(ordered, vec![true, true, false]);
    }

    #[test]
    fn el_nombre_guardado_lleva_siempre_su_extension() {
        let mut package = package("x64", false);
        assert_eq!(package.stored_name(), "paquete_x64.msix");
        package.file_name = "Microsoft.App_1.0_x64__hash.msix".into();
        assert_eq!(package.stored_name(), "Microsoft.App_1.0_x64__hash.msix");
        package.file_name = String::new();
        assert_eq!(package.stored_name(), "package_9TEST.msix");
    }

    #[test]
    fn el_arbol_xml_lee_atributos_texto_y_entidades() {
        let document = xml::parse(
            r#"<s:Envelope xmlns:s="http://x"><Body><Url>https://a/b?c=1&amp;d=2</Url>
               <File FileName="app.msix" Size="42"><AdditionalDigest Algorithm="SHA256">abc</AdditionalDigest></File>
               </Body></s:Envelope>"#,
        )
        .unwrap();
        let body = document.path(&["Envelope", "Body"]).unwrap();
        // La entidad se resuelve: una dirección con parámetros llega entera.
        assert_eq!(body.text_of("Url").as_deref(), Some("https://a/b?c=1&d=2"));
        let file = body.child("File").unwrap();
        assert_eq!(file.attr("FileName"), Some("app.msix"));
        assert_eq!(file.attr("Size"), Some("42"));
        assert_eq!(
            file.child("AdditionalDigest").unwrap().inner_text().trim(),
            "abc"
        );
    }

    #[test]
    fn se_prefiere_la_huella_sha256_a_la_historica() {
        let document = xml::parse(
            r#"<File Digest="viejo" DigestAlgorithm="SHA1">
                 <AdditionalDigest Algorithm="SHA1">otro</AdditionalDigest>
                 <AdditionalDigest Algorithm="SHA-256">nuevo</AdditionalDigest>
               </File>"#,
        )
        .unwrap();
        let file = document.child("File").unwrap();
        let (digest, algorithm) = preferred_digest(file);
        assert_eq!(digest.as_deref(), Some("nuevo"));
        assert_eq!(algorithm.as_deref(), Some("SHA-256"));

        // Sin SHA-256 se conserva lo que haya, para no quedarse sin comprobar.
        let solo_sha1 = xml::parse(r#"<File Digest="viejo" DigestAlgorithm="SHA1"/>"#).unwrap();
        let (digest, algorithm) = preferred_digest(solo_sha1.child("File").unwrap());
        assert_eq!(digest.as_deref(), Some("viejo"));
        assert_eq!(algorithm.as_deref(), Some("SHA1"));
    }

    const SYNC_RESPONSE: &str = r#"
<Envelope><Body><SyncUpdatesResponse><SyncUpdatesResult>
  <ExtendedUpdateInfo><Updates>
    <Update>
      <ID>1</ID>
      <Xml>
        <ExtendedProperties PackageIdentityName="Contoso.App" IsAppxFramework="false" />
        <Files><File FileName="app.msix" InstallerSpecificIdentifier="Contoso.App_1.0.0.0_x64__abc" Digest="ZGlnZXN0" DigestAlgorithm="SHA1" Size="120" Modified="2024-01-02T03:04:05.000" /></Files>
      </Xml>
    </Update>
    <Update>
      <ID>2</ID>
      <Xml>
        <ExtendedProperties PackageIdentityName="Contoso.App" IsAppxFramework="false" />
        <Files><File FileName="app.msix" InstallerSpecificIdentifier="Contoso.App_2.0.0.0_x64__abc" Digest="ZGlnZXN0" DigestAlgorithm="SHA1" Size="130" Modified="2025-06-07T08:09:10.000" /></Files>
      </Xml>
    </Update>
    <Update>
      <ID>3</ID>
      <Xml>
        <ExtendedProperties PackageIdentityName="Contoso.Runtime" IsAppxFramework="true" />
        <Files><File FileName="runtime.appx" InstallerSpecificIdentifier="Contoso.Runtime_1.0.0.0_x86__abc" Size="10" Modified="2024-01-01T00:00:00.000" /></Files>
      </Xml>
    </Update>
    <Update>
      <ID>4</ID>
      <Xml>
        <ExtendedProperties PackageIdentityName="Contoso.Secret" IsAppxFramework="false" />
        <Files><File FileName="app.eappx" InstallerSpecificIdentifier="Contoso.Secret_1.0.0.0_x64__abc" Size="10" /></Files>
      </Xml>
    </Update>
    <Update>
      <ID>5</ID>
      <Xml>
        <ExtendedProperties PackageIdentityName="Microsoft.Advertising" IsAppxFramework="false" />
        <Files><File FileName="ads.appx" InstallerSpecificIdentifier="Microsoft.Advertising.Xaml_1.0_x64__abc" Size="10" /></Files>
      </Xml>
    </Update>
  </Updates></ExtendedUpdateInfo>
  <NewUpdates>
    <UpdateInfo><ID>1</ID><Xml>
      <UpdateIdentity UpdateID="u-1" RevisionNumber="1" />
      <ApplicabilityRules><Metadata><AppxPackageMetadata><AppxMetadata PackageMoniker="Contoso.App_1.0.0.0_x64__abc" /></AppxPackageMetadata></Metadata></ApplicabilityRules>
    </Xml></UpdateInfo>
    <UpdateInfo><ID>2</ID><Xml>
      <UpdateIdentity UpdateID="u-2" RevisionNumber="3" />
      <ApplicabilityRules><Metadata><AppxPackageMetadata><AppxMetadata PackageMoniker="Contoso.App_2.0.0.0_x64__abc" /></AppxPackageMetadata></Metadata></ApplicabilityRules>
    </Xml></UpdateInfo>
    <UpdateInfo><ID>3</ID><Xml>
      <UpdateIdentity UpdateID="u-3" RevisionNumber="1" />
      <ApplicabilityRules><Metadata><AppxPackageMetadata><AppxMetadata PackageMoniker="Contoso.Runtime_1.0.0.0_x86__abc" /></AppxPackageMetadata></Metadata></ApplicabilityRules>
    </Xml></UpdateInfo>
    <UpdateInfo><ID>5</ID><Xml>
      <UpdateIdentity UpdateID="u-5" RevisionNumber="1" />
      <ApplicabilityRules><Metadata><AppxPackageMetadata><AppxMetadata PackageMoniker="Microsoft.Advertising.Xaml_1.0_x64__abc" /></AppxPackageMetadata></Metadata></ApplicabilityRules>
    </Xml></UpdateInfo>
  </NewUpdates>
</SyncUpdatesResult></SyncUpdatesResponse></Body></Envelope>"#;

    #[test]
    fn de_la_respuesta_salen_los_paquetes_instalables_y_solo_esos() {
        let packages = parse_package_list(SYNC_RESPONSE, "9TEST").unwrap();
        let names: Vec<&str> = packages
            .iter()
            .map(|package| package.file_name.as_str())
            .collect();
        // La versión antigua, el paquete cifrado, la publicidad y el paquete sin
        // identidad se quedan fuera; el framework entra marcado como tal.
        assert_eq!(
            names,
            vec![
                "Contoso.App_2.0.0.0_x64__abc",
                "Contoso.Runtime_1.0.0.0_x86__abc"
            ]
        );

        let app = &packages[0];
        assert_eq!(app.arch, "x64");
        assert_eq!(app.size, 130);
        assert_eq!(app.update_id.as_deref(), Some("u-2"));
        assert_eq!(app.revision_number.as_deref(), Some("3"));
        assert_eq!(app.file_type, "msix");
        assert!(!app.is_dependency);
        assert!(packages[1].is_dependency);
    }

    #[test]
    fn una_respuesta_sin_paquetes_no_es_un_error() {
        assert!(parse_package_list("<Envelope><Body/></Envelope>", "9TEST")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn el_enlace_de_descarga_se_elige_por_su_huella() {
        let response = r#"
<GetExtendedUpdateInfo2Response><GetExtendedUpdateInfo2Result><FileLocations>
  <FileLocation><FileDigest>uno</FileDigest><Url>https://uno/a.msix</Url></FileLocation>
  <FileLocation><FileDigest>dos</FileDigest><Url>https://dos/b.msix?t=1&amp;s=2</Url></FileLocation>
</FileLocations></GetExtendedUpdateInfo2Result></GetExtendedUpdateInfo2Response>"#;
        assert_eq!(
            parse_download_url(response, Some("dos")).unwrap(),
            "https://dos/b.msix?t=1&s=2"
        );
        // Sin huella con la que desempatar, la primera es la respuesta.
        assert_eq!(
            parse_download_url(response, None).unwrap(),
            "https://uno/a.msix"
        );
        assert!(parse_download_url("<vacio/>", None).is_err());
    }

    #[test]
    fn sha1_y_sha256_dan_los_valores_conocidos() {
        assert_eq!(
            hex(&sha1_bytes(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hex(&sha1_bytes(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        );
        // Una entrada de más de un bloque, donde el relleno importa.
        assert_eq!(
            hex(&sha1_bytes(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        use sha2::Digest;
        assert_eq!(
            hex(&sha2::Sha256::digest(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn la_huella_se_acepta_en_hexadecimal_y_en_base64() {
        let directory = std::env::temp_dir().join("winslim-msstore-test");
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("huella.bin");
        std::fs::write(&file, b"abc").unwrap();

        // Win32 la publica ya en hexadecimal.
        assert!(verify_digest(
            &file,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            "SHA256"
        )
        .unwrap());
        // UWP la publica en base64, y a veces nombra el algoritmo con guion.
        assert!(verify_digest(&file, "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=", "SHA-1").unwrap());
        // Una huella que no cuadra se rechaza, y un algoritmo desconocido
        // tampoco puede darse por bueno.
        assert!(!verify_digest(&file, "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=", "SHA256").unwrap());
        assert!(!verify_digest(&file, "loquesea", "MD5").unwrap());
        assert!(!verify_digest(&file, "  ", "SHA256").unwrap());

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn de_la_ficha_win32_salen_los_instaladores_con_sus_modificadores() {
        let details = serde_json::json!({
            "installer": {
                "architectures": {
                    "x64": {
                        "sourceUri": "https://ejemplo.test/app-x64.exe",
                        "hash": "AABBCC",
                        "args": "\"/silent\" /norestart"
                    },
                    "arm64": { "sourceUri": "" }
                }
            }
        });
        let packages = win32_packages_from_details("XPTEST", &details);
        assert_eq!(packages.len(), 1);
        let package = &packages[0];
        assert_eq!(package.arch, "x64");
        assert_eq!(package.file_name, "app-x64.exe");
        assert_eq!(package.file_type, "exe");
        assert_eq!(package.digest.as_deref(), Some("aabbcc"));
        assert_eq!(package.digest_algorithm.as_deref(), Some("SHA256"));
        assert_eq!(package.silent_args.as_deref(), Some("/silent /norestart"));
    }

    #[test]
    fn cada_producto_descarga_en_su_propia_carpeta() {
        let directory = download_dir("9WZDNCRFJ3PZ", "rp");
        assert!(directory.ends_with("RP"));
        assert!(directory
            .to_string_lossy()
            .contains("msstore_9WZDNCRFJ3PZ"));

        let dependency = package("x86", true);
        let path = package_path(&directory, &dependency);
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "Dependencies");
    }
}
