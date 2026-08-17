const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const NAV = [
  { id: "home", label: "Inicio", icon: "🏠", filter: null },
  { id: "featured", label: "Destacados", icon: "🔥", filter: "featured" },
  { id: "updates", label: "Actualizaciones", icon: "🚀", filter: "__UPDATES__" },
  { id: "games", label: "Juegos", icon: "🎮", filter: "Juegos" },
  { id: "emulators", label: "Emuladores", icon: "🕹️", filter: "Emuladores" },
  { id: "browsers", label: "Navegadores", icon: "🌐", filter: "Navegadores" },
  { id: "dev", label: "Desarrollo", icon: "💻", filter: "Desarrollo" },
  { id: "ai", label: "IA", icon: "🤖", filter: "IA" },
  { id: "utils", label: "Utilidades", icon: "🛠️", filter: "Utilidades" },
  { id: "multimedia", label: "Multimedia", icon: "🎬", filter: "Multimedia" },
  { id: "product", label: "Productividad", icon: "📝", filter: "Productividad" },
  { id: "social", label: "Social y Comunicación", icon: "💬", filter: "Social y Comunicación" },
  { id: "installed", label: "Mis aplicaciones", icon: "✅", filter: "__INSTALLED__" },
];

const FEATURED_ORDER = [
  "winslim_terminal", "powertoys", "vscode", "brave", "seven_zip",
  "vlc", "obs_studio", "rustdesk", "steam", "discord",
];

const FEATURED_RANK = new Map(FEATURED_ORDER.map((id, index) => [id, index]));

// Keep hostname normalisation outside the hot icon path. The catalog contains
// hundreds of applications and rebuilding this object for every card costs
// more than the lookup itself.
const ICON_DOMAIN_MAP = Object.freeze({
  "cdn.akamai.steamstatic.com": "steampowered.com",
  "ubistatic3-a.akamaihd.net": "ubisoft.com",
  "origin-a.akamaihd.net": "ea.com",
  "dl.google.com": "chrome.com",
  "c2rsetup.officeapps.live.com": "visualstudio.microsoft.com",
  "dl.pstmn.io": "postman.com",
  "desktop.docker.com": "docker.com",
  "updates.signal.org": "signal.org",
  "download.cpuid.com": "cpuid.com",
  "www.techpowerup.com": "techpowerup.com",
  "osdn.net": "crystalmark.info",
  "buildbot.libretro.com": "libretro.com",
  "cdn.plutonium.pw": "plutonium.pw",
  "download.scdn.co": "spotify.com",
  "get.videolan.org": "videolan.org",
  "download.windscribe.com": "windscribe.com",
  "download.anydesk.com": "anydesk.com",
  "dl.dolphin-emu.org": "dolphin-emu.org",
  "downloads.vivaldi.com": "vivaldi.com",
  "www.bandisoft.com": "bandisoft.com",
  "sourceforge.net": "dosbox.com",
  "www.python.org": "python.org",
});

const REAL_SECTION_NAV = NAV.filter((item) =>
  item.filter && !item.filter.startsWith("__") && item.filter !== "featured"
);
const KNOWN_CATALOG_SECTIONS = new Set(REAL_SECTION_NAV.map((item) => item.filter));
const ACTIVE_TASK_STATES = new Set([
  "queued", "downloading", "paused", "installing", "cancelling",
]);

const PROJECT_SLOGANS = [
  "Tu software esencial, reunido en un solo lugar.",
  "Instala, actualiza y organiza tu equipo con menos esfuerzo.",
  "Una tienda ligera para un sistema más limpio y práctico.",
  "Todo lo que necesitas. Sin ruido, sin complicaciones.",
  "Descubre aplicaciones útiles y mantén tu equipo preparado.",
  "Tu equipo a tu manera: rápido, ordenado y bajo control.",
  "Menos búsquedas. Más tiempo para hacer lo que importa.",
  "Tu centro de control: software optimizado, accesible y al día.",
  "Todo tu software en un clic: sencillo, directo y personalizado.",
  "Gestión inteligente de aplicaciones para un rendimiento impecable.",
  "Tu biblioteca de programas: rápida, organizada y siempre actualizada.",
];

function chooseProjectSlogan() {
  try {
    const previous = localStorage.getItem("winslimcenter-last-slogan");
    const available = PROJECT_SLOGANS.filter((slogan) => slogan !== previous);
    const slogan = available[Math.floor(Math.random() * available.length)] || PROJECT_SLOGANS[0];
    localStorage.setItem("winslimcenter-last-slogan", slogan);
    return slogan;
  } catch {
    return PROJECT_SLOGANS[Math.floor(Math.random() * PROJECT_SLOGANS.length)];
  }
}

const THEME_PRESETS = {
  plata: {
    label: "Plata",
    mode: "dark",
    swatch: ["#eeeeee", "#383838", "#101010"],
    default_accent: "#d8d8d8",
    vars: {
      "--bg-app": "#181818",
      "--bg-sidebar": "#101010",
      "--bg-sb-hover": "#202020",
      "--bg-sb-active": "#303030",
      "--bg-topbar": "#0c0c0c",
      "--bg-card": "#202020",
      "--bg-card-hover": "#282828",
      "--bg-input": "#202020",
      "--bg-badge": "#282828",
      "--border": "rgba(255, 255, 255, 0.14)",
      "--text-dark": "#f2f2f2",
      "--text-medium": "#d0d0d0",
      "--text-light": "#a4a4a4",
      "--sb-text": "#eeeeee",
      "--green": "#d8d8d8",
      "--red": "#bcbcbc",
    },
  },
};

const ACCENT_CHOICES = [
  "#c7ced6", "#f8fafc", "#e2e8f0", "#cbd5e1", "#94a3b8", "#64748b",
  "#475569", "#1f2937", "#0f172a", "#d4d4d8", "#c0c7d0",
  "#aeb8c5", "#8b95a7", "#7c8aa5", "#b0b7be", "#e5e7eb",
];

const ACCENT_PALETTE = [
  "#3b82f6", "#8b5cf6", "#10b981", "#f97316", "#ec4899",
  "#06b6d4", "#f59e0b", "#6366f1", "#14b8a6", "#84cc16",
  "#ef4444", "#a855f7", "#0ea5e9", "#eab308", "#1e293b",
];

const SRC_LABELS = {
  direct: "Directa",
  wget: "WGet",
  winget: "WinGet",
  github_release: "GitHub Rel",
  github_repo: "GitHub Repo",
  webapp: "App web",
};

const STATE_LABELS = {
  queued: "En cola",
  downloading: "Descargando",
  paused: "Pausado",
  installing: "Instalando",
  cancelling: "Cancelando",
  done: "Completado",
  error: "Error",
  cancelled: "Cancelado",
};

const state = {
  catalog: [],
  // Filled from the backend, which reads it from src-tauri/Cargo.toml. Never
  // written here: a second copy of the number is a second place to forget.
  appVersion: "",
  installed: {},
  statuses: {},
  settings: { theme: "plata", accent: "#c7ced6" },
  tasks: [],
  section: "home",
  search: "",
  finished: new Set(),
  operationAppId: null,
  consoleFilter: "all",
  busy: {},
  resolvedIcons: {},
  resolvedIconSignatures: {},
  projectSlogan: chooseProjectSlogan(),
  scanningUpdates: false,
  lastUpdateScan: null,
};

const derived = {
  catalogGeneration: 0,
  catalogById: new Map(),
  searchBlobs: new Map(),
  taskByAppId: new Map(),
  iconCandidatesById: new Map(),
  iconSignaturesById: new Map(),
  consoleNames: [],
  installedCount: 0,
  updatesCount: 0,
};

function replaceCatalog(catalog) {
  derived.catalogGeneration += 1;
  state.catalog = Array.isArray(catalog) ? catalog : [];
  derived.catalogById = new Map(state.catalog.map((app) => [app.id, app]));
  derived.iconCandidatesById = new Map(
    state.catalog.map((app) => [app.id, buildBaseIconCandidates(app)]),
  );
  derived.iconSignaturesById = new Map(
    [...derived.iconCandidatesById].map(([appId, candidates]) => [appId, JSON.stringify(candidates)]),
  );
  for (const appId of Object.keys(state.resolvedIcons)) {
    const signature = derived.iconSignaturesById.get(appId);
    if (!signature || state.resolvedIconSignatures[appId] !== signature) {
      delete state.resolvedIcons[appId];
      delete state.resolvedIconSignatures[appId];
    }
  }
  derived.searchBlobs = new Map(
    state.catalog.map((app) => [
      app.id,
      [app.name, app.description, app.author, app.category, app.section, app.id, ...(app.console_tags || [])]
        .map((part) => String(part || ""))
        .join(" ")
        .toLocaleLowerCase("es-ES"),
    ]),
  );
  const consoles = new Set(
    state.catalog
      .filter((app) => app.section === "Emuladores")
      .flatMap((app) => Array.isArray(app.console_tags) ? app.console_tags : []),
  );
  const preferredOrder = [
    "PS1", "PS2", "PS3", "PSP", "Xbox", "Xbox 360", "GameCube", "Wii", "Wii U",
    "Game Boy", "Game Boy Color", "Game Boy Advance", "NES", "SNES", "Nintendo 64", "Sega", "DOS", "Multiplata",
  ];
  derived.consoleNames = [
    ...preferredOrder.filter((name) => consoles.has(name)),
    ...[...consoles].filter((name) => !preferredOrder.includes(name)).sort(),
  ];
}

function replaceStatuses(statuses) {
  const previous = state.statuses;
  const next = statuses && typeof statuses === "object" ? statuses : {};
  const ids = new Set([...Object.keys(previous), ...Object.keys(next)]);
  const changedIds = new Set();
  let installedChanged = false;
  let updatesChanged = false;
  const signature = (status) => [
    status?.installed, status?.version, status?.origin, status?.update_available,
    status?.latest_version, status?.can_uninstall, status?.can_launch,
  ].join("|");
  for (const id of ids) {
    if (signature(previous[id]) === signature(next[id])) continue;
    changedIds.add(id);
    if (!!previous[id]?.installed !== !!next[id]?.installed) installedChanged = true;
    if (!!previous[id]?.update_available !== !!next[id]?.update_available) updatesChanged = true;
  }
  state.statuses = next;
  let installed = 0;
  let updates = 0;
  for (const status of Object.values(state.statuses)) {
    if (status?.installed) installed += 1;
    if (status?.update_available) updates += 1;
  }
  derived.installedCount = installed;
  derived.updatesCount = updates;
  return { ids: changedIds, installedChanged, updatesChanged };
}

function reconcileStatusChanges(changes) {
  if (!changes?.ids?.size) return;
  if (changes.installedChanged || changes.updatesChanged) renderSidebar();
  const componentVisibilityChanged = changes.installedChanged &&
    [...changes.ids].some((id) => findApp(id)?.source_type === "component");
  if (
    componentVisibilityChanged ||
    (state.section === "installed" && changes.installedChanged) ||
    (state.section === "updates" && changes.updatesChanged)
  ) {
    renderContent();
    return;
  }
  updateVisibleAppStatuses(changes.ids);
}

function replaceTasks(tasks) {
  state.tasks = Array.isArray(tasks) ? tasks : [];
  derived.taskByAppId = new Map();
  for (const task of state.tasks) {
    if (ACTIVE_TASK_STATES.has(task.state)) derived.taskByAppId.set(task.app_id, task);
  }
}

let statusResetTimer = null;
let pendingStatusLog = "";
let lastLoggedStatus = "";
let statusLogTimer = null;

function clientLog(level, event, details = "") {
  const text = typeof details === "string" ? details : JSON.stringify(details);
  return invoke("write_log", { level, event, details: text }).catch(() => {});
}

window.addEventListener("error", (event) => {
  clientLog("error", "javascript-error", {
    message: event.message,
    source: event.filename,
    line: event.lineno,
    column: event.colno,
    stack: event.error?.stack || "",
  });
});

window.addEventListener("unhandledrejection", (event) => {
  clientLog("error", "unhandled-rejection", String(event.reason?.stack || event.reason || "desconocido"));
});

function normalizeHex(hex) {
  if (!hex) return "c7ced6";
  let h = String(hex).replace("#", "").trim();
  if (h.length === 3) h = h.split("").map((c) => c + c).join("");
  if (/^[0-9a-fA-F]{6}$/.test(h)) return h.toLowerCase();
  return "c7ced6";
}

function pickAccent(app, index) {
  if (app?.accent_color) return `#${normalizeHex(app.accent_color)}`;
  return ACCENT_PALETTE[index % ACCENT_PALETTE.length];
}

function applyTheme(themeId, accent) {
  const preset = THEME_PRESETS[themeId] || THEME_PRESETS.plata;
  const root = document.documentElement;
  Object.entries(preset.vars).forEach(([k, v]) => root.style.setProperty(k, v));
  const acc = `#${normalizeHex(accent || preset.default_accent)}`;
  root.style.setProperty("--accent", acc);
  root.style.setProperty("--accent-hover", acc);
  root.style.setProperty("--sb-text-active", acc);
  root.style.colorScheme = preset.mode;
  // Appearance changes must not discard the build variant remembered for
  // applications such as Thorium; updates reuse that choice later in the same
  // session.
  state.settings = { ...state.settings, theme: themeId, accent: acc };
}

