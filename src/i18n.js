/**
 * Traducción de la interfaz.
 *
 * La tienda se escribe en español y cada texto vive donde se pinta, en main.js
 * e index.html. Este módulo no cambia eso: con otro idioma elegido vigila el
 * documento y sustituye cada texto español por su traducción en cuanto se
 * inserta —el observador corre antes de que el navegador pinte, así que nunca
 * se ve el original—, igual que los atributos que se leen en pantalla. Ningún
 * render necesita saber en qué idioma está, y un texto que falte en el
 * diccionario sale en español en lugar de romper nada.
 *
 * Cada idioma es un archivo de locales/ que solo se carga cuando se elige. Sus
 * claves son el texto español tal y como se pinta; las que llevan {0}, {1}…
 * son plantillas: el hueco recoge lo variable (un nombre, un número) y se
 * traduce a su vez si es un texto conocido. Lo que no está entero en el
 * diccionario se prueba por partes: párrafos, tramos separados por « · » y
 * frases, porque así se componen los mensajes largos. Añadir un texto es
 * añadir su línea en cada archivo; añadir un idioma, su archivo y su fila aquí.
 *
 * Las descripciones del catálogo embebido también viven en los diccionarios y
 * main.js las traduce antes de pintarlas. Lo que llega de terceros —notas de
 * versión, fichas de Microsoft Store, nombres de producto— conserva
 * translate="no" porque no se puede asumir que su idioma de origen sea español.
 */

export const LANGUAGES = [
  { code: "es", name: "Español" },
  { code: "en", name: "English" },
  { code: "pt", name: "Português" },
  { code: "fr", name: "Français" },
  { code: "de", name: "Deutsch" },
  { code: "it", name: "Italiano" },
  { code: "pl", name: "Polski" },
  { code: "ro", name: "Română" },
  { code: "ru", name: "Русский" },
  { code: "uk", name: "Українська" },
  { code: "zh", name: "中文" },
  { code: "ja", name: "日本語" },
  { code: "ko", name: "한국어" },
  { code: "hi", name: "हिन्दी" },
  { code: "ar", name: "العربية", rtl: true },
];

const SOURCE_LANGUAGE = "es";
// Un Windows en un idioma que la tienda no tiene se entiende mejor en inglés.
const FALLBACK_LANGUAGE = "en";
const STORAGE_KEY = "winslimcenter-language";
const ATTRIBUTES = ["title", "aria-label", "placeholder", "alt"];
const SKIPPED = new Set(["SCRIPT", "STYLE", "TEXTAREA", "CODE", "PRE", "svg"]);
const PLACEHOLDER = /\{(\d)\}/g;
const SENTENCE_BREAK = /[.?!…]\s+[¿¡A-ZÁÉÍÓÚÑ]/;
const SEGMENT_BREAK = /\s·\s/;

const MEMO_LIMIT = 8000;

let current = SOURCE_LANGUAGE;
let preference = null;
let exact = null;
let patterns = [];
// Textos ya traducidos al idioma actual; se vacía al cambiar de idioma.
const memo = new Map();
let observer = null;
// Lo que la página escribió en cada nodo y lo que este módulo puso encima, para
// poder volver a traducir desde el original al cambiar de idioma y para
// distinguir un texto nuevo de la traducción que acaba de escribirse.
const textSources = new WeakMap();
const attributeSources = new WeakMap();

const isSupported = (code) => LANGUAGES.some((language) => language.code === code);

/** El idioma de Windows, que WebView2 expone como el del navegador. */
export function systemLanguage() {
  const tags = [...(navigator.languages || []), navigator.language].filter(Boolean);
  for (const tag of tags) {
    const base = String(tag).toLowerCase().split("-")[0];
    if (isSupported(base)) return base;
  }
  return FALLBACK_LANGUAGE;
}

/** El código elegido en la configuración, o `null` si sigue al sistema. */
export function languagePreference() {
  return preference;
}

export function currentLanguage() {
  return current;
}

/**
 * La última elección, guardada también en el navegador para traducir la
 * pantalla de arranque antes de que el backend conteste. La de verdad está en
 * settings.json, y manda en cuanto llega.
 */
export function cachedLanguagePreference() {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    return isSupported(saved) ? saved : null;
  } catch {
    return null;
  }
}

// Los diccionarios ya cargados, por idioma: volver a uno no lo descarga otra vez.
const dictionaries = new Map();

