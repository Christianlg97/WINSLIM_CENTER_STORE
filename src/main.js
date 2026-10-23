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
  { id: "msstore", label: "Microsoft Store", icon: "🛍️", filter: "__MSSTORE__" },
  // Lo que hay en este equipo, no en el catálogo: va en el grupo «Local» de la
  // barra lateral, junto al directorio de aplicaciones.
  { id: "installed", label: "Aplicaciones instaladas", icon: "✅", filter: "__INSTALLED__", group: "local" },
];

/**
 * La sección de la Microsoft Store no lista el catálogo: lo que enseña llega
 * de los servidores de Microsoft cuando se busca. Se anota aquí para que las
 * partes que recorren el catálogo —el buscador de arriba, los contadores de la
 * página— sepan que esta sección no les pertenece.
 */
const MSSTORE_SECTION = "msstore";
const MSSTORE_SUPPRESSED_UPDATES_KEY = "winslimcenter-msstore-suppressed-updates-v1";

/**
 * Versiones que Windows ya rechazó porque el equipo tiene un paquete superior.
 *
 * Se guardan por familia y versión publicada. Un reescaneo de la misma versión
 * no debe volver a ensuciar Pendientes; una versión distinta elimina sola la
 * exclusión y vuelve a mostrarse.
 */