function escapeHtml(s) {
  return String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function resolveIconUrl(app) {
  if (app?.icon_url) return app.icon_url;
  if (app?.download_url) {
    try {
      const url = new URL(app.download_url);

      // GitHub repository avatars identify an author or organisation, not the app.
      // Apps hosted there must provide an explicit original icon in the catalog.
      if (url.hostname.includes("github.com")) {
        return null;
      }

      // Map CDN or installer hostnames to official brand domains for Clearbit logo lookup.
      const mappedMozillaDomain = url.hostname === "download.mozilla.org"
        ? (app.id?.includes("thunderbird") ? "thunderbird.net" : "firefox.com")
        : null;
      let targetDomain = mappedMozillaDomain || ICON_DOMAIN_MAP[url.hostname] || url.hostname;
      if (targetDomain.includes("epicgames.com")) targetDomain = "epicgames.com";
      return `https://logo.clearbit.com/${encodeURIComponent(targetDomain)}?size=180`;
    } catch (e) {
      // ignore malformed download urls and let the fallback letter appear
    }
  }
  return null;
}

function buildBaseIconCandidates(app) {
  const icon = resolveIconUrl(app);
  // Los iconos del catálogo de serie viajan dentro del ejecutable. Cuando la
  // entrada señala uno de ellos no hay nada que preguntarle a internet: el
  // archivo está ahí y los sustitutos remotos solo servirían para tapar un
  // error de empaquetado que conviene ver.
  if (icon && !/^([a-z][a-z0-9+.-]*:)?\/\//i.test(icon)) return [icon];

  const urls = [icon];
  const source = app?.download_url || (app?.github_repo ? `https://github.com/${app.github_repo}` : "");
  try {
    const domain = new URL(source).hostname;
    if (domain && domain !== "github.com" && !domain.endsWith(".github.com")) {
      urls.push(`https://www.google.com/s2/favicons?domain=${encodeURIComponent(domain)}&sz=128`);
      urls.push(`https://icons.duckduckgo.com/ip3/${encodeURIComponent(domain)}.ico`);
    }
  } catch {
    // The letter avatar below remains the final, fully local fallback.
  }
  return [...new Set(urls.filter(Boolean))];
}

function iconCandidates(app) {
  const base = derived.iconCandidatesById.get(app?.id) || buildBaseIconCandidates(app);
  const now = Date.now();
  return [...new Set([state.resolvedIcons[app?.id], ...base].filter(Boolean))].filter((url) => {
    const failedAt = failedIconUrls.get(url);
    if (!failedAt) return true;
    if (now - failedAt < ICON_FAILURE_TTL_MS) return false;
    failedIconUrls.delete(url);
    return true;
  });
}

const failedIconUrls = new Map();
const ICON_FAILURE_TTL_MS = 5 * 60 * 1000;
let persistIconCacheTimer = null;

function rememberResolvedIcon(appId, url, expectedSignature, expectedGeneration) {
  if (!appId || !url) return false;
  const currentSignature = derived.iconSignaturesById.get(appId);
  const currentCandidates = derived.iconCandidatesById.get(appId) || [];
  if (
    Number(expectedGeneration) !== derived.catalogGeneration ||
    expectedSignature !== currentSignature ||
    !currentCandidates.includes(url)
  ) {
    return false;
  }
  failedIconUrls.delete(url);
  state.resolvedIcons[appId] = url;
  state.resolvedIconSignatures[appId] = currentSignature;
  // One timer writes the whole cache: the icons that arrive while it waits are
  // already in `state` and travel with it. Their answer is still yes — the icon
  // was accepted — and the caller shows the image based on that answer.
  if (persistIconCacheTimer === null) {
    persistIconCacheTimer = setTimeout(() => {
      persistIconCacheTimer = null;
      try {
        const entries = Object.fromEntries(
          Object.entries(state.resolvedIcons).map(([appId, resolvedUrl]) => [appId, {
            url: resolvedUrl,
            signature: state.resolvedIconSignatures[appId],
          }]),
        );
        localStorage.setItem("winslimcenter-resolved-icons-v2", JSON.stringify(entries));
      } catch {
        // The browser cache still provides the same optimisation for this run.
      }
    }, 500);
  }
  return true;
}

function hydrateResolvedIcons() {
  try {
    const cached = JSON.parse(localStorage.getItem("winslimcenter-resolved-icons-v2") || "{}");
    for (const [appId, entry] of Object.entries(cached)) {
      const signature = derived.iconSignaturesById.get(appId);
      if (
        signature && entry?.signature === signature && typeof entry.url === "string" && entry.url &&
        (derived.iconCandidatesById.get(appId) || []).includes(entry.url)
      ) {
        state.resolvedIcons[appId] = entry.url;
        state.resolvedIconSignatures[appId] = signature;
      }
    }
  } catch {
    // A corrupt/private localStorage is just a cache miss.
  }
}

window.__nextAppIcon = (img) => {
  img.classList.remove("is-loaded");
  img.parentElement?.classList.remove("has-loaded-icon");
  const candidates = String(img.dataset.iconCandidates || "").split("|").filter(Boolean);
  const current = candidates[Number(img.dataset.iconIndex || 0)];
  if (current) failedIconUrls.set(current, Date.now());
  const nextIndex = Number(img.dataset.iconIndex || 0) + 1;
  if (nextIndex < candidates.length) {
    img.dataset.iconIndex = String(nextIndex);
    img.src = candidates[nextIndex];
    return;
  }
  img.style.display = "none";
};

window.__appIconLoaded = (img) => {
  const appId = img.dataset.appId;
  const candidates = String(img.dataset.iconCandidates || "").split("|").filter(Boolean);
  const url = candidates[Number(img.dataset.iconIndex || 0)] || img.currentSrc || img.src;
  // Remembering it is a matter of the cache, and the cache may well refuse an
  // icon that belongs to a catalog already replaced. What is on screen does not
  // depend on that: this image painted the icon its own card asked for, so it
  // is shown. Hiding it would leave the letter of an application that does have
  // an icon.
  rememberResolvedIcon(
    appId,
    url,
    img.dataset.iconSignature || "",
    Number(img.dataset.catalogGeneration),
  );
  img.classList.add("is-loaded");
  img.parentElement?.classList.add("has-loaded-icon");
};

function renderAvatar(app, fallback, eager = false) {
  const candidates = iconCandidates(app);
  const iconSignature = derived.iconSignaturesById.get(app?.id) || "";
  const catalogGeneration = derived.catalogGeneration;
  const safeFallback = escapeHtml(fallback || "?");
  const padding = Math.max(0, Math.min(35, Number(app?.icon_padding ?? 9) || 0));
  const fit = app?.icon_fit === "contain" ? "contain" : "cover";
  const position = app?.icon_position === "left" ? "left center" : "center";
  if (candidates.length) {
    return `
      <img src="${escapeHtml(candidates[0])}" alt="${escapeHtml(app.name)} logo" width="180" height="180" loading="${eager ? "eager" : "lazy"}" ${eager ? 'fetchpriority="high"' : ""} decoding="async" style="padding:${padding}%;object-fit:${fit};object-position:${position}"
        data-app-id="${escapeHtml(app?.id || "")}"
        data-icon-signature="${escapeHtml(iconSignature)}" data-catalog-generation="${catalogGeneration}"
        data-icon-index="0" data-icon-candidates="${escapeHtml(candidates.join("|"))}"
        onload="window.__appIconLoaded(this)" onerror="window.__nextAppIcon(this)" />
      <span class="avatar-fallback" aria-hidden="true">${safeFallback}</span>
    `;
  }
  return `<span class="avatar-fallback" aria-hidden="true">${safeFallback}</span>`;
}

function avatarBackground(app, fallback) {
  return app?.icon_background ? `#${normalizeHex(app.icon_background)}` : fallback;
}

function setStatus(text, color) {
  if (statusResetTimer !== null) {
    clearTimeout(statusResetTimer);
    statusResetTimer = null;
  }
  const statusText = document.getElementById("status-text");
  const statusDot = document.getElementById("status-dot");
  const nextText = String(text);
  if (statusText.textContent !== nextText) statusText.textContent = nextText;
  if (color && statusDot.style.color !== color) statusDot.style.color = color;

  pendingStatusLog = nextText;
  if (statusLogTimer === null) {
    statusLogTimer = setTimeout(() => {
      statusLogTimer = null;
      if (pendingStatusLog === lastLoggedStatus) return;
      lastLoggedStatus = pendingStatusLog;
      void clientLog("debug", "status", pendingStatusLog);
    }, 250);
  }
}

function idleStatusSummary() {
  const updates = updatesCount();
  return updates
    ? `${updates} ${updates === 1 ? "actualización encontrada" : "actualizaciones encontradas"}`
    : `Todo al día · ${state.catalog.length} aplicaciones`;
}

function setTransientStatus(text, color, durationMs = 5000) {
  setStatus(text, color);
  statusResetTimer = setTimeout(() => {
    statusResetTimer = null;
    setStatus(idleStatusSummary(), "var(--green)");
  }, durationMs);
}

function setProgress(pct) {
  const progress = document.getElementById("status-progress");
  const track = document.getElementById("status-progress-track");
  const fill = document.getElementById("status-progress-fill");
  const value = Math.max(0, Math.min(100, Math.round(Number(pct) || 0)));
  if (progress.dataset.value === String(value)) return;
  progress.dataset.value = String(value);
  progress.textContent = `${value}%`;
  const hidden = value <= 0 || value >= 100;
  progress.classList.toggle("hidden", hidden);
  track.classList.toggle("hidden", hidden);
  fill.style.width = `${value}%`;
}

function renderAppVersion() {
  const version = document.getElementById("app-version");
  // Blank until the backend answers, rather than a number invented here that
  // could disagree with the build actually running.
  if (version) version.textContent = state.appVersion ? `v${state.appVersion}` : "";
}

function appStatus(id) {
  return (
    state.statuses[id] || {
      installed: false,
      version: "1.0",
      origin: "none",
      update_available: false,
      can_uninstall: false,
      can_launch: false,
    }
  );
}

/**
 * The rescan control, shared by the empty-state card and the section toolbar.
 *
 * It reuses the same stroked refresh glyph as the top bar's Refrescar button —
 * it is the same gesture — and the store's own busy vocabulary (`disabled` plus
 * `aria-busy`), so it breathes and drops its hover sheen exactly like every
 * other button that is working.
 */
function scanUpdatesButtonHtml({ hero = false } = {}) {
  const scanning = state.scanningUpdates;
  const size = hero ? 16 : 14;
  const icon = `
    <svg class="btn-svg-icon scan-icon" width="${size}" height="${size}" viewBox="0 0 24 24"
      fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"
      stroke-linejoin="round" aria-hidden="true">
      <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
      <path d="M3 3v5h5" />
      <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
      <path d="M16 16h5v5" />
    </svg>`;
  const cls = hero ? "btn primary empty-updates-scan" : "btn ghost updates-toolbar-scan";
  return `
    <button type="button" class="${cls}" id="btn-scan-updates"
      ${scanning ? 'disabled aria-busy="true"' : ""}
      title="Comparar tus aplicaciones instaladas con el repositorio de WinGet">
      ${icon}<span>${scanning ? "Consultando WinGet…" : "Buscar actualizaciones"}</span>
    </button>`;
}

function lastScanLabel() {
  if (state.scanningUpdates) return "Consultando WinGet...";
  if (!state.lastUpdateScan) return "Todas las apps vigentes";
  const time = state.lastUpdateScan.toLocaleTimeString("es-ES", {
    hour: "2-digit",
    minute: "2-digit",
  });
  return `Última comprobación con WinGet · ${time}`;
}

/**
 * Explicit "look for updates now" pass.
 *
 * Detection runs first so that apps installed or removed since the last scan are
 * taken into account, and only then does the backend ask WinGet — otherwise the
 * scan would compare against a stale list of installed packages.
 */
async function scanForUpdates() {
  if (state.scanningUpdates) return;
  state.scanningUpdates = true;
  // This one button reports on itself: it turns into its own busy state and the
  // status bar counts the scan down. Covering the list on top of that would
  // hide the very card the button lives in.
  suppressShellBusy = true;
  renderContent();
  setStatus("Consultando WinGet en busca de actualizaciones...", "var(--accent)");
  setProgress(30);
  try {
    replaceStatuses((await invoke("refresh_statuses")) || state.statuses);
    setProgress(65);
    replaceStatuses((await invoke("check_updates")) || state.statuses);
    state.lastUpdateScan = new Date();
    const updates = updatesCount();
    clientLog("info", "updates-scan", { updates });
    if (updates) {
      setStatus(
        `${updates} ${updates === 1 ? "actualización encontrada" : "actualizaciones encontradas"}`,
        "var(--accent)",
      );
    } else {
      setTransientStatus("WinGet no reporta actualizaciones pendientes.", "var(--green)", 5000);
    }
  } catch (error) {
    setTransientStatus(`No se pudo comprobar con WinGet: ${error}`, "var(--red)", 8000);
    clientLog("warn", "updates-scan", String(error?.stack || error));
  } finally {
    state.scanningUpdates = false;
    suppressShellBusy = false;
    hideShellBusy();
    setProgress(100);
    renderSidebar();
    renderContent();
  }
}

function installedCount() {
  return derived.installedCount;
}

function updatesCount() {
  return derived.updatesCount;
}

function sectionFilter(app) {
  const nav = NAV.find((n) => n.id === state.section);
  if (!nav || nav.filter == null) return true;
  if (nav.filter === "__INSTALLED__") return !!appStatus(app.id).installed;
  if (nav.filter === "__UPDATES__") return !!appStatus(app.id).update_available;
  if (nav.filter === "featured") return !!app.featured;
  if (app.section !== nav.filter) return false;
  if (state.section === "emulators" && state.consoleFilter !== "all") {
    return Array.isArray(app.console_tags) && app.console_tags.includes(state.consoleFilter);
  }
  return true;
}

function searchFilter(app, query) {
  return !query || (derived.searchBlobs.get(app.id) || "").includes(query);
}

// Los programas que vienen dentro de otra aplicación solo tienen sentido cuando
// esa aplicación está puesta: hasta entonces no hay nada que ofrecer, porque no
// se instalan por su cuenta.
function componentVisible(app) {
  return app.source_type !== "component" || appStatus(app.id).installed;
}

function filteredApps() {
  const query = state.search.trim().toLocaleLowerCase("es-ES");
  const apps = state.catalog.filter((app) =>
    componentVisible(app) && sectionFilter(app) && searchFilter(app, query)
  );
  if (state.section !== "featured") return apps;
  return apps.sort((a, b) => (FEATURED_RANK.get(a.id) ?? 999) - (FEATURED_RANK.get(b.id) ?? 999));
}

function validateCatalog(data) {
  if (!Array.isArray(data)) throw new Error("El catálogo debe ser una lista JSON de aplicaciones.");
  const ids = new Set();
  data.forEach((app, index) => {
    if (!app || typeof app !== "object" || Array.isArray(app)) {
      throw new Error(`La entrada ${index + 1} debe ser un objeto.`);
    }
    for (const field of ["id", "name", "source_type", "section"]) {
      if (typeof app[field] !== "string" || !app[field].trim()) {
        throw new Error(`La entrada ${index + 1} no tiene un campo ${field} válido.`);
      }
    }
    if (!KNOWN_CATALOG_SECTIONS.has(app.section)) {
      throw new Error(
        `La entrada ${index + 1} ("${app.name}") usa la sección desconocida "${app.section}". ` +
        `Secciones admitidas: ${[...KNOWN_CATALOG_SECTIONS].join(", ")}.`,
      );
    }
    if (ids.has(app.id)) throw new Error(`El identificador "${app.id}" está duplicado.`);
    ids.add(app.id);
  });
  return data;
}

let sidebarRenderFrame = null;
let contentRenderFrame = null;

function renderSidebar() {
  if (sidebarRenderFrame !== null) return;
  sidebarRenderFrame = requestAnimationFrame(() => {
    sidebarRenderFrame = null;
    renderSidebarNow();
  });
}

function renderSidebarNow() {
  const el = document.getElementById("sidebar");
  el.innerHTML = `
    <div class="sb-head">
      <span class="brand-mark"><img src="assets/winslim-center-logo.png" width="904" height="904" alt="" /></span>
      <div>
        <strong>Biblioteca</strong>
        <span>${installedCount()} instaladas</span>
      </div>
    </div>
    <div class="sb-label">SECCIONES</div>
    ${NAV.map(
      (n) => `
      <button type="button" class="nav-btn ${state.section === n.id ? "active" : ""}" data-nav="${n.id}">
        <span class="nav-ico">${n.icon}</span>${escapeHtml(n.label)}
        ${n.id === "updates" && updatesCount() > 0 ? `<span class="nav-badge-count">${updatesCount()}</span>` : ""}
      </button>`
    ).join("")}
    <div class="sb-label">ACCIONES</div>
    <button type="button" class="action-btn" data-action="theme">🎨  Apariencia</button>
    <button type="button" class="action-btn" data-action="folder">📁  Directorio local</button>
  `;
}

function actionButtons(app, variant = "card") {
  const st = appStatus(app.id);
  const id = escapeHtml(app.id);
  const task = derived.taskByAppId.get(app.id);
  const busy = state.busy[app.id];
  if (task || busy) {
    const operation = task?.state || busy;
    const label = {
      queued: "En cola…",
      downloading: "Descargando…",
      paused: "Pausado",
      installing: "Instalando…",
      cancelling: "Cancelando…",
      uninstalling: "Desinstalando…",
      launching: "Abriendo…",
      updating: "Actualizando…",
    }[operation] || "Procesando…";
    const cls = variant === "hero" ? "btn soft" : "btn secondary";
    return `<button type="button" class="${cls}" disabled aria-busy="true">${label}</button>`;
  }

  // Un componente llega dentro de otra aplicación: no se instala ni se
  // desinstala por su cuenta, así que lo único que ofrece es abrirse.
  if (app.source_type === "component") {
    if (!st.installed) return "";
    const launchCls = variant === "hero" ? "btn white" : "btn primary";
    return `<button type="button" class="${launchCls}" data-launch="${id}">Abrir</button>`;
  }

  if (!st.installed) {
    const cls = variant === "hero" ? "btn white" : "btn primary";
    const label = app.source_type === "web" ? "Obtener" : "Instalar";
    return `<button type="button" class="${cls}" data-install="${id}">${label}</button>`;
  }

  const parts = [];
  if (st.update_available) {
    const updateCls = variant === "hero" ? "btn white" : "btn secondary";
    parts.push(`<button type="button" class="${updateCls}" data-update="${id}">Actualizar</button>`);
  } else if (st.can_launch) {
    const launchCls = variant === "hero" ? "btn white" : "btn primary";
    parts.push(`<button type="button" class="${launchCls}" data-launch="${id}">Abrir</button>`);
  }

  const uninstallCls = variant === "hero" ? "btn soft" : "btn danger";
  parts.push(`<button type="button" class="${uninstallCls}" data-uninstall="${id}">Desinstalar</button>`);

  return parts.join("");
}

function updateVersionBadge(st, modal = false) {
  if (!st.update_available) return "";
  const current = String(st.version || "desconocida").replace(/^v(?=\d)/i, "");
  const latest = String(st.latest_version || "más reciente").replace(/^v(?=\d)/i, "");
  const cls = modal ? "update-version-badge modal-version" : "update-version-badge";
  return `<span class="${cls}" title="Versión instalada y versión disponible">Actualización · v${escapeHtml(current)} → v${escapeHtml(latest)}</span>`;
}

function cardMetaHtml(app) {
  const st = appStatus(app.id);
  const version = st.installed ? st.version : app.version || "1.0";
  // Shown for anything installed, whoever installed it: the plain "en el
  // sistema" text only appeared for programs Windows had registered, so the
  // ones the store put there itself showed nothing at all.
  const origin =
    st.update_available
      ? `<span class="origin-tag update-origin">actualización disponible</span>`
      : st.installed
        ? `<span class="installed-tag"><span class="badge-dot green"></span>Ya instalado</span>`
        : "";
  return `
    <strong>${escapeHtml(app.name)}${origin}</strong>
    <small>${escapeHtml(app.author || "—")}  ·  v${escapeHtml(version)}</small>
    ${updateVersionBadge(st)}`;
}

function cardHtml(app, index, prioritizeIcon = false) {
  const accent = pickAccent(app, index);
  const avatarBg = avatarBackground(app, accent);
  const letter = (app.name || app.id || "A")[0].toUpperCase();
  return `
    <article class="app-card" data-app-id="${escapeHtml(app.id)}" tabindex="0"
      aria-label="Ver detalles de ${escapeHtml(app.name)}" style="--card-accent:${accent}">
      <div class="card-top">
        <div class="card-avatar" style="background:${avatarBg}">${renderAvatar(app, letter, prioritizeIcon && index < 8)}</div>
        <div>
          ${cardMetaHtml(app)}
        </div>
      </div>
      <p class="card-desc">${escapeHtml(app.description || "")}</p>
      <div class="card-actions">${actionButtons(app, "card")}</div>
    </article>`;
}

function sectionHtml(title, apps, { prioritizeIcons = false } = {}) {
  if (!apps.length) return "";
  return `
    <section class="section">
      <div class="section-head">
        <h3>${escapeHtml(title)}</h3>
        <span>${apps.length} apps</span>
      </div>
      <div class="grid">${apps.map((a, i) => cardHtml(a, i, prioritizeIcons)).join("")}</div>
    </section>`;
}

function projectHeroHtml() {
  return `
    <section class="hero project-hero" aria-label="Presentación de WinSlimCenter">
      <div class="hero-left">
        <div class="project-hero-logo"><img src="assets/winslim-center-logo.png" width="904" height="904" alt="Logotipo de WinSlimCenter" /></div>
        <div class="project-hero-copy">
          <div class="project-kicker">TU CENTRO DE APLICACIONES Y HERRAMIENTAS</div>
          <h2>WinSlimCenter</h2>
          <p class="project-slogan">${escapeHtml(state.projectSlogan)}</p>
          <p class="desc">Una experiencia sencilla para descubrir, instalar, abrir, actualizar y desinstalar tus aplicaciones desde un catálogo cuidado.</p>
        </div>
      </div>
      <div class="project-hero-mark" aria-hidden="true">W</div>
      <div class="hero-tag">Ligero · Directo · Organizado</div>
    </section>`;
}

/// Which list was on screen the last time it was drawn, so that redrawing the
/// same one can put the scroll back where it was.
let lastRenderedView = null;

function renderContent() {
  if (contentRenderFrame !== null) return;
  contentRenderFrame = requestAnimationFrame(() => {
    contentRenderFrame = null;
    renderContentNow();
  });
}

function renderContentNow() {
  const apps = filteredApps();
  const searching = !!state.search.trim();
  const label = NAV.find((n) => n.id === state.section)?.label || "Inicio";
  let html = `
    <div class="page-title">
      <h1>${escapeHtml(label)}</h1>
      <span>${apps.length} aplicaciones</span>
    </div>`;

  // With pending updates on screen the empty-state card is not rendered, so the
  // rescan action needs its own place to live.
  if (state.section === "updates" && apps.length) {
    html += `
      <div class="updates-toolbar">
        <span class="updates-toolbar-note">${escapeHtml(lastScanLabel())}</span>
        ${scanUpdatesButtonHtml()}
      </div>`;
  }

  if (state.section === "emulators") {
    html += `
      <div class="console-filters" aria-label="Filtrar emuladores por consola">
        <button type="button" class="console-filter ${state.consoleFilter === "all" ? "active" : ""}" data-console-filter="all">Todas</button>
        ${derived.consoleNames.map((consoleName) => `
          <button type="button" class="console-filter ${state.consoleFilter === consoleName ? "active" : ""}"
            data-console-filter="${escapeHtml(consoleName)}">${escapeHtml(consoleName)}</button>
        `).join("")}
      </div>`;
  }

  if (state.section === "home" && !searching) {
    html += projectHeroHtml();
    const blocks = [
      ["Destacados", (a) => a.featured],
      ...REAL_SECTION_NAV.map((nav) => [nav.label, (app) => app.section === nav.filter]),
    ];
    for (const [title, pred] of blocks) {
      const blockApps = apps.filter(pred);
      if (title === "Destacados") {
        blockApps.sort((a, b) => (FEATURED_RANK.get(a.id) ?? 999) - (FEATURED_RANK.get(b.id) ?? 999));
      }
      html += sectionHtml(title, blockApps, { prioritizeIcons: title === "Destacados" });
    }
    // A catalog coming from an older/custom file must never silently lose an
    // application just because its section is not yet represented in NAV.
    const unknownSections = new Map();
    for (const app of apps) {
      if (KNOWN_CATALOG_SECTIONS.has(app.section)) continue;
      const title = String(app.section || "Sin categoría").trim() || "Sin categoría";
      if (!unknownSections.has(title)) unknownSections.set(title, []);
      unknownSections.get(title).push(app);
    }
    for (const [title, blockApps] of unknownSections) html += sectionHtml(title, blockApps);
  } else {
    html += sectionHtml(label, apps, { prioritizeIcons: true });
  }

  if (!apps.length) {
    if (state.section === "updates") {
      const scanning = state.scanningUpdates;
      html += `
        <div class="empty-updates-wrap">
          <div class="empty-updates-card" aria-label="Sin actualizaciones pendientes">
            <div class="empty-updates-hero">
              <div class="empty-updates-badge-container">
                <div class="empty-updates-logo-wrapper">
                  <img src="assets/winslim-center-logo.png" width="904" height="904" alt="WinSlimCenter" />
                </div>
                <div class="empty-updates-check-badge">✓</div>
              </div>
              <h2>Todo tu software está al día</h2>
              <p>WinSlimCenter ha verificado el catálogo de tus aplicaciones instaladas y no hay actualizaciones pendientes en este momento.</p>
              ${scanUpdatesButtonHtml({ hero: true })}
              <p class="empty-updates-hint">${
                scanning
                  ? "Comparando las versiones instaladas con el repositorio de WinGet."
                  : "Consulta WinGet en busca de versiones nuevas de tus aplicaciones instaladas."
              }</p>
              <div class="empty-updates-status">
                <span class="pulse-dot"></span> ${escapeHtml(lastScanLabel())}
              </div>
            </div>
          </div>
        </div>`;
    } else {
      html += `<div class="empty"><h3>Sin resultados</h3><p>Prueba con otro buscador o sección.</p></div>`;
    }
  }

  const content = document.getElementById("content");
  // Replacing the markup sends the scroll back to the top. Redrawing the same
  // list — which is what happens on the refresh after installing or
  // uninstalling — has to leave it where the user was, or the card they were
  // working with disappears off screen just as they reach for its buttons.
  // Moving to another section, searching or filtering does start at the top,
  // because that is a different list.
  const view = `${state.section}|${state.search}|${state.consoleFilter}`;
  const restoreTo = view === lastRenderedView ? content.scrollTop : 0;
  lastRenderedView = view;

  content.innerHTML = html;
  // `scrollTop` is clamped by the browser. Avoid reading scrollHeight and
  // clientHeight immediately after replacing the DOM, which forced a complete
  // synchronous layout of every card.
  content.scrollTop = restoreTo;
}

function updateVisibleAppActions(appIds) {
  const wanted = appIds instanceof Set ? appIds : new Set(appIds || []);
  if (!wanted.size) return;
  document.querySelectorAll("#content .app-card[data-app-id]").forEach((card) => {
    const id = card.dataset.appId;
    if (!wanted.has(id)) return;
    const app = findApp(id);
    const actions = card.querySelector(".card-actions");
    if (app && actions) actions.innerHTML = actionButtons(app, "card");
  });
}

function updateVisibleAppStatuses(appIds) {
  const wanted = appIds instanceof Set ? appIds : new Set(appIds || []);
  if (!wanted.size) return;
  document.querySelectorAll("#content .app-card[data-app-id]").forEach((card) => {
    const id = card.dataset.appId;
    if (!wanted.has(id)) return;
    const app = findApp(id);
    if (!app) return;
    const meta = card.querySelector(".card-top > div:last-child");
    const actions = card.querySelector(".card-actions");
    if (meta) meta.innerHTML = cardMetaHtml(app);
    if (actions) actions.innerHTML = actionButtons(app, "card");
  });
}

let shellDelegationBound = false;

function bindShellDelegation() {
  if (shellDelegationBound) return;
  shellDelegationBound = true;
  const sidebar = document.getElementById("sidebar");
  const content = document.getElementById("content");

  sidebar.addEventListener("click", (event) => {
    const navButton = event.target.closest("[data-nav]");
    if (navButton) {
      const nextSection = navButton.dataset.nav;
      if (nextSection && (state.section !== nextSection || state.consoleFilter !== "all")) {
        state.section = nextSection;
        state.consoleFilter = "all";
        void clientLog("info", "navigation", `Sección seleccionada: ${state.section}`);
        renderSidebar();
        renderContent();
      }
      return;
    }

    const actionButton = event.target.closest("[data-action]");
    if (actionButton?.dataset.action === "theme") {
      void clientLog("info", "action", "Abriendo selector de apariencia.");
      openThemePicker();
    } else if (actionButton?.dataset.action === "folder") {
      void clientLog("info", "action", "Abriendo carpeta de aplicaciones.");
      invoke("open_apps_dir").catch((error) => {
        setStatus(`No se pudo abrir la carpeta: ${error}`, "var(--red)");
        showAlertModal("Error al abrir la carpeta", String(error));
      });
    }
  });

  content.addEventListener("click", (event) => {
    const consoleButton = event.target.closest("[data-console-filter]");
    if (consoleButton) {
      const nextFilter = consoleButton.dataset.consoleFilter || "all";
      if (state.consoleFilter !== nextFilter) {
        state.consoleFilter = nextFilter;
        renderContent();
      }
      return;
    }
    if (event.target.closest("#btn-scan-updates")) {
      void scanForUpdates();
      return;
    }

    const action = event.target.closest("[data-install], [data-update], [data-uninstall], [data-launch]");
    if (action) {
      if (action.dataset.install) void installApp(action.dataset.install, false);
      else if (action.dataset.update) void installApp(action.dataset.update, true);
      else if (action.dataset.uninstall) void uninstallApp(action.dataset.uninstall);
      else if (action.dataset.launch) void launchApp(action.dataset.launch);
      return;
    }

    const card = event.target.closest(".app-card[data-app-id]");
    if (card && !event.target.closest("button")) openAppModal(card.dataset.appId);
  });

  content.addEventListener("keydown", (event) => {
    if (!["Enter", " "].includes(event.key) || event.target.closest("button")) return;
    const card = event.target.closest(".app-card[data-app-id]");
    if (!card) return;
    event.preventDefault();
    openAppModal(card.dataset.appId);
  });
}

function openAppModal(id) {
  const app = findApp(id);
  if (!app) return;
  const st = appStatus(id);
  const accent = pickAccent(app, 0);
  const avatarBg = avatarBackground(app, accent);
  const letter = (app.name || app.id || "A")[0].toUpperCase();
  const avatar = renderAvatar(app, letter, true);
  const version = st.installed ? st.version : app.version || "1.0";

  // Installed is installed, with or without an update waiting: the update badge
  // used to take this one's place, so an application with a version pending
  // showed nothing saying it was already on the machine. They say different
  // things and both are worth saying.
  const installedBanner = st.installed
    ? `<div class="installed-badge-banner"><span class="badge-dot green"></span>Ya instalado</div>
       ${st.update_available ? `<div>${updateVersionBadge(st, true)}</div>` : ""}`
    : "";

  let actionBtnsHtml = "";
  if (!st.installed) {
    actionBtnsHtml = `<button type="button" class="btn primary" id="modal-app-install">${app.source_type === "web" ? "Obtener" : "Instalar"}</button>`;
  } else {
    actionBtnsHtml = `
      ${st.update_available
        ? `<button type="button" class="btn secondary" id="modal-app-update">Actualizar</button>`
        : st.can_launch ? `<button type="button" class="btn primary" id="modal-app-launch">Abrir</button>` : ""}
      <button type="button" class="btn danger" id="modal-app-uninstall">Desinstalar</button>
    `;
  }

  openModal(`
    <div class="confirm-dialog app-detail-modal">
      <div class="confirm-dialog-header">
        <div class="card-avatar" style="background:${avatarBg}; width: 56px; height: 56px; border-radius: 16px; font-size: 22px; flex-shrink: 0; overflow: hidden;">
          ${avatar}
        </div>
        <div>
          <h2 class="confirm-dialog-title" style="font-size: 20px; font-weight: 700;">${escapeHtml(app.name)}</h2>
          <small style="color: var(--text-medium); font-size: 13px; font-weight: 500;">
            Por ${escapeHtml(app.author || "—")}  ·  v${escapeHtml(version)}  ·  ${escapeHtml(app.category || "")}
          </small>
          <div>${installedBanner}</div>
        </div>
      </div>
      <p class="confirm-dialog-msg" style="margin-top: 14px; font-size: 14px; line-height: 1.6; color: var(--text-main);">
        ${escapeHtml(app.description || "")}
      </p>
      <div class="modal-foot" style="margin-top: 24px;">
        <button type="button" class="btn ghost" id="modal-app-close">Cerrar</button>
        ${actionBtnsHtml}
      </div>
    </div>
  `);

  document.getElementById("modal-app-close").onclick = closeModal;

  const btnInst = document.getElementById("modal-app-install");
  if (btnInst) btnInst.onclick = () => { closeModal(); installApp(id, false); };

  const btnLaunch = document.getElementById("modal-app-launch");
  if (btnLaunch) btnLaunch.onclick = () => { closeModal(); launchApp(id); };

  const btnUpdate = document.getElementById("modal-app-update");
  if (btnUpdate) btnUpdate.onclick = () => { closeModal(); installApp(id, true); };

  const btnUninstall = document.getElementById("modal-app-uninstall");
  if (btnUninstall) btnUninstall.onclick = () => { closeModal(); uninstallApp(id); };
}

function findApp(id) {
  return derived.catalogById.get(id);
}

async function installApp(id, isUpdate = false) {
  const app = findApp(id);
  if (!app) return;
  if (app.source_type === "web") {
    showConfirmModal({
      title: `Obtener ${app.name}`,
      message: `Este producto requiere la web oficial para descargar, comprar, iniciar sesión o elegir la versión compatible. ¿Deseas abrirla ahora?`,
      app,
      confirmText: "Abrir web oficial",
      confirmVariant: "primary",
      onConfirm: async () => {
        try {
          await invoke("open_url", { url: app.web_url });
          setStatus(`Web oficial abierta: ${app.name}`, "var(--accent)");
        } catch (error) {
          setStatus(`No se pudo abrir la web oficial de ${app.name}`, "var(--red)");
          showAlertModal("Error al abrir la web oficial", String(error));
        }
      },
    });
    return;
  }
  const st = appStatus(id);
  if (st.installed && !isUpdate && !st.update_available) {
    showAlertModal("Aplicación instalada", `'${app.name}' ya está instalada en el equipo.`);
    return;
  }
  if (st.installed && isUpdate && !st.update_available) {
    showAlertModal("Aplicación actualizada", `'${app.name}' ya está en su versión más reciente.`);
    return;
  }

  const title = isUpdate ? `Actualizar ${app.name}` : `Instalar ${app.name}`;
  const message = isUpdate
    ? `¿Deseas actualizar '${app.name}' a la versión más reciente?`
    : `¿Deseas instalar '${app.name}' en tu equipo?`;
  const confirmText = isUpdate ? "Actualizar" : "Instalar";

  // An application published in several builds asks which one on the way in.
  // An update never asks: it reinstalls the build already chosen, because
  // changing it is changing the program.
  const remembered = state.settings?.variants?.[id];
  const choices =
    app.variants && !isUpdate
      ? { ...app.variants, selected: remembered || app.variants.default }
      : null;

  showConfirmModal({
    title,
    message,
    app,
    confirmText,
    confirmVariant: "primary",
    choices,
    onConfirm: async (picked) => {
      const variant = picked || remembered || app.variants?.default || null;
      state.busy[id] = isUpdate ? "updating" : "installing";
      updateVisibleAppActions(new Set([id]));
      state.finished.delete(id);
      renderDlPanel();
      showPackageOperationModal(app, isUpdate);
      try {
        await invoke("install_app", {
          appEntry: app,
          forceUpdate: !!isUpdate || !!st.update_available,
          variant,
        });
      } catch (e) {
        delete state.busy[id];
        state.operationAppId = null;
        closeModal();
        const text = String(e);
        // A rejection can mean the backend already knows this app is installed
        // or already has a task queued. Pull its cheap committed snapshot so
        // status and task controls agree without a global Windows/WinGet scan.
        try {
          await reconcileRuntimeFromBootstrap();
        } catch (snapshotError) {
          void clientLog("warn", "install-rejected-state", String(snapshotError?.stack || snapshotError));
        }
        updateVisibleAppStatuses(new Set([id]));
        setStatus(`Error: ${text}`, "var(--red)");

        showAlertModal("Error de instalación", text);
      }
    },
  });
}

async function uninstallApp(id) {
  const app = findApp(id);
  if (!app) return;
  const st = appStatus(id);
  if (!st.can_uninstall) {
    // Only packaged applications reach this point. Sending everyone to the
    // Control Panel was advice that could not be followed: an application whose
    // uninstall entry is gone does not appear there at all.
    showAlertModal(
      "La gestiona Windows",
      "Esta aplicación está empaquetada y solo Windows puede quitarla. Ve a Configuración > Aplicaciones > Aplicaciones instaladas y desinstálala desde ahí."
    );
    return;
  }

  showConfirmModal({
    title: `Desinstalar ${app.name}`,
    message: `¿Estás seguro de que deseas desinstalar '${app.name}' de tu equipo?`,
    app,
    confirmText: "Desinstalar",
    confirmVariant: "danger",
    onConfirm: async () => {
      state.operationAppId = null;
      state.busy[id] = "uninstalling";
      updateVisibleAppActions(new Set([id]));
      showBackgroundOperationModal(
        app,
        `Desinstalación de ${app.name}`,
        "Desinstalando el programa...",
      );
      // Some uninstallers open their own window and wait for the user. Without
      // an escape hatch the modal stayed up forever with no way out, so after a
      // while it offers to step aside while the uninstaller keeps running.
      const escapeHatch = setTimeout(() => {
        const status = document.getElementById("package-operation-status");
        if (status) {
          status.textContent = "Continúa desinstalando...";
        }
        const actions = document.getElementById("package-operation-actions");
        if (actions && !actions.querySelector("#operation-dismiss")) {
          actions.innerHTML =
            '<button type="button" class="btn ghost" id="operation-dismiss">Seguir en segundo plano</button>';
          actions.querySelector("#operation-dismiss").addEventListener("click", () => {
            closeModal();
            setStatus(`${app.name}: desinstalación en curso en segundo plano`, "var(--accent)");
          });
        }
      }, 20000);
      try {
        // El backend decide el mensaje: no es lo mismo haber quitado el programa
        // que descubrir que nunca estuvo y haber limpiado lo que lo daba por
        // instalado.
        const outcome = await invoke("uninstall_app", { appId: id });
        closeModal();
        const changes = await refreshInstalledFromBootstrap();
        reconcileStatusChanges(changes);
        setTransientStatus(`${app.name} se desinstaló correctamente`, "var(--green)", 5000);
        showAlertModal(
          "Desinstalación completada",
          outcome || `${app.name} se desinstaló correctamente del equipo.`,
        );
      } catch (e) {
        closeModal();
        const message = String(e);
        try {
          const changes = await refreshInstalledFromBootstrap();
          reconcileStatusChanges(changes);
        } catch {
          // Conservamos el error original de desinstalación o limpieza.
        }
        // "Se ejecutó la desinstalación, pero..." means the uninstaller ran and
        // Windows still lists the app: a warning, not an outright failure.
        const partial =
          message.includes("La aplicación se desinstaló") ||
          message.includes("Se ejecutó la desinstalación");
        setTransientStatus(
          partial ? `${app.name}: desinstalación sin confirmar` : `Error al desinstalar ${app.name}`,
          partial ? "var(--text-medium)" : "var(--red)",
          6000,
        );
        showAlertModal(
          partial ? "Desinstalación completada con advertencias" : "Error al desinstalar",
          message,
        );
      } finally {
        clearTimeout(escapeHatch);
        delete state.busy[id];
        updateVisibleAppStatuses(new Set([id]));
      }
    },
  });
}

async function launchApp(id) {
  const app = findApp(id);
  state.busy[id] = "launching";
  updateVisibleAppActions(new Set([id]));
  try {
    const msg = await invoke("launch_app", { appId: id });
    setTransientStatus(msg || `Lanzando: ${app?.name || id}`, "var(--accent)", 5000);
  } catch (e) {
    const text = String(e);
    const elevationPrefix = "__WINSLIM_ELEVATION_REQUIRED__:";
    const elevationRequired = text.includes(elevationPrefix)
      || /(?:requiere elevaci[oó]n|error\s*740)/i.test(text);
    if (elevationRequired) {
      showElevationFallbackModal(id, app, text.replace(elevationPrefix, "").trim());
    } else {
      showAlertModal("Error al abrir", text);
    }
  } finally {
    delete state.busy[id];
    updateVisibleAppActions(new Set([id]));
  }
}

function showElevationFallbackModal(id, app, reason) {
  const appName = app?.name || id;
  openModal(`
    <div class="confirm-dialog">
      <div class="confirm-dialog-header">
        <div>
          <h2 class="confirm-dialog-title">Se requieren permisos de administrador</h2>
        </div>
      </div>
      <p class="confirm-dialog-msg">${escapeHtml(reason)}</p>
      <p class="confirm-dialog-msg" style="margin-top: 10px; color: var(--text-medium);">
        WinSlimCenter puede volver a abrir ${escapeHtml(appName)} mediante el diálogo seguro de UAC de Windows.
      </p>
      <div class="modal-foot" style="margin-top: 20px;">
        <button type="button" class="btn ghost" id="modal-elevation-cancel">Cancelar</button>
        <button type="button" class="btn primary" id="modal-elevation-run">Ejecutar como administrador</button>
      </div>
    </div>
  `);

  document.getElementById("modal-elevation-cancel").onclick = closeModal;
  document.getElementById("modal-elevation-run").onclick = async () => {
    const button = document.getElementById("modal-elevation-run");
    button.disabled = true;
    button.textContent = "Solicitando permisos...";
    try {
      const message = await invoke("launch_app_elevated", { appId: id });
      closeModal();
      setTransientStatus(message || `Lanzando como administrador: ${appName}`, "var(--accent)", 5000);
    } catch (error) {
      closeModal();
      showAlertModal(
        "No se pudo ejecutar como administrador",
        String(error || "Windows no proporcionó un motivo para rechazar la elevación."),
      );
    }
  };
}

function stateColor(s) {
  const map = {
    queued: "var(--text-light)",
    downloading: "var(--accent)",
    paused: "var(--accent)",
    installing: "var(--accent)",
    cancelling: "var(--text-medium)",
    done: "var(--green)",
    error: "var(--red)",
    cancelled: "var(--text-light)",
  };
  return map[s] || "var(--text-light)";
}

async function syncDownloadTasks() {
  try {
    const previousIds = new Set(derived.taskByAppId.keys());
    const tasks = await invoke("get_tasks");
    replaceTasks(tasks || []);
    updateVisibleAppActions(new Set([...previousIds, ...derived.taskByAppId.keys()]));
    renderDlPanel();
  } catch (e) {
    console.error("No se pudieron actualizar las descargas", e);
  }
}

async function invokeDownloadAction(cmd, args = {}) {
  try {
    const previousIds = new Set(derived.taskByAppId.keys());
    await invoke(cmd, args);
    replaceTasks((await invoke("get_tasks")) || []);
    updateVisibleAppActions(new Set([...previousIds, ...derived.taskByAppId.keys()]));
    renderDlPanel();
    updatePackageOperation(state.tasks);
  } catch (e) {
    console.error("Download action error:", e);
  }
}

function renderDlPanel() {
  const tasks = state.tasks;

  if (tasks.length) {
    const latest = tasks[tasks.length - 1];
    if (["queued", "downloading", "paused", "installing", "cancelling"].includes(latest.state)) {
      setStatus(`${latest.name} — ${latest.status}`, stateColor(latest.state));
    } else if (latest.state === "done" && !state.finished.has(latest.app_id)) {
      state.finished.add(latest.app_id);
      setTransientStatus(`✓ ${latest.status || `${latest.name} instalado`}`, "var(--green)", 5000);
    } else if (latest.state === "error" && !state.finished.has(latest.app_id)) {
      state.finished.add(latest.app_id);
      setTransientStatus(`✖ ${latest.name} error`, "var(--red)", 5000);
    }
  }

  const activeTasks = tasks.filter((t) =>
    ["queued", "downloading", "paused", "installing", "cancelling"].includes(t.state)
  );
  if (activeTasks.length) {
    const avg = Math.round(
      activeTasks.reduce((s, t) => s + (t.progress || 0), 0) / activeTasks.length
    );
    setProgress(avg);
  } else if (tasks.length && tasks.every((t) => t.state === "done")) {
    setProgress(100);
  } else {
    setProgress(0);
  }
}

let modalReturnFocus = null;
let modalLocked = false;

function closeModal() {
  document.getElementById("modal-backdrop").classList.add("hidden");
  document.getElementById("modal").innerHTML = "";
  modalLocked = false;
  if (modalReturnFocus instanceof HTMLElement) modalReturnFocus.focus();
  modalReturnFocus = null;
}

function openModal(html, wide = false, locked = false) {
  const backdrop = document.getElementById("modal-backdrop");
  const modal = document.getElementById("modal");
  if (backdrop.classList.contains("hidden")) modalReturnFocus = document.activeElement;
  modalLocked = locked;
  modal.classList.toggle("wide", wide);
  // Cleared here rather than on close, so the treatment can never leak from the
  // modal that asked for it into the next one.
  modal.classList.remove("modal-hero");
  modal.innerHTML = html;
  const heading = modal.querySelector("h2");
  if (heading) {
    heading.id = "modal-title";
    modal.setAttribute("aria-labelledby", heading.id);
  } else {
    modal.removeAttribute("aria-labelledby");
  }
  backdrop.classList.remove("hidden");
  backdrop.onclick = (e) => {
    if (!locked && e.target === backdrop) closeModal();
  };
  modal.querySelector("button, input, textarea, select, [tabindex]:not([tabindex='-1'])")?.focus();
}

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && !modalLocked && !document.getElementById("modal-backdrop").classList.contains("hidden")) {
    event.preventDefault();
    closeModal();
  }
});