async function loadDictionary(code) {
  if (!dictionaries.has(code)) {
    dictionaries.set(
      code,
      import(`./locales/${code}.js`).then((module) => module.default).catch((error) => {
        dictionaries.delete(code);
        throw error;
      }),
    );
  }
  return dictionaries.get(code);
}

function compile(dictionary) {
  exact = null;
  patterns = [];
  memo.clear();
  if (!dictionary) return;
  exact = new Map();
  for (const [source, target] of Object.entries(dictionary)) {
    if (!target) continue;
    if (!source.includes("{")) {
      exact.set(source, target);
      continue;
    }
    const literals = source.split(PLACEHOLDER).filter((_, index) => index % 2 === 0);
    const regex = new RegExp(
      `^${source
        .split(/(\{\d\})/)
        .map((piece) => (/^\{\d\}$/.test(piece) ? "([\\s\\S]+?)" : piece.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")))
        .join("")}$`,
    );
    const order = [...source.matchAll(PLACEHOLDER)].map((match) => Number(match[1]));
    patterns.push({
      regex,
      order,
      target,
      anchor: literals.reduce((longest, piece) => (piece.trim().length > longest.length ? piece.trim() : longest), ""),
      weight: literals.join("").length,
    });
  }
  // Lo más concreto primero: «Instalar {0} aplicaciones» antes que «Instalar {0}».
  patterns.sort((a, b) => b.weight - a.weight);
}

function fromPatterns(text, depth) {
  for (const pattern of patterns) {
    if (pattern.anchor && !text.includes(pattern.anchor)) continue;
    const match = pattern.regex.exec(text);
    // Un hueco recoge un nombre, un número o una ruta, nunca otra frase ni otro
    // tramo « · »: si lo hiciera, «Se instalarán en {0}.» se tragaría la frase
    // siguiente y ésta se quedaría sin traducir. Eso se prueba luego por partes.
    if (!match || match.slice(1).some((part) => SENTENCE_BREAK.test(part) || SEGMENT_BREAK.test(part))) {
      continue;
    }
    const values = [];
    pattern.order.forEach((slot, index) => {
      const part = match[index + 1];
      values[slot] = (depth < 2 ? direct(part, depth + 1) : null) ?? part;
    });
    const result = pattern.target.replace(PLACEHOLDER, (_, slot) => values[Number(slot)] ?? "");
    // Una plantilla que lo deja todo igual («{0} → {1}» sobre texto sin
    // traducir) no es un acierto: que lo intenten las demás.
    if (result !== text) return result;
  }
  return null;
}

function direct(text, depth = 0) {
  const hit = exact.get(text);
  if (hit !== undefined) return hit;
  const collapsed = text.replace(/\s+/g, " ").trim();
  if (!collapsed) return null;
  return exact.get(collapsed) ?? fromPatterns(collapsed, depth);
}

/** Traduce trozo a trozo; `null` si ninguno cambió. */
function byParts(text, separator, translatePart) {
  const pieces = text.split(separator);
  let changed = false;
  const result = pieces.map((piece, index) => {
    // split() con un grupo deja los separadores en los índices impares.
    if (index % 2 === 1 || !piece.trim()) return piece;
    const translated = translatePart(piece);
    if (translated === null || translated === piece) return piece;
    changed = true;
    return translated;
  });
  return changed ? result.join("") : null;
}

function lookup(text) {
  if (/\n\s*\n/.test(text)) {
    return byParts(text, /(\n\s*\n)/, (part) => lookup(part.trim()));
  }
  const whole = direct(text);
  if (whole !== null) return whole;
  if (SEGMENT_BREAK.test(text)) {
    const segments = byParts(text, /(\s+·\s+)/, lookup);
    if (segments !== null) return segments;
  }
  const collapsed = text.replace(/\s+/g, " ").trim();
  if (SENTENCE_BREAK.test(collapsed)) {
    return byParts(collapsed, /((?<=[.?!…])\s+(?=[¿¡A-ZÁÉÍÓÚÑ]))/, (sentence) => direct(sentence));
  }
  return null;
}

/** Un texto de la interfaz en el idioma actual (el original si no hay traducción). */
export function translate(text) {
  if (!exact || typeof text !== "string" || !/\p{L}/u.test(text)) return text;
  // Cada tarjeta repite los mismos botones, autores y versiones, y cada
  // render vuelve a pintarlos: lo ya resuelto no se busca otra vez.
  const cached = memo.get(text);
  if (cached !== undefined) return cached;
  const [, lead, core, trail] = /^(\s*)([\s\S]*?)(\s*)$/.exec(text);
  const translated = lookup(core);
  const result = translated === null ? text : lead + translated + trail;
  if (memo.size >= MEMO_LIMIT) memo.clear();
  memo.set(text, result);
  return result;
}

