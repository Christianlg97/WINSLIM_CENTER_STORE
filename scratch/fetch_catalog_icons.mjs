// Descarga el icono de cada entrada del catálogo a src/assets/catalog para que
// la tienda no dependa de internet: los iconos viajan dentro del .exe.
//
//   node scratch/fetch_catalog_icons.mjs            solo descarga y reporta
//   node scratch/fetch_catalog_icons.mjs --apply    además reescribe apps.json
//   node scratch/fetch_catalog_icons.mjs --force    vuelve a bajar lo ya guardado
//
// Cada aplicación intenta varias fuentes en orden y se queda con la primera que
// devuelve una imagen válida. El informe final nombra a las que se han quedado
// sin icono para poder resolverlas a mano.
import { execFile } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = path.resolve(SCRIPT_DIR, "..");
const CATALOG_PATH = path.join(ROOT_DIR, "src-tauri", "apps.json");
const ICON_DIR = path.join(ROOT_DIR, "src", "assets", "catalog");
const ICON_PREFIX = "assets/catalog";
const REPORT_PATH = path.join(SCRIPT_DIR, ".icon-report.json");
const APPLY = process.argv.includes("--apply");
const FORCE = process.argv.includes("--force");
// `--only=steam,discord` vuelve a resolver solo esas entradas, sin tocar el resto.
const ONLY = new Set(
  (process.argv.find((argument) => argument.startsWith("--only="))?.slice("--only=".length) || "")
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean),
);
const CONCURRENCY = 6;
const MIN_ICON_SIDE = 32;
const TIMEOUT_MS = 25_000;
const USER_AGENT =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Safari/537.36";

// Cloudflare rechaza el TLS de Node en esta máquina y el almacén de raíces de
// Windows no reconoce su cadena, así que cdn.simpleicons.org es inalcanzable.
// El repositorio oficial vive en GitHub, que sí responde, y sirve exactamente
// el mismo dibujo: solo hay que aplicarle el color que pedía la URL.
const SIMPLE_ICONS_BRANCHES = ["master", "develop"];
const DASHBOARD_ICONS_REPOS = [
  "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main",
  "https://raw.githubusercontent.com/walkxcode/dashboard-icons/main",
];

// Aplicaciones cuyo icono no aparece en ninguna colección y cuyo sitio solo
// publica un favicon de 16 px: aquí se nombra a mano la imagen buena, que es
// siempre la que el propio fabricante usa para el producto.
const ICON_OVERRIDES = {
  // El icono de los libros, tal cual lo sirve win-rar.com para iOS (180 px).
  winrar: "https://www.win-rar.com/fileadmin/images/apple-touch-icon.png",
  // Icono de producto de IObit, el mismo candado de la ficha de Unlocker.
  iobit_unlocker: "https://www.iobit.com/tpl/images/product-icons/unlocker_96.png",
  // Bandisoft solo publica 16 px como favicon; su icono para iOS es el bueno.
  bandizip: "https://bandisoft.com/apple-touch-icon.png",
  playnite: "https://playnite.link/applogo.png",
  revo_uninstaller: "https://upload.wikimedia.org/wikipedia/commons/8/83/Revouninstallerpro_icon.png",
  jdownloader: "https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/png/jdownloader.png",
  // La entrada no fija color de fondo, así que la silueta va en el rojo de la
  // marca en lugar del blanco habitual: sobre el plata del tema se lee.
  redis_cli: {
    url: "https://raw.githubusercontent.com/simple-icons/simple-icons/master/icons/redis.svg",
    tint: "DC382D",
  },
  // Los tres siguientes solo existen como logotipo alargado; el catálogo les
  // pone `icon_fit: contain` para que se vean enteros.
  gforth: "https://upload.wikimedia.org/wikipedia/commons/a/a5/Gforth_Logo.png",
  coretemp: "https://www.alcpu.com/CoreTemp/main_data/cortemptitle.png",
  disk_genius: "https://www.diskgenius.com/public/images/dg-logo.png",
};

function isLocalIcon(url) {
  return typeof url === "string" && url.startsWith("assets/");
}