function showPackageOperationModal(app, isUpdate) {
  state.operationAppId = app.id;
  showBackgroundOperationModal(
    app,
    `${isUpdate ? "Actualizando" : "Instalando"} ${app.name}`,
    "Preparando el paquete...",
    true,
  );
  const actions = document.getElementById("package-operation-actions");
  if (actions) {
    actions.innerHTML = '<button type="button" class="btn ghost" id="operation-cancel">Cancelar</button>';
    bindOperationCancel(app.id, "Cancelando la instalación...");
  }
}

function bindOperationCancel(appId, message) {
  document.getElementById("operation-cancel")?.addEventListener("click", async (event) => {
    event.currentTarget.disabled = true;
    event.currentTarget.textContent = "Cancelando...";
    const status = document.getElementById("package-operation-status");
    if (status) status.textContent = message;
    await invokeDownloadAction("cancel_download", { appId });
  });
}

/**
 * `withProgress` is for the operations that can measure themselves. An
 * uninstall cannot — the program's own uninstaller reports to nobody — and a
 * bar sitting at zero throughout would say something untrue about it.
 */
function showBackgroundOperationModal(app, title, initialStatus, withProgress = false) {
  const avatar = renderAvatar(app, app.name?.[0] || "?", true);
  const accent = pickAccent(app, 0);
  const avatarBg = avatarBackground(app, accent);
  openModal(`
    <div class="package-operation" data-operation-app="${escapeHtml(app.id)}">
      <div class="package-operation-brand">
        <div class="card-avatar" style="background:${avatarBg}; width:52px; height:52px; border-radius:15px; overflow:hidden;">
          ${avatar}
        </div>
        <div>
          <small>WinSlimCenter</small>
          <h2>${escapeHtml(title)}</h2>
        </div>
      </div>
      <div class="package-operation-dots" aria-hidden="true"><span></span><span></span><span></span></div>
      <p id="package-operation-status">${escapeHtml(initialStatus)}</p>
      ${
        withProgress
          ? `<div class="package-operation-progress" id="package-operation-progress"
                  role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0">
               <span class="package-operation-progress-fill" id="package-operation-progress-fill"></span>
             </div>`
          : ""
      }
      <div class="package-operation-actions" id="package-operation-actions"></div>
    </div>
  `, false, true);
}