function excluded(element) {
  return !element || Boolean(element.closest?.('[translate="no"], textarea, code, pre, script, style'));
}

/**
 * El texto que toca mostrar para lo que hay ahora en un nodo o atributo. Si lo
 * que hay no es lo último que se escribió aquí, la página puso un texto nuevo y
 * ése pasa a ser el original. `null` cuando ya está como debe.
 */
function refreshRecord(record, value) {
  if (record && record.output === value) {
    if (record.language === current) return null;
  } else {
    record = { source: value };
  }
  record.language = current;
  record.output = translate(record.source);
  return record;
}

function translateTextNode(node) {
  const value = node.data;
  if (!value.trim()) return;
  const record = refreshRecord(textSources.get(node), value);
  if (!record) return;
  textSources.set(node, record);
  if (record.output !== value) node.data = record.output;
}

function translateAttributes(element) {
  for (const name of ATTRIBUTES) {
    const value = element.getAttribute(name);
    if (!value) continue;
    let records = attributeSources.get(element);
    if (!records) attributeSources.set(element, (records = {}));
    const record = refreshRecord(records[name], value);
    if (!record) continue;
    records[name] = record;
    if (record.output !== value) element.setAttribute(name, record.output);
  }
}

function translateTree(root) {
  if (root.nodeType === Node.TEXT_NODE) {
    if (!excluded(root.parentElement)) translateTextNode(root);
    return;
  }
  if (root.nodeType !== Node.ELEMENT_NODE || excluded(root)) return;
  translateAttributes(root);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
    acceptNode(node) {
      if (node.nodeType === Node.TEXT_NODE) return NodeFilter.FILTER_ACCEPT;
      if (SKIPPED.has(node.nodeName) || node.getAttribute("translate") === "no") return NodeFilter.FILTER_REJECT;
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeType === Node.TEXT_NODE) translateTextNode(node);
    else translateAttributes(node);
  }
}

function onMutations(records) {
  for (const record of records) {
    if (record.type === "childList") {
      record.addedNodes.forEach((node) => {
        if (node.isConnected) translateTree(node);
      });
    } else if (record.type === "characterData") {
      if (!excluded(record.target.parentElement)) translateTextNode(record.target);
    } else if (record.type === "attributes" && !excluded(record.target)) {
      translateAttributes(record.target);
    }
  }
}

// Cada elección invalida las anteriores que aún estén cargando su diccionario:
// gana la última, no la que tarde menos en llegar.
let request = 0;

/**
 * Aplica una elección de idioma (`null` = el del sistema) y vuelve a traducir
 * lo que ya hay en pantalla. Responde si el idioma efectivo cambió.
 */
export async function setLanguagePreference(code) {
  const metricStartedAt = globalThis.performance?.now?.();
  try {
    const ticket = ++request;
    preference = isSupported(code) ? code : null;
    try {
      if (preference) localStorage.setItem(STORAGE_KEY, preference);
      else localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Sin almacenamiento el arranque sale un instante en el idioma anterior.
    }
    const next = preference || systemLanguage();
    if (next === current && (observer || next === SOURCE_LANGUAGE)) return false;
    let dictionary = null;
    if (next !== SOURCE_LANGUAGE) {
      try {
        dictionary = await loadDictionary(next);
      } catch (error) {
        // Sin su archivo, el idioma que hubiera sigue en pantalla.
        console.warn(`[i18n] No se pudo cargar el idioma ${next}:`, error);
        return false;
      }
    }
    if (ticket !== request) return false;
    const changed = next !== current;
    current = next;
    observer?.disconnect();
    observer = null;
    compile(dictionary);
    document.documentElement.lang = next;
    // El árabe se lee de derecha a izquierda: la interfaz entera se refleja.
    document.documentElement.dir = LANGUAGES.find((language) => language.code === next)?.rtl ? "rtl" : "ltr";
    // Con el español también se recorre: deshace lo que hubiera traducido antes.
    translateTree(document.documentElement);
    if (exact) {
      observer = new MutationObserver(onMutations);
      observer.observe(document.documentElement, {
        subtree: true,
        childList: true,
        characterData: true,
        attributes: true,
        attributeFilter: ATTRIBUTES,
      });
    }
    return changed;
  } finally {
    if (metricStartedAt != null) {
      globalThis.window?.__recordFrontendMetric?.("traduccion", metricStartedAt, {
        idioma: isSupported(code) ? code : "automatico",
      });
    }
  }
}