/// El formato real del archivo, leído de sus primeros bytes.
///
/// El servidor puede mentir en el `Content-Type` o devolver una página de error
/// con estado 200, así que la extensión sale siempre de aquí.
function sniffExtension(buffer) {
  if (buffer.length < 16) return null;
  if (buffer.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]))) return "png";
  if (buffer[0] === 0xff && buffer[1] === 0xd8 && buffer[2] === 0xff) return "jpg";
  if (buffer.subarray(0, 6).toString("ascii") === "GIF89a" || buffer.subarray(0, 6).toString("ascii") === "GIF87a") {
    return "gif";
  }
  if (buffer.subarray(0, 4).toString("ascii") === "RIFF" && buffer.subarray(8, 12).toString("ascii") === "WEBP") {
    return "webp";
  }
  if (buffer[0] === 0x00 && buffer[1] === 0x00 && buffer[2] === 0x01 && buffer[3] === 0x00) return "ico";
  if (buffer[0] === 0x42 && buffer[1] === 0x4d) return "bmp";
  const head = buffer.subarray(0, 512).toString("utf8").trim().toLowerCase();
  if (head.startsWith("<?xml") || head.startsWith("<svg") || head.startsWith("<!doctype svg")) {
    return buffer.toString("utf8").toLowerCase().includes("<svg") ? "svg" : null;
  }
  return null;
}

/// El mayor de los dibujos que guarda un `.ico`.
///
/// Un icono de Windows es una lista de imágenes y casi siempre abre por la de
/// 16 px; quedarse con la primera descartaría archivos que traen 256 px detrás.
function icoSize(buffer) {
  const count = buffer.length > 6 ? buffer.readUInt16LE(4) : 0;
  let best = { width: 0, height: 0 };
  for (let index = 0; index < count; index += 1) {
    const entry = 6 + index * 16;
    if (entry + 16 > buffer.length) break;
    const width = buffer[entry] === 0 ? 256 : buffer[entry];
    const height = buffer[entry + 1] === 0 ? 256 : buffer[entry + 1];
    if (width > best.width) best = { width, height };
  }
  return best;
}

/// Las dimensiones del JPEG viven en su marcador de inicio de trama.
function jpegSize(buffer) {
  let offset = 2;
  while (offset + 9 < buffer.length) {
    if (buffer[offset] !== 0xff) {
      offset += 1;
      continue;
    }
    const marker = buffer[offset + 1];
    const length = buffer.readUInt16BE(offset + 2);
    // Todos los SOF salvo los de tablas y reinicio describen la imagen.
    if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) {
      return { width: buffer.readUInt16BE(offset + 7), height: buffer.readUInt16BE(offset + 5) };
    }
    offset += 2 + length;
  }
  return { width: 0, height: 0 };
}

function webpSize(buffer) {
  const format = buffer.subarray(12, 16).toString("ascii");
  if (format === "VP8X" && buffer.length > 30) {
    return {
      width: 1 + (buffer[24] | (buffer[25] << 8) | (buffer[26] << 16)),
      height: 1 + (buffer[27] | (buffer[28] << 8) | (buffer[29] << 16)),
    };
  }
  if (format === "VP8 " && buffer.length > 30) {
    return { width: buffer.readUInt16LE(26) & 0x3fff, height: buffer.readUInt16LE(28) & 0x3fff };
  }
  if (format === "VP8L" && buffer.length > 25) {
    const bits = buffer.readUInt32LE(21);
    return { width: 1 + (bits & 0x3fff), height: 1 + ((bits >> 14) & 0x3fff) };
  }
  return { width: 0, height: 0 };
}

/// Las dimensiones sirven para detectar iconos demasiado pobres para una ficha.
function imageSize(buffer, ext) {
  if (ext === "png" && buffer.length > 24) {
    return { width: buffer.readUInt32BE(16), height: buffer.readUInt32BE(20) };
  }
  if (ext === "ico") return icoSize(buffer);
  if (ext === "jpg") return jpegSize(buffer);
  if (ext === "gif" && buffer.length > 10) {
    return { width: buffer.readUInt16LE(6), height: buffer.readUInt16LE(8) };
  }
  if (ext === "webp" && buffer.length > 16) return webpSize(buffer);
  if (ext === "bmp" && buffer.length > 26) {
    return { width: buffer.readUInt32LE(18), height: buffer.readUInt32LE(22) };
  }
  if (ext === "svg") {
    // Un vector no tiene resolución: cualquier tamaño de ficha le vale.
    return { width: Infinity, height: Infinity };
  }
  return { width: 0, height: 0 };
}

async function fetchWithNode(url) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const response = await fetch(url, {
      signal: controller.signal,
      redirect: "follow",
      headers: { "user-agent": USER_AGENT, accept: "image/*,*/*;q=0.8" },
    });
    if (!response.ok) return { error: `HTTP ${response.status}` };
    return { buffer: Buffer.from(await response.arrayBuffer()) };
  } catch (error) {
    return { error: error.cause?.code || error.message };
  } finally {
    clearTimeout(timer);
  }
}