/**
 * Turns the operation dialog into its finished state instead of closing it.
 *
 * The user has been watching a progress dialog: dropping them back into the
 * list the instant it ends leaves them to hunt for the card they were just
 * working with. It stays put with the only two things worth doing next.
 */
function showOperationCompleted(app, { canLaunch, isUpdate, changed }) {
  const dialog = document.querySelector(".package-operation");
  if (!dialog) return;
  // Escape becomes a way out again now that nothing is in progress.
  modalLocked = false;

  const dots = dialog.querySelector(".package-operation-dots");
  if (dots) {
    dots.classList.add("done");
    // Drawn rather than typed: the ✓ of a text font arrives with whatever
    // weight and shape that font happens to give it, and next to the rest of
    // the interface it looked like a leftover character.
    dots.innerHTML = `
      <span aria-hidden="true">
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor"
             stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
          <path d="M4.5 12.5 10 18 19.5 6.5" />
        </svg>
      </span>`;
  }
  // Nothing left to measure: the tick and the sentence below it say everything
  // a full bar would, and one more full-width element would only crowd them.
  dialog.querySelector(".package-operation-progress")?.remove();
  const title = dialog.querySelector("h2");
  if (title && app) title.textContent = app.name;
  // The same dialog serves the Install and Update buttons, and WinGet answers a
  // package that was already current without installing anything: telling the
  // user it was installed would be wrong in two of the three cases.
  const outcome = !changed
    ? "ya estaba en su última versión"
    : isUpdate
      ? "se actualizó correctamente"
      : "se instaló correctamente";
  const status = document.getElementById("package-operation-status");
  if (status) {
    status.textContent = app?.name
      ? `${app.name} ${outcome}`
      : outcome.charAt(0).toUpperCase() + outcome.slice(1);
  }

  const actions = document.getElementById("package-operation-actions");
  if (!actions) return;
  actions.innerHTML = `
    ${canLaunch ? '<button type="button" class="btn primary" id="operation-launch">Lanzar</button>' : ""}
    <button type="button" class="btn ghost" id="operation-close">Cerrar</button>
  `;
  actions.querySelector("#operation-launch")?.addEventListener("click", () => {
    closeModal();
    if (app) launchApp(app.id);
  });
  actions.querySelector("#operation-close")?.addEventListener("click", closeModal);
  actions.querySelector("button")?.focus();
}