function loadMsStoreSuppressedUpdates() {
  try {
    const parsed = JSON.parse(localStorage.getItem(MSSTORE_SUPPRESSED_UPDATES_KEY) || "{}");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function saveMsStoreSuppressedUpdates(entries) {
  try {
    localStorage.setItem(MSSTORE_SUPPRESSED_UPDATES_KEY, JSON.stringify(entries));
  } catch {
    // En modo privado el almacenamiento puede no estar disponible. La
    // exclusión sigue funcionando durante la sesión actual.
  }
}

/**
 * Las arquitecturas entre las que se puede elegir. `auto` es la del equipo, y
 * es la respuesta correcta salvo cuando se descarga un paquete para otro.
 */
const MSSTORE_ARCHS = [
  { id: "auto", label: "Automática" },
  { id: "x64", label: "x64" },
  { id: "arm64", label: "ARM64" },
  { id: "all", label: "Todas" },
];

const FEATURED_ORDER = [
  "winslim_terminal", "winslim_usb_creator", "powertoys", "vscode", "brave", "seven_zip",
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
  // Mientras el primer escaneo completo del equipo está en marcha. Las fichas de
  // `statusPending` son aquellas de las que nadie ha contestado todavía —ni la
  // pasada rápida del arranque ni la instantánea de la sesión anterior— y se
  // dibujan como «comprobando» en vez de ofrecer instalar algo que quizá ya
  // está puesto.
  verifyingStatuses: false,
  statusPending: new Set(),
  // «Actualizar todo», mientras dura. Cada fila se sigue por el mismo
  // identificador con el que el backend nombra su tarea —el id del catálogo, o
  // `msstore:PRODUCTO`—, así que los eventos de progreso y de fin caen en su
  // sitio sin traducir nada. `null` cuando no hay ninguna en marcha ni resumen
  // sin leer.
  bulkUpdate: null,
  // Las fichas del catálogo marcadas para instalarlas juntas, por id. Un `Set`
  // conserva el orden en que se eligieron, que es el orden en que entran en la
  // cola. Dura lo que la sesión: elegir es preparar una tanda, no un ajuste.
  selection: new Set(),
  // La Microsoft Store vive aparte del catálogo: su búsqueda, sus filtros y
  // sus resultados no sobreviven a un reinicio y no tienen por qué.
  msstore: {
    options: null,
    ring: null,
    arch: "auto",
    query: "",
    lastQuery: "",
    results: [],
    details: {},
    loading: false,
    error: "",
    installing: {},
    // Lo que Windows tiene puesto de la tienda, y lo que se sabe de sus
    // versiones. Lo primero se pregunta al equipo y es instantáneo; lo segundo
    // hay que ir a buscarlo al servicio de entrega y tarda.
    installed: [],
    installedLoaded: false,
    updates: {},
    suppressedUpdates: loadMsStoreSuppressedUpdates(),
    updateFamilyByProduct: {},
    scanning: false,
    lastScan: null,
    busy: {},
    // Lo que la Microsoft Store contesta a lo que se escribe en la barra de
    // arriba, que es una pregunta distinta de la que hace su propia sección:
    // se guardan aparte para que una no borre la otra al volver.
    inline: {
      // La consulta que describen los resultados guardados, y la última que
      // se lanzó. Difieren mientras una respuesta está en el aire.
      query: "",
      requested: "",
      results: [],
      loading: false,
      error: "",
    },
  },
  // Lo que WinGet y Chocolatey contestan a lo que se escribe en la barra de
  // arriba. Cada uno guarda lo suyo, igual que la Microsoft Store, para que el
  // que tarde no borre lo que el otro ya trajo. De Chocolatey se recuerda
  // además si el equipo lo tiene: `null` mientras no se ha preguntado.
  repos: {
    winget: { query: "", requested: "", results: [], loading: false, error: "" },
    choco: {
      query: "",
      requested: "",
      results: [],
      loading: false,
      error: "",
      available: null,
      version: "",
      // Si la tienda tiene el token de administrador, que es lo que Chocolatey
      // necesita para todo. Se supone que sí mientras no se sepa: su manifiesto
      // lo pide, y avisar de lo contrario antes de haberlo comprobado sería
      // alarmar por nada.
      elevated: true,
    },
  },
};

const derived = {
  catalogGeneration: 0,
  catalogById: new Map(),
  searchBlobs: new Map(),
  taskByAppId: new Map(),
  iconCandidatesById: new Map(),
  iconSignaturesById: new Map(),
  consoleNames: [],
  // Los paquetes de WinGet y de Chocolatey que esta sesión ha mandado
  // instalar. No son catálogo —no salen en ninguna lista ni se guardan— pero
  // durante su instalación el resto de la tienda tiene que poder encontrarlos
  // por su identificador, que es lo que hace `findApp`.
  repoPackages: new Map(),
  // Los paquetes que el catálogo ya cubre, en minúsculas, para descontarlos de
  // lo que contesten los repositorios: la tienda no puede ofrecer dos veces el
  // mismo programa con dos fichas distintas.
  catalogPackageKeys: { winget: new Set(), choco: new Set() },
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
  derived.catalogPackageKeys = {
    winget: new Set(
      state.catalog
        .map((app) => String(app.winget_id || "").toLowerCase())
        .filter(Boolean),
    ),
    choco: new Set(
      state.catalog.map((app) => String(app.choco_id || "").toLowerCase()).filter(Boolean),
    ),
  };
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

function replaceRepoEntries(entries) {
  state.repoEntries = Array.isArray(entries) ? entries : [];
}

function knownApps() {
  const extra = (state.repoEntries || []).filter((entry) => !state.catalog.some((app) =>
    app.id === entry.id || (entry.winget_id && app.winget_id === entry.winget_id) ||
    (entry.choco_id && app.choco_id === entry.choco_id)
  ));
  return [...state.catalog, ...extra];
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
    status?.pending_restart,
    status?.latest_version, status?.can_uninstall, status?.can_launch,
  ].join("|");
  for (const id of ids) {
    if (signature(previous[id]) === signature(next[id])) continue;
    changedIds.add(id);
    if (!!previous[id]?.installed !== !!next[id]?.installed) installedChanged = true;
    if (!!previous[id]?.update_available !== !!next[id]?.update_available) updatesChanged = true;
  }
  state.statuses = next;
  return { ids: changedIds, installedChanged, updatesChanged };
}

function reconcileStatusChanges(changes) {
  if (!changes?.ids?.size) return;
  if (changes.installedChanged || changes.updatesChanged) renderSidebar();
  const componentVisibilityChanged = changes.installedChanged &&
    [...changes.ids].some((id) => findApp(id)?.source_type === "component");
  if (
    componentVisibilityChanged ||
    (repoInlineFits() && state.search.trim() && Object.values(state.repos).some((repo) => repo.results.length)) ||
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
      const normalizedTargetDomain = String(targetDomain).toLowerCase();
      if (normalizedTargetDomain === "epicgames.com" || normalizedTargetDomain.endsWith(".epicgames.com")) {
        targetDomain = "epicgames.com";
      }
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
    : `Todo al día · ${visibleCatalogCount()} aplicaciones`;
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

/// La respuesta del primer escaneo del equipo, mientras se espera.
let statusVerification = null;

/// Para las tandas: se arman con lo que el equipo tiene de verdad, no con lo
/// que recordaba la sesión anterior.
async function untilStatusesVerified() {
  const verification = statusVerification;
  if (!verification) return;
  setStatus("Comprobando el equipo antes de empezar…", "var(--accent)");
  await verification.catch(() => {});
}

function isStatusPending(id) {
  return state.verifyingStatuses && state.statusPending.has(id);
}

function hasPendingStatuses() {
  return state.verifyingStatuses && state.statusPending.size > 0;
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
      title="Comparar tus aplicaciones instaladas con WinGet y con la Microsoft Store">
      ${icon}<span>${scanning ? "Buscando actualizaciones…" : "Buscar actualizaciones"}</span>
    </button>`;
}

/**
 * El botón que lanza todas las actualizaciones pendientes de una vez.
 *
 * Mientras hay una tanda en marcha no se desactiva: se convierte en la manera
 * de volver a su diálogo, que es lo que quiere quien lo pulsa entonces. Cuando
 * no queda nada que pedir —todo en marcha ya— desaparece en lugar de quedarse
 * apagado prometiendo algo que no haría.
 */
function updateAllButtonHtml() {
  const running = isBulkUpdateRunning();
  const pending = running ? 0 : pendingUpdateItems().length;
  if (!running && !pending) return "";
  return `
    <button type="button" class="btn primary updates-toolbar-all" id="btn-update-all"
      ${running ? 'aria-busy="true"' : ""}
      title="Actualiza a la vez todo lo que tiene versión nueva: primero el catálogo de WinSlimCenter y después la Microsoft Store">
      <svg class="btn-svg-icon" width="14" height="14" viewBox="0 0 24 24" fill="none"
        stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M12 3v11" />
        <path d="m7.5 10.5 4.5 4.5 4.5-4.5" />
        <path d="M4 20h16" />
      </svg>
      <span>${
        running
          ? state.bulkUpdate.mode === "install" ? "Instalación en cola…" : "Actualizando todo…"
          : `Actualizar todo (${pending})`
      }</span>
    </button>`;
}

function lastScanLabel() {
  if (state.scanningUpdates) return "Consultando WinGet y la Microsoft Store...";
  if (!state.lastUpdateScan) return "Todas las apps vigentes";
  const time = state.lastUpdateScan.toLocaleTimeString("es-ES", {
    hour: "2-digit",
    minute: "2-digit",
  });
  // Sin nombrar sólo a WinGet: la comprobación pregunta a los dos sitios, y
  // decir uno hacía pensar que el otro se quedaba sin mirar.
  return `Última comprobación · ${time}`;
}

/// El logotipo de la tienda con su distintivo: un tic, o la lupa de una
/// búsqueda sin coincidencias.
function emptyUpdatesBadgeHtml(mark, extraClass = "") {
  return `
    <div class="empty-updates-badge-container" aria-hidden="true">
      <div class="empty-updates-logo-wrapper">
        <img src="assets/winslim-center-logo.png" width="904" height="904" alt="" />
      </div>
      <div class="empty-updates-check-badge ${extraClass}">${mark}</div>
    </div>`;
}

/**
 * «Actualizaciones» cuando no queda nada pendiente: un panel a lo ancho, arriba
 * y en el mismo sitio que la barra que aparece cuando sí las hay, con el botón
 * de volver a comprobar a la derecha. Una tarjeta estrecha centrada en todo el
 * alto de la sección dejaba más hueco que contenido.
 */
function updatesUpToDateCardHtml() {
  return `
    <div class="empty-updates-card" aria-label="Sin actualizaciones pendientes">
      ${emptyUpdatesBadgeHtml("✓")}
      <div class="empty-updates-copy">
        <h2>Todo tu software está al día</h2>
        <p>WinSlimCenter ha comparado tus aplicaciones instaladas con WinGet y con la Microsoft Store, y no hay versiones nuevas pendientes.</p>
        <div class="empty-updates-status">
          <span class="pulse-dot"></span> ${escapeHtml(lastScanLabel())}
        </div>
      </div>
      <div class="empty-updates-action">${scanUpdatesButtonHtml({ hero: true })}</div>
    </div>`;
}

/**
 * El mismo panel cuando sí hay actualizaciones pendientes pero ninguna casa con
 * lo escrito en la barra. Misma silueta para que la sección no cambie de cara
 * al escribir: sólo cambian el distintivo, el texto y la acción, que aquí es
 * vaciar la búsqueda para volver a verlas todas.
 */
function updatesSearchMissCardHtml(query) {
  const pending = updatesCount();
  const pendingLabel = `${pending} ${pending === 1 ? "actualización pendiente" : "actualizaciones pendientes"}`;
  const lens = `
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      stroke-width="3" stroke-linecap="round" stroke-linejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-4-4" />
    </svg>`;
  return `
    <div class="empty-updates-card empty-updates-search-miss" aria-label="Ninguna actualización coincide con la búsqueda">
      ${emptyUpdatesBadgeHtml(lens, "empty-updates-search-badge")}
      <div class="empty-updates-copy">
        <h2>Sin actualizaciones para «${escapeHtml(query)}»</h2>
        <p>Tienes ${pendingLabel}, pero ninguna coincide con esa búsqueda. Prueba con otro nombre o vacía la barra para verlas todas.</p>
        <div class="empty-updates-status">
          <span class="pulse-dot"></span> ${escapeHtml(lastScanLabel())}
        </div>
      </div>
      <div class="empty-updates-action">
        <button type="button" class="btn primary empty-updates-clear" id="btn-clear-search"
          title="Vaciar la barra de búsqueda y volver a ver todas las actualizaciones pendientes">
          <svg class="btn-svg-icon" width="16" height="16" viewBox="0 0 24 24" fill="none"
            stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <rect x="3" y="3" width="7" height="7" rx="1.5" />
            <rect x="14" y="3" width="7" height="7" rx="1.5" />
            <rect x="3" y="14" width="7" height="7" rx="1.5" />
            <rect x="14" y="14" width="7" height="7" rx="1.5" />
          </svg>
          <span>${pending === 1 ? "Ver la actualización pendiente" : `Ver las ${pending} actualizaciones`}</span>
        </button>
      </div>
    </div>`;
}

/**
 * Explicit "look for updates now" pass.
 *
 * Detection runs first so that apps installed or removed since the last scan are
 * taken into account, and only then does the backend ask WinGet — otherwise the
 * scan would compare against a stale list of installed packages.
 */
/// Mientras «Buscar actualizaciones» espera a la Microsoft Store con la barra
/// de estado a la vista.
let msStoreProgressVisible = false;

/// «7 de 17» en vez de un «consultando» sin final a la vista. El avance llega
/// de cualquier consulta completa en marcha, también de la que empezó el
/// arranque y que ésta reutiliza.
function reportMsStoreScanProgress({ done = 0, total = 0 } = {}) {
  if (!msStoreProgressVisible || !total) return;
  setStatus(`Consultando la Microsoft Store… ${done} de ${total}`, "var(--accent)");
  setProgress(85 + Math.round((14 * done) / total));
}

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
    // La Microsoft Store se pregunta después y por su cuenta: es otro servicio,
    // tarda lo suyo y un fallo suyo no puede invalidar lo que ya dijo WinGet.
    setStatus("Consultando la Microsoft Store...", "var(--accent)");
    setProgress(85);
    msStoreProgressVisible = true;
    await ensureMsStoreInstalled({ force: true });
    await scanMsStoreUpdates({ quiet: true });
    const updates = updatesCount();
    clientLog("info", "updates-scan", { updates });
    if (updates) {
      setStatus(
        `${updates} ${updates === 1 ? "actualización encontrada" : "actualizaciones encontradas"}`,
        "var(--accent)",
      );
    } else {
      setTransientStatus("No hay actualizaciones pendientes.", "var(--green)", 5000);
    }
  } catch (error) {
    setTransientStatus(`No se pudo comprobar las actualizaciones: ${error}`, "var(--red)", 8000);
    clientLog("warn", "updates-scan", String(error?.stack || error));
  } finally {
    msStoreProgressVisible = false;
    state.scanningUpdates = false;
    suppressShellBusy = false;
    hideShellBusy();
    setProgress(100);
    renderSidebar();
    renderContent();
  }
}

// Todas las cifras salen de las mismas fichas que se dibujan, nunca del mapa de
// estados en bruto ni del catálogo entero. Ese mapa trae también los
// componentes de una suite, que sólo se enseñan con ella instalada, y, por
// duplicado, los paquetes de WinGet que el catálogo ya tiene con otro
// identificador: la barra de estado decía 279 aplicaciones donde «Inicio»
// enseñaba 275, y el globo de «Actualizaciones» podía contar una que la sección
// no lista.

/// Las fichas del catálogo que la tienda enseña.
function visibleCatalogCount() {
  return state.catalog.filter(componentVisible).length;
}

function visibleKnownApps(predicate) {
  return knownApps().filter((app) => componentVisible(app) && predicate(appStatus(app.id))).length;
}

function installedCount() {
  return visibleKnownApps((status) => status.installed);
}

/// Las actualizaciones pendientes son las del catálogo y las de la Microsoft
/// Store: el usuario ve una sola sección y una sola cifra, porque para él es
/// una sola cosa.
function updatesCount() {
  return visibleKnownApps((status) => status.update_available) + msStoreUpdatesList().length;
}

function sectionFilter(app) {
  const nav = NAV.find((n) => n.id === state.section);
  if (!nav || nav.filter == null) return true;
  // La Microsoft Store no enseña el catálogo propio, así que ninguna de sus
  // aplicaciones le pertenece.
  if (nav.filter === "__MSSTORE__") return false;
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
  return !query || (derived.searchBlobs.get(app.id) || [app.name, app.description, app.id].join(" ").toLocaleLowerCase("es-ES")).includes(query);
}

// Los programas que vienen dentro de otra aplicación solo tienen sentido cuando
// esa aplicación está puesta: hasta entonces no hay nada que ofrecer, porque no
// se instalan por su cuenta.
function componentVisible(app) {
  return app.source_type !== "component" || appStatus(app.id).installed;
}

function filteredApps() {
  const query = state.search.trim().toLocaleLowerCase("es-ES");
  const source = ["installed", "updates"].includes(state.section) ? knownApps() : state.catalog;
  const apps = source.filter((app) =>
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

/// La cifra de «Actualizaciones», o un giro discreto mientras se buscan y aún
/// no hay ninguna que contar.
function updatesNavBadgeHtml() {
  const count = updatesCount();
  if (count > 0) return `<span class="nav-badge-count">${count}</span>`;
  if (state.scanningUpdates) {
    return `<span class="nav-badge-scan" role="status" aria-label="Buscando actualizaciones"></span>`;
  }
  return "";
}

function renderSidebarNow() {
  const el = document.getElementById("sidebar");
  el.innerHTML = `
    <div class="sb-label">CATEGORÍAS</div>
    ${NAV.filter((n) => n.group !== "local").map(navButtonHtml).join("")}
    <div class="sb-label">LOCAL</div>
    ${NAV.filter((n) => n.group === "local").map(navButtonHtml).join("")}
    <button type="button" class="action-btn" data-action="folder">
      <span class="nav-ico">📁</span><span class="nav-label">Directorio local</span>
    </button>
  `;
}

function navButtonHtml(n) {
  return `
    <button type="button" class="nav-btn ${state.section === n.id ? "active" : ""}" data-nav="${n.id}">
      <span class="nav-ico">${n.icon}</span><span class="nav-label">${escapeHtml(n.label)}</span>
      ${n.id === "updates" ? updatesNavBadgeHtml() : ""}
    </button>`;
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

  // Sin respuesta del equipo todavía: un hueco con la forma del botón que
  // vendrá, como hacen las tiendas mientras cargan, y no un «Instalar» que
  // podría ser mentira.
  if (isStatusPending(app.id)) {
    return `<span class="btn-skeleton${variant === "hero" ? " hero" : ""}" role="status" aria-label="Comprobando si está instalada"></span>`;
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
  if (st.pending_restart) {
    const latest = versionLabel(st.latest_version, "la versión nueva");
    const cls = modal ? "update-version-badge modal-version" : "update-version-badge";
    return `<span class="${cls}" title="La versión nueva ya está instalada; Windows la aplicará al cerrar la aplicación">Instalada ${escapeHtml(latest)} · ciérrala para aplicarla</span>`;
  }
  if (!st.update_available) return "";
  const current = versionLabel(st.version, "versión desconocida");
  const latest = versionLabel(st.latest_version, "la más reciente");
  const cls = modal ? "update-version-badge modal-version" : "update-version-badge";
  return `<span class="${cls}" title="Versión instalada y versión disponible">Actualización · ${escapeHtml(current)} → ${escapeHtml(latest)}</span>`;
}

function cardMetaHtml(app) {
  const st = appStatus(app.id);
  const version = st.installed ? st.version : app.version;
  // Shown for anything installed, whoever installed it: the plain "en el
  // sistema" text only appeared for programs Windows had registered, so the
  // ones the store put there itself showed nothing at all.
  // Three states, not two. A package Windows has installed but not yet applied
  // is neither "up to date" nor "update available": offering the update again
  // sends the user to re-download something already on the disk, which is what
  // the store did to Claude twice in a row.
  const origin =
    st.pending_restart
      ? `<span class="origin-tag update-origin">reinicia para aplicar</span>`
      : st.update_available
        ? `<span class="origin-tag update-origin">actualización disponible</span>`
        : st.installed
          ? `<span class="installed-tag"><span class="badge-dot green"></span>Ya instalado</span>`
          : "";
  return `
    <strong>${escapeHtml(app.name)}${origin}</strong>
    <small>${escapeHtml(app.author || "—")}  ·  ${escapeHtml(versionLabel(version))}</small>
    ${updateVersionBadge(st)}`;
}

function cardHtml(app, index, prioritizeIcon = false) {
  const accent = pickAccent(app, index);
  const avatarBg = avatarBackground(app, accent);
  const letter = (app.name || app.id || "A")[0].toUpperCase();
  const selectable = isQueueable(app);
  const selected = selectable && state.selection.has(app.id);
  return `
    <article class="app-card${selected ? " is-selected" : ""}" data-app-id="${escapeHtml(app.id)}" tabindex="0"
      ${selectable ? "data-selectable" : ""}
      aria-label="Ver detalles de ${escapeHtml(app.name)}" style="--card-accent:${accent}">
      ${selectable ? cardSelectHtml(app, selected) : ""}
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

/// Tarjetas vacías con la silueta de las de verdad, para lo que todavía se está
/// buscando. Sin texto que leer: el título de la sección dice qué pasa.
function skeletonSectionHtml(title, count) {
  const card = `
    <div class="app-card skeleton-card" aria-hidden="true">
      <div class="card-top">
        <div class="card-avatar skeleton-block"></div>
        <div class="skeleton-lines"><i></i><i></i></div>
      </div>
      <div class="skeleton-lines skeleton-desc"><i></i><i></i></div>
      <div class="card-actions"><span class="btn-skeleton"></span></div>
    </div>`;
  return `
    <section class="section" aria-busy="true">
      <div class="section-head">
        <h3>${escapeHtml(title)}</h3>
      </div>
      <div class="grid">${card.repeat(count)}</div>
    </section>`;
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

// ---------------------------------------------------------------------------
// Selección para instalar en cola
// ---------------------------------------------------------------------------
//
// Se van marcando fichas del catálogo —en cualquier sección, con el círculo de
// su esquina, con Ctrl+clic o desde su ficha— y se instalan todas de una vez
// con la misma tanda y el mismo diálogo que «Actualizar todo». Abajo aparece
// una barra con lo elegido; Escape la vacía.

/**
 * Si la ficha puede entrar en la cola.
 *
 * Solo lo que la tienda instala por sí misma y nada más que eso: del catálogo
 * (un resultado de WinGet o de Chocolatey no tiene tarjeta en él), sin instalar
 * y sin nada en marcha. «Obtener» lleva a una web oficial y un componente llega
 * dentro de otra aplicación, así que ninguno de los dos se puede pedir.
 */
function isQueueable(app) {
  if (!app || derived.catalogById.get(app.id) !== app) return false;
  if (app.source_type === "web" || app.source_type === "component") return false;
  if (state.busy[app.id] || derived.taskByAppId.has(app.id)) return false;
  // Hasta que el equipo conteste no se sabe si hay algo que instalar.
  if (isStatusPending(app.id)) return false;
  // Una fila de la tanda que aún espera su turno no está ocupada todavía, pero
  // ya está pedida.
  if (isBulkUpdateRunning() && state.bulkUpdate.byKey.has(app.id)) return false;
  return !appStatus(app.id).installed;
}

/// Saca lo que ha dejado de poder pedirse: lo que se instaló por otro camino,
/// lo que ya está en marcha o lo que el catálogo recargado ya no trae.
function pruneSelection() {
  for (const id of state.selection) {
    if (!isQueueable(findApp(id))) state.selection.delete(id);
  }
}

function cardSelectHtml(app, selected) {
  return `
    <button type="button" class="card-select" data-select="${escapeHtml(app.id)}"
      ${cardSelectAttributes(app, selected)}>
      <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor"
        stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M5 12.5l4.5 4.5L19 7.5" />
      </svg>
    </button>`;
}

function cardSelectAttributes(app, selected) {
  const name = escapeHtml(app.name);
  return selected
    ? `aria-pressed="true" aria-label="Quitar ${name} de la cola de instalación" title="Quitar de la cola"`
    : `aria-pressed="false" aria-label="Añadir ${name} a la cola de instalación" title="Añadir a la cola de instalación"`;
}

/**
 * Pone una tarjeta ya dibujada al día con la selección.
 *
 * El botón se retoca en su sitio en vez de rehacerse: quien lo acaba de pulsar
 * con el teclado tiene el foco en él y no tiene por qué perderlo.
 */
function syncCardSelection(card, app) {
  const selectable = isQueueable(app);
  const selected = selectable && state.selection.has(app.id);
  card.toggleAttribute("data-selectable", selectable);
  card.classList.toggle("is-selected", selected);
  const toggle = card.querySelector(":scope > .card-select");
  if (!selectable) {
    toggle?.remove();
    return;
  }
  if (!toggle) {
    card.insertAdjacentHTML("afterbegin", cardSelectHtml(app, selected));
    return;
  }
  if (toggle.getAttribute("aria-pressed") === String(selected)) return;
  const name = app.name || app.id;
  toggle.setAttribute("aria-pressed", String(selected));
  toggle.setAttribute(
    "aria-label",
    selected ? `Quitar ${name} de la cola de instalación` : `Añadir ${name} a la cola de instalación`,
  );
  toggle.title = selected ? "Quitar de la cola" : "Añadir a la cola de instalación";
}

function refreshSelectionUi() {
  pruneSelection();
  document.querySelectorAll("#content .app-card[data-app-id]").forEach((card) => {
    const app = findApp(card.dataset.appId);
    if (app) syncCardSelection(card, app);
  });
  renderQueueDock();
}

function toggleSelection(id) {
  const app = findApp(id);
  if (!isQueueable(app)) return;
  if (state.selection.has(id)) state.selection.delete(id);
  else state.selection.add(id);
  refreshSelectionUi();
}

function clearSelection() {
  if (!state.selection.size) return;
  state.selection.clear();
  refreshSelectionUi();
}

/// La barra vive dentro de `.main`, por encima de la lista y de la barra de
/// estado: se crea la primera vez que hace falta y se queda.
function ensureQueueDock() {
  let dock = document.getElementById("queue-dock");
  if (dock) return dock;
  const main = document.querySelector(".main");
  if (!main) return null;
  dock = document.createElement("div");
  dock.id = "queue-dock";
  dock.className = "queue-dock";
  dock.setAttribute("role", "region");
  dock.setAttribute("aria-label", "Cola de instalación");
  dock.inert = true;
  dock.addEventListener("click", (event) => {
    const button = event.target.closest("[data-queue]");
    if (button?.dataset.queue === "clear") clearSelection();
    else if (button?.dataset.queue === "install") void installSelection();
  });
  main.appendChild(dock);
  return dock;
}

/**
 * La barra de la cola: lo elegido, cuántas son y qué hacer con ellas.
 *
 * Al vaciarse se queda con su último contenido mientras se va, para que la
 * salida no sea un recuadro en blanco deslizándose hacia abajo.
 */
function renderQueueDock() {
  const dock = ensureQueueDock();
  if (!dock) return;
  pruneSelection();
  const apps = [...state.selection].map(findApp).filter(Boolean);
  const count = apps.length;
  document.getElementById("content")?.classList.toggle("selection-active", count > 0);
  dock.classList.toggle("is-visible", count > 0);
  dock.inert = !count;
  if (!count) {
    // Olvidar la firma permite reconstruir la misma selección al volver a elegirla.
    delete dock.dataset.signature;
    delete dock.dataset.appIds;
    return;
  }

  const running = isBulkUpdateRunning();
  const signature = `${apps.map((app) => app.id).join(",")}|${running}`;
  if (dock.dataset.signature === signature) return;
  dock.dataset.signature = signature;

  // Las cuatro últimas elegidas, la más reciente a la derecha y por encima: la
  // que se acaba de marcar se ve llegar. Lo que no cabe se cuenta delante.
  const previousIds = new Set((dock.dataset.appIds || "").split(",").filter(Boolean));
  dock.dataset.appIds = apps.map((app) => app.id).join(",");
  const shown = apps.slice(-4);
  dock.innerHTML = `
    <div class="queue-dock-stack" aria-hidden="true">
      ${count > shown.length ? `<span class="queue-dock-more">+${count - shown.length}</span>` : ""}
      ${shown
        .map((app, index) => {
          const accent = pickAccent(app, index);
          // A la vista desde el primer momento y como mucho cuatro: sin espera
          // perezosa, que dejaría la letra en lugar del icono al aparecer.
          return `<span class="card-avatar queue-dock-avatar${previousIds.has(app.id) ? "" : " is-new"}" style="background:${avatarBackground(app, accent)}">${renderAvatar(app, (app.name || "?")[0].toUpperCase(), true)}</span>`;
        })
        .join("")}
    </div>
    <div class="queue-dock-copy">
      <strong>${count} ${count === 1 ? "aplicación seleccionada" : "aplicaciones seleccionadas"}</strong>
      <small>${
        running
          ? "Hay una tanda en marcha: podrás instalar éstas cuando termine."
          : "Se instalarán en cola, en el orden en que las elegiste."
      }</small>
    </div>
    <div class="queue-dock-actions">
      <button type="button" class="btn ghost" data-queue="clear" title="Quitar todas de la selección (Esc)">Vaciar</button>
      <button type="button" class="btn primary queue-dock-install" data-queue="install"
        ${running ? 'aria-busy="true" title="Ver la tanda en marcha"' : ""}>
        <svg class="btn-svg-icon" width="14" height="14" viewBox="0 0 24 24" fill="none"
          stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M12 3v11" />
          <path d="m7.5 10.5 4.5 4.5 4.5-4.5" />
          <path d="M4 20h16" />
        </svg>
        <span>${running ? "Tanda en marcha…" : `Instalar (${count})`}</span>
      </button>
    </div>`;
}

/// La lista que se repasa antes de confirmar, numerada en el orden de la cola.
function queueReviewHtml(items) {
  return `
    <ol class="queue-review" aria-label="Orden de instalación">
      ${items
        .map((item, index) => {
          const accent = pickAccent(item.app, index);
          const meta = [item.app.author, item.app.category].filter(Boolean).join(" · ");
          return `
        <li class="queue-review-item">
          <span class="queue-review-index">${index + 1}</span>
          <span class="card-avatar queue-review-avatar" style="background:${avatarBackground(item.app, accent)}">${renderAvatar(item.app, (item.name || "?")[0].toUpperCase(), index < 8)}</span>
          <span class="queue-review-copy">
            <strong>${escapeHtml(item.name)}</strong>
            ${meta ? `<small>${escapeHtml(meta)}</small>` : ""}
          </span>
        </li>`;
        })
        .join("")}
    </ol>`;
}

/**
 * «Instalar» en la barra de la cola: repasa lo elegido y lo manda a la tanda.
 *
 * Lo que una instalación suelta preguntaría se resuelve aquí de una vez. Una
 * aplicación publicada en varias ediciones lleva la que se eligió antes o, si
 * nunca se eligió, la recomendada; el diálogo dice cuál, porque con Thorium
 * equivocarse hacia arriba es un navegador que no arranca. Y si alguna dejó
 * datos de una instalación anterior en la carpeta de la tienda, se pregunta
 * qué hacer con ellos, igual que al actualizar todo.
 */
async function installSelection() {
  if (isBulkUpdateRunning()) {
    showBulkUpdateModal();
    return;
  }
  await untilStatusesVerified();
  pruneSelection();
  let apps = [...state.selection].map(findApp).filter(Boolean);
  if (!apps.length) {
    renderQueueDock();
    return;
  }
  // Las aplicaciones web de la tanda necesitan el navegador de la tienda antes
  // de empezar. Sin él se quedan fuera y el resto sigue adelante.
  if (apps.some((app) => app.source_type === "webapp") && !(await ensureWebBrowser())) {
    apps = apps.filter((app) => app.source_type !== "webapp");
    if (!apps.length) return;
  }
  const items = apps.map((app) => ({
    key: app.id,
    kind: "catalog",
    appId: app.id,
    app,
    name: app.name,
    from: "",
    to: "",
  }));

  try {
    await Promise.all(items.map(async (item) => {
      item.portableInfo = await invoke("portable_data_info", { appId: item.appId, forUpdate: true });
    }));
  } catch (error) {
    showAlertModal("No se pudieron comprobar los datos", String(error));
    return;
  }
  const portables = items.filter((item) => item.portableInfo);
  const editions = items
    .filter((item) => item.app.variants?.options?.length)
    .map((item) => {
      const wanted = state.settings?.variants?.[item.appId] || item.app.variants.default;
      const option =
        item.app.variants.options.find((candidate) => candidate.id === wanted) ||
        item.app.variants.options[0];
      return `${item.name}: ${option?.name || wanted}`;
    });
  const count = items.length;

  showConfirmModal({
    title: `Instalar ${count} ${count === 1 ? "aplicación" : "aplicaciones"}`,
    subtitle: "Cola de instalación",
    message:
      "Se instalarán en este orden. Las descargas corren en paralelo y las instalaciones " +
      "se aplican una detrás de otra; si algún instalador abre su propio asistente, " +
      "síguelo cuando aparezca." +
      (editions.length
        ? `\n\nEdición que se instalará — ${editions.join(" · ")}. Si necesitas otra, ` +
          "instálala por separado desde su ficha."
        : "") +
      (portables.length
        ? `\n\nPortables: ${portables.map((item) => item.name).join(", ")}.` +
          portableDataMessage(portables[0].portableInfo)
        : ""),
    details: queueReviewHtml(items),
    choices: portables.length ? portableDataChoices(portables[0].portableInfo) : null,
    confirmText: `Instalar en cola (${count})`,
    confirmVariant: "primary",
    onConfirm: (picked) => {
      for (const item of portables) item.portableData = picked;
      // La selección se convierte en la tanda: lo que queda en pantalla ya no
      // es una lista por decidir sino una instalación en marcha.
      state.selection.clear();
      const started = startBulkUpdate(items, "install");
      refreshSelectionUi();
      return started;
    },
  });
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

// ---------------------------------------------------------------------------
// Microsoft Store
// ---------------------------------------------------------------------------
//
// Una sección que no lee el catálogo. Lo que enseña llega de los servidores de
// Microsoft en el momento de buscar, así que tiene su propio buscador, sus
// propios filtros y su propia lista; lo demás —las tarjetas, la ficha, el
// diálogo de instalación, la barra de progreso— es exactamente lo mismo que usa
// el resto de la tienda, porque no hay razón para que se parezca a otra cosa.

const MSSTORE_KIND_LABELS = {
  uwp: "Paquete de Windows",
  win32: "Instalador clásico",
};

/**
 * El nombre que el backend le da a la tarea de este producto, en
 * `msstore::task_id`. Se compone aquí también porque el diálogo de progreso
 * tiene que existir antes de que la orden llegue a contestar con él.
 */
function msStoreTaskId(productId) {
  return `msstore:${String(productId || "").toUpperCase()}`;
}

/** Las imágenes de Microsoft viajan a veces sin esquema. */
function msStoreImageUrl(raw) {
  const url = String(raw || "").trim();
  if (!url) return "";
  return url.startsWith("//") ? `https:${url}` : url;
}

/** Un producto vestido de aplicación, para lo que ya sabe dibujar aplicaciones. */
function msStoreAppShape(product) {
  if (!product) return null;
  return {
    id: msStoreTaskId(product.product_id),
    name: product.title || product.product_id,
    author: product.publisher || "Microsoft Store",
    icon_url: msStoreImageUrl(product.icon_url) || null,
    icon_padding: 4,
  };
}

/**
 * El producto, esté en la lista que esté.
 *
 * La sección Microsoft Store y el bloque que cuelga de la barra de arriba
 * guardan sus resultados por separado, y las tarjetas que dibujan son las
 * mismas. Mirar sólo en la primera dejaba el botón «Instalar» de la segunda sin
 * hacer nada: no había producto que instalar, y no había forma de saberlo.
 */
function msStoreProductById(productId) {
  const id = String(productId || "").toUpperCase();
  const lists = [state.msstore.results, state.msstore.inline.results];
  for (const list of lists) {
    const found = (list || []).find((product) => product.product_id === id);
    if (found) return found;
  }
  return null;
}

function msStoreInstalledByFamily(family) {
  const wanted = String(family || "").toLowerCase();
  return (
    state.msstore.installed.find((app) => app.family.toLowerCase() === wanted) || null
  );
}

/** La lista local es la misma que usa «Aplicaciones instaladas». La marca
 * del resultado puede ser anterior a una instalación o venir sin completar. */
function msStoreInstalledRecord(product) {
  if (!product) return null;
  const families = [product.installed_family, ...(product.package_families || [])].filter(Boolean);
  for (const family of families) {
    const installed = msStoreInstalledByFamily(family);
    if (installed) return installed;
  }
  // Una comprobación de versiones puede conocer el producto aun cuando la
  // búsqueda no publique las familias. Vincular por identidad, no por nombre.
  const id = String(product.product_id || "").toUpperCase();
  if (id) {
    const installed = state.msstore.installed.find((app) =>
      String(msStoreUpdateOf(app.family)?.product_id || "").toUpperCase() === id);
    if (installed) return installed;
  }
  // Una lista local ya cargada también confirma las desinstalaciones y evita
  // conservar el estado antiguo de resultados que siguen abiertos.
  if (state.msstore.installedLoaded || !product.installed || !product.installed_family) return null;
  return {
    family: product.installed_family,
    full_name: "",
    name: product.title || product.installed_family,
    display_name: product.title || product.installed_family,
    version: product.installed_version || "",
    install_location: "",
    can_uninstall: !!product.can_uninstall,
    launch_target: null,
  };
}

/** Lo que se sabe de la versión publicada de una familia, si ya se preguntó. */
function msStoreUpdateOf(family) {
  return state.msstore.updates[String(family || "").toLowerCase()] || null;
}

function msStoreUpdateVersion(report) {
  return String(report?.latest_version || "").trim();
}

/** Oculta la misma versión rechazada y libera automáticamente una nueva. */
function applyMsStoreUpdateSuppression(report) {
  const store = state.msstore;
  const family = String(report?.family || "").toLowerCase();
  const skipped = store.suppressedUpdates[family];
  if (!family || !skipped) return report;

  const published = msStoreUpdateVersion(report);
  const sameVersion = published && published === String(skipped.latestVersion || "");
  if (report?.update_available && sameVersion) {
    return { ...report, update_available: false, suppressed: true };
  }

  // Un informe concluyente sin actualización, o una versión publicada nueva,
  // deja obsoleta la exclusión. Los errores de red no deben borrarla.
  if (!report?.error && (!report?.update_available || (published && !sameVersion))) {
    delete store.suppressedUpdates[family];
    saveMsStoreSuppressedUpdates(store.suppressedUpdates);
  }
  return report;
}

function suppressMsStoreUpdate(family, report) {
  const store = state.msstore;
  const key = String(family || "").toLowerCase();
  const latestVersion = msStoreUpdateVersion(report);
  if (!key || !latestVersion) return false;

  store.suppressedUpdates[key] = {
    latestVersion,
    productId: String(report?.product_id || "").toUpperCase(),
  };
  saveMsStoreSuppressedUpdates(store.suppressedUpdates);
  if (store.updates[key]) {
    store.updates[key] = { ...store.updates[key], update_available: false, suppressed: true };
  }
  void clientLog(
    "info",
    "msstore-update-suppressed",
    `${family}: se oculta la versión ${latestVersion} después de que Windows la rechazara`,
  );
  return true;
}

/** Las aplicaciones instaladas para las que el canal publica algo más nuevo. */
function msStoreUpdatesList() {
  return state.msstore.installed.filter(
    (app) => msStoreUpdateOf(app.family)?.update_available
  );
}

/**
 * Una aplicación instalada, vestida de aplicación del catálogo para que la
 * dibujen las mismas funciones. El nombre y el icono buenos llegan con la
 * comprobación de versiones; hasta entonces valen los que da Windows.
 */
function msStoreInstalledShape(app) {
  const known = msStoreUpdateOf(app.family);
  return {
    id: `msstore-app:${app.family}`,
    name: known?.title || app.display_name || app.name,
    author: "Microsoft Store",
    icon_url: msStoreImageUrl(known?.icon_url) || null,
    icon_padding: 4,
  };
}

async function ensureMsStoreInstalled({ force = false } = {}) {
  const store = state.msstore;
  if (store.installedLoaded && !force) return store.installed;
  try {
    store.installed = (await invoke("msstore_installed")) || [];
    store.installedLoaded = true;
  } catch (error) {
    void clientLog("warn", "msstore-installed", String(error?.stack || error));
  }
  return store.installed;
}

/** La comprobación de la tienda que está en marcha, para poder esperarla. */
let msStoreScanInFlight = null;
/** La completa —todas las instaladas— y con qué canal se pidió. */
let msStoreFullScanInFlight = null;

/**
 * Pregunta al canal elegido qué versión publica de cada aplicación.
 *
 * Cuesta dos viajes por aplicación, así que sin `families` —que es como lo
 * llama el escaneo general— puede tardar unos segundos. Lo que devuelve se
 * acumula: una comprobación de dos aplicaciones no borra lo que se sabía de
 * las demás.
 */
async function scanMsStoreUpdates(options = {}) {
  const store = state.msstore;
  // Una comprobación completa ya en marcha con el mismo canal y arquitectura
  // contesta lo mismo que otra lanzada ahora, así que se espera a ésa. Encadenar
  // una segunda hacía que «Buscar actualizaciones» pulsado durante la del
  // arranque esperase a que terminara y después repitiese las diecisiete
  // consultas.
  const full = !options.families;
  const running = msStoreFullScanInFlight;
  if (full && running && (running.ring ?? store.ring) === store.ring && running.arch === store.arch) {
    if (!options.quiet) renderContent();
    await running.scan.catch(() => {});
    return;
  }
  // Lo demás se encadena en vez de descartarse. Pulsar «Buscar actualizaciones»
  // mientras se consultaban sólo unas pocas dejaba el resto sin hacer, y el
  // botón terminaba contestando «no hay nada» por lo que aún no había mirado
  // nadie.
  const previous = msStoreScanInFlight;
  const scan = (async () => {
    if (previous) await previous;
    return runMsStoreUpdatesScan(options);
  })();
  msStoreScanInFlight = scan;
  if (full) msStoreFullScanInFlight = { scan, ring: store.ring, arch: store.arch };
  // La marca de ocupado la lleva el encadenado y no cada consulta: si la
  // pusiera cada una, entre la que acaba y la que espera se apagaría, y el
  // botón parpadearía disponible sin estarlo.
  store.scanning = true;
  if (!options.quiet) renderContent();
  try {
    await scan;
  } finally {
    if (msStoreFullScanInFlight?.scan === scan) msStoreFullScanInFlight = null;
    if (msStoreScanInFlight === scan) {
      msStoreScanInFlight = null;
      store.scanning = false;
    }
    renderSidebar();
    renderContent();
  }
}

/// Una consulta al canal, sin más. Del ocupado y del redibujado se encarga
/// quien la encadena.
async function runMsStoreUpdatesScan({ families = null } = {}) {
  const store = state.msstore;
  await ensureMsStoreOptions();
  if (!families) {
    await ensureMsStoreInstalled();
    families = store.installed.map((app) => app.family);
  }
  if (!families.length) return;

  try {
    const reports =
      (await invoke("msstore_check_updates", {
        ring: store.ring,
        arch: store.arch,
        families,
      })) || [];
    for (const report of reports) {
      store.updates[report.family.toLowerCase()] = applyMsStoreUpdateSuppression(report);
      if (report.error) {
        void clientLog("debug", "msstore-updates", `${report.family}: ${report.error}`);
      }
    }
    store.lastScan = new Date();
  } catch (error) {
    void clientLog("warn", "msstore-updates", String(error?.stack || error));
  }
}

async function ensureMsStoreOptions() {
  const store = state.msstore;
  if (store.options) return store.options;
  try {
    store.options = await invoke("msstore_options");
    if (!store.ring) store.ring = store.options.default_ring;
  } catch (error) {
    // Sin los canales la sección sigue siendo utilizable: el backend aplica el
    // suyo por defecto y la fila de filtros simplemente no se dibuja.
    void clientLog("warn", "msstore-options", String(error?.stack || error));
  }
  return store.options;
}

function msStoreArchLabel(archId) {
  const label = MSSTORE_ARCHS.find((arch) => arch.id === archId)?.label || archId;
  const host = state.msstore.options?.host_arch;
  return archId === "auto" && host ? `${label} · ${host}` : label;
}

/** Lo mismo, dicho dentro de una frase en lugar de dentro de un botón. */
function msStoreArchPhrase() {
  const { arch, options } = state.msstore;
  if (arch === "all") return "todas las arquitecturas";
  if (arch !== "auto") return arch;
  const host = options?.host_arch;
  return host ? `la arquitectura de este equipo (${host})` : "la arquitectura de este equipo";
}

function msStoreRingLabel() {
  const rings = state.msstore.options?.rings || [];
  return rings.find((ring) => ring.id === state.msstore.ring)?.label || "Retail (Base)";
}

function msStoreBytes(bytes) {
  const value = Number(bytes);
  if (!Number.isFinite(value) || value <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit += 1;
  }
  return `${size >= 10 || unit === 0 ? Math.round(size) : size.toFixed(1)} ${units[unit]}`;
}

function msStoreDate(raw) {
  if (!raw) return "";
  const date = new Date(raw);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString("es-ES", { year: "numeric", month: "short", day: "numeric" });
}

function msStoreSearchHtml() {
  const store = state.msstore;
  return `
    <form class="ms-toolbar" id="ms-search-form" role="search">
      <div class="ms-search-field">
        <span class="ms-search-icon" aria-hidden="true">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="7" /><line x1="16.5" y1="16.5" x2="21" y2="21" />
          </svg>
        </span>
        <input id="ms-search" class="ms-search-input" type="text" autocomplete="off"
          spellcheck="false" aria-label="Buscar en la Microsoft Store"
          placeholder="Nombre de la aplicación, o el enlace de su ficha en apps.microsoft.com"
          value="${escapeHtml(store.query)}" />
      </div>
      <button type="submit" class="btn primary" id="ms-search-go"${store.loading ? " disabled aria-busy=\"true\"" : ""}>
        ${store.loading ? "Buscando…" : "Buscar"}
      </button>
    </form>`;
}

function msStoreFiltersHtml() {
  const store = state.msstore;
  const rings = store.options?.rings || [];
  if (!rings.length) return "";
  return `
    <div class="ms-filters">
      <div class="ms-filter-group" role="group" aria-label="Canal de publicación">
        <span class="ms-filter-label">Canal</span>
        ${rings
          .map(
            (ring) => `
          <button type="button" class="console-filter ${store.ring === ring.id ? "active" : ""}"
            data-ms-ring="${escapeHtml(ring.id)}">${escapeHtml(ring.label)}</button>`
          )
          .join("")}
      </div>
      <div class="ms-filter-group" role="group" aria-label="Arquitectura del paquete">
        <span class="ms-filter-label">Arquitectura</span>
        ${MSSTORE_ARCHS.map(
          (arch) => `
          <button type="button" class="console-filter ${store.arch === arch.id ? "active" : ""}"
            data-ms-arch="${escapeHtml(arch.id)}">${escapeHtml(msStoreArchLabel(arch.id))}</button>`
        ).join("")}
      </div>
    </div>`;
}

function msStoreHeroHtml() {
  return `
    <section class="hero ms-hero" aria-label="Qué es esta sección">
      <div class="hero-left">
        <div class="ms-hero-logo" aria-hidden="true">
          <img src="assets/microsoft_store.png" alt="" />
        </div>
        <div class="project-hero-copy">
          <div class="project-kicker">CATÁLOGO OFICIAL DE MICROSOFT</div>
          <h2>Microsoft Store</h2>
          <p class="project-slogan">Lo que publica Microsoft, instalado desde aquí.</p>
          <p class="desc">
            Busca una aplicación por su nombre o pega el enlace de su ficha. WinSlimCenter
            resuelve el paquete que le corresponde a este equipo, lo descarga del servicio de
            entrega de Windows y lo instala con sus dependencias. Sólo se ofrecen productos
            gratuitos.
          </p>
        </div>
      </div>
      <div class="hero-tag">Canal · ${escapeHtml(msStoreRingLabel())}</div>
    </section>`;
}

/// La insignia de versión de una aplicación instalada, con la misma forma que
/// la del catálogo: la que hay puesta y la que publica el canal.
function msStoreVersionBadge(app, { modal = false } = {}) {
  const report = msStoreUpdateOf(app.family);
  if (!report?.update_available) return "";
  const current = versionLabel(app.version, "versión desconocida");
  const latest = versionLabel(report.latest_version, "la más reciente");
  const cls = modal ? "update-version-badge modal-version" : "update-version-badge";
  return `<span class="${cls}" title="Versión instalada y versión publicada en el canal ${escapeHtml(
    msStoreRingLabel()
  )}">Actualización · ${escapeHtml(current)} → ${escapeHtml(latest)}</span>`;
}

function msStoreOriginTag(app) {
  return msStoreUpdateOf(app.family)?.update_available
    ? `<span class="origin-tag update-origin">actualización disponible</span>`
    : `<span class="installed-tag"><span class="badge-dot green"></span>Ya instalado</span>`;
}

/**
 * Los botones de una aplicación de la tienda que ya está instalada.
 *
 * Sigue la misma regla que el catálogo: o se actualiza o se abre —nunca las dos
 * cosas—, y desinstalar siempre está. Los paquetes que Windows se reserva no
 * ofrecen un botón que no funcionaría.
 */
function msStoreInstalledActions(app, { productId = null, variant = "card" } = {}) {
  const busy = state.msstore.busy[app.family];
  const installing = productId && state.msstore.installing[productId];
  if (busy || installing) {
    const label = { uninstalling: "Desinstalando…", launching: "Abriendo…" }[busy] || "Actualizando…";
    const cls = variant === "modal" ? "btn secondary" : "btn secondary";
    return `<button type="button" class="${cls}" disabled aria-busy="true">${label}</button>`;
  }

  const report = msStoreUpdateOf(app.family);
  const family = escapeHtml(app.family);
  const parts = [];
  if (report?.update_available && report.product_id) {
    parts.push(
      `<button type="button" class="btn secondary" data-ms-update="${escapeHtml(report.product_id)}"
        data-ms-family="${family}">Actualizar</button>`
    );
  } else if (app.launch_target) {
    parts.push(`<button type="button" class="btn primary" data-ms-launch="${family}">Abrir</button>`);
  }
  if (app.can_uninstall) {
    parts.push(`<button type="button" class="btn danger" data-ms-uninstall="${family}">Desinstalar</button>`);
  } else {
    parts.push(
      `<button type="button" class="btn installed-label" disabled
        title="Windows no permite quitar este paquete: forma parte del sistema.">La protege Windows</button>`
    );
  }
  return parts.join("");
}

function msStoreCardHtml(product, index) {
  const shape = msStoreAppShape(product);
  const accent = pickAccent(shape, index);
  const avatarBg = avatarBackground(shape, accent);
  const letter = (product.title || product.product_id || "M")[0].toUpperCase();
  const kind = MSSTORE_KIND_LABELS[product.kind] || "";
  const installing = state.msstore.installing[product.product_id];
  // Un resultado que el equipo ya tiene puesto deja de ser una oferta y pasa a
  // ser la aplicación instalada, con lo que se puede hacer con ella.
  const installed = msStoreInstalledRecord(product);

  const tags = installed
    ? msStoreOriginTag(installed)
    : kind
      ? `<span class="origin-tag ms-kind-tag">${escapeHtml(kind)}</span>`
      : "";
  const meta = installed
    ? `${escapeHtml(product.publisher || "Microsoft Store")}  ·  ${escapeHtml(versionLabel(installed.version, "versión desconocida"))}`
    : `${escapeHtml(product.publisher || "Microsoft Store")}  ·  Gratis`;

  let actions;
  if (installing) {
    actions = `<button type="button" class="btn secondary" disabled aria-busy="true">${
      installing === "update" ? "Actualizando…" : "Instalando…"
    }</button>`;
  } else if (installed) {
    actions = msStoreInstalledActions(installed, { productId: product.product_id });
  } else {
    actions = `<button type="button" class="btn primary" data-ms-install="${escapeHtml(product.product_id)}">Instalar</button>`;
  }

  return `
    <article class="app-card ms-card" data-ms-id="${escapeHtml(product.product_id)}" tabindex="0"
      aria-label="Ver detalles de ${escapeHtml(product.title || product.product_id)}"
      style="--card-accent:${accent}">
      <div class="card-top">
        <div class="card-avatar" style="background:${avatarBg}">${renderAvatar(shape, letter, index < 8)}</div>
        <div>
          <strong>${escapeHtml(product.title || product.product_id)}
            ${tags}
          </strong>
          <small>${meta}</small>
          ${installed ? msStoreVersionBadge(installed) : ""}
        </div>
      </div>
      <p class="card-desc">${escapeHtml(product.description || "")}</p>
      <div class="card-actions">${actions}</div>
    </article>`;
}

/// La tarjeta de una aplicación instalada de la que sólo se sabe lo que cuenta
/// Windows, sin haber pasado por la tienda.
function msStoreInstalledCardHtml(app, index) {
  const shape = msStoreInstalledShape(app);
  const accent = pickAccent(shape, index);
  const avatarBg = avatarBackground(shape, accent);
  const letter = (shape.name || "M")[0].toUpperCase();
  const report = msStoreUpdateOf(app.family);
  // Sólo es una ficha si se sabe de qué producto es; hasta entonces la tarjeta
  // no promete una pantalla que no puede abrir.
  const productAttribute = report?.product_id
    ? ` data-ms-id="${escapeHtml(report.product_id)}" tabindex="0" aria-label="Ver detalles de ${escapeHtml(shape.name)}"`
    : "";
  return `
    <article class="app-card ms-card ms-installed-card" data-ms-family="${escapeHtml(app.family)}"${productAttribute}
      style="--card-accent:${accent}">
      <div class="card-top">
        <div class="card-avatar" style="background:${avatarBg}">${renderAvatar(shape, letter, index < 8)}</div>
        <div>
          <strong>${escapeHtml(shape.name)}
            ${msStoreOriginTag(app)}
          </strong>
          <small>Microsoft Store  ·  ${escapeHtml(versionLabel(app.version, "versión desconocida"))}</small>
          ${msStoreVersionBadge(app)}
        </div>
      </div>
      <p class="card-desc">${escapeHtml(app.name)}</p>
      <div class="card-actions">${msStoreInstalledActions(app)}</div>
    </article>`;
}

/// El control que sale a preguntar por las versiones publicadas. Reutiliza el
/// icono y el vocabulario de ocupado del resto de la tienda.
function msStoreScanButtonHtml() {
  const scanning = state.msstore.scanning;
  return `
    <button type="button" class="btn updates-toolbar-scan" id="btn-ms-scan"
      ${scanning ? 'disabled aria-busy="true"' : ""}
      title="Consulta al canal ${escapeHtml(msStoreRingLabel())} qué versión publica de cada aplicación instalada">
      <svg class="btn-svg-icon scan-icon" width="14" height="14" viewBox="0 0 24 24" fill="none"
        stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
        <path d="M3 3v5h5" />
        <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
        <path d="M16 16h5v5" />
      </svg>
      ${scanning ? "Comprobando…" : "Comprobar versiones"}
    </button>`;
}

/// Lo que hay que escribir antes de ir a preguntar a Microsoft. Con una o dos
/// letras el catálogo propio ya está filtrando y la tienda devolvería medio
/// mundo, así que la consulta ni se lanza.
const MSSTORE_INLINE_MIN_CHARS = 3;

/// Cuántos resultados de la Microsoft Store caben en la subsección antes de
/// que deje de ser un apunte y pase a tapar el catálogo, que es lo que se
/// estaba buscando.
const MSSTORE_INLINE_LIMIT = 8;

/// Lo que se espera desde la última tecla. Más largo que el del catálogo, que
/// filtra en memoria: aquí cada intento es un viaje a los servidores de
/// Microsoft y escribir un nombre entero no puede costar seis.
const MSSTORE_INLINE_DELAY = 550;

let inlineMsStoreTimer = null;

/// Dónde tiene sentido colgar los resultados de la Microsoft Store.
///
/// En su propia sección sobra, que ya tiene buscador. En «Aplicaciones instaladas» y
/// en «Actualizaciones» también: ahí ya hay un bloque titulado Microsoft Store
/// que habla de lo que el equipo tiene puesto, y un segundo con el mismo
/// título ofreciendo cosas que instalar sólo confundiría.
function inlineMsStoreFits() {
  return !["installed", "updates", MSSTORE_SECTION].includes(state.section);
}

/// Decide si la barra de arriba tiene que ir a preguntar a la Microsoft Store.
///
/// Se llama a cada tecla. Vaciar la caja —o irse a la propia sección de la
/// tienda, que tiene su buscador— borra el bloque: dejar los resultados de una
/// búsqueda anterior debajo de otra lista los haría pasar por respuesta a algo
/// que nadie ha preguntado.
function scheduleInlineMsStoreSearch() {
  const inline = state.msstore.inline;
  const query = state.search.trim();
  clearTimeout(inlineMsStoreTimer);
  if (!inlineMsStoreFits() || query.length < MSSTORE_INLINE_MIN_CHARS) {
    if (inline.query || inline.requested || inline.loading || inline.error) {
      Object.assign(inline, {
        query: "",
        requested: "",
        results: [],
        loading: false,
        error: "",
      });
      renderContent();
    }
    return;
  }
  // Ya contestada: volver sobre lo mismo tras borrar una letra y reescribirla
  // no vale otro viaje.
  if (inline.requested === query) return;
  inlineMsStoreTimer = setTimeout(() => void runInlineMsStoreSearch(query), MSSTORE_INLINE_DELAY);
}

/// Lo que hay que hacer con lo escrito arriba: preguntárselo a los tres
/// catálogos de fuera. El propio se filtra en memoria y no pasa por aquí.
///
/// Un único punto de entrada para no tener que acordarse de los tres cada vez
/// que la barra cambia, que son cuatro sitios distintos del programa.
function scheduleInlineSearches() {
  scheduleInlineMsStoreSearch();
  scheduleInlineRepoSearches();
  if (repoInlineFits() && state.search.trim().length >= MSSTORE_INLINE_MIN_CHARS) {
    void ensureChocoStatus();
  }
}

async function runInlineMsStoreSearch(query) {
  const inline = state.msstore.inline;
  inline.requested = query;
  inline.loading = true;
  inline.error = "";
  renderContent();
  void clientLog("info", "msstore", `Búsqueda desde la barra: ${query}`);
  try {
    const response = await invoke("msstore_search", { query });
    // Mientras se preguntaba se ha seguido escribiendo: esta respuesta ya no
    // es la de lo que hay en la caja, y pintarla sería mentir sobre qué se
    // buscó.
    if (inline.requested !== query) return;
    inline.results = Array.isArray(response?.products) ? response.products : [];
    inline.query = query;
    inline.loading = false;
    renderContent();
    // Saber cuáles ya están puestas es lo que convierte «Instalar» en «Abrir»
    // en esas tarjetas. Se pide una sola vez por sesión y no retiene la lista.
    if (inline.results.length) void ensureMsStoreInstalled().then(() => renderContent());
  } catch (error) {
    if (inline.requested !== query) return;
    inline.loading = false;
    inline.results = [];
    inline.query = query;
    inline.error = String(error);
    renderContent();
    void clientLog("warn", "msstore", `Búsqueda desde la barra fallida: ${error}`);
  }
}

/// La subsección que cuelga de los resultados del catálogo: lo que la
/// Microsoft Store ofrece para lo mismo que se acaba de escribir.
///
/// Va debajo y con su propio encabezado a propósito. Son dos catálogos con
/// reglas distintas —uno revisado a mano, el otro el de Microsoft entero— y
/// mezclarlos haría pasar por seleccionado algo que no lo está.
function msStoreInlineSectionHtml() {
  const inline = state.msstore.inline;
  const query = state.search.trim();
  if (!inlineMsStoreFits()) return "";
  if (query.length < MSSTORE_INLINE_MIN_CHARS) return "";

  const head = (note) => `
    <div class="section-head ms-section-head">
      <h3>Microsoft Store</h3>
      <span>${note}</span>
      <button type="button" class="btn" data-ms-see-all="${escapeHtml(query)}"
        title="Abrir la sección Microsoft Store con esta búsqueda">Ver en la Microsoft Store</button>
    </div>`;

  if (inline.loading && inline.requested === query) {
    return `
      <section class="section ms-inline-section">
        ${head("Consultando el catálogo oficial…")}
        <div class="ms-inline-note"><span class="pulse-dot"></span> Buscando «${escapeHtml(query)}» en la Microsoft Store…</div>
      </section>`;
  }
  // Todavía no hay respuesta para esto: el bloque no aparece hasta que la haya.
  if (inline.query !== query) return "";
  if (inline.error) {
    return `
      <section class="section ms-inline-section">
        ${head("No se pudo consultar")}
        <div class="ms-inline-note bad">${escapeHtml(inline.error)}</div>
      </section>`;
  }
  if (!inline.results.length) {
    return `
      <section class="section ms-inline-section">
        ${head("Sin resultados")}
        <div class="ms-inline-note">La Microsoft Store no publica nada gratuito que coincida con «${escapeHtml(query)}».</div>
      </section>`;
  }

  const shown = inline.results.slice(0, MSSTORE_INLINE_LIMIT);
  const rest = inline.results.length - shown.length;
  return `
    <section class="section ms-inline-section">
      ${head(`${inline.results.length} ${inline.results.length === 1 ? "producto" : "productos"} · ${escapeHtml(msStoreRingLabel())}`)}
      <div class="grid">${shown.map(msStoreCardHtml).join("")}</div>
      ${
        rest > 0
          ? `<div class="ms-inline-note">Y ${rest} ${rest === 1 ? "resultado más" : "resultados más"} en la sección Microsoft Store.</div>`
          : ""
      }
    </section>`;
}

// --- Los otros dos repositorios -------------------------------------------
//
// Escribir en la barra de arriba pregunta a cuatro sitios: al catálogo propio,
// que se filtra en memoria; a la Microsoft Store, que tiene su bloque justo
// encima; y a WinGet y a Chocolatey, que son estos. Cada uno cuelga en su
// propia subsección y ninguno espera a los demás: WinGet tardando cinco
// segundos no puede dejar en blanco lo que Chocolatey ya ha traído.
//
// Y van separados del catálogo por lo mismo que la Microsoft Store: uno está
// revisado a mano y los otros son repositorios enteros. Mezclarlos haría pasar
// por seleccionado algo que nadie ha seleccionado.

/// Los dos repositorios, descritos en vez de programados dos veces. Lo único
/// que los distingue es a quién se le pregunta, cómo se llama lo que contesta
/// y con qué campo se instala después.
const REPOS = {
  winget: {
    label: "WinGet",
    command: "winget_search",
    /// El prefijo del identificador con el que viaja al backend. Lleva guión y
    /// no dos puntos porque el instalador puede acabar creando una carpeta con
    /// ese nombre, y Windows no admite `:` en una.
    prefix: "winget-",
    source_type: "winget",
    package_field: "winget_id",
    accent: "#0078d4",
    empty: (query) => `WinGet no publica ningún paquete que coincida con «${query}».`,
    /// De la respuesta del backend a lo que dibuja una tarjeta.
    normalize: (item) => ({
      key: item.id,
      name: item.name || item.id,
      version: item.version || "",
      publisher: item.id.includes(".") ? item.id.split(".")[0] : "WinGet",
      description: "",
      icon: null,
      page_url: "",
      warning: "",
    }),
  },
  choco: {
    label: "Chocolatey",
    command: "choco_search",
    prefix: "choco-",
    source_type: "choco",
    package_field: "choco_id",
    accent: "#80b5e3",
    empty: (query) => `El repositorio de la comunidad no tiene nada que coincida con «${query}».`,
    normalize: (item) => ({
      key: item.id,
      name: item.title || item.id,
      version: item.version || "",
      publisher: item.author || "Chocolatey",
      description: item.summary || "",
      icon: item.icon_url || null,
      page_url: item.page_url || "",
      // Chocolatey modera los paquetes de la comunidad, y uno sin ese visto
      // bueno se enseña igual pero diciéndolo: es la diferencia entre lo que
      // alguien ha revisado y lo que alguien acaba de subir.
      warning: item.approved ? "" : "Sin moderar",
    }),
  },
};

/// Cuántos resultados enseña cada repositorio. Los mismos que la Microsoft
/// Store, y por lo mismo: pasando de ahí dejan de ser un apunte debajo del
/// catálogo y pasan a taparlo.
const REPO_INLINE_LIMIT = 6;

const repoInlineTimers = {};

/// Dónde tiene sentido colgar los repositorios: donde se está buscando algo que
/// instalar. En «Aplicaciones instaladas» y «Actualizaciones» se mira lo que ya está
/// puesto, y en la sección de la Microsoft Store se está en otra tienda.
function repoInlineFits() {
  return !["installed", "updates", MSSTORE_SECTION].includes(state.section);
}

/// Manda a los dos repositorios la pregunta que hay escrita arriba.
///
/// Se llama a cada tecla, con el mismo trato que la Microsoft Store: por debajo
/// del mínimo de letras no se pregunta, y vaciar la caja borra lo traído para
/// que no quede como respuesta a algo que ya nadie pregunta.
function scheduleInlineRepoSearches() {
  const query = state.search.trim();
  for (const repoId of Object.keys(REPOS)) {
    const inline = state.repos[repoId];
    clearTimeout(repoInlineTimers[repoId]);
    if (!repoInlineFits() || query.length < MSSTORE_INLINE_MIN_CHARS) {
      if (inline.query || inline.requested || inline.loading || inline.error) {
        Object.assign(inline, {
          query: "",
          requested: "",
          results: [],
          loading: false,
          error: "",
        });
        renderContent();
      }
      continue;
    }
    if (inline.requested === query) continue;
    repoInlineTimers[repoId] = setTimeout(
      () => void runInlineRepoSearch(repoId, query),
      MSSTORE_INLINE_DELAY,
    );
  }
}

async function runInlineRepoSearch(repoId, query) {
  const repo = REPOS[repoId];
  const inline = state.repos[repoId];
  inline.requested = query;
  inline.loading = true;
  inline.error = "";
  renderContent();
  void clientLog("info", repoId, `Búsqueda desde la barra: ${query}`);
  try {
    const results = await invoke(repo.command, { query });
    // Mientras se preguntaba se ha seguido escribiendo: pintar esto ahora sería
    // mentir sobre qué se buscó.
    if (inline.requested !== query) return;
    // Lo que el catálogo ya ofrece no se repite abajo: una tarjeta revisada a
    // mano y otra sin revisar del mismo programa, con dos botones «Instalar»
    // que hacen cosas distintas, es la peor respuesta posible a un nombre.
    inline.results = (Array.isArray(results) ? results : [])
      .map(repo.normalize)
      .filter((pkg) => !derived.catalogPackageKeys[repoId].has(pkg.key.toLowerCase()));
    inline.query = query;
    inline.loading = false;
    renderContent();
  } catch (error) {
    if (inline.requested !== query) return;
    inline.loading = false;
    inline.results = [];
    inline.query = query;
    inline.error = String(error);
    renderContent();
    void clientLog("warn", repoId, `Búsqueda desde la barra fallida: ${error}`);
  }
}

/// Si el equipo tiene Chocolatey. Se pregunta una vez por sesión, la primera
/// que hace falta, y sólo para poder avisar antes de pulsar de que la primera
/// instalación traerá también el gestor.
let chocoStatusProbe = null;

async function ensureChocoStatus() {
  if (state.repos.choco.available !== null) return;
  // Se guarda la promesa, no sólo el resultado: entre que se pregunta y que
  // llega la respuesta hay sitio para dos teclas más, y cada una lanzaba su
  // propia consulta porque `available` seguía sin contestar. En el diario se
  // veían tres `choco --version` seguidos para una sola búsqueda.
  if (chocoStatusProbe) return chocoStatusProbe;
  chocoStatusProbe = (async () => {
    try {
      const status = await invoke("choco_status");
      state.repos.choco.available = !!status?.available;
      state.repos.choco.version = status?.version || "";
      state.repos.choco.elevated = status?.elevated !== false;
    } catch (error) {
      // No saberlo no impide instalar: el backend lo comprueba otra vez y se
      // ocupa. Sólo se pierde el aviso previo.
      state.repos.choco.available = false;
      void clientLog("warn", "choco", `No se pudo comprobar Chocolatey: ${error}`);
    } finally {
      chocoStatusProbe = null;
    }
    renderContent();
  })();
  return chocoStatusProbe;
}

/// El identificador con el que un paquete de fuera del catálogo viaja por toda
/// la tienda: su tarea de descarga, su diálogo y su fila en el panel.
function repoAppId(repoId, packageKey) {
  return `${REPOS[repoId].prefix}${packageKey}`;
}

/// Un paquete vestido de aplicación, para lo que ya sabe dibujar aplicaciones.
function repoAppShape(repoId, pkg) {
  const repo = REPOS[repoId];
  return {
    id: repoAppId(repoId, pkg.key),
    name: pkg.name,
    author: pkg.publisher,
    description: pkg.description,
    icon_url: pkg.icon || null,
    icon_padding: 8,
    accent_color: repo.accent,
  };
}

/// La entrada de catálogo que el backend necesita para instalarlo.
///
/// No se guarda en `apps.json` ni se pretende: dura lo que dura la instalación
/// y describe exactamente un paquete de un repositorio.
function repoCatalogEntry(repoId, pkg) {
  const repo = REPOS[repoId];
  return {
    id: repoAppId(repoId, pkg.key),
    name: pkg.name,
    description: pkg.description || `Paquete de ${repo.label}: ${pkg.key}`,
    version: pkg.version || "latest",
    author: pkg.publisher,
    source_type: repo.source_type,
    [repo.package_field]: pkg.key,
    icon_url: pkg.icon || null,
    accent_color: repo.accent,
  };
}

function repoPackageByAppId(appId) {
  for (const [repoId, repo] of Object.entries(REPOS)) {
    if (!appId.startsWith(repo.prefix)) continue;
    const key = appId.slice(repo.prefix.length);
    const found = state.repos[repoId].results.find((pkg) => pkg.key === key);
    if (found) return { repoId, pkg: found };
  }
  return null;
}

function installedRepoApp(repoId, pkg) {
  const entry = repoCatalogEntry(repoId, pkg);
  if (appStatus(entry.id).installed) return entry;
  const packageField = REPOS[repoId].package_field;
  return knownApps().find((app) => appStatus(app.id).installed && (
    String(app[packageField] || "").toLowerCase() === pkg.key.toLowerCase() ||
    String(app.name || "").trim().toLowerCase() === pkg.name.trim().toLowerCase()
  ));
}

function repoCardHtml(repoId, pkg, index) {
  const repo = REPOS[repoId];
  const appId = repoAppId(repoId, pkg.key);
  const shape = repoAppShape(repoId, pkg);
  const accent = repo.accent;
  const avatarBg = avatarBackground(shape, accent);
  const letter = (pkg.name || pkg.key || "?")[0].toUpperCase();
  const busy = state.busy[appId];
  const installedApp = installedRepoApp(repoId, pkg);
  // Sin versión que enseñar —WinGet contesta «Unknown» a menudo— no se dice
  // nada antes que «vUnknown».
  const meta = [pkg.publisher, versionLabel(pkg.version, "")]
    .filter(Boolean)
    .join("  ·  ");

  const actions = busy
    ? `<button type="button" class="btn secondary" disabled aria-busy="true">Instalando…</button>`
    : installedApp
      ? actionButtons(installedApp)
      : `<button type="button" class="btn primary" data-repo-install="${escapeHtml(repoId)}"
         data-repo-package="${escapeHtml(pkg.key)}">Instalar</button>${
        pkg.page_url
          ? `<button type="button" class="btn ghost" data-repo-page="${escapeHtml(pkg.page_url)}">Ver ficha</button>`
          : ""
      }`;

  return `
    <article class="app-card repo-card" data-repo-card="${escapeHtml(appId)}"
      style="--card-accent:${accent}">
      <div class="card-top">
        <div class="card-avatar" style="background:${avatarBg}">${renderAvatar(shape, letter, false)}</div>
        <div>
          ${installedApp ? cardMetaHtml(installedApp) : `
            <strong>${escapeHtml(pkg.name)}
              <span class="origin-tag repo-tag">${escapeHtml(repo.label)}</span>
              ${pkg.warning ? `<span class="origin-tag repo-warning-tag">${escapeHtml(pkg.warning)}</span>` : ""}
            </strong>
            <small>${escapeHtml(meta)}</small>`}
          <small class="repo-package-id">${escapeHtml(pkg.key)}</small>
        </div>
      </div>
      <p class="card-desc">${escapeHtml(pkg.description || "")}</p>
      <div class="card-actions">${actions}</div>
    </article>`;
}

/// La subsección de un repositorio: lo que ofrece para lo que se acaba de
/// escribir.
function repoInlineSectionHtml(repoId) {
  const repo = REPOS[repoId];
  const inline = state.repos[repoId];
  const query = state.search.trim();
  if (!repoInlineFits()) return "";
  if (query.length < MSSTORE_INLINE_MIN_CHARS) return "";

  const head = (note) => `
    <div class="section-head ms-section-head">
      <h3>${escapeHtml(repo.label)}</h3>
      <span>${note}</span>
    </div>`;

  if (inline.loading && inline.requested === query) {
    return `
      <section class="section repo-inline-section">
        ${head("Consultando el repositorio…")}
        <div class="ms-inline-note"><span class="pulse-dot"></span> Buscando «${escapeHtml(query)}» en ${escapeHtml(repo.label)}…</div>
      </section>`;
  }
  if (inline.query !== query) return "";
  if (inline.error) {
    return `
      <section class="section repo-inline-section">
        ${head("No se pudo consultar")}
        <div class="ms-inline-note bad">${escapeHtml(inline.error)}</div>
      </section>`;
  }
  if (!inline.results.length) {
    return `
      <section class="section repo-inline-section">
        ${head("Sin resultados")}
        <div class="ms-inline-note">${escapeHtml(repo.empty(query))}</div>
      </section>`;
  }

  const shown = inline.results.slice(0, REPO_INLINE_LIMIT);
  const rest = inline.results.length - shown.length;
  // El aviso va aquí y no en el diálogo de confirmación porque es lo que
  // cambia la decisión: instalar el primer paquete de Chocolatey instala
  // también Chocolatey, y eso se sabe antes de pulsar, no después.
  //
  // Y va con su propia clase, no con la de las notas de debajo: aquélla
  // reserva el aire por arriba, que es donde lo necesita quien cierra una
  // rejilla, y aquí el aire hace falta por abajo para no quedar pegado a la
  // primera fila de tarjetas.
  const chocoNote =
    repoId !== "choco"
      ? ""
      : state.repos.choco.elevated === false
        ? // Sin permisos de administrador no hay instalación posible, y eso
          // pesa más que decir si Chocolatey está o no: se dice esto en su
          // lugar, con lo que hay que hacer.
          `<div class="repo-inline-lead">WinSlimCenter no se está ejecutando como administrador, y Chocolatey lo necesita para instalar. Ábrelo como administrador para poder instalar desde aquí.</div>`
        : state.repos.choco.available === false
          ? `<div class="repo-inline-lead">Este equipo no tiene Chocolatey. La primera instalación lo instalará antes que el paquete.</div>`
          : "";
  return `
    <section class="section repo-inline-section">
      ${head(`${inline.results.length} ${inline.results.length === 1 ? "paquete" : "paquetes"}`)}
      ${chocoNote}
      <div class="grid">${shown.map((pkg, index) => repoCardHtml(repoId, pkg, index)).join("")}</div>
      ${
        rest > 0
          ? `<div class="ms-inline-note">Y ${rest} ${rest === 1 ? "paquete más" : "paquetes más"} en ${escapeHtml(repo.label)}. Afina la búsqueda para verlos.</div>`
          : ""
      }
    </section>`;
}

/// Instala un paquete que no está en el catálogo.
///
/// Se apunta en `derived.repoPackages` antes de pedirlo para que todo lo que ya
/// existe —el diálogo de progreso, el panel de descargas, el aviso del final—
/// lo encuentre por su identificador igual que a una aplicación del catálogo.
async function installRepoPackage(repoId, packageKey) {
  const repo = REPOS[repoId];
  const pkg = state.repos[repoId].results.find((item) => item.key === packageKey);
  if (!repo || !pkg) return;
  const appId = repoAppId(repoId, pkg.key);
  if (state.busy[appId]) {
    showAlertModal("Instalación en curso", `'${pkg.name}' ya se está instalando.`);
    return;
  }
  // Lo apuntado de instalaciones ya terminadas deja de hacer falta: quien
  // preguntaba por ellas era el diálogo que ya se cerró.
  for (const [known] of derived.repoPackages) {
    if (!state.busy[known]) derived.repoPackages.delete(known);
  }
  const entry = repoCatalogEntry(repoId, pkg);
  derived.repoPackages.set(appId, entry);

  const needsChoco = repoId === "choco" && state.repos.choco.available === false;
  const start = async () => {
    state.busy[appId] = "installing";
    state.finished.delete(appId);
    renderContent();
    renderDlPanel();
    showPackageOperationModal(entry, false);
    try {
      await invoke("install_app", {
        appEntry: entry,
        forceUpdate: false,
        variant: null,
        closeRunning: false,
      });
    } catch (error) {
      delete state.busy[appId];
      state.operationAppId = null;
      closeModal();
      renderContent();
      setStatus(`Error: ${error}`, "var(--red)");
      showAlertModal("Error de instalación", String(error));
    }
  };

  showConfirmModal({
    title: `Instalar ${pkg.name}`,
    message: needsChoco
      ? `'${pkg.name}' viene de Chocolatey, que este equipo todavía no tiene. Se instalará primero Chocolatey y después el paquete. ¿Continuar?`
      : `¿Deseas instalar '${pkg.name}' desde ${repo.label}?`,
    app: entry,
    confirmText: "Instalar",
    confirmVariant: "primary",
    onConfirm: () => void start(),
  });
}

function msStoreMatchesSearch(app, query) {
  if (!query) return true;
  return `${app.display_name} ${app.name} ${app.family}`.toLocaleLowerCase("es-ES").includes(query);
}

/// La subsección de «Aplicaciones instaladas» con lo que vino de la Microsoft Store.
function msStoreInstalledSectionHtml(query) {
  const apps = state.msstore.installed.filter((app) => msStoreMatchesSearch(app, query));
  if (!apps.length) return "";
  const pending = msStoreUpdatesList().length;
  return `
    <section class="section">
      <div class="section-head ms-section-head">
        <h3>Desde la Microsoft Store</h3>
        <span>${apps.length} ${apps.length === 1 ? "aplicación" : "aplicaciones"}${
          pending ? ` · ${pending} con actualización` : ""
        }</span>
        ${msStoreScanButtonHtml()}
      </div>
      <div class="grid">${apps.map(msStoreInstalledCardHtml).join("")}</div>
    </section>`;
}

/// El bloque de actualizaciones pendientes que vienen de la Microsoft Store.
function msStoreUpdatesSectionHtml(query) {
  const apps = msStoreUpdatesList().filter((app) => msStoreMatchesSearch(app, query));
  if (!apps.length) return "";
  return `
    <section class="section">
      <div class="section-head ms-section-head">
        <h3>Microsoft Store</h3>
        <span>${apps.length} ${apps.length === 1 ? "actualización" : "actualizaciones"} · ${escapeHtml(msStoreRingLabel())}</span>
        ${msStoreScanButtonHtml()}
      </div>
      <div class="grid">${apps.map(msStoreInstalledCardHtml).join("")}</div>
    </section>`;
}

function renderMsStoreNow() {
  const store = state.msstore;
  const content = document.getElementById("content");
  // La caja de búsqueda vive dentro de la lista que se redibuja, así que hay
  // que devolverle el foco y el cursor donde estaban o escribir en ella se
  // interrumpe sola en cuanto llegan los resultados.
  const focused = document.activeElement;
  const keepFocus = focused?.id === "ms-search";
  const caret = keepFocus ? focused.selectionStart : null;

  const count = store.results.length;
  let html = `
    <div class="page-title">
      <h1>Microsoft Store</h1>
      <span>${
        store.lastQuery
          ? `${count} ${count === 1 ? "resultado" : "resultados"}`
          : "Catálogo oficial de Microsoft"
      }</span>
    </div>
    ${msStoreSearchHtml()}
    ${msStoreFiltersHtml()}`;

  if (store.loading) {
    html += `
      <div class="empty ms-loading">
        <div class="ms-loading-status"><span class="pulse-dot"></span> Consultando la Microsoft Store…</div>
        <p>Se busca en el catálogo oficial de Microsoft, no en el de WinSlimCenter.</p>
      </div>`;
  } else if (store.error) {
    html += `
      <div class="empty">
        <h3>No se pudo consultar la Microsoft Store</h3>
        <p>${escapeHtml(store.error)}</p>
      </div>`;
  } else if (!store.lastQuery) {
    html += msStoreHeroHtml();
  } else if (!count) {
    html += `
      <div class="empty">
        <h3>Sin resultados</h3>
        <p>No hay productos gratuitos que coincidan con «${escapeHtml(store.lastQuery)}».</p>
      </div>`;
  } else {
    html += `
      <section class="section">
        <div class="section-head">
          <h3>Resultados de «${escapeHtml(store.lastQuery)}»</h3>
          <span>${count} ${count === 1 ? "producto" : "productos"} · ${escapeHtml(msStoreRingLabel())}</span>
        </div>
        <div class="grid">${store.results.map(msStoreCardHtml).join("")}</div>
      </section>`;
  }

  const view = `msstore|${store.lastQuery}|${store.loading}`;
  const restoreTo = view === lastRenderedView ? content.scrollTop : 0;
  lastRenderedView = view;
  replaceContent(content, html, restoreTo);

  if (keepFocus) {
    const input = document.getElementById("ms-search");
    if (input) {
      input.focus();
      if (caret != null) input.setSelectionRange(caret, caret);
    }
  }
}

async function runMsStoreSearch() {
  const store = state.msstore;
  const query = store.query.trim();
  if (!query) {
    store.results = [];
    store.lastQuery = "";
    store.error = "";
    renderContent();
    return;
  }
  if (store.loading) return;

  store.loading = true;
  store.error = "";
  renderContent();
  setStatus(`Buscando «${query}» en la Microsoft Store…`, "var(--accent)");
  void clientLog("info", "msstore", `Búsqueda en la Microsoft Store: ${query}`);

  try {
    const response = await invoke("msstore_search", { query });
    const products = Array.isArray(response?.products) ? response.products : [];
    store.results = products;
    if (products.length) void ensureMsStoreInstalled().then(() => renderContent());
    store.lastQuery = query;
    store.error = "";
    store.loading = false;
    renderContent();
    setTransientStatus(
      products.length
        ? `${products.length} ${products.length === 1 ? "resultado" : "resultados"} en la Microsoft Store`
        : `Sin resultados para «${query}» en la Microsoft Store`,
      products.length ? "var(--green)" : "var(--text-light)",
    );
    // De los resultados, los que el equipo ya tiene puestos son casi siempre
    // uno o ninguno: preguntar por sus versiones cuesta poco y es lo que
    // convierte «ya instalada» en «tiene actualización».
    const families = products
      .filter((product) => product.installed && product.installed_family)
      .map((product) => product.installed_family)
      .filter((family) => !msStoreUpdateOf(family));
    if (families.length) void scanMsStoreUpdates({ families, quiet: true });

    // Quien pega el enlace de una ficha no está buscando: ya sabe lo que
    // quiere, así que se le abre directamente.
    if (response?.direct && products.length === 1) openMsStoreModal(products[0].product_id);
  } catch (error) {
    store.loading = false;
    store.results = [];
    store.lastQuery = query;
    store.error = String(error);
    renderContent();
    setTransientStatus("Error al buscar en la Microsoft Store", "var(--red)", 6000);
    void clientLog("warn", "msstore", `Búsqueda fallida: ${error}`);
  }
}

/** Los datos de la ficha que merece la pena enseñar, cuando Microsoft los da. */
function msStoreFactsHtml(details) {
  const facts = [
    ["Versión", details.version],
    ["Tamaño", msStoreBytes(details.approximateSizeInBytes || details.maxInstallSizeInBytes)],
    [
      "Valoración",
      Number(details.averageRating) > 0
        ? `${Number(details.averageRating).toFixed(1)} / 5${
            details.ratingCountFormatted ? ` · ${details.ratingCountFormatted}` : ""
          }`
        : "",
    ],
    [
      "Actualizado",
      msStoreDate(details.packageLastUpdateDateUtc || details.lastUpdateDateUtc),
    ],
    ["Categoría", details.subcategoryName || (details.categories || [])[0]],
    [
      "Idiomas",
      Array.isArray(details.supportedLanguages) && details.supportedLanguages.length
        ? String(details.supportedLanguages.length)
        : "",
    ],
  ].filter(([, value]) => value);

  if (!facts.length) return "";
  return `
    <dl class="ms-facts">
      ${facts
        .map(
          ([label, value]) => `
        <div class="ms-fact">
          <dt>${escapeHtml(label)}</dt>
          <dd>${escapeHtml(value)}</dd>
        </div>`
        )
        .join("")}
    </dl>`;
}

/**
 * Las capturas de la ficha, en la tira que se desplaza a lo ancho.
 *
 * A la altura que caben aquí no se lee lo que enseñan, así que cada una es un
 * botón que las abre a tamaño completo. Botón y no imagen suelta: así se llega
 * con el tabulador y se abre con Enter sin inventar nada, y el foco se ve.
 */
function msStoreShotsHtml(details) {
  const sources = [details.screenshots, details.images, details.previews].find(
    (list) => Array.isArray(list) && list.length,
  );
  const shots = (sources || [])
    .map((image) => msStoreImageUrl(image?.url))
    .filter(Boolean)
    .slice(0, 8);
  if (!shots.length) return "";
  return `
    <div class="ms-shots" aria-label="Capturas de la aplicación" data-shots="${escapeHtml(
      JSON.stringify(shots),
    )}">
      ${shots
        .map(
          (url, index) => `
        <button type="button" class="ms-shot" data-shot-index="${index}"
          aria-label="Ampliar la captura ${index + 1} de ${shots.length}">
          <img src="${escapeHtml(url)}" alt="" loading="lazy" decoding="async"
            onerror="this.closest('.ms-shot')?.remove()" />
        </button>`
        )
        .join("")}
    </div>`;
}

/// La captura que se está mirando ampliada, y la tira a la que pertenece.
/// `null` cuando el visor no está abierto.
let shotViewer = null;

/**
 * Abre una captura a tamaño completo por encima de la ficha.
 *
 * Capa propia, no `openModal`: el diálogo del producto tiene que seguir donde
 * estaba —con su ficha ya pedida y su desplazamiento— para volver a él al
 * cerrar el visor. Reutilizar el único hueco de modal lo habría borrado.
 */
function openShotViewer(shots, index) {
  if (!Array.isArray(shots) || !shots.length) return;
  closeShotViewer();
  const overlay = document.createElement("div");
  overlay.className = "shot-viewer";
  overlay.setAttribute("role", "dialog");
  overlay.setAttribute("aria-modal", "true");
  overlay.setAttribute("aria-label", "Captura ampliada");
  overlay.innerHTML = `
    <button type="button" class="shot-viewer-close" data-shot-close aria-label="Cerrar la captura">
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        stroke-width="2.4" stroke-linecap="round" aria-hidden="true">
        <path d="M6 6l12 12M18 6L6 18" />
      </svg>
    </button>
    ${
      shots.length > 1
        ? `<button type="button" class="shot-viewer-nav prev" data-shot-step="-1" aria-label="Captura anterior">
             <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
               <path d="M15 5l-7 7 7 7" />
             </svg>
           </button>
           <button type="button" class="shot-viewer-nav next" data-shot-step="1" aria-label="Captura siguiente">
             <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor"
               stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
               <path d="M9 5l7 7-7 7" />
             </svg>
           </button>`
        : ""
    }
    <figure class="shot-viewer-stage">
      <img class="shot-viewer-image" alt="" decoding="async" />
    </figure>
    ${shots.length > 1 ? '<div class="shot-viewer-count" aria-live="polite"></div>' : ""}
  `;
  document.body.appendChild(overlay);
  shotViewer = {
    shots,
    index: Math.max(0, Math.min(shots.length - 1, Number(index) || 0)),
    overlay,
    returnFocus: document.activeElement,
  };

  // Pinchar el fondo cierra; pinchar la imagen o un control, no.
  overlay.addEventListener("click", (event) => {
    if (event.target.closest("[data-shot-close]")) {
      closeShotViewer();
      return;
    }
    const step = event.target.closest("[data-shot-step]");
    if (step) {
      stepShotViewer(Number(step.dataset.shotStep));
      return;
    }
    if (!event.target.closest(".shot-viewer-stage")) closeShotViewer();
  });

  renderShotViewer();
  overlay.querySelector(".shot-viewer-close")?.focus();
}

function renderShotViewer() {
  if (!shotViewer) return;
  const { shots, index, overlay } = shotViewer;
  const image = overlay.querySelector(".shot-viewer-image");
  if (image) {
    image.src = shots[index];
    image.alt = `Captura ${index + 1} de ${shots.length}`;
  }
  const count = overlay.querySelector(".shot-viewer-count");
  if (count) count.textContent = `${index + 1} / ${shots.length}`;
}

/// Pasa a la captura siguiente o anterior, dando la vuelta por los extremos.
function stepShotViewer(delta) {
  if (!shotViewer || shotViewer.shots.length < 2) return;
  const total = shotViewer.shots.length;
  shotViewer.index = (shotViewer.index + delta + total) % total;
  renderShotViewer();
}

function closeShotViewer() {
  if (!shotViewer) return;
  const { overlay, returnFocus } = shotViewer;
  shotViewer = null;
  overlay.remove();
  if (returnFocus instanceof HTMLElement && document.contains(returnFocus)) returnFocus.focus();
}

/**
 * El producto detrás de un identificador, esté o no en los resultados.
 *
 * Una tarjeta de «Aplicaciones instaladas» conoce su producto pero nunca pasó por una
 * búsqueda, así que la ficha se pide igual que cuando se pega su enlace.
 */
async function msStoreResolveProduct(productId) {
  const id = String(productId || "").toUpperCase();
  const known = msStoreProductById(id);
  if (known) return known;
  try {
    const response = await invoke("msstore_search", { query: id });
    const product = (response?.products || [])[0];
    if (product) state.msstore.results = [...state.msstore.results, product];
    return product || null;
  } catch (error) {
    void clientLog("warn", "msstore", `No se pudo abrir la ficha de ${id}: ${error}`);
    return null;
  }
}

async function openMsStoreModal(productId) {
  const id = String(productId || "").toUpperCase();
  const product = await msStoreResolveProduct(id);
  if (!product) {
    showAlertModal(
      "Ficha no disponible",
      `La Microsoft Store no devolvió la ficha de ${id}.`,
    );
    return;
  }

  const shape = msStoreAppShape(product);
  const accent = pickAccent(shape, 0);
  const avatarBg = avatarBackground(shape, accent);
  const letter = (product.title || id || "M")[0].toUpperCase();
  const kind = MSSTORE_KIND_LABELS[product.kind] || "";
  const installed = msStoreInstalledRecord(product);
  const report = installed ? msStoreUpdateOf(installed.family) : null;

  openModal(`
    <div class="confirm-dialog app-detail-modal ms-detail" data-ms-detail="${escapeHtml(id)}">
      <div class="confirm-dialog-header app-detail-header">
        <div class="card-avatar app-detail-avatar" style="background:${avatarBg};">
          ${renderAvatar(shape, letter, true)}
        </div>
        <div class="app-detail-heading">
          <h2 class="confirm-dialog-title app-detail-title">${escapeHtml(product.title || id)}</h2>
          ${appDetailMetaHtml([`Por ${product.publisher || "Microsoft Store"}`, "Gratis", kind])}
          <div><span class="origin-tag ms-kind-tag">Microsoft Store · ${escapeHtml(id)}</span></div>
          ${
            installed
              ? `<div class="installed-badge-banner"><span class="badge-dot green"></span>Instalada · ${escapeHtml(versionLabel(installed.version, "versión desconocida"))}</div>
                 ${report?.update_available ? `<div>${msStoreVersionBadge(installed, { modal: true })}</div>` : ""}`
              : ""
          }
        </div>
      </div>
      ${appDescriptionHtml(product.description)}
      <div id="ms-detail-extra" class="ms-detail-extra">
        <p class="ms-detail-loading">Consultando la ficha en la Microsoft Store…</p>
      </div>
      <div class="modal-foot">
        <button type="button" class="btn ghost" id="ms-modal-close">Cerrar</button>
        <button type="button" class="btn secondary" id="ms-modal-web">Ver en la web</button>
        ${
          installed
            ? `${
                report?.update_available && report.product_id
                  ? `<button type="button" class="btn secondary" id="ms-modal-update">Actualizar</button>`
                  : installed.launch_target
                    ? `<button type="button" class="btn primary" id="ms-modal-launch">Abrir</button>`
                    : ""
              }
               ${
                 installed.can_uninstall
                   ? `<button type="button" class="btn danger" id="ms-modal-uninstall">Desinstalar</button>`
                   : `<button type="button" class="btn installed-label" disabled>La protege Windows</button>`
               }`
            : `<button type="button" class="btn primary" id="ms-modal-install">Instalar</button>`
        }
      </div>
    </div>
  `);

  document.getElementById("ms-modal-close").onclick = closeModal;
  const installButton = document.getElementById("ms-modal-install");
  if (installButton) {
    installButton.onclick = () => {
      closeModal();
      void installMsStoreProduct(id);
    };
  }
  const updateButton = document.getElementById("ms-modal-update");
  if (updateButton) {
    updateButton.onclick = () => {
      closeModal();
      updateMsStoreApp(report.product_id, installed.family);
    };
  }
  const launchButton = document.getElementById("ms-modal-launch");
  if (launchButton) {
    launchButton.onclick = () => {
      closeModal();
      void launchMsStoreApp(installed.family);
    };
  }
  const uninstallButton = document.getElementById("ms-modal-uninstall");
  if (uninstallButton) {
    uninstallButton.onclick = () => {
      closeModal();
      void uninstallMsStoreApp(installed.family);
    };
  }
  document.getElementById("ms-modal-web").onclick = () => {
    invoke("open_url", { url: `https://apps.microsoft.com/detail/${id}` }).catch((error) => {
      setStatus(`No se pudo abrir la ficha: ${error}`, "var(--red)");
    });
  };

  let details = state.msstore.details[id];
  if (!details) {
    try {
      details = await invoke("msstore_details", { productId: id });
      state.msstore.details[id] = details;
    } catch (error) {
      void clientLog("warn", "msstore", `Ficha no disponible para ${id}: ${error}`);
      details = null;
    }
  }

  // El usuario puede haber cerrado la ficha —o abierto otra— mientras Microsoft
  // contestaba.
  const extra = document.getElementById("ms-detail-extra");
  if (!extra || extra.closest("[data-ms-detail]")?.dataset.msDetail !== id) return;
  if (!details) {
    extra.innerHTML = "";
    return;
  }
  const description = document.querySelector(".ms-detail .confirm-dialog-msg");
  if (description && !description.textContent.trim() && details.description) {
    description.textContent = details.description;
  }
  extra.innerHTML = `${msStoreFactsHtml(details)}${msStoreShotsHtml(details)}`;

  // La tira de capturas se acaba de crear, así que su escucha se pone aquí y
  // no en la delegación general: aquella vive en la lista de la tienda, y esto
  // es del diálogo, que se va entero cuando se cierra.
  const strip = extra.querySelector(".ms-shots");
  if (strip) {
    let shots = [];
    try {
      shots = JSON.parse(strip.dataset.shots || "[]");
    } catch {
      shots = [];
    }
    strip.addEventListener("click", (event) => {
      const shot = event.target.closest("[data-shot-index]");
      if (shot) openShotViewer(shots, Number(shot.dataset.shotIndex));
    });
  }
}

/**
 * Instala —o actualiza— un producto de la Microsoft Store.
 *
 * Actualizar es instalar la versión que publica el canal encima de la que hay:
 * Windows sustituye el paquete registrado, así que no hay dos operaciones que
 * distinguir, sólo dos maneras de contarlo.
 */
function isMsStoreNewerPackageError(error) {
  const text = String(error || "");
  return /0x80073D06/i.test(text) || /(?:versi[oó]n|version)\s+superior/i.test(text);
}

function showMsStoreInstallError(error, { name = "", newerPackage = false, suppressed = false } = {}) {
  const detail = String(error || "Error desconocido").trim();
  if (!newerPackage) {
    showAlertModal("Error de instalación", detail);
    return;
  }

  openModal(`
    <div class="confirm-dialog msstore-error-dialog">
      <h2 class="confirm-dialog-title">Actualización no aplicable</h2>
      <p class="confirm-dialog-msg msstore-error-lead">
        Windows ya tiene una versión más reciente de uno de los componentes que necesita
        ${escapeHtml(name || "esta aplicación")}. No es necesario sustituirla por una anterior.
      </p>
      <div class="msstore-error-resolution">
        ${
          suppressed
            ? "Esta versión se retiró de las actualizaciones pendientes. Volverá a aparecer automáticamente cuando Microsoft publique una versión distinta."
            : "Windows no hizo cambios en el paquete instalado."
        }
      </div>
      <details class="msstore-error-details">
        <summary>Ver detalle técnico</summary>
        <div class="msstore-error-raw">${escapeHtml(detail)}</div>
      </details>
      <div class="modal-foot">
        <button type="button" class="btn primary" id="modal-btn-close">Aceptar</button>
      </div>
    </div>
  `);
  document.getElementById("modal-btn-close").onclick = closeModal;
}

async function installMsStoreProduct(
  productId,
  { product = null, isUpdate = false, family = null } = {},
) {
  const id = String(productId || "").toUpperCase();
  const store = state.msstore;
  // Si la tarjeta que se pulsó no está en ninguna lista se pide la ficha, como
  // hace el modal. Quedarse callado aquí es lo que hacía que pulsar «Instalar»
  // pareciera que el botón no existía.
  const target = product || (await msStoreResolveProduct(id));
  if (!target) {
    showAlertModal(
      "Producto no disponible",
      `La Microsoft Store no devolvió la ficha de ${id}, así que no se puede instalar.`,
    );
    return;
  }
  const name = target.title || id;
  if (store.installing[id]) {
    showAlertModal("Instalación en curso", `'${name}' ya se está instalando.`);
    return;
  }

  const shape = msStoreAppShape(target);
  const start = async ({ closeRunning = false } = {}) => {
    store.installing[id] = isUpdate ? "update" : "install";
    if (isUpdate && family) store.updateFamilyByProduct[id] = family;
    else delete store.updateFamilyByProduct[id];
    state.finished.delete(msStoreTaskId(id));
    renderContent();
    state.operationAppId = msStoreTaskId(id);
    showBackgroundOperationModal(
      shape,
      `${isUpdate ? "Actualizando" : "Instalando"} ${name}`,
      "Consultando la Microsoft Store…",
      true,
    );
    const actions = document.getElementById("package-operation-actions");
    if (actions) {
      actions.innerHTML =
        '<button type="button" class="btn ghost" id="operation-cancel">Cancelar</button>';
      bindOperationCancel(msStoreTaskId(id), "Cancelando la instalación…");
    }
    try {
      await invoke("msstore_install", {
        productId: id,
        name,
        ring: store.ring,
        arch: store.arch,
        family,
        closeRunning,
      });
    } catch (error) {
      const newerPackage = isUpdate && isMsStoreNewerPackageError(error);
      const report = family ? msStoreUpdateOf(family) : null;
      const suppressed = newerPackage && suppressMsStoreUpdate(family, report);
      delete store.installing[id];
      delete store.updateFamilyByProduct[id];
      state.operationAppId = null;
      closeModal();
      renderSidebar();
      renderContent();
      setStatus(`Error: ${error}`, "var(--red)");
      showMsStoreInstallError(error, { name, newerPackage, suppressed });
    }
  };

  // Lo mismo que se pregunta por una aplicación del catálogo: Windows aplaza
  // la actualización de un paquete en uso y la aplica cuando el usuario cierra
  // la aplicación, que es horas después y se parece mucho a no haber hecho nada.
  // Y como allí, la pregunta viaja en paralelo al diálogo en vez de retrasarlo.
  const blockerProbe =
    isUpdate && family ? msStoreBlockingRunningApp(family) : Promise.resolve(null);

  showConfirmModal({
    title: `${isUpdate ? "Actualizar" : "Instalar"} ${name}`,
    message: isUpdate
      ? `Se descargará la versión que publica el canal ${msStoreRingLabel()} ` +
        `para ${msStoreArchPhrase()} y Windows sustituirá la instalada. ¿Continuar?`
      : `Se descargará desde la Microsoft Store por el canal ${msStoreRingLabel()}, ` +
        `para ${msStoreArchPhrase()}, junto con las dependencias que necesite, ` +
        `y se instalará en Windows. ¿Continuar?`,
    app: shape,
    confirmText: isUpdate ? "Actualizar" : "Instalar",
    confirmVariant: "primary",
    onConfirm: async () => {
      const blocker = await blockerProbe;
      if (blocker) {
        showRunningAppModal({
          app: shape,
          blocker,
          isUpdate: true,
          onClose: () => start({ closeRunning: true }),
          onAnyway: () => start({ closeRunning: false }),
        });
        return;
      }
      await start();
    },
  });
}

/// Si la aplicación de la tienda que se va a actualizar está abierta. Como su
/// gemela del catálogo, nunca deja caer la operación por no poder preguntarlo.
async function msStoreBlockingRunningApp(family) {
  try {
    return await invoke("msstore_running_blocker", { family });
  } catch (error) {
    void clientLog("warn", "running-blocker", String(error?.stack || error));
    return null;
  }
}

/// Actualiza una aplicación instalada, que es su producto reinstalado.
function updateMsStoreApp(productId, family) {
  const installed = msStoreInstalledByFamily(family);
  const report = msStoreUpdateOf(family);
  void installMsStoreProduct(productId, {
    isUpdate: true,
    family,
    product: msStoreProductById(productId) || {
      product_id: String(productId || "").toUpperCase(),
      title: report?.title || installed?.display_name || family,
      publisher: "Microsoft Store",
      icon_url: report?.icon_url || null,
    },
  });
}

async function launchMsStoreApp(family) {
  const app = msStoreInstalledByFamily(family);
  if (!app) return;
  state.msstore.busy[family] = "launching";
  renderContent();
  try {
    setStatus(await invoke("msstore_launch", { family }), "var(--accent)");
  } catch (error) {
    setStatus(`No se pudo abrir ${app.display_name}: ${error}`, "var(--red)");
    showAlertModal("Error al abrir la aplicación", String(error));
  } finally {
    delete state.msstore.busy[family];
    renderContent();
  }
}

async function uninstallMsStoreApp(family) {
  const app = msStoreInstalledByFamily(family);
  if (!app) return;
  const shape = msStoreInstalledShape(app);
  showConfirmModal({
    title: `Desinstalar ${shape.name}`,
    message:
      `Windows quitará el paquete de este usuario. Si la aplicación está abierta ` +
      `se cerrará antes, porque de lo contrario la retirada quedaría pendiente. ` +
      `¿Estás seguro?`,
    app: shape,
    confirmText: "Desinstalar",
    confirmVariant: "danger",
    onConfirm: async () => {
      state.msstore.busy[family] = "uninstalling";
      state.operationAppId = null;
      renderContent();
      showBackgroundOperationModal(
        shape,
        `Desinstalación de ${shape.name}`,
        "Quitando el paquete de Windows…",
      );
      try {
        const outcome = await invoke("msstore_uninstall", { family });
        closeModal();
        delete state.msstore.updates[String(family).toLowerCase()];
        await ensureMsStoreInstalled({ force: true });
        setTransientStatus(`${shape.name} se desinstaló correctamente`, "var(--green)", 5000);
        showAlertModal("Desinstalación completada", outcome);
      } catch (error) {
        closeModal();
        await ensureMsStoreInstalled({ force: true });
        setTransientStatus(`No se pudo desinstalar ${shape.name}`, "var(--red)", 5000);
        showAlertModal("Error al desinstalar", String(error));
      } finally {
        delete state.msstore.busy[family];
        renderSidebar();
        renderContent();
      }
    },
  });
}

/// Which list was on screen the last time it was drawn, so that redrawing the
/// same one can put the scroll back where it was.
let lastRenderedView = null;

/// The section (and console filter) the content was last drawn for: a redraw of
/// that same section knows what was already on screen.
let lastEntranceScope = null;

// Lo que entra con `fadeUp` al aparecer en la lista.
const ENTERING_BLOCKS = ".section, .hero, .empty-updates-card";

function enteringBlockKey(block) {
  const classes = [...block.classList].filter((name) => name !== "is-settled").join(".");
  return `${classes}|${block.querySelector(".section-head h3")?.textContent.trim() || ""}`;
}

function enteringCardKey(card) {
  const { appId, msId, msFamily, repoCard } = card.dataset;
  return appId || msId || msFamily || repoCard || "";
}

function paintedIconKey(img) {
  const { iconCandidates, iconSignature, catalogGeneration } = img.dataset;
  return `${iconCandidates}|${iconSignature}|${catalogGeneration}|${img.alt}|${img.getAttribute("style")}`;
}

/// Lo que hay a la vista antes de redibujar: cada bloque con sus tarjetas, lo
/// que lleva ya de su entrada si aún no ha terminado (null si ya terminó) y los
/// iconos ya pintados.
function contentOnScreen(content) {
  const entering = new Map();
  for (const animation of content.getAnimations({ subtree: true })) {
    if (animation.animationName === "fadeUp" && animation.playState === "running") {
      entering.set(animation.effect?.target, Number(animation.currentTime) || 0);
    }
  }
  const blocks = new Map();
  for (const block of content.querySelectorAll(ENTERING_BLOCKS)) {
    const cards = new Map();
    for (const card of block.querySelectorAll(".app-card")) {
      cards.set(enteringCardKey(card), entering.get(card) ?? null);
    }
    blocks.set(enteringBlockKey(block), { elapsed: entering.get(block) ?? null, cards });
  }
  const icons = new Map();
  for (const img of content.querySelectorAll("img.is-loaded[data-icon-candidates]")) {
    const key = paintedIconKey(img);
    if (!icons.has(key)) icons.set(key, []);
    icons.get(key).push(img);
  }
  return { blocks, icons };
}

/// Un redibujado de la misma sección no vuelve a hacer entrar lo que ya estaba.
///
/// El contenido se rehace entero con cada cambio, y con él la entrada de cada
/// bloque y el fundido de cada icono: al terminar las comprobaciones del
/// arranque, o con cada repositorio que contestaba a una búsqueda, la página
/// entera volvía a subir y parpadeaba. Un bloque que ya estaba se queda quieto,
/// sus iconos siguen pintados (el mismo nodo, sin volver a cargarse) y solo
/// entran las tarjetas que no estaban. Lo que seguía entrando cuando llegó el
/// redibujado sigue desde donde iba, en vez de saltar al final. Al cambiar de
/// sección todo entra como siempre.
function replaceContent(content, html, scrollTop) {
  const scope = `${state.section}|${state.consoleFilter}`;
  const before = scope === lastEntranceScope ? contentOnScreen(content) : null;
  lastEntranceScope = scope;
  content.innerHTML = html;
  if (before) {
    const resume = (element, elapsed) => {
      element.style.animationDelay = `-${Math.round(elapsed)}ms`;
    };
    for (const block of content.querySelectorAll(ENTERING_BLOCKS)) {
      const shown = before.blocks.get(enteringBlockKey(block));
      if (!shown) continue;
      if (shown.elapsed === null) block.classList.add("is-settled");
      else resume(block, shown.elapsed);
      for (const card of block.querySelectorAll(".app-card")) {
        const key = enteringCardKey(card);
        if (shown.cards.get(key) === null) continue;
        card.classList.add("is-entering");
        if (shown.cards.has(key)) resume(card, shown.cards.get(key));
      }
    }
    for (const img of content.querySelectorAll("img[data-icon-candidates]")) {
      const painted = before.icons.get(paintedIconKey(img))?.pop();
      if (!painted) continue;
      img.replaceWith(painted);
      painted.parentElement?.classList.add("has-loaded-icon");
    }
  }
  content.scrollTop = scrollTop;
  skipHiddenEntrances(content);
}

/// Lo que entra fuera de la vista no se anima.
///
/// Nadie lo ve, y cada bloque o tarjeta que se anima es una capa que el
/// navegador pinta al empezar y retira al terminar. Inicio entero son once
/// bloques y casi trescientas tarjetas, casi todo por debajo del borde: animarlo
/// todo a la vez era lo que hacía tosco el final de la entrada al volver a él.
/// Lo que ya iba entrando (con su retraso negativo) sigue a lo suyo.
function skipHiddenEntrances(content) {
  const candidates = [
    ...[...content.querySelectorAll(ENTERING_BLOCKS)].filter(
      (block) => !block.classList.contains("is-settled"),
    ),
    ...content.querySelectorAll(".app-card.is-entering"),
  ].filter((element) => !element.style.animationDelay);
  if (!candidates.length) return;
  // Todo se mide antes de tocar nada: una sola maquetación.
  const view = content.getBoundingClientRect();
  const hidden = candidates.filter((element) => {
    const box = element.getBoundingClientRect();
    return box.bottom <= view.top || box.top >= view.bottom;
  });
  for (const element of hidden) {
    if (element.classList.contains("app-card")) element.classList.remove("is-entering");
    else element.classList.add("is-settled");
  }
}

function renderContent() {
  if (contentRenderFrame !== null) return;
  contentRenderFrame = requestAnimationFrame(() => {
    contentRenderFrame = null;
    renderContentNow();
  });
}

function renderContentNow() {
  // La Microsoft Store no se dibuja a partir del catálogo: no hay lista que
  // filtrar ni recuento de aplicaciones que dar.
  if (state.section === MSSTORE_SECTION) {
    renderMsStoreNow();
    // Lo elegido en el catálogo sigue elegido aquí: la cola no es de una sección.
    renderQueueDock();
    return;
  }

  const apps = filteredApps();
  const searching = !!state.search.trim();
  const label = NAV.find((n) => n.id === state.section)?.label || "Inicio";
  // Las aplicaciones de la Microsoft Store cuentan en las dos secciones donde
  // aparecen: dejarlas fuera de la cifra diría que hay menos de las que se ven.
  const searchQuery = state.search.trim().toLocaleLowerCase("es-ES");
  const storeApps =
    state.section === "installed"
      ? state.msstore.installed.filter((app) => msStoreMatchesSearch(app, searchQuery))
      : state.section === "updates"
        ? msStoreUpdatesList().filter((app) => msStoreMatchesSearch(app, searchQuery))
        : [];
  const total = apps.length + storeApps.length;
  let html = `
    <div class="page-title">
      <h1>${escapeHtml(label)}</h1>
      <span>${total} ${total === 1 ? "aplicación" : "aplicaciones"}</span>
    </div>`;

  // With pending updates on screen the empty-state card is not rendered, so the
  // rescan action needs its own place to live.
  if (state.section === "updates" && total) {
    html += `
      <div class="updates-toolbar">
        <span class="updates-toolbar-note">${escapeHtml(lastScanLabel())}</span>
        <div class="updates-toolbar-actions">
          ${updateAllButtonHtml()}
          ${scanUpdatesButtonHtml()}
        </div>
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

  // Mientras el primer escaneo no ha contestado, «Aplicaciones instaladas» puede estar
  // a medias: lo que falta se anuncia con tarjetas fantasma en vez de dar la
  // lista por terminada o decir que no hay nada.
  const checkingInstalled = state.section === "installed" && hasPendingStatuses();
  if (checkingInstalled) html += skeletonSectionHtml("Buscando en el equipo…", apps.length ? 3 : 6);

  // Lo que vino de la Microsoft Store vive en el mismo sitio que el resto: es
  // software instalado en el equipo y no tiene por qué estar en otra pantalla.
  // Va en su propio bloque porque de él se saben cosas distintas.
  const storeSection =
    state.section === "installed"
      ? msStoreInstalledSectionHtml(searchQuery)
      : state.section === "updates"
        ? msStoreUpdatesSectionHtml(searchQuery)
        : "";
  html += storeSection;

  // Y por último, cuando se está buscando, lo que la Microsoft Store ofrece
  // para lo mismo. Debajo del catálogo propio porque es un añadido a la
  // búsqueda, no la búsqueda.
  const inlineStoreSection = searching ? msStoreInlineSectionHtml() : "";
  // Y debajo de todo, los dos repositorios de paquetes. En este orden a
  // propósito: el catálogo primero porque está revisado, la Microsoft Store
  // después porque la firma Microsoft, y los repositorios al final porque son
  // los más grandes y los menos seleccionados de los cuatro.
  const inlineRepoSections = searching
    ? Object.keys(REPOS)
        .map((repoId) => repoInlineSectionHtml(repoId))
        .join("")
    : "";

  if (!apps.length && !storeSection && !checkingInstalled) {
    if (state.section === "updates") {
      // Con actualizaciones pendientes y un texto arriba que no casa con
      // ninguna, lo que falta son coincidencias, no actualizaciones: decir
      // «todo al día» sería mentir.
      html +=
        searching && updatesCount()
          ? updatesSearchMissCardHtml(state.search.trim())
          : updatesUpToDateCardHtml();
    } else if (inlineStoreSection || inlineRepoSections) {
      // No decir «sin resultados» a secas teniendo justo debajo bloques llenos
      // de ellos: lo que no tiene nada es este catálogo, y eso es lo que se
      // dice.
      html += `
        <div class="empty">
          <h3>Nada en el catálogo de WinSlimCenter</h3>
          <p>Ninguna de las aplicaciones seleccionadas coincide con «${escapeHtml(state.search.trim())}». Abajo, lo que ofrecen la Microsoft Store, WinGet y Chocolatey.</p>
        </div>`;
    } else {
      html += `<div class="empty"><h3>Sin resultados</h3><p>Prueba con otro buscador o sección.</p></div>`;
    }
  }

  html += inlineStoreSection;
  html += inlineRepoSections;

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

  // `scrollTop` is clamped by the browser, so scrollHeight and clientHeight are
  // never read. The one layout taken right after replacing the DOM is the one
  // that decides which entrances are on screen (skipHiddenEntrances), and it
  // only happens when something is about to enter.
  replaceContent(content, html, restoreTo);
  renderQueueDock();
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
    // Lo que acaba de ponerse en marcha deja de poder elegirse, y al revés.
    if (app) syncCardSelection(card, app);
  });
  if (state.selection.size) renderQueueDock();
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
    syncCardSelection(card, app);
  });
  if (state.selection.size) renderQueueDock();
}

const APP_ACTION_KINDS = ["install", "update", "uninstall", "launch"];

function runAppAction(kind, id, options = {}) {
  if (kind === "install") return installApp(id, false, options);
  if (kind === "update") return installApp(id, true);
  if (kind === "uninstall") return uninstallApp(id);
  return launchApp(id);
}

/**
 * Lo que se pulsa mientras el primer escaneo del equipo sigue en marcha espera
 * a su respuesta. La tarjeta enseña lo que sabía la sesión anterior, y un
 * «Instalar» sobre algo que ya está puesto —o un «Abrir» sobre algo que ya no
 * está— no debe ejecutarse a ciegas. Si después la acción sigue teniendo
 * sentido, se hace; si no, la tarjeta ya enseña la buena.
 */
async function afterStatusesVerified(kind, id, button = null, options = {}) {
  const verification = statusVerification;
  if (!verification) return runAppAction(kind, id, options);
  if (button) {
    button.disabled = true;
    button.setAttribute("aria-busy", "true");
    button.textContent = "Comprobando…";
  }
  await verification.catch(() => {});
  const app = findApp(id);
  const stillOffered = !!app && actionButtons(app).includes(`data-${kind}="${escapeHtml(id)}"`);
  updateVisibleAppActions(new Set([id]));
  // Las tarjetas de WinGet y Chocolatey no se redibujan por identificador.
  if (button?.isConnected && !button.closest(".app-card[data-app-id]")) renderContent();
  if (stillOffered) return runAppAction(kind, id, options);
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
        scheduleInlineSearches();
        renderSidebar();
        renderContent();
        // Los canales se piden al entrar y se dibujan cuando llegan: la
        // sección ya está en pantalla mientras tanto.
        if (nextSection === MSSTORE_SECTION) {
          void Promise.all([ensureMsStoreOptions(), ensureMsStoreInstalled()]).then(renderContent);
        }
        // Las dos secciones donde también aparece lo instalado desde la tienda.
        if (nextSection === "installed" || nextSection === "updates") {
          void ensureMsStoreInstalled().then(() => {
            renderSidebar();
            renderContent();
          });
        }
      }
      return;
    }

    const actionButton = event.target.closest("[data-action]");
    if (actionButton?.dataset.action === "folder") {
      void clientLog("info", "action", "Abriendo carpeta de aplicaciones.");
      invoke("open_apps_dir").catch((error) => {
        setStatus(`No se pudo abrir la carpeta: ${error}`, "var(--red)");
        showAlertModal("Error al abrir la carpeta", String(error));
      });
    }
  });

  content.addEventListener("submit", (event) => {
    if (!event.target.closest("#ms-search-form")) return;
    event.preventDefault();
    void runMsStoreSearch();
  });

  content.addEventListener("input", (event) => {
    if (event.target.id !== "ms-search") return;
    // Sólo se anota: la lista no se filtra mientras se escribe porque no está
    // aquí, y redibujarla robaría el cursor a cada tecla.
    state.msstore.query = event.target.value;
  });

  content.addEventListener("click", (event) => {
    const seeAll = event.target.closest("[data-ms-see-all]");
    if (seeAll) {
      state.msstore.query = seeAll.dataset.msSeeAll;
      state.section = MSSTORE_SECTION;
      // La barra de arriba se vacía al entrar: dentro de la sección manda su
      // propio buscador, y dejar texto arriba haría que la siguiente tecla
      // devolviera al catálogo a mitad de consulta.
      state.search = "";
      const bar = document.getElementById("search");
      if (bar) bar.value = "";
      scheduleInlineSearches();
      void clientLog("info", "msstore", `Búsqueda llevada a la sección: ${state.msstore.query}`);
      renderSidebar();
      renderContent();
      void Promise.all([ensureMsStoreOptions(), ensureMsStoreInstalled()]).then(() =>
        runMsStoreSearch(),
      );
      return;
    }

    const ringButton = event.target.closest("[data-ms-ring]");
    if (ringButton) {
      const ring = ringButton.dataset.msRing;
      if (state.msstore.ring !== ring) {
        state.msstore.ring = ring;
        void clientLog("info", "msstore", `Canal seleccionado: ${ring}`);
        renderContent();
      }
      return;
    }

    const archButton = event.target.closest("[data-ms-arch]");
    if (archButton) {
      const arch = archButton.dataset.msArch;
      if (state.msstore.arch !== arch) {
        state.msstore.arch = arch;
        void clientLog("info", "msstore", `Arquitectura seleccionada: ${arch}`);
        renderContent();
      }
      return;
    }

    if (event.target.closest("#btn-ms-scan")) {
      void scanMsStoreUpdates();
      return;
    }

    const msInstall = event.target.closest("[data-ms-install]");
    if (msInstall) {
      void installMsStoreProduct(msInstall.dataset.msInstall);
      return;
    }

    const repoInstall = event.target.closest("[data-repo-install]");
    if (repoInstall) {
      void installRepoPackage(repoInstall.dataset.repoInstall, repoInstall.dataset.repoPackage);
      return;
    }

    const repoPage = event.target.closest("[data-repo-page]");
    if (repoPage) {
      void invoke("open_url", { url: repoPage.dataset.repoPage });
      return;
    }

    const msUpdate = event.target.closest("[data-ms-update]");
    if (msUpdate) {
      updateMsStoreApp(msUpdate.dataset.msUpdate, msUpdate.dataset.msFamily);
      return;
    }

    const msLaunch = event.target.closest("[data-ms-launch]");
    if (msLaunch) {
      void launchMsStoreApp(msLaunch.dataset.msLaunch);
      return;
    }

    const msUninstall = event.target.closest("[data-ms-uninstall]");
    if (msUninstall) {
      void uninstallMsStoreApp(msUninstall.dataset.msUninstall);
      return;
    }

    const msCard = event.target.closest(".ms-card[data-ms-id]");
    if (msCard) {
      if (!event.target.closest("button")) void openMsStoreModal(msCard.dataset.msId);
      return;
    }

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
    if (event.target.closest("#btn-update-all")) {
      void updateEverything();
      return;
    }
    if (event.target.closest("#btn-clear-search")) {
      // Se vacía también la caja de arriba, que es la que manda: dejar el
      // texto ahí haría que la siguiente tecla volviera a filtrar por él.
      state.search = "";
      const bar = document.getElementById("search");
      if (bar) bar.value = "";
      scheduleInlineSearches();
      renderContent();
      return;
    }

    const selectToggle = event.target.closest("[data-select]");
    if (selectToggle) {
      toggleSelection(selectToggle.dataset.select);
      return;
    }

    const action = event.target.closest("[data-install], [data-update], [data-uninstall], [data-launch]");
    if (action) {
      const kind = APP_ACTION_KINDS.find((name) => action.dataset[name]);
      // Mayús al instalar vuelve a preguntar con qué navegador se abren las
      // aplicaciones web de la tienda.
      if (kind) void afterStatusesVerified(kind, action.dataset[kind], action, { chooseBrowser: event.shiftKey });
      return;
    }

    const card = event.target.closest(".app-card[data-app-id]");
    if (card && !event.target.closest("button")) {
      // Ctrl+clic marca sin abrir la ficha: el gesto de siempre para elegir
      // varias cosas de una lista sin perderla de vista.
      if ((event.ctrlKey || event.metaKey) && card.hasAttribute("data-selectable")) {
        toggleSelection(card.dataset.appId);
      } else {
        openAppModal(card.dataset.appId);
      }
    }
  });

  content.addEventListener("keydown", (event) => {
    if (!["Enter", " "].includes(event.key) || event.target.closest("button")) return;
    const msCard = event.target.closest(".ms-card[data-ms-id]");
    if (msCard) {
      event.preventDefault();
      void openMsStoreModal(msCard.dataset.msId);
      return;
    }
    const card = event.target.closest(".app-card[data-app-id]");
    if (!card) return;
    event.preventDefault();
    openAppModal(card.dataset.appId);
  });
}

/**
 * La descripción de una ficha, como tarjeta con su etiqueta.
 *
 * El texto iba suelto en un `confirm-dialog-msg`, que conserva los saltos y
 * espacios tal cual para los avisos que los necesitan. Escrito en su propia
 * línea dentro de la plantilla, se llevaba también la sangría del código: un
 * hueco encima, la primera línea desplazada y otro hueco debajo. Aquí el texto
 * entra recortado y sólo cuentan los saltos que trae de verdad —las fichas de
 * la Microsoft Store vienen en párrafos y con listas de una línea por punto—.
 */
function appDescriptionHtml(text) {
  const paragraphs = String(text || "")
    .replace(/\r\n?/g, "\n")
    .split(/\n\s*\n/)
    .map((paragraph) => paragraph.trim())
    .filter(Boolean);
  if (!paragraphs.length) return "";
  return `
    <section class="app-description">
      <h3 class="app-description-label">Descripción</h3>
      <div class="app-description-text">
        ${paragraphs.map((paragraph) => `<p>${escapeHtml(paragraph)}</p>`).join("")}
      </div>
    </section>`;
}

// La línea bajo el nombre de una ficha: autor, versión y categoría, separados
// por puntos que pone la hoja de estilos para que no viajen con el texto.
function appDetailMetaHtml(parts) {
  return `<p class="app-detail-meta">${parts
    .filter(Boolean)
    .map((part) => `<span>${escapeHtml(part)}</span>`)
    .join("")}</p>`;
}

// Cómo se enseña una versión, en cualquier sitio de la tienda. «latest» es lo
// que dice el catálogo cuando la versión la decide el origen al instalar: para
// quien lee, eso es la última versión, no «vlatest». Un número lleva su «v» una
// sola vez —WinGet y GitHub a veces la traen puesta— y una etiqueta como
// «nightly» o «LTS» se enseña tal cual. `unknown` es lo que se dice cuando no
// hay número: «Última versión» para lo que se va a instalar, y otra cosa en una
// comparación, donde «Última versión → v2.0» no tendría sentido.
function versionLabel(version, unknown = "Última versión") {
  const text = String(version ?? "").trim();
  if (!text || /^(latest|lastest|unknown|desconocida)$/i.test(text)) return unknown;
  if (/^v\d/i.test(text)) return `v${text.slice(1)}`;
  if (/^\d/.test(text)) return `v${text}`;
  return text[0].toUpperCase() + text.slice(1);
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
       ${st.update_available || st.pending_restart ? `<div>${updateVersionBadge(st, true)}</div>` : ""}`
    : "";

  let actionBtnsHtml = "";
  if (isStatusPending(id)) {
    // Se vuelve a dibujar en cuanto el equipo conteste.
    actionBtnsHtml = `<button type="button" class="btn primary" disabled aria-busy="true">Comprobando…</button>`;
  } else if (!st.installed) {
    // Quien está leyendo la ficha puede querer dejarla para la cola en vez de
    // instalarla ahora mismo.
    const queued = state.selection.has(id);
    const queueBtn = isQueueable(app)
      ? `<button type="button" class="btn ghost" id="modal-app-queue">${queued ? "Quitar de la cola" : "Añadir a la cola"}</button>`
      : "";
    actionBtnsHtml = `${queueBtn}<button type="button" class="btn primary" id="modal-app-install">${app.source_type === "web" ? "Obtener" : "Instalar"}</button>`;
  } else {
    actionBtnsHtml = `
      ${st.update_available
        ? `<button type="button" class="btn secondary" id="modal-app-update">Actualizar</button>`
        : st.can_launch ? `<button type="button" class="btn primary" id="modal-app-launch">Abrir</button>` : ""}
      <button type="button" class="btn danger" id="modal-app-uninstall">Desinstalar</button>
    `;
  }

  openModal(`
    <div class="confirm-dialog app-detail-modal" data-app-id="${escapeHtml(id)}">
      <div class="confirm-dialog-header app-detail-header">
        <div class="card-avatar app-detail-avatar" style="background:${avatarBg};">
          ${avatar}
        </div>
        <div class="app-detail-heading">
          <h2 class="confirm-dialog-title app-detail-title">${escapeHtml(app.name)}</h2>
          ${appDetailMetaHtml([`Por ${app.author || "—"}`, versionLabel(version), app.category])}
          ${installedBanner ? `<div>${installedBanner}</div>` : ""}
        </div>
      </div>
      ${appDescriptionHtml(app.description)}
      <div class="modal-foot">
        <button type="button" class="btn ghost" id="modal-app-close">Cerrar</button>
        ${actionBtnsHtml}
      </div>
    </div>
  `);

  document.getElementById("modal-app-close").onclick = closeModal;

  const btnInst = document.getElementById("modal-app-install");
  if (btnInst) {
    btnInst.onclick = (event) => {
      closeModal();
      void afterStatusesVerified("install", id, null, { chooseBrowser: event.shiftKey });
    };
  }

  const btnQueue = document.getElementById("modal-app-queue");
  if (btnQueue) btnQueue.onclick = () => { closeModal(); toggleSelection(id); };

  const btnLaunch = document.getElementById("modal-app-launch");
  if (btnLaunch) btnLaunch.onclick = () => { closeModal(); void afterStatusesVerified("launch", id); };

  const btnUpdate = document.getElementById("modal-app-update");
  if (btnUpdate) btnUpdate.onclick = () => { closeModal(); void afterStatusesVerified("update", id); };

  const btnUninstall = document.getElementById("modal-app-uninstall");
  if (btnUninstall) btnUninstall.onclick = () => { closeModal(); void afterStatusesVerified("uninstall", id); };
}

/// La ficha abierta que esperaba al escaneo, para volver a dibujarla con la
/// respuesta en cuanto llegue.
function appModalAwaitingStatus() {
  const open = document.querySelector("#modal .app-detail-modal[data-app-id]");
  return open && isStatusPending(open.dataset.appId) ? open.dataset.appId : null;
}

/// La aplicación detrás de un identificador.
///
/// Primero el catálogo, que es de quien habla casi todo. Después los paquetes
/// de WinGet y de Chocolatey que se están instalando ahora mismo: llegaron de
/// una búsqueda y no del catálogo, pero su instalación pasa por el mismo
/// diálogo y el mismo panel, y ésos preguntan por aquí.
function findApp(id) {
  const known = derived.catalogById.get(id) || (state.repoEntries || []).find((app) => app.id === id) || derived.repoPackages.get(id);
  if (known) return known;
  const found = repoPackageByAppId(id);
  return found ? repoCatalogEntry(found.repoId, found.pkg) : undefined;
}

const portableBackupChoices = new Map();

function portableDataChoices(info, uninstall = false) {
  return {
    label: "Datos de la aplicación portable",
    selected: "preserve",
    hint: "Esta operación puede borrar configuración, perfiles o partidas guardados en la carpeta del programa.",
    options: [
      { id: "preserve", name: "Conservar mis datos (recomendado)", description: uninstall
        ? "Guarda una copia completa antes de quitar la aplicación."
        : "Guarda una copia completa y recupera los datos identificables en la nueva versión. Si algún ajuste no se recupera automáticamente, seguirá en la copia." },
      { id: "delete", name: "Eliminar los datos de esta instalación", description: "Quita también sus datos locales. Las copias de seguridad anteriores se mantienen." },
    ],
  };
}

function portableDataMessage(info) {
  return info ? `\n\nEs una aplicación portable. Elige qué hacer con sus datos. Las copias se guardan en: ${info.backup_root}` : "";
}

/**
 * El navegador con el que la tienda abre sus aplicaciones web.
 *
 * Es una elección de la tienda y sólo de ella: el predeterminado de Windows no
 * se toca, y en un Windows aligerado puede incluso nombrar un Edge que ya no
 * existe. Se pregunta la primera vez y se recuerda; con Mayús pulsada al
 * instalar se vuelve a preguntar.
 */
async function webBrowserSetup() {
  try {
    return (await invoke("web_browsers")) || { browsers: [], selected: null };
  } catch (error) {
    void clientLog("warn", "web-browsers", String(error?.stack || error));
    return { browsers: [], selected: null };
  }
}

function webBrowserChoices({ browsers, selected }) {
  return {
    label: "Navegador de las aplicaciones web",
    hint:
      "Solo para WinSlimCenter: el navegador predeterminado de Windows no cambia. " +
      "Para elegir otro más adelante, mantén pulsada Mayús al pulsar «Instalar».",
    selected: selected || browsers.find((browser) => browser.windows_default)?.path || browsers[0]?.path,
    options: browsers.map((browser) => ({
      id: browser.path,
      name: browser.name,
      description: [
        browser.app_mode
          ? "Se abre en una ventana propia, como una aplicación"
          : "Se abre como una pestaña normal",
        browser.windows_default ? "predeterminado de Windows" : "",
        browser.path === selected ? "el que usa ahora la tienda" : "",
      ].filter(Boolean).join(" · "),
    })),
  };
}

function showNoBrowserModal() {
  showConfirmModal({
    title: "No hay ningún navegador",
    message:
      "Las aplicaciones web se abren en un navegador y no se ha encontrado ninguno instalado " +
      "en el equipo. Instala uno desde la categoría Navegadores y vuelve a intentarlo.",
    confirmText: "Ver navegadores",
    onConfirm: () => {
      state.section = "browsers";
      state.consoleFilter = "all";
      renderSidebar();
      renderContent();
    },
  });
}

/// Guarda la elección. Devuelve si se pudo, y si no lo dice.
async function rememberWebBrowser(path) {
  try {
    await invoke("set_web_browser", { path });
    return true;
  } catch (error) {
    showAlertModal("No se pudo usar ese navegador", String(error));
    return false;
  }
}

/**
 * Para lo que no pasa por el diálogo de «Instalar» —la cola y «Abrir»—: si la
 * tienda aún no tiene navegador, lo pregunta en un diálogo propio. Contesta si
 * se puede seguir.
 */
async function ensureWebBrowser() {
  const setup = await webBrowserSetup();
  if (setup.selected) return true;
  if (!setup.browsers.length) {
    showNoBrowserModal();
    return false;
  }
  return new Promise((resolve) => {
    showConfirmModal({
      title: "Navegador para las aplicaciones web",
      message:
        "Las aplicaciones web se instalan como un acceso directo que abre el sitio con el " +
        "navegador que elijas.",
      choices: webBrowserChoices(setup),
      confirmText: "Usar este navegador",
      onCancel: () => resolve(false),
      onConfirm: async (picked) => resolve(await rememberWebBrowser(picked)),
    });
  });
}

async function installApp(id, isUpdate = false, { chooseBrowser = false } = {}) {
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

  // Una aplicación web se abre con el navegador que eligió el usuario para la
  // tienda. Si aún no hay ninguno elegido —o se pidió cambiarlo con Mayús—, la
  // pregunta va en el mismo diálogo de instalar, en lugar de fallar después.
  let browserSetup = null;
  if (app.source_type === "webapp" && !isUpdate) {
    browserSetup = await webBrowserSetup();
    if (!browserSetup.browsers.length) {
      showNoBrowserModal();
      return;
    }
    if (browserSetup.selected && !chooseBrowser) browserSetup = null;
  }

  // Asked even when the app does not look installed: the backend refuses to
  // replace a non-empty folder without this choice, whatever the status says.
  let portableInfo = null;
  try {
    portableInfo = await invoke("portable_data_info", { appId: id, forUpdate: true });
  } catch (error) {
    showAlertModal("No se pudieron comprobar los datos", String(error));
    return;
  }
  const title = isUpdate ? `Actualizar ${app.name}` : `Instalar ${app.name}`;

  // An application published in several builds asks which one on the way in.
  // An update never asks: it reinstalls the build already chosen, because
  // changing it is changing the program.
  const remembered = state.settings?.variants?.[id];
  const choices = browserSetup ? webBrowserChoices(browserSetup) :
    portableInfo ? portableDataChoices(portableInfo) :
    app.variants && !isUpdate
      ? { ...app.variants, selected: remembered || app.variants.default }
      : null;

  // Everything the two dialogs below end up doing. Which one asked, and what
  // the user answered, only decides what arrives here.
  const start = async ({ variant = null, closeRunning = false, portableData = null } = {}) => {
    portableBackupChoices.delete(id);
    if (portableData === "preserve") portableBackupChoices.set(id, portableInfo);
    state.busy[id] = isUpdate ? "updating" : "installing";
    updateVisibleAppActions(new Set([id]));
    state.finished.delete(id);
    renderDlPanel();
    showPackageOperationModal(app, isUpdate);
    try {
      await invoke("install_app", {
        appEntry: app,
        forceUpdate: !!isUpdate || !!st.update_available,
        variant: variant || remembered || app.variants?.default || null,
        closeRunning,
        portableData,
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
  };

  // An application that is open is asked about before anything is downloaded:
  // Windows will not swap a package in use, and an ordinary installer cannot
  // overwrite files the running copy holds open. Discovering that afterwards
  // meant spending the download to report a success that never happened.
  //
  // Preguntárselo a Windows cuesta arrancar un PowerShell y pasa del segundo,
  // y esperando esa respuesta antes de dibujar nada el botón se quedaba mudo
  // el rato justo para parecer que no había hecho caso. La pregunta se lanza
  // aquí y el diálogo sale ya: la respuesta no hace falta hasta que el usuario
  // conteste que sí, y para entonces lleva rato resuelta.
  const blockerProbe =
    isUpdate || st.installed ? blockingRunningApp(id) : Promise.resolve(null);

  // Una actualización publicada en GitHub enseña lo que trae: las notas de la
  // release que se va a instalar, no las de la última que haya.
  const notesRepo =
    isUpdate && app.source_type === "github_release" && app.github_repo ? app.github_repo : null;
  const target = isUpdate ? String(st.latest_version || "").trim() : "";

  showConfirmModal({
    title,
    message: (isUpdate
      ? `¿Deseas actualizar '${app.name}' a la versión más reciente?`
      : `¿Deseas instalar '${app.name}' en tu equipo?`) + portableDataMessage(portableInfo),
    app,
    subtitle:
      target && st.version
        ? `${app.author ? `${app.author} · ` : ""}${versionLabel(st.version, "versión desconocida")} → ${versionLabel(target, "la más reciente")}`
        : null,
    details: notesRepo ? releaseNotesShell() : "",
    confirmText: isUpdate ? "Actualizar" : "Instalar",
    confirmVariant: "primary",
    choices,
    onConfirm: async (picked) => {
      if (browserSetup && !(await rememberWebBrowser(picked))) return;
      const selection = browserSetup ? {} : portableInfo ? { portableData: picked } : { variant: picked };
      const blocker = await blockerProbe;
      if (blocker) {
        showRunningAppModal({
          app,
          blocker,
          isUpdate,
          onClose: () => start({ ...selection, closeRunning: true }),
          onAnyway: blocker.packaged ? () => start(selection) : null,
        });
        return;
      }
      await start(selection);
    },
  });

  if (notesRepo) {
    const section = document.querySelector("#modal .release-notes");
    const page = `https://github.com/${notesRepo}/releases${target ? `/tag/${encodeURIComponent(target)}` : ""}`;
    loadReleaseNotes(notesRepo, target)
      .then((notes) => fillReleaseNotes(section, notes, page))
      .catch((error) => {
        void clientLog("warn", "release-notes", `${notesRepo}@${target}: ${error}`);
        fillReleaseNotes(section, null, `https://github.com/${notesRepo}/releases`);
      });
  }
}

/**
 * The application is open and the operation cannot go ahead over it.
 *
 * Two answers, and no hidden third: close it now — which means killing what it
 * has running, said plainly because it costs the user whatever was on screen —
 * or leave it for later. A packaged application gets one more, because Windows
 * really can hold the update and apply it on the next quit; for everything else
 * that button would only promise an installer that is going to fail.
 */
function showRunningAppModal({ app, blocker, isUpdate, onClose, onAnyway = null }) {
  const verb = isUpdate ? "actualización" : "instalación";
  const count =
    blocker.processes > 1
      ? ` Ahora mismo tiene ${blocker.processes} procesos en marcha.`
      : "";
  showConfirmModal({
    title: `${blocker.name} está en ejecución`,
    message:
      `No se puede aplicar la ${verb} mientras ${blocker.name} esté abierta.${count}` +
      ` Cierra la aplicación —guarda antes lo que tengas sin guardar— y continúa,` +
      ` o deja la ${verb} para más tarde.` +
      (onAnyway
        ? ` También puedes instalarla igualmente: Windows la guardará y la aplicará` +
          ` la próxima vez que cierres la aplicación.`
        : ""),
    app,
    confirmText: `Cerrar ${blocker.name} y ${isUpdate ? "actualizar" : "instalar"}`,
    confirmVariant: "primary",
    secondary: onAnyway ? "Instalar igualmente" : null,
    cancelText: "Más tarde",
    onCancel: () => {
      setTransientStatus(
        `${blocker.name} sigue abierta: la ${verb} queda pendiente.`,
        "var(--text-medium)",
        6000,
      );
    },
    onConfirm: async (_picked, choice) => {
      if (choice === "secondary" && onAnyway) {
        await onAnyway();
        return;
      }
      setStatus(`Cerrando ${blocker.name}…`, "var(--accent)");
      await onClose();
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

  // Windows defers the removal of a package in use exactly as it defers an
  // update, so the same question is worth asking here.
  let portableInfo;
  let uninstallBlocker;
  try {
    [uninstallBlocker, portableInfo] = await Promise.all([
      blockingRunningApp(id), invoke("portable_data_info", { appId: id }),
    ]);
  } catch (error) {
    showAlertModal("No se pudieron comprobar los datos", String(error));
    return;
  }
  showConfirmModal({
    title: `Desinstalar ${app.name}`,
    message: (uninstallBlocker
      ? `${uninstallBlocker.name} está en ejecución y se cerrará antes de quitarla: ` +
        (uninstallBlocker.packaged
          ? "mientras siga en uso, Windows deja la retirada del paquete pendiente y no llega a completarse."
          : "mientras siga abierta no se pueden borrar sus archivos y la desinstalación falla a medias.")
      : `¿Estás seguro de que deseas desinstalar '${app.name}' de tu equipo?`) + portableDataMessage(portableInfo),
    choices: portableInfo ? portableDataChoices(portableInfo, true) : null,
    app,
    confirmText: uninstallBlocker ? `Cerrar ${uninstallBlocker.name} y desinstalar` : "Desinstalar",
    confirmVariant: "danger",
    onConfirm: async (picked) => {
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
        const outcome = await invoke("uninstall_app", {
          appId: id,
          closeRunning: !!uninstallBlocker,
          portableData: portableInfo ? picked : null,
        });
        closeModal();
        const changes = await refreshInstalledFromBootstrap();
        reconcileStatusChanges(changes);
        setTransientStatus(`${app.name} se desinstaló correctamente`, "var(--green)", 5000);
        const message = outcome || `${app.name} se desinstaló correctamente del equipo.`;
        if (portableInfo && picked === "preserve") {
          showConfirmModal({
            title: "Desinstalación completada",
            message: `${message}\n\nTus datos se conservan en una copia completa en ${portableInfo.backup_root}.`,
            confirmText: "Abrir copias", cancelText: "Cerrar", onConfirm: openPortableBackups,
          });
        } else showAlertModal("Desinstalación completada", message);
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
  // Una aplicación web se abre con el navegador de la tienda; si nunca se
  // eligió, se pregunta ahora en vez de fallar.
  if (app?.source_type === "webapp" && !(await ensureWebBrowser())) return;
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
      <p class="confirm-dialog-msg" style="margin-top: 10px; color: var(--text-medium);">WinSlimCenter puede volver a abrir ${escapeHtml(appName)} mediante el diálogo seguro de UAC de Windows.</p>
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
    updateBulkOperation(state.tasks);
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
  // El visor de capturas se apoya en la ficha que hay debajo. Si esa ficha se
  // va —la cierra el usuario, o la sustituye un aviso del backend—, el visor no
  // puede quedarse flotando sobre una pantalla que ya no es la suya.
  closeShotViewer();
  document.getElementById("modal-backdrop").classList.add("hidden");
  document.getElementById("modal").innerHTML = "";
  modalLocked = false;
  if (modalReturnFocus instanceof HTMLElement) modalReturnFocus.focus();
  modalReturnFocus = null;
  // El resumen de «Actualizar todo» sólo vive en su diálogo: cerrarlo —por el
  // botón, por la tecla de escape o pinchando fuera— lo da por leído y devuelve
  // el botón de la barra a su estado normal. Mientras la tanda esté en marcha
  // el diálogo está bloqueado y por aquí no se pasa.
  if (state.bulkUpdate?.finished) {
    state.bulkUpdate = null;
    renderContent();
  }
}

function openModal(html, wide = false, locked = false, presentation = "") {
  // Por lo mismo: lo que ampliaba el visor pertenecía al diálogo anterior.
  closeShotViewer();
  const backdrop = document.getElementById("modal-backdrop");
  const modal = document.getElementById("modal");
  if (backdrop.classList.contains("hidden")) modalReturnFocus = document.activeElement;
  modalLocked = locked;
  modal.classList.toggle("wide", wide);
  // El foco fuerza una maquetación: preparar el tamaño y la decoración antes
  // de mostrar evita un fotograma intermedio del diálogo genérico.
  modal.classList.toggle("modal-hero", presentation === "hero" || presentation === "about");
  modal.classList.toggle("modal-about", presentation === "about");
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
  modal.querySelector("button, input, textarea, select, [tabindex]:not([tabindex='-1'])")?.focus({ preventScroll: true });
}

document.addEventListener("keydown", (event) => {
  // El visor de capturas se abre por encima del diálogo del producto. Mientras
  // esté, la tecla de escape es suya: cerrarlo devuelve a la ficha, no la
  // cierra también. Y las flechas pasan de una captura a otra.
  if (shotViewer) {
    if (event.key === "Escape") {
      event.preventDefault();
      closeShotViewer();
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      stepShotViewer(event.key === "ArrowRight" ? 1 : -1);
    }
    return;
  }
  const modalOpen = !document.getElementById("modal-backdrop").classList.contains("hidden");
  if (event.key === "Escape" && !modalLocked && modalOpen) {
    event.preventDefault();
    closeModal();
  } else if (
    event.key === "Escape" && !modalOpen && state.selection.size &&
    !event.target.closest?.("input, textarea")
  ) {
    // Sin diálogo abierto, Escape deshace la selección: es lo que se espera de
    // él delante de una lista con cosas marcadas. En el buscador no, que allí
    // se está escribiendo.
    event.preventDefault();
    clearSelection();
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

async function openPortableBackups() {
  try { await invoke("open_portable_backups"); }
  catch (error) { showAlertModal("No se pudieron abrir las copias", String(error)); }
}

/**
 * Turns the operation dialog into its finished state instead of closing it.
 *
 * The user has been watching a progress dialog: dropping them back into the
 * list the instant it ends leaves them to hunt for the card they were just
 * working with. It stays put with the only two things worth doing next.
 */
function showOperationCompleted(app, { canLaunch, isUpdate, changed, pendingRestart = false, onLaunch = null }) {
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
  // Windows staged the package instead of applying it, because the application
  // was open. Calling that "se actualizó correctamente" is what sent the user
  // back to the store to update something already installed.
  const outcome = pendingRestart
    ? "se descargó e instaló · ciérralo y vuelve a abrirlo para aplicarlo"
    : !changed
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
    ${portableBackupChoices.has(app?.id) ? '<button type="button" class="btn ghost" id="operation-backups">Abrir copia de datos</button>' : ""}
    <button type="button" class="btn ghost" id="operation-close">Cerrar</button>
  `;
  // Una aplicación de la Microsoft Store no se abre como las del catálogo, así
  // que quien la conoce dice cómo hacerlo.
  actions.querySelector("#operation-launch")?.addEventListener("click", () => {
    closeModal();
    if (onLaunch) onLaunch();
    else if (app) launchApp(app.id);
  });
  actions.querySelector("#operation-close")?.addEventListener("click", closeModal);
  actions.querySelector("#operation-backups")?.addEventListener("click", openPortableBackups);
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

// ---------------------------------------------------------------------------
// Actualizar todo
//
// Una sola orden por cada aplicación pendiente, todas lanzadas seguidas y sin
// esperarse entre ellas: el backend ya reparte cuatro descargas a la vez y
// serializa la instalación, así que lo que hay que decidir aquí es el orden en
// que entran en esa cola. Primero el catálogo de la tienda y después la
// Microsoft Store, que es lo propio antes que lo de fuera.
//
// El diálogo no es el de una sola aplicación repetido: aquella tiene una frase
// y una barra porque no hay nada más que contar, y aquí lo que importa es
// cuántas van, cuáles siguen en marcha y cuál se ha torcido. Una fila por
// aplicación, con su propio estado y su propia barra, y arriba el total.
// ---------------------------------------------------------------------------

/// Los estados de los que ya no se sale: la fila no vuelve a moverse.
const BULK_FINAL_STATES = new Set(["done", "current", "error", "cancelled"]);

const BULK_STATE_LABELS = {
  pending: "En espera",
  queued: "En cola",
  downloading: "Descargando",
  paused: "Pausado",
  installing: "Instalando",
  cancelling: "Cancelando",
  done: "Actualizada",
  current: "Ya al día",
  error: "Error",
  cancelled: "Cancelada",
};

/// La etiqueta de un estado según lo que haga la tanda: lo que termina bien al
/// actualizar queda «Actualizada»; en la cola de instalación, «Instalada».
function bulkStateLabel(key) {
  if (state.bulkUpdate?.mode === "install") {
    if (key === "done") return "Instalada";
    if (key === "current") return "Ya instalada";
  }
  return BULK_STATE_LABELS[key] || key;
}

/// El verbo de la tanda en marcha, para los títulos.
function bulkVerb() {
  return state.bulkUpdate?.mode === "install" ? "Instalando" : "Actualizando";
}

function isBulkItemFinal(item) {
  return BULK_FINAL_STATES.has(item?.state);
}

function isBulkUpdateRunning() {
  return !!state.bulkUpdate && !state.bulkUpdate.finished;
}

/**
 * Todo lo que tiene versión nueva, en el orden en que se va a pedir.
 *
 * Lo que ya está en marcha por su cuenta queda fuera: el backend rechazaría la
 * segunda orden por tener la tarea abierta, y contarla aquí sería prometer un
 * trabajo que no se va a hacer.
 */
function pendingUpdateItems() {
  const busyKey = (key) => !!state.busy[key] || derived.taskByAppId.has(key);
  const items = [];
  for (const app of knownApps()) {
    const status = appStatus(app.id);
    if (!status.update_available || busyKey(app.id)) continue;
    items.push({
      key: app.id,
      kind: "catalog",
      appId: app.id,
      app,
      name: app.name,
      from: status.version || "",
      to: status.latest_version || "",
    });
  }
  for (const installed of msStoreUpdatesList()) {
    const report = msStoreUpdateOf(installed.family);
    const productId = String(report?.product_id || "").toUpperCase();
    if (!productId) continue;
    const key = msStoreTaskId(productId);
    if (state.msstore.installing[productId] || busyKey(key)) continue;
    const shape = msStoreInstalledShape(installed);
    items.push({
      key,
      kind: "msstore",
      productId,
      family: installed.family,
      app: shape,
      name: shape.name,
      from: installed.version || "",
      to: report.latest_version || "",
    });
  }
  return items;
}

/**
 * El botón «Actualizar todo».
 *
 * Con una tanda en marcha vuelve a abrir su diálogo en lugar de empezar otra:
 * lo que se quiere ver entonces es cómo va la que hay.
 */
async function updateEverything() {
  if (isBulkUpdateRunning()) {
    showBulkUpdateModal();
    return;
  }
  await untilStatusesVerified();
  const items = pendingUpdateItems();
  if (!items.length) {
    showAlertModal(
      "Nada que actualizar",
      "No queda ninguna aplicación con versión nueva pendiente de instalar.",
    );
    return;
  }

  try {
    await Promise.all(items.filter((item) => item.kind === "catalog").map(async (item) => {
      item.portableInfo = await invoke("portable_data_info", { appId: item.appId, forUpdate: true });
    }));
  } catch (error) {
    showAlertModal("No se pudieron comprobar los datos", String(error));
    return;
  }
  const portables = items.filter((item) => item.portableInfo);
  const fromStore = items.filter((item) => item.kind === "catalog").length;
  const fromMsStore = items.length - fromStore;
  const origins = [
    fromStore ? `${fromStore} del catálogo de WinSlimCenter` : "",
    fromMsStore ? `${fromMsStore} de la Microsoft Store` : "",
  ].filter(Boolean);

  showConfirmModal({
    title: `Actualizar ${items.length} ${items.length === 1 ? "aplicación" : "aplicaciones"}`,
    message:
      `Se actualizarán a la vez ${origins.join(" y ")}. Se empieza por el catálogo de ` +
      `WinSlimCenter y después va la Microsoft Store; las descargas corren en paralelo y ` +
      `las instalaciones se aplican una detrás de otra.\n\n` +
      `Las aplicaciones que estén abiertas se cerrarán para poder aplicar su actualización: ` +
      `guarda antes lo que tengas sin guardar.` + (portables.length
        ? `\n\nPortables: ${portables.map((item) => item.name).join(", ")}.` + portableDataMessage(portables[0].portableInfo) : ""),
    choices: portables.length ? portableDataChoices(portables[0].portableInfo) : null,
    confirmText: "Actualizar todo",
    confirmVariant: "primary",
    onConfirm: (picked) => {
      for (const item of portables) item.portableData = picked;
      return startBulkUpdate(items);
    },
  });
}

/**
 * Registra la tanda, abre su diálogo y va soltando las órdenes.
 *
 * Cada `invoke` vuelve en cuanto el backend ha apuntado la tarea, así que
 * esperarlas en fila no serializa el trabajo: sólo fija el orden en que entran
 * en la cola de descargas, que es justo lo que se quiere.
 *
 * `mode` es «update» para «Actualizar todo» e «install» para la cola de lo
 * seleccionado: la maquinaria es la misma, cambia lo que se le pide al backend
 * y cómo se cuenta.
 */
async function startBulkUpdate(items, mode = "update") {
  const installing = mode === "install";
  const bulk = {
    mode,
    items: items.map((item) => ({
      ...item,
      state: "pending",
      progress: 0,
      status: "En espera de su turno…",
      dispatched: false,
      dispatching: false,
    })),
    byKey: new Map(),
    startedAt: new Date(),
    cancelling: false,
    finished: false,
  };
  for (const item of bulk.items) bulk.byKey.set(item.key, item);
  state.bulkUpdate = bulk;
  // El diálogo de una sola aplicación cede el sitio: aquí manda éste.
  state.operationAppId = null;
  // El velo de recarga se levanta y se baja con cada instalación que acaba. Con
  // diez en marcha sería un parpadeo constante sobre una lista que además nadie
  // está mirando, porque delante está este diálogo contándolo.
  suppressShellBusy = true;
  hideShellBusy();
  showBulkUpdateModal();
  renderContent();
  clientLog("info", "bulk-update", {
    mode,
    total: bulk.items.length,
    catalog: bulk.items.filter((item) => item.kind === "catalog").length,
    msstore: bulk.items.filter((item) => item.kind === "msstore").length,
  });

  for (const item of bulk.items) {
    if (bulk.cancelling) {
      finishBulkItem(item.key, "cancelled", "Cancelada antes de empezar");
      continue;
    }
    try {
      item.dispatching = true;
      if (item.kind === "catalog") {
        state.busy[item.appId] = installing ? "installing" : "updating";
        state.finished.delete(item.appId);
        await invoke("install_app", {
          appEntry: item.app,
          forceUpdate: !installing,
          // La edición elegida en su día se respeta: actualizar es reinstalar
          // la misma, nunca cambiar de programa por el camino.
          variant: state.settings?.variants?.[item.appId] || item.app.variants?.default || null,
          portableData: item.portableData || null,
          // Lo que aún no está instalado no puede estar abierto.
          closeRunning: !installing,
        });
      } else {
        state.msstore.installing[item.productId] = "update";
        state.msstore.updateFamilyByProduct[item.productId] = item.family;
        state.finished.delete(item.key);
        await invoke("msstore_install", {
          productId: item.productId,
          name: item.name,
          ring: state.msstore.ring,
          arch: state.msstore.arch,
          family: item.family,
          closeRunning: true,
        });
      }
      item.dispatched = true;
      item.dispatching = false;
      if (bulk.cancelling && !isBulkItemFinal(item)) {
        await invokeDownloadAction("cancel_download", { appId: item.key });
      }
      if (item.state === "pending") {
        item.state = "queued";
        item.status = "En cola";
      }
    } catch (error) {
      item.dispatching = false;
      // El backend rechazó la orden —ya instalada, ya en cola—: eso es el final
      // de esta fila, no de la tanda.
      if (item.kind === "catalog") delete state.busy[item.appId];
      else {
        delete state.msstore.installing[item.productId];
        delete state.msstore.updateFamilyByProduct[item.productId];
      }
      finishBulkItem(item.key, "error", String(error));
      void clientLog("warn", "bulk-update", `${item.name}: ${error}`);
    }
    updateBulkUpdateModal();
  }
  updateVisibleAppActions(new Set(bulk.items.map((item) => item.key)));
  settleBulkUpdate();
}

/// Da una fila por terminada. Lo que llegue después de esto no la mueve.
function finishBulkItem(key, outcome, status) {
  const item = state.bulkUpdate?.byKey.get(key);
  if (!item || isBulkItemFinal(item)) return;
  item.state = outcome;
  item.progress = outcome === "error" || outcome === "cancelled" ? item.progress : 100;
  item.status = status || BULK_STATE_LABELS[outcome] || "";
  updateBulkUpdateModal();
}

/// Cuando ya no queda ninguna fila viva, cierra la tanda y pone al día lo que
/// se sabe del equipo: una sola vez, en vez de una por aplicación.
function settleBulkUpdate() {
  const bulk = state.bulkUpdate;
  if (!bulk || bulk.finished) return;
  if (!bulk.items.every(isBulkItemFinal)) return;
  bulk.finished = true;
  // Ya no hay nada en marcha: la tecla de escape vuelve a servir.
  modalLocked = false;
  suppressShellBusy = false;
  updateBulkUpdateModal();

  const done = bulk.items.filter((item) => item.state === "done").length;
  const failed = bulk.items.filter((item) => item.state === "error").length;
  const installing = bulk.mode === "install";
  setTransientStatus(
    failed
      ? installing
        ? `Instalación en cola terminada · ${done} ${done === 1 ? "instalada" : "instaladas"}, ${failed} con error`
        : `Actualización múltiple terminada · ${done} al día, ${failed} con error`
      : installing
        ? `${done} ${done === 1 ? "aplicación instalada" : "aplicaciones instaladas"}`
        : `${done} ${done === 1 ? "aplicación actualizada" : "aplicaciones actualizadas"}`,
    failed ? "var(--red)" : "var(--green)",
    7000,
  );

  void (async () => {
    try {
      await ensureMsStoreInstalled({ force: true });
      await reconcileRuntimeFromBootstrap();
      replaceStatuses((await invoke("check_updates")) || state.statuses);
      state.lastUpdateScan = new Date();
    } catch (error) {
      void clientLog("warn", "bulk-update-refresh", String(error?.stack || error));
    } finally {
      renderSidebar();
      renderContent();
    }
    void scanMsStoreUpdates({ quiet: true });
  })();
}

/// Cancela lo que quede vivo. Lo que aún no se ha pedido se descarta aquí
/// mismo; de lo demás se encarga el backend, que es quien tiene la descarga.
async function cancelBulkUpdate() {
  const bulk = state.bulkUpdate;
  if (!bulk || bulk.finished || bulk.cancelling) return;
  bulk.cancelling = true;
  updateBulkUpdateModal();
  for (const item of bulk.items) {
    if (isBulkItemFinal(item)) continue;
    if (item.dispatching) {
      item.status = "Cancelación pendiente de confirmar la orden…";
      continue;
    }
    if (!item.dispatched) {
      finishBulkItem(item.key, "cancelled", "Cancelada antes de empezar");
      continue;
    }
    await invokeDownloadAction("cancel_download", { appId: item.key });
  }
  settleBulkUpdate();
}

/**
 * El diálogo de la tanda.
 *
 * Se dibuja una vez con lo que no cambia —icono, nombre, versiones— y a partir
 * de ahí sólo se reescriben el estado, la barra y la línea de cada fila. Una
 * lista de diez que se rehiciera entera en cada evento de progreso perdería el
 * desplazamiento cada pocas décimas.
 */
function showBulkUpdateModal() {
  const bulk = state.bulkUpdate;
  if (!bulk) return;
  const groups = [
    ["Catálogo de WinSlimCenter", bulk.items.filter((item) => item.kind === "catalog")],
    ["Microsoft Store", bulk.items.filter((item) => item.kind === "msstore")],
  ].filter(([, items]) => items.length);

  openModal(`
    <div class="bulk-operation">
      <div class="bulk-operation-brand">
        <div class="bulk-operation-mark" aria-hidden="true">
          <img src="assets/winslim-center-logo.png" width="904" height="904" alt="" />
        </div>
        <div class="bulk-operation-heading">
          <small>WinSlimCenter</small>
          <!-- Sin id propio: openModal le pone el suyo al primer h2 para
               nombrar el diálogo, y se lo pisaría. -->
          <h2 data-role="title">${bulkVerb()} ${bulk.items.length} ${
            bulk.items.length === 1 ? "aplicación" : "aplicaciones"
          }</h2>
        </div>
        <span class="bulk-operation-tally" id="bulk-operation-tally"></span>
      </div>
      <div class="bulk-operation-overall">
        <div class="bulk-operation-track" id="bulk-operation-progress"
             role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="0">
          <span class="bulk-operation-fill" id="bulk-operation-progress-fill"></span>
        </div>
        <p class="bulk-operation-lead" id="bulk-operation-lead"></p>
      </div>
      <div class="bulk-operation-list">
        ${groups
          .map(
            ([title, items]) => `
          <div class="bulk-group">
            <span class="bulk-group-title">${escapeHtml(title)}</span>
            <span class="bulk-group-count">${items.length}</span>
          </div>
          ${items.map(bulkRowHtml).join("")}`,
          )
          .join("")}
      </div>
      <div class="bulk-operation-actions" id="bulk-operation-actions"></div>
    </div>
  `, true, !bulk.finished);
  updateBulkUpdateModal();
}

function bulkRowHtml(item, index) {
  const accent = pickAccent(item.app, index);
  const avatarBg = avatarBackground(item.app, accent);
  const letter = (item.name || "?")[0].toUpperCase();
  const versions =
    item.from && item.to
      ? `<span class="bulk-row-versions">${escapeHtml(versionLabel(item.from, "versión desconocida"))} → ${escapeHtml(versionLabel(item.to, "la más reciente"))}</span>`
      : "";
  return `
    <article class="bulk-row" data-bulk-key="${escapeHtml(item.key)}" style="--card-accent:${accent}">
      <div class="card-avatar bulk-row-avatar" style="background:${avatarBg}">
        ${renderAvatar(item.app, letter)}
      </div>
      <div class="bulk-row-body">
        <div class="bulk-row-head">
          <strong>${escapeHtml(item.name)}</strong>
          ${versions}
          <span class="bulk-row-state" data-role="state">${escapeHtml(BULK_STATE_LABELS.pending)}</span>
        </div>
        <div class="bulk-row-track"><span class="bulk-row-fill" data-role="fill"></span></div>
        <small class="bulk-row-status" data-role="status">${escapeHtml(item.status)}</small>
      </div>
    </article>`;
}

function updateBulkUpdateModal() {
  const bulk = state.bulkUpdate;
  if (!bulk) return;
  const list = document.querySelector(".bulk-operation");
  if (!list) return;

  for (const item of bulk.items) {
    const row = list.querySelector(`.bulk-row[data-bulk-key="${CSS.escape(item.key)}"]`);
    if (!row) continue;
    const nextState = item.state;
    if (row.dataset.state !== nextState) {
      row.dataset.state = nextState;
      const state_ = row.querySelector('[data-role="state"]');
      if (state_) state_.textContent = bulkStateLabel(nextState);
    }
    const value = Math.max(0, Math.min(100, Math.round(Number(item.progress) || 0)));
    if (row.dataset.progress !== String(value)) {
      row.dataset.progress = String(value);
      const fill = row.querySelector('[data-role="fill"]');
      if (fill) fill.style.width = `${value}%`;
    }
    const status = row.querySelector('[data-role="status"]');
    if (status && status.textContent !== item.status) status.textContent = item.status;
  }

  const total = bulk.items.length;
  const settled = bulk.items.filter(isBulkItemFinal);
  const done = bulk.items.filter((item) => item.state === "done" || item.state === "current").length;
  const failed = bulk.items.filter((item) => item.state === "error").length;
  const cancelled = bulk.items.filter((item) => item.state === "cancelled").length;
  const overall = Math.round(
    bulk.items.reduce(
      (sum, item) => sum + (isBulkItemFinal(item) ? 100 : Math.min(100, Number(item.progress) || 0)),
      0,
    ) / Math.max(1, total),
  );

  const progress = document.getElementById("bulk-operation-progress");
  if (progress && progress.getAttribute("aria-valuenow") !== String(overall)) {
    progress.setAttribute("aria-valuenow", String(overall));
    const fill = document.getElementById("bulk-operation-progress-fill");
    if (fill) fill.style.width = `${overall}%`;
  }
  // La barra llena significa «ya no queda nada por decidir», no «ha salido
  // bien»: una tanda cancelada entera la deja igual de llena que una que se
  // instaló. Al terminar, el color dice cuál de las dos fue.
  list.dataset.outcome = bulk.finished ? (failed || cancelled ? "issues" : "ok") : "running";
  const tally = document.getElementById("bulk-operation-tally");
  if (tally) tally.textContent = `${settled.length} / ${total}`;

  const installing = bulk.mode === "install";
  const title = list.querySelector('h2[data-role="title"]');
  if (title) {
    title.textContent = bulk.finished
      ? failed || cancelled
        ? installing ? "Instalación en cola terminada" : "Actualización múltiple terminada"
        : installing ? "Todo instalado" : "Todo actualizado"
      : `${bulkVerb()} ${total} ${total === 1 ? "aplicación" : "aplicaciones"}`;
  }

  const lead = document.getElementById("bulk-operation-lead");
  if (lead) {
    if (bulk.finished) {
      lead.textContent = [
        installing ? `${done} ${done === 1 ? "instalada" : "instaladas"}` : `${done} al día`,
        failed ? `${failed} con error` : "",
        cancelled ? `${cancelled} ${cancelled === 1 ? "cancelada" : "canceladas"}` : "",
      ]
        .filter(Boolean)
        .join(" · ");
    } else if (bulk.cancelling) {
      lead.textContent = "Cancelando lo que queda…";
    } else {
      // El backend sirve cuatro descargas a la vez y el resto espera turno.
      // Contarlas todas como «en marcha» diría que hay diecinueve moviéndose
      // cuando sólo se mueven cuatro; separarlas explica por qué las demás
      // llevan un rato quietas en cero.
      const moving = bulk.items.filter((item) =>
        ["downloading", "installing", "cancelling", "paused"].includes(item.state),
      ).length;
      const waiting = bulk.items.filter((item) =>
        ["pending", "queued"].includes(item.state),
      ).length;
      lead.textContent =
        `${settled.length} de ${total} ${total === 1 ? "completada" : "completadas"}` +
        (moving ? ` · ${moving} en marcha` : "") +
        (waiting ? ` · ${waiting} esperando turno` : "");
    }
  }

  const actions = document.getElementById("bulk-operation-actions");
  if (!actions) return;
  const signature = `${bulk.finished}|${bulk.cancelling}`;
  if (actions.dataset.signature === signature) return;
  actions.dataset.signature = signature;
  actions.innerHTML = bulk.finished
    ? (bulk.items.some((item) => item.portableData === "preserve" && item.state === "done")
      ? '<button type="button" class="btn ghost" id="bulk-operation-backups">Abrir copias de datos</button>' : "") +
      '<button type="button" class="btn primary" id="bulk-operation-close">Cerrar</button>'
    : `<button type="button" class="btn ghost" id="bulk-operation-cancel"${
        bulk.cancelling ? " disabled" : ""
      }>${bulk.cancelling ? "Cancelando…" : "Cancelar todo"}</button>`;
  actions.querySelector("#bulk-operation-close")?.addEventListener("click", closeModal);
  actions.querySelector("#bulk-operation-backups")?.addEventListener("click", openPortableBackups);
  actions.querySelector("#bulk-operation-cancel")?.addEventListener("click", () =>
    void cancelBulkUpdate(),
  );
  actions.querySelector("button")?.focus();
}

/// Lo que las tareas del backend dicen de cada fila, en cada latido.
///
/// Los estados finales los fija el evento de fin, que sabe cosas que la tarea
/// no cuenta —si la versión cambió, si Windows dejó el paquete a la espera—:
/// de aquí sólo salen el progreso y la línea de estado.
function updateBulkOperation(tasks) {
  const bulk = state.bulkUpdate;
  if (!bulk || bulk.finished) return;
  let changed = false;
  for (const task of tasks) {
    const item = bulk.byKey.get(task.app_id);
    if (!item || isBulkItemFinal(item)) continue;
    const progress = Math.max(0, Math.min(100, Number(task.progress) || 0));
    const status = task.status || BULK_STATE_LABELS[item.state] || "";
    const next = ACTIVE_TASK_STATES.has(task.state) ? task.state : item.state;
    if (item.progress !== progress || item.status !== status || item.state !== next) {
      item.progress = progress;
      item.status = status;
      item.state = next;
      changed = true;
    }
  }
  if (changed) updateBulkUpdateModal();
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

// `details` es marcado propio de este archivo —nunca texto del usuario ni de
// la red—, y va entre las opciones y los botones.
function showConfirmModal({ title, message, app, subtitle = null, details = "", confirmText = "Confirmar", confirmVariant = "primary", choices = null, secondary = null, cancelText = "Cancelar", onCancel = null, onConfirm }) {
  const avatar = app ? renderAvatar(app, app.name?.[0] || "?", true) : "";
  const accent = app ? pickAccent(app, 0) : "var(--accent)";
  const avatarBg = app ? avatarBackground(app, accent) : accent;
  const sub = subtitle ?? (app ? [app.author, versionLabel(app.version)].filter(Boolean).join(" · ") : "");

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
            sub.trim()
              ? `<small style="color: var(--text-medium); font-size: 12px; font-weight: 500;">${escapeHtml(sub)}</small>`
              : ""
          }
        </div>
      </div>
      <p class="confirm-dialog-msg">${escapeHtml(message)}</p>
      ${renderChoices(choices)}
      ${details}
      <div class="modal-foot" style="margin-top: 20px;">
        <button type="button" class="btn ghost" id="modal-btn-cancel">${escapeHtml(cancelText)}</button>
        ${secondary ? `<button type="button" class="btn ghost" id="modal-btn-secondary">${escapeHtml(secondary)}</button>` : ""}
        <button type="button" class="btn ${confirmVariant}" id="modal-btn-confirm">${escapeHtml(confirmText)}</button>
      </div>
    </div>
  `);

  document.getElementById("modal-btn-cancel").onclick = () => {
    closeModal();
    onCancel?.();
  };
  const answer = async (choice) => {
    const picked = document.querySelector('input[name="modal-choice"]:checked')?.value;
    closeModal();
    if (onConfirm) await onConfirm(picked, choice);
  };
  document.getElementById("modal-btn-confirm").onclick = () => answer("confirm");
  document
    .getElementById("modal-btn-secondary")
    ?.addEventListener("click", () => answer("secondary"));
}

// Whether this operation needs the application closed first. Asked before
// anything is downloaded: Windows decides whether it can apply a package the
// instant WinGet hands it over, so an application left open turns the operation
// into one that finishes later, silently, whenever the user happens to quit it.
// The store used to discover that afterwards — having spent the download — and
// report it as success.
async function blockingRunningApp(id) {
  try {
    return await invoke("running_blocker", { appId: id });
  } catch (error) {
    void clientLog("warn", "running-blocker", String(error?.stack || error));
    return null;
  }
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
    setStatus(`Catálogo recargado: ${visibleCatalogCount()} aplicaciones`, "var(--green)");
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
  // over one another would only darken the window twice. The start-up check
  // reports through the cards it has not answered for yet and the loading
  // line, and the store stays usable underneath.
  if (shellBusyOverlay || syncModalOverlay || suppressShellBusy || state.verifyingStatuses) return;
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
  // Retirar el velo sin escalar ni fundir su desenfoque evita saltos del compositor.
  overlay.remove();
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
  if (!syncModalOverlay) return Promise.resolve();
  const overlay = syncModalOverlay;
  syncModalOverlay = null;
  // Capturar esta capa: el respaldo nunca debe retirar una sincronización nueva.
  return new Promise((resolve) => {
    let timer;
    const remove = () => {
      clearTimeout(timer);
      overlay.removeEventListener("animationend", onEnd);
      overlay.remove();
      resolve();
    };
    const onEnd = (event) => {
      if (event.target === overlay && event.animationName === "bootSplashOut") remove();
    };
    overlay.addEventListener("animationend", onEnd);
    overlay.classList.add("is-leaving");
    const duration = getComputedStyle(overlay).animationDuration;
    const milliseconds = parseFloat(duration) * (duration.trim().endsWith("ms") ? 1 : 1000);
    timer = setTimeout(remove, (Number.isFinite(milliseconds) ? milliseconds : 110) + 100);
  });
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
    document.body.classList.add("refreshing-store");
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
      // Descubrir la rejilla ya pintada, sin una pausa artificial al 100 %.
      await afterFirstPaint();
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
      await closeSyncModal();
      button.disabled = false;
      button.innerHTML = original;
    }
    setProgress(100);
    if (!silent) document.body.classList.remove("refreshing-store");
  }
}

/** Mantener el arranque hasta recuperar la red, sin reiniciar la aplicación. */
function waitForStartupConnection() {
  const splash = document.getElementById("boot-splash");
  if (!splash) return Promise.resolve();
  const app = document.getElementById("app");
  const copy = splash.querySelector(".boot-splash-copy");
  const subtitle = copy.querySelector("span");
  const originalSubtitle = subtitle.textContent;
  splash.classList.add("is-connection-pending");
  app.inert = true;
  const panel = document.createElement("div");
  panel.className = "boot-offline-actions";
  panel.hidden = true;
  panel.innerHTML = `
    <p id="boot-connection-help" aria-live="polite">Conecta el equipo a Internet para acceder a tus aplicaciones y descubrir otras nuevas.</p>
    <button type="button" class="btn primary" id="boot-retry">
      <svg class="btn-svg-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/>
        <path d="M3 3v5h5"/>
        <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16"/>
        <path d="M16 16h5v5"/>
      </svg>
      <span>Reintentar</span>
    </button>`;
  splash.appendChild(panel);
  const button = panel.querySelector("button");
  const label = button.querySelector("span");
  const help = panel.querySelector("p");
  let checking = false;
  let complete = false;
  return new Promise((resolve) => {
    const retry = async () => {
      if (checking || complete) return;
      checking = true;
      button.disabled = true;
      label.textContent = "Comprobando…";
      panel.setAttribute("aria-busy", "true");
      let connected = false;
      let failed = false;
      try {
        connected = navigator.onLine !== false && await invoke("check_connection");
      } catch (error) {
        failed = true;
        void clientLog("warn", "startup-connection", String(error));
      }
      checking = false;
      if (connected) {
        complete = true;
        window.removeEventListener("online", retry);
        panel.remove();
        splash.classList.remove("is-offline", "is-connection-pending");
        splash.setAttribute("role", "status");
        splash.setAttribute("aria-label", "Iniciando WinSlimCenter");
        splash.removeAttribute("aria-modal");
        subtitle.textContent = originalSubtitle;
        app.inert = false;
        resolve();
        return;
      }
      const firstAttempt = panel.hidden;
      splash.classList.add("is-offline");
      splash.setAttribute("role", "dialog");
      splash.setAttribute("aria-modal", "true");
      splash.setAttribute("aria-label", "Sin conexión a Internet");
      copy.removeAttribute("aria-hidden");
      subtitle.textContent = "Sin conexión a Internet";
      help.textContent = failed
        ? "No hemos podido comprobar la conexión. Vuelve a intentarlo."
        : firstAttempt
          ? "Conecta el equipo a Internet para acceder a tus aplicaciones y descubrir otras nuevas."
          : "Seguimos sin conexión. Comprueba el Wi-Fi o el cable de red y vuelve a intentarlo.";
      panel.hidden = false;
      panel.setAttribute("aria-busy", "false");
      button.disabled = false;
      label.textContent = "Reintentar";
      button.focus({ preventScroll: true });
    };
    button.addEventListener("click", retry);
    window.addEventListener("online", retry);
    void retry();
  });
}

/// Retira la pantalla de arranque de `index.html`. Dos fotogramas: en el
/// primero se dibuja la tienda debajo y en el segundo ya está en pantalla, así
/// que el fundido descubre el catálogo entero y no una ventana a medio pintar.
/// Lo que la pantalla de arranque se queda a la vista, como poco, desde que se
/// muestra la ventana: la tienda se pinta en milisegundos y, sin un mínimo, de
/// ella sólo se veía un destello antes del fundido.
const BOOT_SPLASH_MIN_MS = 750;

function dismissBootSplash() {
  const splash = document.getElementById("boot-splash");
  if (!splash || splash.dataset.leaving) return;
  splash.dataset.leaving = "true";
  // La ventana puede seguir oculta esperando al logotipo y la letra: el mínimo
  // cuenta desde que se muestra, no desde que la tienda está lista debajo.
  void Promise.all([afterFirstPaint(), window.__bootSplashShown]).then(() => {
    // Sin la marca de index.html (fuera de Tauri) no hay ventana que esperar.
    const shownAt = window.__bootSplashShownAt;
    const wait = shownAt === undefined ? 0 : Math.max(0, shownAt + BOOT_SPLASH_MIN_MS - performance.now());
    setTimeout(() => {
      let cleanupTimer;
      const remove = () => {
        clearTimeout(cleanupTimer);
        splash.removeEventListener("animationend", onEnd);
        splash.remove();
      };
      const onEnd = (event) => {
        // El logo y el texto también animan: solo la salida de la capa completa
        // puede retirarla, para no cortar el fundido por un evento de un hijo.
        if (event.target === splash && event.animationName === "bootSplashOut") remove();
      };
      splash.addEventListener("animationend", onEnd);
      splash.classList.add("done");
      // Respaldo si WebView no entrega animationend. CSS sigue siendo la fuente
      // de la duración, tanto si se expresa en segundos como en milisegundos.
      const duration = getComputedStyle(splash).animationDuration;
      const milliseconds = parseFloat(duration) * (duration.trim().endsWith("ms") ? 1 : 1000);
      cleanupTimer = setTimeout(remove, (Number.isFinite(milliseconds) ? milliseconds : 320) + 100);
    }, wait);
  });
}

/// La línea fina bajo la cabecera que dice «sigo trabajando» sin tapar nada.
function setLoadingLine(active) {
  document.getElementById("loading-line")?.classList.toggle("active", active);
}

const afterFirstPaint = () =>
  new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));

/**
 * Lo que el arranque comprueba con la tienda ya en pantalla y usable.
 *
 * Nada de pantallas de carga: la ventana se abre con lo que sabía la sesión
 * anterior y esto lo corrige por detrás. Las fichas de las que nadie sabía nada
 * esperan con un botón fantasma, la línea de arriba dice que se está trabajando
 * y, cuando el equipo contesta, sólo cambia lo que tenía que cambiar.
 */
async function finishStartupInBackground() {
  // Se crea antes de esperar a que se pinte la ventana: lo que se pulse desde
  // el primer momento ya tiene a qué esperar.
  statusVerification = (async () => {
    // Dos fotogramas para que WebView pinte la tienda completa antes de lanzar
    // los procesos de Windows, el menú Inicio y WinGet.
    await afterFirstPaint();
    // Lo instalado desde la Microsoft Store sale de Windows sin ir a la red y
    // va a la par que el escaneo, que tarda más: sin ello «Aplicaciones instaladas»
    // empezaría a medias.
    const [, statuses] = await Promise.all([
      ensureMsStoreInstalled({ force: true }),
      invoke("refresh_statuses"),
    ]);
    return statuses;
  })();
  setLoadingLine(true);
  setStatus("Comprobando las aplicaciones del equipo…", "var(--accent)");

  try {
    const statuses = await statusVerification;
    const reopenModal = appModalAwaitingStatus();
    const hadPending = state.statusPending.size > 0;
    const changes = replaceStatuses(statuses || state.statuses);
    finishStatusVerification();
    if (hadPending) {
      revealVerifiedActions();
      renderSidebar();
      renderContent();
    } else {
      reconcileStatusChanges(changes);
      // Lo de la Microsoft Store llega por su lado y sólo se ve aquí.
      if (state.section === "installed") renderContent();
    }
    if (reopenModal) openAppModal(reopenModal);
  } catch (error) {
    finishStatusVerification();
    renderSidebar();
    renderContent();
    setLoadingLine(false);
    setStatus(`La tienda está disponible, pero falló la comprobación del sistema: ${error}`, "var(--red)");
    clientLog("error", "startup-background", String(error?.stack || error));
    void checkStoreUpdate();
    return;
  }

  // Si el usuario ya pidió buscarlas mientras tanto, esa búsqueda es ésta y
  // ya informa de sí misma.
  if (state.scanningUpdates) {
    setLoadingLine(false);
    void checkStoreUpdate();
    return;
  }
  // Las actualizaciones van después, porque dependen de lo instalado, y ya sin
  // ninguna ficha esperando: la sección y su cifra lo dicen mientras tanto.
  state.scanningUpdates = true;
  renderSidebar();
  if (state.section === "updates") renderContent();
  setStatus("Buscando actualizaciones disponibles…", "var(--accent)");
  try {
    const updateChanges = replaceStatuses((await invoke("check_updates")) || state.statuses);
    state.lastUpdateScan = new Date();
    reconcileStatusChanges(updateChanges);
    const updates = updatesCount();
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
  } finally {
    state.scanningUpdates = false;
    setLoadingLine(false);
    setProgress(100);
    renderSidebar();
    if (state.section === "updates") renderContent();
    void checkStoreUpdate();
    // Preguntar al servicio de entrega por cada aplicación de la tienda cuesta
    // dos viajes por aplicación, así que se hace con la tienda ya en pantalla y
    // sin retenerla: cuando conteste, aparecen sus actualizaciones.
    void scanMsStoreUpdates({ quiet: true });
  }
}

/// Los botones que sustituyen a las siluetas entran con un fundido corto. Sólo
/// en ese redibujado: en cualquier otro aparecer de golpe es lo correcto.
function revealVerifiedActions() {
  const content = document.getElementById("content");
  content.classList.add("statuses-revealed");
  setTimeout(() => content.classList.remove("statuses-revealed"), 700);
}

function finishStatusVerification() {
  state.verifyingStatuses = false;
  state.statusPending = new Set();
  statusVerification = null;
}

async function refreshInstalledFromBootstrap() {
  const data = await invoke("get_bootstrap");
  state.appVersion = data.app_version || state.appVersion;
  state.installed = data.installed || {};
  replaceRepoEntries(data.repo_entries || []);
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
  replaceRepoEntries(data.repo_entries || []);
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
  updateBulkOperation(state.tasks);
  return { data, changes };
}

let storeUpdateRunning = false;
async function startStoreUpdate() {
  if (storeUpdateRunning) return;
  storeUpdateRunning = true;
  try {
    setStatus("Descargando la nueva versión de la tienda...", "var(--accent)");
    const msg = await invoke("update_center_app");
    setStatus(String(msg || "Actualización iniciada"), "var(--accent)");
  } catch (e) {
    setStatus(`Error de actualización: ${e}`, "var(--red)");
    showAlertModal("Error de actualización", String(e));
  } finally {
    storeUpdateRunning = false;
    setProgress(0);
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
  const catalogSize = visibleCatalogCount();
  openModal(`
    <div class="about-dialog">
      <div class="about-head">
        <div class="about-logo">
          <img src="assets/winslim-center-logo.png" width="904" height="904" alt="WinSlimCenter" />
        </div>
        <div class="about-title">
          <h2 class="about-name">WinSlimCenter</h2>
          <p class="about-version">${state.appVersion ? `Versión ${escapeHtml(state.appVersion)}` : "Tienda de aplicaciones"}</p>
        </div>
      </div>
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
  `, false, false, "about");
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
    note.textContent = "Consultando las versiones publicadas en GitHub…";
    note.className = "about-note";
    void clientLog("info", "self-update", "Comprobación manual pedida desde Acerca de.");
    setStatus("Comprobando si hay una versión nueva de la tienda…", "var(--accent)");
    try {
      // The very same call the store makes on its own when it starts, so a
      // release that shows the update dialog there shows it here too.
      const update = await runStoreUpdateCheck();
      if (update) {
        // The About dialog is gone by now: the update one took its place.
        setTransientStatus(
          `WinSlimCenter v${update.version} está publicada`,
          "var(--green)",
          8000,
        );
        return;
      }
      note.textContent = `Ya tienes la versión más reciente${state.appVersion ? ` (v${state.appVersion})` : ""}.`;
      note.className = "about-note ok";
      setTransientStatus("WinSlimCenter está al día", "var(--green)", 5000);
    } catch (error) {
      // Asked for on purpose, so the answer cannot be a silent shrug the way it
      // is when the check runs by itself at start-up — and it says what actually
      // happened, because "comprueba la conexión" sent the user chasing a
      // network problem that was never there.
      note.textContent = String(error) || "No se pudo consultar GitHub.";
      note.className = "about-note bad";
      void clientLog("warn", "self-update", `Comprobación manual fallida: ${error?.stack || error}`);
      setTransientStatus("No se pudo comprobar si hay una versión nueva", "var(--red)", 8000);
    } finally {
      // The dialog may have been replaced by the update one while this ran, in
      // which case there is no button left to put back.
      if (check.isConnected) {
        check.disabled = false;
        check.textContent = label;
      }
    }
  };
}

// ---- Novedades: las notas que el autor escribe al publicar una release ----

// El hueco de las notas dentro de un diálogo. Nace con un esqueleto de carga y
// lo rellena `fillReleaseNotes` cuando las notas llegan.
function releaseNotesShell() {
  return `
    <section class="release-notes" aria-live="polite" aria-busy="true">
      <header class="release-notes-head">
        <span class="release-notes-label">Novedades</span>
        <span class="release-notes-meta"></span>
        <button type="button" class="release-notes-link hidden">
          Ver en GitHub
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor"
            stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <line x1="7" y1="17" x2="17" y2="7"/>
            <polyline points="8 7 17 7 17 16"/>
          </svg>
        </button>
      </header>
      <div class="release-notes-body">
        <div class="release-notes-skeleton" aria-hidden="true"><i></i><i></i><i></i><i></i></div>
      </div>
    </section>`;
}

/**
 * Pone las notas en el hueco que dejó `releaseNotesShell`.
 *
 * `notes` es lo que devuelve el backend (`ReleaseNotes`), o `null` cuando no
 * se pudieron leer: entonces queda dicho, con el enlace a la página de la
 * release para quien quiera leerlas allí.
 */
function fillReleaseNotes(section, notes, pageUrl) {
  if (!section?.isConnected) return;
  section.setAttribute("aria-busy", "false");
  const meta = section.querySelector(".release-notes-meta");
  const body = section.querySelector(".release-notes-body");
  const link = section.querySelector(".release-notes-link");
  const openExternal = (url) =>
    invoke("open_url", { url }).catch((error) =>
      setTransientStatus(`No se pudo abrir el enlace: ${error}`, "var(--red)", 6000)
    );
  const page = notes?.url || pageUrl;
  if (page) {
    link.classList.remove("hidden");
    link.onclick = () => openExternal(page);
  }

  body.replaceChildren();
  const say = (text) => {
    const line = document.createElement("p");
    line.className = "release-notes-empty";
    line.textContent = text;
    body.appendChild(line);
  };
  if (!notes) {
    say("No se pudieron cargar las notas de esta versión.");
    return;
  }

  // La etiqueta sólo se enseña cuando dice una versión: la tienda publica
  // todo bajo «latest», que no le cuenta nada a nadie.
  const tag = String(notes.tag || "").trim();
  const date = formatReleaseDate(notes.updated);
  meta.innerHTML = `
    ${/\d/.test(tag) ? `<span class="release-notes-tag">${escapeHtml(tag)}</span>` : ""}
    ${date ? `<span class="release-notes-date">${escapeHtml(date)}</span>` : ""}`;

  const title = String(notes.title || "").trim();
  if (title && title.toLowerCase() !== tag.toLowerCase()) {
    const heading = document.createElement("p");
    heading.className = "release-notes-title";
    heading.textContent = title;
    body.appendChild(heading);
  }
  const content = notes.html ? sanitizeReleaseHtml(notes.html) : null;
  if (content && (content.textContent.trim() || content.querySelector("img"))) {
    body.appendChild(content);
  } else {
    say("El autor no publicó notas para esta versión.");
  }

  // Los enlaces se abren en el navegador del sistema, nunca dentro de la
  // tienda; las capturas, en el mismo visor que las de las fichas.
  body.onclick = (event) => {
    const shot = event.target.closest(".release-notes-shot");
    if (shot) {
      // Contadas al pulsar: una captura que no cargó ya no está en la tira.
      const shots = [...body.querySelectorAll(".release-notes-shot")];
      openShotViewer(
        shots.map((item) => item.querySelector("img").src),
        shots.indexOf(shot)
      );
      return;
    }
    const anchor = event.target.closest("[data-href]");
    if (anchor) {
      event.preventDefault();
      openExternal(anchor.dataset.href);
    }
  };
  body.onkeydown = (event) => {
    if (event.key === "Enter" && event.target.matches("[data-href]")) {
      event.preventDefault();
      openExternal(event.target.dataset.href);
    }
  };
}

function formatReleaseDate(value) {
  const date = new Date(value || "");
  if (Number.isNaN(date.getTime())) return "";
  return new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short", year: "numeric" }).format(date);
}

// Lo que se conserva de las notas tal como GitHub las dibuja. Todo lo demás se
// desenvuelve —su texto se queda— o, si no es texto, se descarta.
const RELEASE_NOTES_TAGS = new Set([
  "P", "BR", "HR", "H1", "H2", "H3", "H4", "H5", "H6", "UL", "OL", "LI", "STRONG", "B", "EM",
  "I", "DEL", "S", "CODE", "PRE", "BLOCKQUOTE", "A", "TABLE", "THEAD", "TBODY", "TR", "TH", "TD",
  "DETAILS", "SUMMARY", "KBD", "SUP", "SUB",
]);
const RELEASE_NOTES_DROPPED = new Set([
  "SCRIPT", "STYLE", "IFRAME", "OBJECT", "EMBED", "FORM", "INPUT", "BUTTON", "TEXTAREA", "SELECT",
  "VIDEO", "AUDIO", "SVG", "MATH", "TEMPLATE", "LINK", "META", "NOSCRIPT", "CANVAS",
]);
// Avisos de GitHub (> [!NOTE] …), con el título en el idioma de la tienda.
const RELEASE_NOTES_ALERTS = {
  note: "Nota",
  tip: "Consejo",
  important: "Importante",
  warning: "Advertencia",
  caution: "Precaución",
};
// Las capturas sólo se cargan de los servidores donde GitHub guarda lo que se
// adjunta a una release; cualquier otra imagen queda como enlace.
const RELEASE_NOTES_IMAGE_HOSTS = /^https:\/\/([a-z0-9-]+\.)?(githubusercontent\.com|github\.com)\//i;

/**
 * Reconstruye el HTML de unas notas con una lista corta de elementos y sin más
 * atributos que los enlaces http(s).
 *
 * Las notas las escribe cualquier autor, y esta ventana puede instalar y
 * desinstalar programas: nada de lo que traigan puede llegar a ejecutarse.
 * DOMParser no ejecuta nada al analizar, y lo que sale de aquí son nodos
 * nuevos, no los suyos.
 */
function sanitizeReleaseHtml(html) {
  const source = new DOMParser().parseFromString(html, "text/html");
  const out = document.createDocumentFragment();
  const isHttp = (url) => /^https?:\/\//i.test(url || "");
  const linkTo = (element, url) => {
    element.dataset.href = url;
    element.title = url;
    element.setAttribute("role", "link");
    element.tabIndex = 0;
  };

  const walk = (from, into) => {
    for (const node of from.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) {
        into.appendChild(document.createTextNode(node.textContent));
        continue;
      }
      if (node.nodeType !== Node.ELEMENT_NODE) continue;
      const tag = node.tagName;
      if (RELEASE_NOTES_DROPPED.has(tag)) continue;
      // Los «#» que GitHub cuelga junto a cada título para enlazarlo.
      if (tag === "A" && node.classList.contains("anchor")) continue;

      if (tag === "DIV" && node.classList.contains("markdown-alert")) {
        const kind =
          Object.keys(RELEASE_NOTES_ALERTS).find((k) => node.classList.contains(`markdown-alert-${k}`)) ||
          "note";
        const box = document.createElement("blockquote");
        box.className = `release-notes-alert ${kind}`;
        walk(node, box);
        const alertTitle = box.querySelector(".release-notes-alert-title");
        if (alertTitle) alertTitle.textContent = RELEASE_NOTES_ALERTS[kind];
        into.appendChild(box);
        continue;
      }

      if (tag === "IMG") {
        const src = node.getAttribute("src");
        if (RELEASE_NOTES_IMAGE_HOSTS.test(src || "")) {
          const shot = document.createElement("button");
          shot.type = "button";
          shot.className = "release-notes-shot";
          shot.setAttribute("aria-label", "Ampliar la captura");
          const image = document.createElement("img");
          image.src = src;
          image.alt = node.getAttribute("alt") || "";
          image.loading = "lazy";
          image.decoding = "async";
          // Las de GitHub van firmadas y caducan: una que ya no carga desaparece
          // en lugar de dejar un hueco roto.
          image.addEventListener("error", () => shot.remove(), { once: true });
          shot.appendChild(image);
          into.appendChild(shot);
        } else if (isHttp(src)) {
          const chip = document.createElement("a");
          chip.className = "release-notes-media";
          chip.textContent = node.getAttribute("alt")?.trim() || "Imagen";
          linkTo(chip, src);
          into.appendChild(chip);
        }
        continue;
      }

      // GitHub envuelve cada captura en un enlace a sí misma: la captura ya se
      // amplía al pulsarla, así que el enlace sobra.
      if (tag === "A" && node.querySelector("img") && !node.textContent.trim()) {
        walk(node, into);
        continue;
      }
      if (!RELEASE_NOTES_TAGS.has(tag)) {
        walk(node, into);
        continue;
      }

      const element = document.createElement(tag.toLowerCase());
      if (tag === "P" && node.classList.contains("markdown-alert-title")) {
        element.className = "release-notes-alert-title";
      } else if (tag === "A") {
        const href = node.getAttribute("href");
        if (isHttp(href)) linkTo(element, href);
      } else if ((tag === "TH" || tag === "TD") && /^(left|center|right)$/.test(node.getAttribute("align") || "")) {
        element.style.textAlign = node.getAttribute("align");
      } else if (tag === "OL" && /^\d+$/.test(node.getAttribute("start") || "")) {
        element.setAttribute("start", node.getAttribute("start"));
      }
      walk(node, element);
      // «### WinSlimCenter 2.3.3 ###»: almohadillas escritas a mano como adorno
      // de un título que ya es un título.
      if (/^H[1-6]$/.test(tag) && !element.children.length) {
        element.textContent = element.textContent.trim().replace(/^#+\s*/, "").replace(/\s+#+$/, "");
      }
      into.appendChild(element);
    }
  };

  walk(source.body, out);

  // Capturas seguidas, como las que acompañan a cada release de la tienda, van
  // juntas en una tira de miniaturas: una debajo de otra a todo el ancho no
  // dejaban ver nada más del diálogo.
  const parents = new Set([...out.querySelectorAll(".release-notes-shot")].map((shot) => shot.parentNode));
  for (const parent of parents) {
    let strip = null;
    for (const child of [...parent.childNodes]) {
      if (child.nodeType === Node.ELEMENT_NODE && child.classList.contains("release-notes-shot")) {
        if (!strip) {
          strip = document.createElement("div");
          strip.className = "release-notes-shots";
          parent.insertBefore(strip, child);
        }
        strip.appendChild(child);
      } else if (strip && child.nodeType === Node.TEXT_NODE && !child.textContent.trim()) {
        child.remove();
      } else {
        strip = null;
      }
    }
  }
  return out;
}

// Las notas de una aplicación de GitHub se piden una vez por sesión y versión:
// abrir y cerrar el diálogo no vuelve a llamar a GitHub.
const releaseNotesRequests = new Map();

function loadReleaseNotes(repo, tag) {
  const key = `${repo}@${tag || ""}`;
  if (!releaseNotesRequests.has(key)) {
    const request = invoke("get_release_notes", { repo, tag: tag || null }).catch((error) => {
      releaseNotesRequests.delete(key);
      throw error;
    });
    releaseNotesRequests.set(key, request);
  }
  return releaseNotesRequests.get(key);
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
      ${releaseNotesShell()}
      <div class="modal-foot" style="margin-top: 6px;">
        <button type="button" class="btn ghost" id="store-update-skip">Omitir</button>
        <button type="button" class="btn primary" id="store-update-now">Actualizar ahora</button>
      </div>
    </div>
  `, false, false, "hero");
  // Las notas llegan con el aviso, en la misma consulta que dijo la versión.
  fillReleaseNotes(
    document.querySelector("#modal .release-notes"),
    update?.notes || null,
    update?.release_page
  );
  document.getElementById("store-update-skip").onclick = () => {
    closeModal();
    setTransientStatus("Actualización omitida. Se volverá a avisar al abrir la tienda.", "var(--text-medium)", 6000);
  };
  document.getElementById("store-update-now").onclick = async () => {
    closeModal();
    await startStoreUpdate();
  };
}

// Asks the repository whether it publishes something newer than this build, and
// offers it when it does.
//
// The single place that decides what a published release means. Start-up and
// the "Buscar actualizaciones" button in the About dialog both come through
// here, so the two can never answer the same question differently.
async function runStoreUpdateCheck() {
  const update = await invoke("check_store_update");
  if (!update) return null;
  void clientLog("info", "self-update", `Versión publicada: ${update.version}, instalada: ${update.current}`);
  showStoreUpdateModal(update);
  return update;
}

// Runs once, after the start-up scan has released the screen, and stays silent
// about its own failures: not reaching GitHub is not something the user has to
// act on, and it must never get in the way of a store that already works.
async function checkStoreUpdate() {
  try {
    await runStoreUpdateCheck();
  } catch (error) {
    void clientLog("warn", "self-update", `No se pudo comprobar la versión publicada: ${error}`);
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
    listen("self-update-progress", (event) => {
      if (!storeUpdateRunning) return;
      setStatus(event.payload.status || "Actualizando WinSlimCenter…", "var(--accent)");
      setProgress(event.payload.progress || 0);
    }),
    listen("msstore-scan-progress", (event) => reportMsStoreScanProgress(event.payload)),
    listen("downloads-changed", (event) => dispatchBackendEvent("downloads-changed", event)),
    listen("background-progress", (event) => dispatchBackendEvent("background-progress", event)),
    listen("install-finished", (event) => dispatchBackendEvent("install-finished", event)),
    listen("msstore-install-finished", (event) =>
      dispatchBackendEvent("msstore-install-finished", event)
    ),
  ];
  let bootstrapPromise;
  try {
    await Promise.all(listenerRegistrations);
    await waitForStartupConnection();
    // The snapshot begins only after all listeners are confirmed active.
    bootstrapPromise = invoke("get_bootstrap");
  } catch (error) {
    setStatus(`Error al registrar eventos: ${error}`, "var(--red)");
    void clientLog("error", "startup-listeners", String(error?.stack || error));
    dismissBootSplash();
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
    scheduleInlineSearches();
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
    // La barra de arriba busca en el catálogo propio. Escribir en ella desde la
    // Microsoft Store, que tiene su propio buscador, no puede quedarse sin
    // efecto: se vuelve al catálogo, que es lo que se estaba pidiendo.
    if (state.section === MSSTORE_SECTION && state.search.trim()) {
      state.section = "home";
      renderSidebar();
    }
    clearTimeout(searchTimer);
    searchTimer = setTimeout(renderContent, 180);
    // La Microsoft Store se pregunta aparte y más despacio: cada intento es un
    // viaje a sus servidores, no un filtro sobre lo que ya está en memoria.
    scheduleInlineSearches();
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
    updateBulkOperation(state.tasks);
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

  // Un producto de la Microsoft Store no está en el catálogo: no hay tarjeta
  // que actualizar ni estado que reconciliar, sólo el diálogo que el usuario
  // tiene delante y el aviso de que Windows ya lo tiene.
  backendEventHandlers.set("msstore-install-finished", async (ev) => {
    const { ok, product_id: productId, name, cancelled = false, error } = ev.payload || {};
    const id = String(productId || "").toUpperCase();
    // Lo que se pidió —instalar o actualizar— sólo lo sabe quien lo pidió, y es
    // lo que decide cómo se cuenta al terminar.
    const wasUpdate = state.msstore.installing[id] === "update";
    const attemptedFamily =
      state.msstore.updateFamilyByProduct[id] ||
      Object.entries(state.msstore.updates).find(
        ([, report]) => String(report?.product_id || "").toUpperCase() === id,
      )?.[0] ||
      null;
    delete state.msstore.installing[id];
    delete state.msstore.updateFamilyByProduct[id];
    // Igual que en el catálogo: dentro de una tanda esto es una fila, no un
    // diálogo. La versión que se sabía de la familia sí deja de valer ahora
    // mismo; volver a preguntar por ella espera al final, con todas.
    const bulkKey = msStoreTaskId(id);
    if (state.bulkUpdate?.byKey.has(bulkKey)) {
      const newerPackage = !ok && isMsStoreNewerPackageError(error);
      const suppressed =
        newerPackage && suppressMsStoreUpdate(attemptedFamily, msStoreUpdateOf(attemptedFamily));
      if (ok && attemptedFamily) delete state.msstore.updates[attemptedFamily];
      finishBulkItem(
        bulkKey,
        ok ? "done" : cancelled ? "cancelled" : suppressed ? "current" : "error",
        ok
          ? "Actualizada correctamente"
          : cancelled
            ? "Cancelada"
            : suppressed
              ? "Windows ya tiene una versión superior"
              : String(error || "Error desconocido"),
      );
      // Sin redibujar la rejilla: está detrás de un diálogo bloqueado y quien
      // cierra la tanda ya la rehace una vez, con todo lo que haya cambiado.
      settleBulkUpdate();
      return;
    }
    let launchFamily = null;
    if (ok) {
      // Windows acaba de registrar o sustituir el paquete: lo que se sabía de
      // su versión dejó de ser cierto en ese mismo instante. Las familias
      // afectadas se conocen por el producto —si vino de una búsqueda— o por lo
      // que ya se había comprobado de él, que es el caso de una actualización
      // lanzada desde «Aplicaciones instaladas».
      const families = new Set(
        (msStoreProductById(id)?.package_families || []).map((family) => family.toLowerCase()),
      );
      for (const [family, report] of Object.entries(state.msstore.updates)) {
        if (String(report?.product_id || "").toUpperCase() === id) families.add(family);
      }
      for (const family of families) delete state.msstore.updates[family];
      await ensureMsStoreInstalled({ force: true });
      const installedNow = [...families].filter((family) => msStoreInstalledByFamily(family));
      if (installedNow.length) {
        void scanMsStoreUpdates({ families: installedNow, quiet: true });
      }
      launchFamily =
        installedNow.find((family) => msStoreInstalledByFamily(family)?.launch_target) || null;
    }
    const shape = msStoreAppShape(msStoreProductById(id)) || {
      id: msStoreTaskId(id),
      name: name || id,
    };
    const ownsModal = state.operationAppId === msStoreTaskId(id);
    if (ownsModal) state.operationAppId = null;

    if (ok) {
      setTransientStatus(
        `${wasUpdate ? "Actualizado" : "Instalado"}: ${shape.name}`,
        "var(--green)",
        5000,
      );
      if (ownsModal) {
        showOperationCompleted(shape, {
          canLaunch: !!launchFamily,
          isUpdate: wasUpdate,
          changed: true,
          onLaunch: launchFamily ? () => void launchMsStoreApp(launchFamily) : null,
        });
      }
    } else {
      if (ownsModal) closeModal();
      if (cancelled) {
        setTransientStatus(`Instalación cancelada: ${shape.name}`, "var(--text-light)", 5000);
      } else {
        const newerPackage = wasUpdate && isMsStoreNewerPackageError(error);
        const report = attemptedFamily ? msStoreUpdateOf(attemptedFamily) : null;
        const suppressed = newerPackage && suppressMsStoreUpdate(attemptedFamily, report);
        setTransientStatus(
          suppressed
            ? `Actualización retirada de pendientes: ${shape.name}`
            : `Error al instalar ${shape.name}`,
          suppressed ? "var(--accent)" : "var(--red)",
          5000,
        );
        showMsStoreInstallError(error, {
          name: shape.name,
          newerPackage,
          suppressed,
        });
      }
    }
    renderSidebar();
    renderContent();
  });

  backendEventHandlers.set("install-finished", async (ev) => {
    const {
      ok,
      app_id,
      error,
      changed = true,
      is_update = false,
      pending_restart = false,
      cancelled = false,
      cancellation_kind = "",
      interrupted = false,
    } = ev.payload;
    delete state.busy[app_id];
    // Clearing the optimistic busy flag is itself a visible state change even
    // when installation changed no version and the later WinGet check fails.
    updateVisibleAppActions(new Set([app_id]));
    // Un paquete de WinGet o de Chocolatey no tiene tarjeta en el catálogo, así
    // que no hay nada que `updateVisibleAppActions` pueda repintar: su
    // subsección se vuelve a dibujar entera. La entrada sigue apuntada hasta el
    // final del manejador, que todavía pregunta por ella para dar el nombre.
    if (derived.repoPackages.has(app_id)) renderContent();
    // Una aplicación de la tanda no tiene diálogo propio ni avisos propios: lo
    // que le pasó se cuenta en su fila, y el recuento de lo instalado se pone
    // al día una sola vez cuando terminan todas, no una vez por cada una.
    if (state.bulkUpdate?.byKey.has(app_id)) {
      const installing = state.bulkUpdate.mode === "install";
      finishBulkItem(
        app_id,
        ok ? (changed || pending_restart ? "done" : "current") : cancelled ? "cancelled" : "error",
        ok
          ? pending_restart
            ? "Descargada e instalada · ciérrala y vuelve a abrirla para aplicarla"
            : changed
              ? installing ? "Instalada correctamente" : "Actualizada correctamente"
              : installing ? "Ya estaba instalada" : "Ya estaba en su última versión"
          : cancelled
            ? "Cancelada"
            : String(error || "Error desconocido"),
      );
      settleBulkUpdate();
      return;
    }
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
        const message = pending_restart
          ? `${app.name}: cierra la aplicación para aplicar la actualización`
          : is_update
            ? (changed ? `Actualizado: ${app.name}` : `${app.name} ya estaba actualizado`)
            : `Instalado: ${app.name}`;
        setTransientStatus(message, "var(--green)", 5000);
      }
      if (ownsModal) {
        showOperationCompleted(app, {
          canLaunch: !!appStatus(app_id).can_launch,
          isUpdate: is_update,
          changed,
          pendingRestart: pending_restart,
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
    replaceRepoEntries(data.repo_entries || []);
    replaceStatuses(data.statuses || {});
    // El escaneo del equipo empieza en cuanto se pinte la tienda; hasta que
    // conteste, lo que nadie sabía se dibuja como «comprobando».
    state.verifyingStatuses = true;
    state.statusPending = new Set(data.status_pending || []);
    state.settings = data.settings || state.settings;
    replaceTasks(data.tasks || []);
    applyTheme(state.settings.theme, state.settings.accent);
    renderAppVersion();
    renderSidebar();
    renderContent();
    renderDlPanel();
    dismissBootSplash();
    setStatus(idleStatusSummary(), "var(--green)");
    clientLog("info", "startup", {
      catalog: state.catalog.length,
      installed: installedCount(),
      pending: state.statusPending.size,
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
    void finishStartupInBackground();
  } catch (e) {
    // Sin escaneo en marcha no hay respuesta que esperar.
    if (!statusVerification) finishStatusVerification();
    dismissBootSplash();
    setStatus(`Error al iniciar: ${e}`, "var(--red)");
    clientLog("error", "startup", String(e?.stack || e));
    console.error(e);
  }
});