/// Segundo intento con curl: Node y Schannel no fallan ante los mismos
/// servidores, y lo que uno rechaza el otro suele traerlo sin problema.
async function fetchWithCurl(url) {
  const target = path.join(tmpdir(), `winslim-icon-${process.pid}-${Math.random().toString(36).slice(2)}`);
  try {
    await execFileAsync("curl", [
      "-sSL", "--fail", "--max-time", String(Math.round(TIMEOUT_MS / 1000)),
      "-A", USER_AGENT, "-o", target, url,
    ]);
    return { buffer: await readFile(target) };
  } catch (error) {
    return { error: `curl: ${String(error.stderr || error.message).trim().slice(0, 120)}` };
  } finally {
    await rm(target, { force: true });
  }
}

async function download(url) {
  const first = await fetchWithNode(url);
  if (first.buffer) return first;
  const second = await fetchWithCurl(url);
  if (second.buffer) return second;
  return { error: `${first.error} / ${second.error}` };
}

function tintSvg(svg, color) {
  const named = { white: "#ffffff", black: "#000000" };
  const fill = named[color?.toLowerCase()] || (/^[0-9a-f]{3,8}$/i.test(color || "") ? `#${color}` : color);
  if (!fill) return svg;
  return svg.replace(/<svg\b([^>]*)>/i, (match, attributes) =>
    `<svg${attributes.replace(/\s+fill="[^"]*"/gi, "")} fill="${fill}">`,
  );
}

function hostOf(url) {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return null;
  }
}

/// El dominio que un servicio de favicon lleva dentro de su propia dirección.
///
/// Muchas entradas solo se instalan por WinGet y no nombran ningún sitio: su
/// única pista sobre la marca es el dominio que ya viajaba en el `icon_url`.
function domainInsideIconService(url) {
  try {
    const parsed = new URL(url);
    if (parsed.hostname.endsWith("google.com")) return parsed.searchParams.get("domain");
    if (parsed.hostname === "icons.duckduckgo.com") {
      return parsed.pathname.replace(/^\/ip3\//, "").replace(/\.ico$/, "") || null;
    }
    if (parsed.hostname === "logo.clearbit.com") return decodeURIComponent(parsed.pathname.slice(1)) || null;
  } catch {
    // Una dirección ilegible simplemente no aporta dominio.
  }
  return null;
}

/// Los dominios de marca de la aplicación, para los servicios de favicon.
///
/// Se prueba también el dominio sin subdominio: Google no conoce
/// `downloads.ntlite.com`, pero sí `ntlite.com`.
function brandDomains(app) {
  const inherited = domainInsideIconService(app.icon_url || "");
  const hosts = [
    hostOf(app.web_url),
    hostOf(app.homepage),
    hostOf(app.download_url),
    inherited?.replace(/^www\./, ""),
    hostOf(app.icon_url),
  ].filter((host) => host && !host.endsWith("github.com") && !host.endsWith("githubusercontent.com"));

  const domains = [];
  for (const host of hosts) {
    if (!domains.includes(host)) domains.push(host);
    const labels = host.split(".");
    const base = labels.length > 2 ? labels.slice(-2).join(".") : host;
    if (!domains.includes(base)) domains.push(base);
  }
  return domains;
}

function slugVariants(app) {
  const raw = [app.id, app.name].filter(Boolean);
  const slugs = new Set();
  for (const value of raw) {
    const slug = String(value)
      .toLowerCase()
      .normalize("NFD")
      .replace(/\p{Diacritic}/gu, "")
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "");
    if (slug) {
      slugs.add(slug);
      slugs.add(slug.replace(/-/g, ""));
    }
  }
  return [...slugs];
}

function dashboardIconCandidates(app) {
  const candidates = [];
  for (const slug of slugVariants(app)) {
    for (const repo of DASHBOARD_ICONS_REPOS) {
      candidates.push({ url: `${repo}/svg/${slug}.svg`, source: "dashboard-icons" });
      candidates.push({ url: `${repo}/png/${slug}.png`, source: "dashboard-icons" });
    }
  }
  return candidates;
}

function faviconCandidates(app) {
  const candidates = [];
  for (const domain of brandDomains(app)) {
    const encoded = encodeURIComponent(domain);
    candidates.push({ url: `https://www.google.com/s2/favicons?domain=${encoded}&sz=256`, source: "google" });
    candidates.push({ url: `https://icons.duckduckgo.com/ip3/${encoded}.ico`, source: "duckduckgo" });
  }
  return candidates;
}