function updatePackageOperation(tasks) {
  if (!state.operationAppId) return;
  let task = null;
  for (let index = tasks.length - 1; index >= 0; index -= 1) {
    if (tasks[index].app_id === state.operationAppId) {
      task = tasks[index];
      break;
    }
  }
  if (!task) return;
  const status = document.getElementById("package-operation-status");
  if (status) status.textContent = task.status || "Procesando paquete...";
  const progress = document.getElementById("package-operation-progress");
  if (progress) {
    const value = Math.max(0, Math.min(100, Number(task.progress) || 0));
    if (progress.getAttribute("aria-valuenow") !== String(value)) {
      progress.setAttribute("aria-valuenow", String(value));
      const fill = document.getElementById("package-operation-progress-fill");
      if (fill) fill.style.width = `${value}%`;
    }
  }
  const actions = document.getElementById("package-operation-actions");
  if (!actions) return;
  const actionsSignature = [task.app_id, task.can_pause, task.can_resume, task.can_cancel, task.state].join("|");
  if (actions.dataset.signature === actionsSignature) return;
  actions.dataset.signature = actionsSignature;
  actions.innerHTML = `
    ${task.can_pause ? '<button type="button" class="btn ghost" id="operation-pause">Pausar</button>' : ""}
    ${task.can_resume ? '<button type="button" class="btn ghost" id="operation-resume">Reanudar</button>' : ""}
    ${task.can_cancel ? '<button type="button" class="btn ghost" id="operation-cancel">Cancelar</button>' : ""}
  `;
  actions.querySelector("#operation-pause")?.addEventListener("click", () =>
    invokeDownloadAction("pause_download", { appId: task.app_id })
  );
  actions.querySelector("#operation-resume")?.addEventListener("click", () =>
    invokeDownloadAction("resume_download", { appId: task.app_id })
  );
  bindOperationCancel(
    task.app_id,
    task.state === "installing"
      ? "Solicitando la cancelación de la instalación..."
      : "Cancelando la descarga...",
  );
}