/// Las fuentes que se probarán, de la más fiel a la más genérica.
function candidatesFor(app) {
  const candidates = [];
  const iconUrl = app.icon_url || "";
  const host = hostOf(iconUrl);

  const override = ICON_OVERRIDES[app.id];
  if (override) {
    candidates.push({ source: "manual", ...(typeof override === "string" ? { url: override } : override) });
  }

  if (host === "cdn.simpleicons.org") {
    const [slug, color] = new URL(iconUrl).pathname.split("/").filter(Boolean);
    for (const branch of SIMPLE_ICONS_BRANCHES) {
      candidates.push({
        url: `https://raw.githubusercontent.com/simple-icons/simple-icons/${branch}/icons/${slug}.svg`,
        source: "simple-icons",
        tint: color,
      });
    }
  }

  const isGenericService = host === "www.google.com" || host === "icons.duckduckgo.com" ||
    host === "logo.clearbit.com";
  // Un favicon del catálogo es un apaño, no una elección: si existe el icono
  // real de la aplicación en dashboard-icons, se prefiere ese.
  if (isGenericService) candidates.push(...dashboardIconCandidates(app));

  if (/^https?:/i.test(iconUrl)) candidates.push({ url: iconUrl, source: "catalog" });

  if (!isGenericService) candidates.push(...dashboardIconCandidates(app));
  candidates.push(...faviconCandidates(app));

  const seen = new Set();
  return candidates.filter((candidate) => {
    if (seen.has(candidate.url)) return false;
    seen.add(candidate.url);
    return true;
  });
}

function existingIconFor(appId, files) {
  const match = files.find((name) => name.replace(/\.[^.]+$/, "") === appId);
  return match ? path.posix.join(ICON_PREFIX, match) : null;
}

async function resolveIcon(app, existingFiles) {
  const revisit = FORCE || ONLY.has(app.id);
  if (isLocalIcon(app.icon_url) && !revisit) {
    // El archivo puede haber cambiado de extensión al optimizarlo; entonces
    // manda el que esté en disco con el identificador de la aplicación.
    if (existsSync(path.join(ROOT_DIR, "src", app.icon_url))) {
      return { id: app.id, status: "local", iconUrl: app.icon_url, source: "repo" };
    }
    const renamed = existingIconFor(app.id, existingFiles);
    if (renamed) return { id: app.id, status: "local", iconUrl: renamed, source: "repo" };
  }
  if (!revisit) {
    const existing = existingIconFor(app.id, existingFiles);
    if (existing) return { id: app.id, status: "cached", iconUrl: existing, source: "cache" };
    // Con `--only` el resto del catálogo se deja tal cual, incluido el hueco.
    if (ONLY.size) return { id: app.id, name: app.name, status: "failed", attempts: ["omitido por --only"] };
  }

  const attempts = [];
  // Un icono diminuto se ve borroso en una ficha de 180 px, así que primero se
  // agotan las fuentes buscando uno digno; si ninguna lo da, vale más el mejor
  // de los pequeños que dejar la aplicación sin cara.
  let smallest = null;
  // Una aplicación web se instala como acceso directo y Windows solo sabe poner
  // mapas de bits en un `.lnk`: para ellas un SVG es el último recurso.
  const prefersRaster = app.source_type === "webapp";
  let vector = null;
  for (const candidate of candidatesFor(app)) {
    const result = await download(candidate.url);
    if (result.error) {
      attempts.push(`${candidate.url} -> ${result.error}`);
      continue;
    }
    let buffer = result.buffer;
    const ext = sniffExtension(buffer);
    if (!ext) {
      attempts.push(`${candidate.url} -> formato no reconocido (${buffer.length} bytes)`);
      continue;
    }
    if (ext === "svg" && candidate.tint) {
      buffer = Buffer.from(tintSvg(buffer.toString("utf8"), candidate.tint), "utf8");
    }
    const size = imageSize(buffer, ext);
    const side = Math.max(size.width, size.height);
    if (ext === "svg" && prefersRaster) {
      attempts.push(`${candidate.url} -> vectorial, reservado por si no aparece un mapa de bits`);
      vector ||= { buffer, ext, size, candidate };
      continue;
    }
    if (side < MIN_ICON_SIDE) {
      attempts.push(`${candidate.url} -> demasiado pequeño (${size.width}x${size.height})`);
      if (side > 0 && side > (smallest?.side || 0)) smallest = { buffer, ext, size, side, candidate };
      continue;
    }
    return saveIcon(app, buffer, ext, size, candidate, attempts);
  }
  for (const spare of [vector, smallest]) {
    if (spare) return saveIcon(app, spare.buffer, spare.ext, spare.size, spare.candidate, attempts);
  }
  return { id: app.id, name: app.name, status: "failed", attempts };
}