/**
 * Renders the builds an application is published in, when there is more than
 * one to pick from. The chosen id is what `onConfirm` receives.
 */
function renderChoices(choices) {
  if (!choices || !choices.options?.length) return "";
  return `
    <fieldset class="choice-group">
      <legend>${escapeHtml(choices.label || "Versión")}</legend>
      ${choices.hint ? `<p class="choice-hint">${escapeHtml(choices.hint)}</p>` : ""}
      ${choices.options
        .map(
          (option, index) => `
        <label class="choice-option">
          <input type="radio" name="modal-choice" value="${escapeHtml(option.id)}"${
            (choices.selected ? option.id === choices.selected : index === 0) ? " checked" : ""
          }>
          <span class="choice-option-body">
            <strong>${escapeHtml(option.name || option.id)}</strong>
            ${option.description ? `<small>${escapeHtml(option.description)}</small>` : ""}
          </span>
        </label>`
        )
        .join("")}
    </fieldset>`;
}

function showConfirmModal({ title, message, app, confirmText = "Confirmar", confirmVariant = "primary", choices = null, onConfirm }) {
  const avatar = app ? renderAvatar(app, app.name?.[0] || "?", true) : "";
  const accent = app ? pickAccent(app, 0) : "var(--accent)";
  const avatarBg = app ? avatarBackground(app, accent) : accent;

  openModal(`
    <div class="confirm-dialog">
      <div class="confirm-dialog-header">
        ${
          app
            ? `
          <div class="card-avatar" style="background:${avatarBg}; width: 48px; height: 48px; border-radius: 14px; font-size: 18px; flex-shrink: 0; overflow: hidden;">
            ${avatar}
          </div>
        `
            : ""
        }
        <div>
          <h2 class="confirm-dialog-title">${escapeHtml(title)}</h2>
          ${
            app
              ? `<small style="color: var(--text-medium); font-size: 12px; font-weight: 500;">${escapeHtml(app.author || "")} ${app.version ? "· v" + escapeHtml(app.version) : ""}</small>`
              : ""
          }
        </div>
      </div>
      <p class="confirm-dialog-msg">${escapeHtml(message)}</p>
      ${renderChoices(choices)}
      <div class="modal-foot" style="margin-top: 20px;">
        <button type="button" class="btn ghost" id="modal-btn-cancel">Cancelar</button>
        <button type="button" class="btn ${confirmVariant}" id="modal-btn-confirm">${escapeHtml(confirmText)}</button>
      </div>
    </div>
  `);

  document.getElementById("modal-btn-cancel").onclick = closeModal;
  document.getElementById("modal-btn-confirm").onclick = async () => {
    const picked = document.querySelector('input[name="modal-choice"]:checked')?.value;
    closeModal();
    if (onConfirm) await onConfirm(picked);
  };
}

function showAlertModal(title, message) {
  openModal(`
    <div class="confirm-dialog">
      <h2 class="confirm-dialog-title">${escapeHtml(title)}</h2>
      <p class="confirm-dialog-msg">${escapeHtml(message)}</p>
      <div class="modal-foot" style="margin-top: 20px;">
        <button type="button" class="btn primary" id="modal-btn-close">Aceptar</button>
      </div>
    </div>
  `);
  document.getElementById("modal-btn-close").onclick = closeModal;
}

function openThemePicker() {
  const draft = {
    theme: state.settings.theme,
    accent: state.settings.accent,
    accent_locked: false,
  };
  const preset = THEME_PRESETS[draft.theme] || THEME_PRESETS.plata;
  openModal(`
    <h2>Personalizar apariencia</h2>
    <p class="sub">Tema neutro · blanco, negro, gris y plata</p>
    <div class="modal-section">
      <label>Tema base</label>
      <div class="preset-grid">
        <button type="button" class="preset-card active" data-theme="plata">
          <div class="swatch">${preset.swatch.map((c) => `<i style="background:${c}"></i>`).join("")}</div>
          <strong>${preset.label}</strong>
          <span>blanco / negro / gris / plata</span>
        </button>
      </div>
    </div>
    <div class="modal-section">
      <label>Color de acento</label>
      <div class="accent-grid">
        ${ACCENT_CHOICES.map(
          (c) =>
            `<button type="button" class="accent-dot ${normalizeHex(c) === normalizeHex(draft.accent) ? "active" : ""}"
              data-accent="${c}" style="background:${c}" aria-label="Usar color de acento ${c}" title="${c}"></button>`
        ).join("")}
      </div>
    </div>
    <div class="modal-section">
      <label>Color personalizado</label>
      <div style="display:flex; align-items:center; gap:12px; flex-wrap:wrap;">
        <input type="color" id="custom-accent-picker" value="${draft.accent}" aria-label="Color de acento personalizado"
          style="width:52px; height:38px; border:none; background:transparent; border-radius:10px; cursor:pointer;" />
        <button type="button" class="btn ghost" id="theme-reset-neutral">Restaurar plata</button>
      </div>
    </div>
    <div class="modal-section preview">
      <div id="theme-preview-label">${preset.label}  ·  acento ${draft.accent}</div>
      <div class="preview-bar" style="background:${preset.vars["--bg-app"]}">
        <div class="side" style="background:${preset.vars["--bg-sidebar"]}"></div>
        <div class="card" style="background:${preset.vars["--bg-card"]}"></div>
        <div class="dot" id="theme-preview-dot" style="background:${draft.accent}"></div>
      </div>
    </div>
    <div class="modal-foot">
      <button type="button" class="btn ghost" id="theme-cancel">Cancelar</button>
      <button type="button" class="btn" id="theme-apply">Aplicar</button>
    </div>
  `);

  const modal = document.getElementById("modal");
  const picker = document.getElementById("custom-accent-picker");
  const previewLabel = document.getElementById("theme-preview-label");
  const previewDot = document.getElementById("theme-preview-dot");
  const updatePreview = () => {
    const normalized = normalizeHex(draft.accent);
    modal.querySelectorAll("[data-accent]").forEach((button) => {
      button.classList.toggle("active", normalizeHex(button.dataset.accent) === normalized);
    });
    const accent = `#${normalized}`;
    if (picker.value !== accent) picker.value = accent;
    previewLabel.textContent = `${preset.label}  ·  acento ${accent}`;
    previewDot.style.background = accent;
  };

  modal.querySelector(".accent-grid").addEventListener("click", (event) => {
    const button = event.target.closest("[data-accent]");
    if (!button) return;
    draft.accent = button.dataset.accent;
    draft.accent_locked = true;
    updatePreview();
  });
  picker.addEventListener("input", (event) => {
    draft.accent = event.target.value;
    draft.accent_locked = true;
    updatePreview();
  });
  document.getElementById("theme-reset-neutral").onclick = () => {
    draft.accent = THEME_PRESETS.plata.default_accent;
    draft.accent_locked = false;
    updatePreview();
  };
  document.getElementById("theme-cancel").onclick = closeModal;
  document.getElementById("theme-apply").onclick = async () => {
    const settings = { theme: draft.theme, accent: `#${normalizeHex(draft.accent)}` };
    try {
      await invoke("save_settings", { settings });
      applyTheme(settings.theme, settings.accent);
      closeModal();
    } catch (error) {
      setStatus(`No se pudo guardar la apariencia: ${error}`, "var(--red)");
      showAlertModal("Error al guardar la apariencia", String(error));
    }
  };
}

async function openCatalogEditor() {
  let templates;
  try {
    templates = (await invoke("get_templates")) || [];
  } catch (error) {
    setStatus(`No se pudo abrir el catálogo: ${error}`, "var(--red)");
    showAlertModal("Error al abrir el catálogo", String(error));
    return;
  }
  openModal(
    `
    <h2>Catálogo de aplicaciones</h2>
    <p class="sub">Edita el JSON y pulsa Guardar. Usa plantillas para empezar rápido.</p>
    <div class="catalog-tools" id="catalog-tools"></div>
    <textarea class="catalog-editor" id="catalog-json">${escapeHtml(JSON.stringify(state.catalog, null, 2))}</textarea>
    <div class="modal-foot">
      <button type="button" class="btn ghost" id="cat-templates">Plantillas</button>
      <button type="button" class="btn ghost" id="cat-cancel">Cancelar</button>
      <button type="button" class="btn" id="cat-save">Guardar</button>
    </div>
  `,
    true
  );

  const tools = document.getElementById("catalog-tools");
  tools.innerHTML = templates
    .map(
      (t, i) =>
        `<button type="button" class="btn ghost sm" data-tmpl="${i}">+ ${escapeHtml((t._comment || `Plantilla ${i + 1}`).slice(0, 28))}</button>`
    )
    .join("");

  const ta = document.getElementById("catalog-json");
  const strip = (t) => {
    const o = { ...t };
    delete o._comment;
    return o;
  };

  tools.querySelectorAll("[data-tmpl]").forEach((btn) => {
    btn.addEventListener("click", () => {
      let current = [];
      try {
        current = JSON.parse(ta.value || "[]");
      } catch {
        current = [];
      }
      if (!Array.isArray(current)) current = [];
      current.push(strip(templates[Number(btn.dataset.tmpl)]));
      ta.value = JSON.stringify(current, null, 2);
    });
  });

  document.getElementById("cat-templates").onclick = () => {
    ta.value = JSON.stringify(templates.map(strip), null, 2);
  };
  document.getElementById("cat-cancel").onclick = closeModal;
  document.getElementById("cat-save").onclick = async () => {
    try {
      const catalogData = validateCatalog(JSON.parse(ta.value));
      const path = await invoke("save_catalog", { apps: catalogData });
      // save_catalog has already rebuilt statuses for the new catalog. Read
      // that coherent snapshot instead of keeping statuses from the old list.
      await reconcileRuntimeFromBootstrap({ includeCatalog: true, includeSettings: true });
      closeModal();
      renderSidebar();
      renderContent();
      showAlertModal("Catálogo guardado", `Archivo actualizado en:\n${path}`);
    } catch (e) {
      showAlertModal("Catálogo no válido", String(e));
    }
  };
}

async function reloadCatalog() {
  try {
    replaceCatalog((await invoke("reload_catalog")) || []);
    hydrateResolvedIcons();
    setStatus(`Catálogo recargado: ${state.catalog.length} apps`, "var(--green)");
    renderSidebar();
    renderContent();
  } catch (error) {
    setStatus(`No se pudo recargar el catálogo: ${error}`, "var(--red)");
    showAlertModal("Error al recargar el catálogo", String(error));
  }
}

/* -------------------------------------------------------------------------- */
/*  Central Sync Loading Modal Helper Functions                               */
/* -------------------------------------------------------------------------- */

/**
 * The loading veil over the list of applications.
 *
 * Same logo and same spinning ring as the sync screen, without its card: what
 * is being reported already has a place to be reported in — the status bar at
 * the foot of the window, which stays visible along with the header. So this
 * covers only the sidebar and the list, which are the parts about to be
 * rewritten, and says nothing the bar is not already saying.
 */
let shellBusyOverlay = null;
let syncModalOverlay = null;
/// Set while an action reports its own progress well enough that covering the
/// list would take more away than it explains.
let suppressShellBusy = false;