async function saveIcon(app, buffer, ext, size, candidate, attempts) {
  const fileName = `${app.id}.${ext}`;
  // Cambiar de formato dejaría atrás el archivo anterior y `existingIconFor`
  // podría luego quedarse con el equivocado.
  for (const stale of await readdir(ICON_DIR)) {
    if (stale.replace(/\.[^.]+$/, "") === app.id && stale !== fileName) {
      await rm(path.join(ICON_DIR, stale), { force: true });
    }
  }
  await writeFile(path.join(ICON_DIR, fileName), buffer);
  return {
    id: app.id,
    status: "downloaded",
    iconUrl: path.posix.join(ICON_PREFIX, fileName),
    source: candidate.source,
    from: candidate.url,
    bytes: buffer.length,
    width: Number.isFinite(size.width) ? size.width : null,
    height: Number.isFinite(size.height) ? size.height : null,
    attempts: attempts.length ? attempts : undefined,
  };
}

async function mapWithLimit(items, limit, worker) {
  const results = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, async () => {
      while (next < items.length) {
        const index = next++;
        results[index] = await worker(items[index], index);
      }
    }),
  );
  return results;
}

/// Reescribe solo el valor de cada `icon_url`.
///
/// El archivo mezcla finales de línea, así que volver a serializarlo entero
/// ensuciaría el diff con miles de líneas que nadie ha tocado.
function rewriteCatalog(raw, catalog, resolutions) {
  const pattern = /("icon_url"\s*:\s*")((?:[^"\\]|\\.)*)(")/g;
  let index = 0;
  const updated = raw.replace(pattern, (match, prefix, value, suffix) => {
    const app = catalog[index];
    const resolution = resolutions[index];
    index += 1;
    if (!app || JSON.parse(`"${value}"`) !== app.icon_url) {
      throw new Error(`Desajuste al reescribir icon_url en la posición ${index}`);
    }
    if (!resolution?.iconUrl) return match;
    return `${prefix}${resolution.iconUrl}${suffix}`;
  });
  if (index !== catalog.length) {
    throw new Error(`Se esperaban ${catalog.length} icon_url y se han encontrado ${index}`);
  }
  return updated;
}

async function main() {
  const raw = await readFile(CATALOG_PATH, "utf8");
  const catalog = JSON.parse(raw);
  await mkdir(ICON_DIR, { recursive: true });
  const existingFiles = await readdir(ICON_DIR);

  const resolutions = await mapWithLimit(catalog, CONCURRENCY, (app) => resolveIcon(app, existingFiles));

  const failed = resolutions.filter((item) => item.status === "failed");
  const downloaded = resolutions.filter((item) => item.status === "downloaded");
  const small = downloaded.filter((item) => item.width && item.width < 128);
  await writeFile(REPORT_PATH, JSON.stringify({ resolutions }, null, 2), "utf8");

  console.log(
    `[iconos] ${catalog.length} aplicaciones: ${downloaded.length} descargadas, ` +
      `${resolutions.filter((item) => item.status === "cached").length} ya en disco, ` +
      `${resolutions.filter((item) => item.status === "local").length} propias del repo, ` +
      `${failed.length} sin icono.`,
  );
  if (small.length) {
    console.log(`[iconos] Por debajo de 128 px: ${small.map((item) => `${item.id}(${item.width})`).join(", ")}`);
  }
  for (const item of failed) {
    console.log(`[iconos] SIN ICONO ${item.id} (${item.name})`);
    for (const attempt of item.attempts.slice(0, 4)) console.log(`           ${attempt}`);
  }

  if (!APPLY) {
    console.log(`[iconos] Informe en ${path.relative(ROOT_DIR, REPORT_PATH)}. Usa --apply para actualizar apps.json.`);
    return;
  }
  if (failed.length) {
    throw new Error("No se actualiza apps.json mientras haya aplicaciones sin icono.");
  }
  await writeFile(CATALOG_PATH, rewriteCatalog(raw, catalog, resolutions), "utf8");
  console.log("[iconos] apps.json actualizado con rutas locales.");
}

await main();