function showShellBusy() {
  // The full sync screen already covers everything, card included. Two veils
  // over one another would only darken the window twice.
  if (shellBusyOverlay || syncModalOverlay || suppressShellBusy) return;
  const shell = document.querySelector(".shell");
  if (!shell) return;
  shellBusyOverlay = document.createElement("div");
  shellBusyOverlay.className = "shell-busy";
  shellBusyOverlay.setAttribute("aria-hidden", "true");
  shellBusyOverlay.innerHTML = `
    <div class="sync-logo-box">
      <div class="sync-logo-ring"></div>
      <div class="sync-logo-wrapper">
        <img src="assets/winslim-center-logo.png" width="904" height="904" alt="" />
      </div>
    </div>`;
  shell.appendChild(shellBusyOverlay);
}

function hideShellBusy() {
  if (!shellBusyOverlay) return;
  const overlay = shellBusyOverlay;
  shellBusyOverlay = null;
  overlay.classList.add("closing");
  // Long enough for the fade to play out, short enough that a rescan finishing
  // and another starting do not stack veils.
  setTimeout(() => overlay.remove(), 280);
}

function showSyncModal(title = "WinSlimCenter", subtitle = "Sincronizando tienda...") {
  if (syncModalOverlay) return;
  syncModalOverlay = document.createElement("div");
  syncModalOverlay.className = "sync-modal-overlay";
  syncModalOverlay.innerHTML = `
    <div class="sync-modal-card" role="dialog" aria-modal="true">
      <div class="sync-logo-box">
        <div class="sync-logo-ring"></div>
        <div class="sync-logo-wrapper">
          <img src="assets/winslim-center-logo.png" width="904" height="904" alt="WinSlimCenter" />
        </div>
      </div>
      <div class="sync-title-group">
        <h2 id="sync-modal-title">${escapeHtml(title)}</h2>
        <p id="sync-modal-subtitle">${escapeHtml(subtitle)}</p>
      </div>
      <div class="sync-progress-container">
        <div class="sync-progress-track">
          <div class="sync-progress-bar" id="sync-modal-bar" style="width: 10%"></div>
        </div>
        <div class="sync-status-row">
          <span class="sync-status-text" id="sync-modal-status">Iniciando escaneo...</span>
          <span class="sync-status-percent" id="sync-modal-percent">10%</span>
        </div>
      </div>
    </div>
  `;
  document.body.appendChild(syncModalOverlay);
}

function updateSyncModal(percent, text) {
  if (!syncModalOverlay) return;
  const bar = document.getElementById("sync-modal-bar");
  const status = document.getElementById("sync-modal-status");
  const percentEl = document.getElementById("sync-modal-percent");
  if (bar && percent != null) {
    const value = Math.min(100, Math.max(0, Number(percent) || 0));
    bar.style.width = `${value}%`;
  }
  if (status && text) status.textContent = text;
  if (percentEl && percent != null) percentEl.textContent = `${Math.min(100, Math.max(0, percent))}%`;
}

function closeSyncModal() {
  if (!syncModalOverlay) return;
  syncModalOverlay.classList.add("closing");
  setTimeout(() => {
    if (syncModalOverlay && syncModalOverlay.parentNode) {
      syncModalOverlay.parentNode.removeChild(syncModalOverlay);
    }
    syncModalOverlay = null;
  }, 280);
}

/**
 * Refreshes catalog, statuses and available updates.
 *
 * The full-screen sync modal belongs to the two moments the user is explicitly
 * waiting for the store: opening the app and pressing Refrescar. Every other
 * refresh — after an install, an update or an error — runs in `silent` mode and
 * reports through the status bar at the bottom, so finishing an operation no
 * longer throws the startup loading screen back in the user's face.
 */
async function refreshStore({ reportErrors = true, silent = false } = {}) {
  const button = document.getElementById("btn-refresh");
  const original = button.innerHTML;
  if (!silent) {
    button.disabled = true;
    button.innerHTML = '<span>↻</span> Refrescando...';
    showSyncModal("Sincronizando Tienda", "Recargando catálogo y escaneando estado del sistema...");
    updateSyncModal(15, "Recargando catálogo de aplicaciones...");
  }
  setStatus("Recargando aplicaciones y buscando actualizaciones...", "var(--accent)");
  setProgress(silent ? 15 : 0);

  try {
    replaceCatalog((await invoke("reload_catalog")) || []);
    hydrateResolvedIcons();
    if (!silent) {
      updateSyncModal(40, "Analizando registro y programas instalados...");
    }
    setProgress(40);
    replaceStatuses((await invoke("refresh_statuses")) || {});
    if (!silent) {
      updateSyncModal(85, "Comprobando versiones y actualizaciones...");
    }
    setStatus("Comprobando versiones y actualizaciones disponibles...", "var(--accent)");
    setProgress(86);
    replaceStatuses((await invoke("check_updates")) || state.statuses);
    state.lastUpdateScan = new Date();
    const updates = updatesCount();
    renderSidebar();
    renderContent();
    if (!silent) {
      updateSyncModal(100, "¡Sincronización completada!");
      await new Promise((r) => setTimeout(r, 350));
    }

    if (updates) {
      setStatus(
        `${updates} ${updates === 1 ? "actualización encontrada" : "actualizaciones encontradas"}`,
        "var(--accent)",
      );
    } else {
      setTransientStatus("Comprobación del sistema completada.", "var(--green)", 4000);
    }
  } catch (error) {
    setStatus(`No se pudo refrescar la tienda: ${error}`, "var(--red)");
    if (reportErrors) showAlertModal("Error al refrescar", String(error));
  } finally {
    if (!silent) {
      closeSyncModal();
      button.disabled = false;
      button.innerHTML = original;
    }
    setProgress(100);
  }
}

async function finishStartupInBackground() {
  showSyncModal("Iniciando WinSlimCenter", "Analizando el equipo y preparando la tienda...");
  updateSyncModal(20, "Escaneando aplicaciones instaladas y registro...");
  try {
    const detectedChanges = replaceStatuses((await invoke("refresh_statuses")) || state.statuses);
    reconcileStatusChanges(detectedChanges);

    updateSyncModal(80, "Comprobando actualizaciones disponibles...");
    setStatus("Comprobando versiones y actualizaciones disponibles...", "var(--accent)");
    setProgress(80);
    
    try {
      const updateChanges = replaceStatuses((await invoke("check_updates")) || state.statuses);
      state.lastUpdateScan = new Date();
      reconcileStatusChanges(updateChanges);
      const updates = updatesCount();
      updateSyncModal(100, "¡Tienda lista y optimizada!");
      await new Promise(r => setTimeout(r, 350));
      
      if (updates) {
        setStatus(
          `${updates} ${updates === 1 ? "actualización encontrada" : "actualizaciones encontradas"}`,
          "var(--accent)",
        );
      } else {
        setTransientStatus("Comprobación del sistema completada.", "var(--green)", 4000);
      }
    } catch (error) {
      setStatus("Aplicaciones detectadas · No se pudieron comprobar todas las actualizaciones", "var(--text-medium)");
      clientLog("warn", "startup-updates", String(error?.stack || error));
    }
  } catch (error) {
    setStatus(`La tienda está disponible, pero falló la comprobación del sistema: ${error}`, "var(--red)");
    clientLog("error", "startup-background", String(error?.stack || error));
  } finally {
    closeSyncModal();
    setProgress(100);
    // Only once the start-up screen has let go of the modal, so the two never
    // fight over it.
    void checkStoreUpdate();
  }
}

async function refreshInstalledFromBootstrap() {
  const data = await invoke("get_bootstrap");
  state.appVersion = data.app_version || state.appVersion;
  state.installed = data.installed || {};
  const changes = replaceStatuses(data.statuses || {});
  renderAppVersion();
  return changes;
}

async function reconcileRuntimeFromBootstrap({ includeCatalog = false, includeSettings = false } = {}) {
  const previousTaskIds = new Set(state.tasks.map((task) => task.app_id));
  const data = await invoke("get_bootstrap");
  if (includeCatalog) {
    replaceCatalog(data.catalog || []);
    hydrateResolvedIcons();
  }
  state.appVersion = data.app_version || state.appVersion;
  state.installed = data.installed || {};
  const changes = replaceStatuses(data.statuses || {});
  replaceTasks(data.tasks || []);
  if (includeSettings) {
    state.settings = data.settings || state.settings;
    applyTheme(state.settings.theme, state.settings.accent);
  }
  renderAppVersion();
  reconcileStatusChanges(changes);
  updateVisibleAppActions(new Set([...previousTaskIds, ...state.tasks.map((task) => task.app_id)]));
  renderDlPanel();
  updatePackageOperation(state.tasks);
  return { data, changes };
}

async function startStoreUpdate() {
  try {
    setStatus("Descargando la nueva versión de la tienda...", "var(--accent)");
    const msg = await invoke("update_center_app");
    setStatus(String(msg || "Actualización iniciada"), "var(--accent)");
  } catch (e) {
    setStatus(`Error de actualización: ${e}`, "var(--red)");
    showAlertModal("Error de actualización", String(e));
  }
}

// One of the people behind the store, as a button that opens their GitHub
// profile. The mark and the arrow say it leads somewhere without a line of text
// having to explain it.
function aboutPersonHtml(handle, role, url) {
  return `
    <button type="button" class="about-person" data-url="${escapeHtml(url)}" title="Abrir ${escapeHtml(url)}">
      <svg class="about-person-mark" width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="M12 .5C5.37.5 0 5.87 0 12.5c0 5.3 3.44 9.8 8.21 11.39.6.11.82-.26.82-.58v-2.03c-3.34.73-4.04-1.61-4.04-1.61-.55-1.39-1.34-1.76-1.34-1.76-1.09-.75.08-.73.08-.73 1.2.08 1.84 1.24 1.84 1.24 1.07 1.83 2.81 1.3 3.5.99.11-.78.42-1.3.76-1.6-2.67-.3-5.47-1.33-5.47-5.93 0-1.31.47-2.38 1.24-3.22-.13-.3-.54-1.52.11-3.18 0 0 1.01-.32 3.3 1.23a11.5 11.5 0 0 1 6 0c2.29-1.55 3.3-1.23 3.3-1.23.65 1.66.24 2.88.12 3.18.77.84 1.23 1.91 1.23 3.22 0 4.61-2.8 5.62-5.48 5.92.43.37.81 1.1.81 2.22v3.29c0 .32.22.7.83.58A12.01 12.01 0 0 0 24 12.5C24 5.87 18.63.5 12 .5z"/>
      </svg>
      <span class="about-person-text">
        <strong>${escapeHtml(handle)}</strong>
        <span>${escapeHtml(role)}</span>
      </span>
      <svg class="about-person-go" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <line x1="7" y1="17" x2="17" y2="7"/>
        <polyline points="8 7 17 7 17 16"/>
      </svg>
    </button>`;
}

function showAboutModal() {
  const catalogSize = state.catalog.length;
  openModal(`
    <div class="about-dialog">
      <div class="about-logo">
        <img src="assets/winslim-center-logo.png" width="904" height="904" alt="WinSlimCenter" />
      </div>
      <h2 class="about-name">WinSlimCenter</h2>
      <p class="about-version">${state.appVersion ? `Versión ${escapeHtml(state.appVersion)}` : "Tienda de aplicaciones"}</p>
      <dl class="about-facts">
        <div class="about-fact people">
          <dt>Desarrollo</dt>
          <dd>
            <div class="about-people">
              ${aboutPersonHtml(
                "Christianlg97",
                "Creador de WinSlimOS y desarrollador de esta tienda.",
                "https://github.com/Christianlg97",
              )}
              ${aboutPersonHtml(
                "tiranosaurio73",
                "Colaborador del proyecto.",
                "https://github.com/tiranosaurio73",
              )}
              ${aboutPersonHtml(
                "Darkeiser003",
                "Colaborador y autor de WinSlimTerminal.",
                "https://github.com/Darkeiser003",
              )}
            </div>
          </dd>
        </div>
        <div class="about-fact">
          <dt>Construido con</dt>
          <dd>
            <strong>Rust · Tauri 2</strong>
            <span>Interfaz en JavaScript, HTML y CSS. Integración con Windows en PowerShell.</span>
          </dd>
        </div>
        <div class="about-fact">
          <dt>Catálogo</dt>
          <dd>
            <strong>${catalogSize} aplicaciones</strong>
            <span>Seleccionadas una a una, cada una desde su origen oficial.</span>
          </dd>
        </div>
        <div class="about-fact">
          <dt>Licencia</dt>
          <dd>
            <strong>GNU GPL v3</strong>
            <span>Software libre: úsalo, estúdialo, modifícalo y compártelo.</span>
          </dd>
        </div>
      </dl>
      <p class="about-note hidden" id="about-note" role="status" aria-live="polite"></p>
      <div class="modal-foot" style="margin-top: 4px;">
        <button type="button" class="btn ghost" id="about-check">Buscar actualizaciones</button>
        <button type="button" class="btn primary" id="about-close">Cerrar</button>
      </div>
    </div>
  `);
  // `modal-about` is what lets the facts list scroll while the buttons stay put:
  // this is the one hero dialog that can outgrow the height a modal is allowed.
  document.getElementById("modal").classList.add("modal-hero", "modal-about");
  document.getElementById("about-close").onclick = closeModal;

  const check = document.getElementById("about-check");
  const note = document.getElementById("about-note");

  document.querySelectorAll(".about-person").forEach((person) => {
    person.addEventListener("click", async () => {
      try {
        await invoke("open_url", { url: person.dataset.url });
      } catch (error) {
        note.textContent = "No se pudo abrir el navegador para ese perfil.";
        note.className = "about-note bad";
        clientLog("warn", "about", `No se pudo abrir ${person.dataset.url}: ${error}`);
      }
    });
  });
  check.onclick = async () => {
    const label = check.textContent;
    check.disabled = true;
    check.textContent = "Buscando…";
    note.className = "about-note hidden";
    try {
      const update = await invoke("check_store_update");
      if (update) {
        closeModal();
        showStoreUpdateModal(update);
        return;
      }
      note.textContent = `Ya tienes la versión más reciente${state.appVersion ? ` (v${state.appVersion})` : ""}.`;
      note.className = "about-note ok";
    } catch (error) {
      // Asked for on purpose, so the answer cannot be a silent shrug the way it
      // is when the check runs by itself at start-up — and it says what actually
      // happened, because "comprueba la conexión" sent the user chasing a
      // network problem that was never there.
      note.textContent = String(error) || "No se pudo consultar GitHub.";
      note.className = "about-note bad";
      clientLog("warn", "self-update", `Comprobación manual fallida: ${error}`);
    } finally {
      check.disabled = false;
      check.textContent = label;
    }
  };
}

// Shown once per run when the repository publishes a build newer than this one.
function showStoreUpdateModal(update) {
  const version = String(update?.version || "").trim();
  const current = String(update?.current || state.appVersion || "").trim();
  const versions = version
    ? `<div class="store-update-versions">
         <span class="from">v${escapeHtml(current)}</span>
         <span class="arrow">→</span>
         <span class="to">v${escapeHtml(version)}</span>
       </div>`
    : "";
  openModal(`
    <div class="store-update">
      <div class="store-update-logo">
        <img src="assets/winslim-center-logo.png" width="904" height="904" alt="WinSlimCenter" />
        <span class="store-update-badge" aria-hidden="true">
          <svg class="store-update-badge-icon" width="17" height="17" viewBox="0 0 24 24" fill="none"
               stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="4" x2="12" y2="14.5"/>
            <polyline points="7.5 10 12 14.5 16.5 10"/>
            <line x1="5.5" y1="19" x2="18.5" y2="19"/>
          </svg>
        </span>
      </div>
      <h2 class="store-update-title">Actualización disponible</h2>
      <p class="store-update-sub">
        Hay una versión más reciente de WinSlimCenter publicada. La tienda se
        cerrará para aplicarla y volverá a abrirse sola.
      </p>
      ${versions}
      <div class="modal-foot" style="margin-top: 6px;">
        <button type="button" class="btn ghost" id="store-update-skip">Omitir</button>
        <button type="button" class="btn primary" id="store-update-now">Actualizar ahora</button>
      </div>
    </div>
  `);
  document.getElementById("modal").classList.add("modal-hero");
  document.getElementById("store-update-skip").onclick = () => {
    closeModal();
    setTransientStatus("Actualización omitida. Se volverá a avisar al abrir la tienda.", "var(--text-medium)", 6000);
  };
  document.getElementById("store-update-now").onclick = async () => {
    closeModal();
    await startStoreUpdate();
  };
}

// Asks the repository whether it publishes something newer than this build.
//
// Runs once, after the start-up scan has released the screen, and stays silent
// about its own failures: not reaching GitHub is not something the user has to
// act on, and it must never get in the way of a store that already works.
async function checkStoreUpdate() {
  try {
    const update = await invoke("check_store_update");
    if (!update) return;
    clientLog("info", "self-update", `Versión publicada: ${update.version}, instalada: ${update.current}`);
    showStoreUpdateModal(update);
  } catch (error) {
    clientLog("warn", "self-update", `No se pudo comprobar la versión publicada: ${error}`);
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  void clientLog("info", "startup", "DOMContentLoaded: iniciando interfaz.");

  // Register every state-bearing event before asking for the snapshot. Events
  // emitted while get_bootstrap is in flight are queued and replayed in order
  // after the snapshot is applied, so neither side can overwrite the other.
  const backendEventHandlers = new Map();
  const queuedBackendEvents = [];
  let bootstrapApplied = false;
  let backendEventChain = Promise.resolve();
  const dispatchBackendEvent = (name, event) => {
    if (!bootstrapApplied) {
      queuedBackendEvents.push([name, event]);
      return;
    }
    backendEventChain = backendEventChain
      .then(() => backendEventHandlers.get(name)?.(event))
      .catch((error) => {
        void clientLog("error", "backend-event", `${name}: ${error?.stack || error}`);
      });
  };
  const listenerRegistrations = [
    listen("downloads-changed", (event) => dispatchBackendEvent("downloads-changed", event)),
    listen("background-progress", (event) => dispatchBackendEvent("background-progress", event)),
    listen("install-finished", (event) => dispatchBackendEvent("install-finished", event)),
  ];
  let bootstrapPromise;
  try {
    await Promise.all(listenerRegistrations);
    // The snapshot begins only after all listeners are confirmed active.
    bootstrapPromise = invoke("get_bootstrap");
  } catch (error) {
    setStatus(`Error al registrar eventos: ${error}`, "var(--red)");
    void clientLog("error", "startup-listeners", String(error?.stack || error));
    return;
  }

  bindShellDelegation();
  let searchTimer = null;
  const searchInput = document.getElementById("search");
  // Commands typed into the search box instead of a name. Each one says what to
  // announce while it works and what to show when it ends, well or badly.
  const searchCommands = new Map([
    [
      "/logs",
      {
        pending: null,
        run: async () => `Registro abierto: ${await invoke("open_logs")}`,
        failure: (error) => `No se pudo abrir el registro: ${error}`,
        errorTitle: "Error al abrir los logs",
      },
    ],
    [
      "/woa",
      {
        pending: "Descargando WOA...",
        run: async () => {
          await invoke("run_woa");
          return "WOA descargado y en ejecución";
        },
        failure: (error) => `No se pudo ejecutar WOA: ${error}`,
        errorTitle: "Error al ejecutar WOA",
      },
    ],
  ]);
  searchInput.addEventListener("keydown", async (event) => {
    if (event.key !== "Enter") return;
    const typed = searchInput.value.trim().toLowerCase();
    const command = searchCommands.get(typed);
    if (!command) return;
    event.preventDefault();
    clearTimeout(searchTimer);
    // The box empties before the command runs, not after: a command that takes
    // seconds would otherwise leave the catalog filtered by a name nobody has.
    searchInput.value = "";
    state.search = "";
    renderContent();
    await clientLog("info", "command", `Comando ${typed} ejecutado desde la barra de búsqueda.`);
    if (command.pending) setStatus(command.pending, "var(--accent)");
    try {
      setTransientStatus(await command.run(), "var(--green)", 10000);
    } catch (error) {
      setTransientStatus(command.failure(error), "var(--red)", 10000);
      showAlertModal(command.errorTitle, String(error));
    }
  });
  searchInput.addEventListener("input", (e) => {
    state.search = e.target.value;
    clearTimeout(searchTimer);
    searchTimer = setTimeout(renderContent, 180);
  });
  document.getElementById("btn-about").addEventListener("click", showAboutModal);
  // Called without arguments on purpose: the click event must not leak into the
  // options object, and this is one of the two places allowed to show the modal.
  document.getElementById("btn-refresh").addEventListener("click", () => refreshStore());

  backendEventHandlers.set("downloads-changed", (ev) => {
    const previousStates = new Map(state.tasks.map((task) => [task.app_id, task.state]));
    replaceTasks(ev.payload.tasks || []);
    const currentStates = new Map(state.tasks.map((task) => [task.app_id, task.state]));
    const affected = new Set([...previousStates.keys(), ...currentStates.keys()]);
    if ([...affected].some((id) => previousStates.get(id) !== currentStates.get(id))) {
      updateVisibleAppActions(affected);
    }
    renderDlPanel();
    updatePackageOperation(state.tasks);
  });

  backendEventHandlers.set("background-progress", (ev) => {
    const { stage, message, progress } = ev.payload || {};
    if (message) {
      if (stage === "complete") setTransientStatus(message, "var(--green)", 4000);
      else setStatus(message, "var(--accent)");
    }
    if (progress != null) setProgress(progress);
    // The rescan after installing or uninstalling rewrites every card on
    // screen. Covering the list while it happens says the store is working on
    // it, instead of leaving a page that is about to change under the cursor.
    if (stage === "complete") hideShellBusy();
    else showShellBusy();
  });

  backendEventHandlers.set("install-finished", async (ev) => {
    const {
      ok,
      app_id,
      error,
      changed = true,
      is_update = false,
      cancelled = false,
      cancellation_kind = "",
      interrupted = false,
    } = ev.payload;
    delete state.busy[app_id];
    // Clearing the optimistic busy flag is itself a visible state change even
    // when installation changed no version and the later WinGet check fails.
    updateVisibleAppActions(new Set([app_id]));
    // A finished installation keeps its dialog, which turns into the completion
    // state below; everything else is replaced by the message that explains it.
    const ownsModal = state.operationAppId === app_id;
    if (ownsModal) {
      state.operationAppId = null;
      if (!ok) closeModal();
    }
    if (ok) {
      await reconcileRuntimeFromBootstrap();
      const app = findApp(app_id);
      if (app) {
        const message = is_update
          ? (changed ? `Actualizado: ${app.name}` : `${app.name} ya estaba actualizado`)
          : `Instalado: ${app.name}`;
        setTransientStatus(message, "var(--green)", 5000);
      }
      if (ownsModal) {
        showOperationCompleted(app, {
          canLaunch: !!appStatus(app_id).can_launch,
          isUpdate: is_update,
          changed,
        });
      }
      // Installation is already confirmed by the backend, so let the user see
      // that result immediately. WinGet version verification is independent
      // and reconciles only the cards it changes when it returns.
      void (async () => {
        try {
          const updateChanges = replaceStatuses((await invoke("check_updates")) || state.statuses);
          state.lastUpdateScan = new Date();
          reconcileStatusChanges(updateChanges);
        } catch (updateError) {
          void clientLog("warn", "post-install-updates", String(updateError?.stack || updateError));
        }
      })();
    } else {
      const app = findApp(app_id);
      const appName = app?.name || app_id;
      // The backend already cleaned the failed operation and probed this exact
      // application. Pull that committed snapshot without repeating the global
      // registry/Start Apps/WinGet scan on an error path.
      try {
        await reconcileRuntimeFromBootstrap();
      } catch (refreshError) {
        void clientLog("warn", "post-install-state", String(refreshError?.stack || refreshError));
      }
      if (cancelled && cancellation_kind === "installation") {
        setTransientStatus(`Instalación cancelada: ${appName}`, "var(--text-light)", 5000);
        showAlertModal(
          "Instalación cancelada",
          String(error || "La instalación fue cancelada por el usuario."),
        );
      } else if (cancelled) {
        setTransientStatus(`Descarga cancelada: ${appName}`, "var(--text-light)", 5000);
      } else if (interrupted) {
        setTransientStatus(`Instalación interrumpida: ${appName}`, "var(--text-medium)", 5000);
        showAlertModal(
          "Instalación interrumpida",
          String(error || "El instalador se cerró antes de completar la instalación."),
        );
      } else {
        setTransientStatus(`Error al instalar ${appName}`, "var(--red)", 5000);
        showAlertModal("Error de instalación", String(error || "Error desconocido"));
      }
    }
  });

  try {
    const data = await bootstrapPromise;
    replaceCatalog(data.catalog || []);
    hydrateResolvedIcons();
    state.appVersion = data.app_version || state.appVersion;
    state.installed = data.installed || {};
    replaceStatuses(data.statuses || {});
    state.settings = data.settings || state.settings;
    replaceTasks(data.tasks || []);
    applyTheme(state.settings.theme, state.settings.accent);
    renderAppVersion();
    renderSidebar();
    renderContent();
    renderDlPanel();
    setStatus(idleStatusSummary(), "var(--green)");
    clientLog("info", "startup", {
      catalog: state.catalog.length,
      installed: installedCount(),
      tasks: state.tasks.length,
      version: state.appVersion,
    });
    bootstrapApplied = true;
    for (const [name, event] of queuedBackendEvents.splice(0)) {
      dispatchBackendEvent(name, event);
    }
    // In particular, a queued install-finished must reconcile its committed
    // backend state before the slower global start-up scan is launched.
    await backendEventChain;
    // Give WebView two frames to display the complete store before starting
    // the slower Windows/Start Apps/Winget and update scans.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => void finishStartupInBackground());
    });
  } catch (e) {
    setStatus(`Error al iniciar: ${e}`, "var(--red)");
    clientLog("error", "startup", String(e?.stack || e));
    console.error(e);
  }
});
