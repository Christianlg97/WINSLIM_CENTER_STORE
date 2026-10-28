const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../src/main.js'), 'utf8').replace(/\r\n/g, '\n');

function load(context, names) {
  for (const name of names) {
    const start = source.search(new RegExp(`(?:async )?function ${name}\\(`));
    assert.ok(start >= 0, name);
    vm.runInContext(source.slice(start, source.indexOf('\n}', start) + 2), context);
  }
}

async function cancellation(kind, reject = false) {
  let acknowledge, refuse;
  const calls = [];
  const ctx = vm.createContext({
    state: { busy: {}, finished: new Set(), settings: {}, msstore: { installing: {}, updateFamilyByProduct: {} } },
    BULK_STATE_LABELS: {}, Date, Map, Set,
    hideShellBusy() {}, showBulkUpdateModal() {}, renderContent() {}, clientLog() {},
    updateBulkUpdateModal() {}, updateVisibleAppActions() {}, settleBulkUpdate() {},
    isBulkItemFinal: i => ['done', 'current', 'error', 'cancelled'].includes(i.state),
    invoke: name => { calls.push(name); return new Promise((resolve, fail) => { acknowledge = resolve; refuse = fail; }); },
    invokeDownloadAction: async name => { calls.push(name); },
  });
  load(ctx, ['startBulkUpdate', 'finishBulkItem', 'cancelBulkUpdate']);
  const pending = ctx.startBulkUpdate([
    { key: 'demo', appId: 'demo', productId: 'demo', name: 'Demo', kind, app: {} },
    { key: 'later', appId: 'later', name: 'Later', kind: 'catalog', app: {} },
  ]);
  await ctx.cancelBulkUpdate();
  assert.notEqual(ctx.state.bulkUpdate.items[0].state, 'cancelled', 'in-flight request must await the backend');
  assert.equal(ctx.state.bulkUpdate.items[1].state, 'cancelled');
  if (reject) refuse(new Error('rejected')); else acknowledge();
  await pending;
  assert.equal(calls.length, reject ? 1 : 2);
  if (!reject) {
    assert.equal(calls[1], 'cancel_download');
    assert.notEqual(ctx.state.bulkUpdate.items[0].state, 'cancelled', 'only completion confirms cancellation');
  } else assert.equal(ctx.state.bulkUpdate.items[0].state, 'error');
}

async function portableUpdate(policy, blocker) {
  let confirmation;
  let request;
  const app = { id: 'demo', name: 'Demo', variants: { default: 'avx2' } };
  const ctx = vm.createContext({
    Map, Set, state: { settings: { variants: { demo: 'sse3' } }, busy: {}, finished: new Set() },
    portableBackupChoices: new Map(),
    findApp: () => app, appStatus: () => ({ installed: true, update_available: true }),
    invoke: async (name, args) => {
      if (name === 'portable_data_info') return { backup_root: 'Backups' };
      if (name === 'install_app') request = args;
    },
    invokeShared: async name => name === 'portable_data_info' ? { backup_root: 'Backups' } : null,
    blockingRunningApp: async () => blocker ? { packaged: false } : null,
    showConfirmModal: options => { confirmation = options; },
    showRunningAppModal: options => { options.onClose(); },
    updateVisibleAppActions() {}, renderDlPanel() {}, showPackageOperationModal() {},
  });
  load(ctx, ['portableDataChoices', 'portableDataMessage', 'installApp']);
  await ctx.installApp('demo', true);
  assert.equal(confirmation.choices.selected, 'preserve');
  assert.match(confirmation.message, /Backups/);
  await confirmation.onConfirm(policy);
  assert.equal(request.portableData, policy);
  assert.equal(request.variant, 'sse3', 'data choices must not change the remembered build');
  assert.equal(request.closeRunning, blocker);
}

async function updaterFailure() {
  let reject, calls = 0;
  const errors = [];
  const ctx = vm.createContext({
    invoke: () => { calls++; return new Promise((_, fail) => { reject = fail; }); },
    setStatus() {}, setProgress() {}, showAlertModal: (title, message) => errors.push(message),
    storeOperationsInProgress: () => false,
  });
  vm.runInContext('let storeUpdateRunning = false;', ctx);
  load(ctx, ['startStoreUpdate']);
  const first = ctx.startStoreUpdate();
  await ctx.startStoreUpdate();
  assert.equal(calls, 1, 'a second click must not start another self update');
  reject(new Error('download interrupted'));
  await first;
  assert.match(errors[0], /download interrupted/);
  assert.equal(vm.runInContext('storeUpdateRunning', ctx), false);
}

const tick = () => new Promise(resolve => setImmediate(resolve));

function busyStore() {
  return {
    tasks: [{ app_id: 'demo', state: 'installing' }],
    busy: { demo: 'updating' },
    bulkUpdate: null,
    msstore: { installing: {}, busy: {} },
  };
}

// Accepting the store update while an installation runs used to fail in the
// backend. It now waits for the installation and then updates.
async function updaterWaitsForRunningOperations() {
  const calls = [];
  const statuses = [];
  const state = busyStore();
  const ctx = vm.createContext({
    state, ACTIVE_TASK_STATES: new Set(['queued', 'downloading', 'paused', 'installing', 'cancelling']),
    invoke: async name => { calls.push(name); return 'ok'; },
    setStatus: text => statuses.push(text), setProgress() {}, showAlertModal() {}, clientLog() {},
    setTimeout: fn => setImmediate(fn),
  });
  vm.runInContext('let storeUpdateRunning = false;', ctx);
  load(ctx, ['storeOperationsInProgress', 'whenStoreIdle', 'startStoreUpdate']);
  const pending = ctx.startStoreUpdate();
  await tick(); await tick();
  assert.equal(calls.length, 0, 'the store must not close over a running installation');
  assert.match(statuses[0], /al terminar las operaciones en curso/);
  // Opening an application is not something to wait for; the installation is.
  state.busy.demo = 'launching';
  state.tasks[0].state = 'done';
  await pending;
  assert.deepEqual(calls, ['update_center_app']);
}

// The start-up offer used to replace the locked dialog of an installation the
// user had started during the scan. It now waits for the installation and for
// the dialog in front to close; asked for from About, it shows straight away.
async function startupOfferWaitsForIdle() {
  const shown = [];
  let modalHidden = false;
  const state = busyStore();
  const ctx = vm.createContext({
    state, ACTIVE_TASK_STATES: new Set(['queued', 'downloading', 'paused', 'installing', 'cancelling']),
    invoke: async () => ({ version: '9.9.9', current: '1.0.0' }),
    clientLog() {}, showStoreUpdateModal: update => shown.push(update.version),
    document: { getElementById: () => ({ classList: { contains: () => modalHidden } }) },
    setTimeout: fn => setImmediate(fn),
  });
  vm.runInContext('let storeUpdateRunning = false; let storeUpdateOffered = false;', ctx);
  load(ctx, ['storeOperationsInProgress', 'modalIsOpen', 'whenStoreIdle', 'runStoreUpdateCheck']);

  const pending = ctx.runStoreUpdateCheck({ waitForIdle: true });
  await tick(); await tick();
  assert.equal(shown.length, 0, 'the offer must not cover a running installation');
  state.tasks[0].state = 'done';
  delete state.busy.demo;
  await tick(); await tick();
  assert.equal(shown.length, 0, 'the offer waits for the completion dialog to close');
  modalHidden = true;
  await pending;
  assert.deepEqual(shown, ['9.9.9']);

  state.busy.demo = 'uninstalling';
  await ctx.runStoreUpdateCheck();
  assert.equal(shown.length, 2, 'a manual check answers at once');

  const repeated = ctx.runStoreUpdateCheck({ waitForIdle: true });
  await tick();
  vm.runInContext('storeUpdateOffered = true;', ctx);
  delete state.busy.demo;
  await repeated;
  assert.equal(shown.length, 2, 'an offer already made is not repeated after waiting');
}

async function queuedInstall() {
  const requests = [];
  const ctx = vm.createContext({
    state: { busy: {}, finished: new Set(), settings: { variants: { thorium: 'sse3' } }, msstore: { installing: {}, updateFamilyByProduct: {} } },
    BULK_STATE_LABELS: {}, Date, Map, Set,
    hideShellBusy() {}, showBulkUpdateModal() {}, renderContent() {}, clientLog() {},
    updateBulkUpdateModal() {}, updateVisibleAppActions() {}, settleBulkUpdate() {},
    isBulkItemFinal: i => ['done', 'current', 'error', 'cancelled'].includes(i.state),
    invoke: async (name, args) => { requests.push({ name, args }); },
    invokeDownloadAction: async () => {},
  });
  load(ctx, ['startBulkUpdate']);
  await ctx.startBulkUpdate([
    { key: 'steam', appId: 'steam', name: 'Steam', kind: 'catalog', app: { id: 'steam' } },
    { key: 'thorium', appId: 'thorium', name: 'Thorium', kind: 'catalog', app: { id: 'thorium', variants: { default: 'avx2' } } },
  ], 'install');
  assert.equal(ctx.state.bulkUpdate.mode, 'install');
  assert.deepEqual(requests.map(r => r.args.appEntry.id), ['steam', 'thorium'], 'the queue keeps the order chosen');
  for (const { args } of requests) {
    assert.equal(args.forceUpdate, false, 'a queued install is not an update');
    assert.equal(args.closeRunning, false, 'nothing uninstalled can be running');
  }
  assert.equal(requests[1].args.variant, 'sse3', 'the remembered build is installed');
  assert.equal(ctx.state.busy.steam, 'installing');
}

function queueEligibility() {
  const catalog = [
    { id: 'steam', source_type: 'wget' },
    { id: 'epic', source_type: 'wget' },
    { id: 'office', source_type: 'web' },
    { id: 'word', source_type: 'component' },
    { id: 'installed', source_type: 'winget' },
    { id: 'queued', source_type: 'wget' },
  ];
  const ctx = vm.createContext({
    Map, Set,
    state: {
      busy: { epic: 'installing' },
      statuses: { installed: { installed: true } },
      bulkUpdate: { finished: false, byKey: new Map([['queued', {}]]) },
    },
    derived: { catalogById: new Map(catalog.map(app => [app.id, app])), taskByAppId: new Map() },
  });
  load(ctx, ['isQueueable', 'isBulkUpdateRunning', 'appStatus', 'isStatusPending']);
  assert.deepEqual(catalog.filter(app => ctx.isQueueable(app)).map(app => app.id), ['steam']);
  assert.equal(ctx.isQueueable({ id: 'winget-demo', source_type: 'winget' }), false, 'repository results have no catalog card');
  ctx.state.verifyingStatuses = true;
  ctx.state.statusPending = new Set(['steam']);
  assert.equal(ctx.isQueueable(catalog[0]), false, 'nothing is queued before the machine has answered for it');
}

// Arranque sin pantalla de carga: lo que nadie sabía espera con una silueta, y
// lo que se pulsa durante el escaneo se decide con su respuesta.
async function startupVerification() {
  const app = { id: 'steam', name: 'Steam', source_type: 'wget' };
  const ran = [];
  let answer;
  const ctx = vm.createContext({
    Map, Set,
    state: { busy: {}, statuses: { steam: { installed: false } }, verifyingStatuses: true, statusPending: new Set(['steam']) },
    derived: { taskByAppId: new Map() },
    escapeHtml: String,
    findApp: () => app,
    updateVisibleAppActions() {}, renderContent() {},
    installApp: (id, update) => ran.push(update ? `update:${id}` : `install:${id}`),
    uninstallApp: id => ran.push(`uninstall:${id}`),
    launchApp: id => ran.push(`launch:${id}`),
  });
  vm.runInContext('let statusVerification = null;', ctx);
  load(ctx, ['isStatusPending', 'appStatus', 'actionButtons', 'runAppAction', 'afterStatusesVerified']);
  assert.match(ctx.actionButtons(app), /btn-skeleton/, 'an unanswered card shows a placeholder, not Install');
  assert.doesNotMatch(ctx.actionButtons(app), /data-install=/);

  // Seeded from the previous session: offered, but held until the scan says.
  ctx.state.statusPending = new Set();
  assert.match(ctx.actionButtons(app), /data-install="steam"/);
  vm.runInContext('statusVerification = new Promise(resolve => { globalThis.__answer = resolve; });', ctx);
  answer = ctx.__answer;
  const button = { disabled: false, textContent: 'Instalar', setAttribute() {}, isConnected: false };
  const click = ctx.afterStatusesVerified('install', 'steam', button);
  assert.equal(button.disabled, true);
  assert.equal(ran.length, 0, 'nothing runs before the scan answers');
  // The scan finds it already installed: Install no longer applies.
  ctx.state.statuses.steam = { installed: true, can_launch: true };
  answer({});
  await click;
  assert.deepEqual(ran, [], 'an action the scan disproved is dropped');

  // Still offered after the answer: it runs.
  vm.runInContext('statusVerification = Promise.resolve({});', ctx);
  await ctx.afterStatusesVerified('launch', 'steam');
  assert.deepEqual(ran, ['launch:steam']);
  vm.runInContext('statusVerification = null;', ctx);
  await ctx.afterStatusesVerified('uninstall', 'steam');
  assert.deepEqual(ran, ['launch:steam', 'uninstall:steam']);
}

function restoredRepositories() {
  const external = { id: 'winget-demo', name: 'Demo', winget_id: 'Vendor.Demo' };
  const ctx = vm.createContext({
    state: { catalog: [], repoEntries: [] },
    derived: { catalogById: new Map(), repoPackages: new Map() },
    repoPackageByAppId: () => null,
  });
  load(ctx, ['replaceRepoEntries', 'knownApps', 'findApp']);
  ctx.replaceRepoEntries([external]);
  assert.equal(ctx.findApp(external.id), external);
  assert.equal(ctx.knownApps().length, 1);
  ctx.state.catalog = [{ id: 'curated-demo', winget_id: 'Vendor.Demo' }];
  assert.equal(ctx.knownApps().length, 1, 'curated packages must not be duplicated');
}

// «Buscar actualizaciones» durante la consulta del arranque espera a ésa en vez
// de repetir las mismas consultas a la Microsoft Store después.
async function msStoreScanReuse() {
  const runs = [];
  const ctx = vm.createContext({
    state: { msstore: { ring: 'Retail', arch: 'auto', scanning: false } },
    renderContent() {}, renderSidebar() {},
    runMsStoreUpdatesScan: options => new Promise(resolve => runs.push({ options, resolve })),
  });
  vm.runInContext('let msStoreScanInFlight = null; let msStoreFullScanInFlight = null;', ctx);
  load(ctx, ['scanMsStoreUpdates']);
  const startup = ctx.scanMsStoreUpdates({ quiet: true });
  await Promise.resolve();
  const button = ctx.scanMsStoreUpdates({ quiet: true });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(runs.length, 1, 'a full scan in flight is reused, not queued again');
  runs[0].resolve();
  await Promise.all([startup, button]);
  assert.equal(runs.length, 1);
  assert.equal(ctx.state.msstore.scanning, false);

  // A different channel is a different question: it waits its turn and runs.
  const first = ctx.scanMsStoreUpdates({ quiet: true });
  ctx.state.msstore.ring = 'RP';
  const other = ctx.scanMsStoreUpdates({ quiet: true });
  await new Promise(resolve => setImmediate(resolve));
  runs[1].resolve();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(runs.length, 3, 'another channel is checked after the running scan');
  runs[2].resolve();
  await Promise.all([first, other]);
  // And a check of a few families is never answered by a full one.
  const subset = ctx.scanMsStoreUpdates({ quiet: true, families: ['Demo_abc'] });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(runs[3].options.families, ['Demo_abc']);
  runs[3].resolve();
  await subset;
}

// Las aplicaciones web preguntan una vez con qué navegador abrirse, lo recuerdan
// y, sin ninguno instalado, lo dicen en lugar de fallar.
async function webBrowserChoice() {
  const thorium = 'C:\\Program Files\\Thorium\\Application\\thorium.exe';
  const firefox = 'C:\\Program Files\\Mozilla Firefox\\firefox.exe';
  let setup = {
    browsers: [
      { name: 'Thorium', path: thorium, app_mode: true, windows_default: true },
      { name: 'Mozilla Firefox', path: firefox, app_mode: false, windows_default: false },
    ],
    selected: null,
  };
  const modals = [];
  const saved = [];
  const ctx = vm.createContext({
    state: {},
    invoke: async (name, args) => {
      if (name === 'web_browsers') return setup;
      if (name === 'set_web_browser') saved.push(args.path);
    },
    showConfirmModal: options => modals.push(options),
    showAlertModal() {}, renderSidebar() {}, renderContent() {}, clientLog() {},
  });
  load(ctx, ['webBrowserSetup', 'webBrowserChoices', 'showNoBrowserModal', 'rememberWebBrowser', 'ensureWebBrowser']);

  assert.equal(ctx.webBrowserChoices(setup).selected, thorium, "Windows' default is the suggestion");
  assert.equal(ctx.webBrowserChoices({ ...setup, selected: firefox }).selected, firefox, 'the store choice wins');

  const asked = ctx.ensureWebBrowser();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(modals.length, 1, 'asked the first time');
  await modals[0].onConfirm(firefox);
  assert.equal(await asked, true);
  assert.deepEqual(saved, [firefox]);

  setup = { ...setup, selected: firefox };
  assert.equal(await ctx.ensureWebBrowser(), true);
  assert.equal(modals.length, 1, 'remembered: not asked again');

  setup = { browsers: [], selected: null };
  assert.equal(await ctx.ensureWebBrowser(), false);
  assert.equal(modals[1].title, 'No hay ningún navegador');
}

// Cada cifra cuenta lo que se ve: ni los componentes de una suite que no está
// instalada ni, dos veces, un paquete de WinGet que el catálogo ya tiene.
function visibleCounters() {
  const catalog = [
    { id: 'steam', source_type: 'wget' },
    { id: 'demo', source_type: 'winget', winget_id: 'Vendor.Demo' },
    { id: 'word', source_type: 'component', component_of: 'microsoft_365' },
  ];
  const ctx = vm.createContext({
    state: {
      catalog,
      repoEntries: [{ id: 'winget-vendor.demo', winget_id: 'Vendor.Demo' }],
      statuses: {
        steam: { installed: true, update_available: true },
        demo: { installed: true, update_available: true },
        'winget-vendor.demo': { installed: true, update_available: true },
        word: { installed: false },
      },
    },
    msStoreUpdatesList: () => [{ family: 'Contoso.Notes_abc' }],
  });
  load(ctx, ['appStatus', 'componentVisible', 'knownApps', 'visibleCatalogCount', 'visibleKnownApps', 'installedCount', 'updatesCount']);
  assert.equal(ctx.visibleCatalogCount(), 2, 'a suite component is not shown, so it is not counted');
  assert.equal(ctx.installedCount(), 2, 'the WinGet duplicate of a catalog entry counts once');
  assert.equal(ctx.updatesCount(), 3, 'two from the catalog and one from the Microsoft Store');
  ctx.state.statuses.word = { installed: true };
  assert.equal(ctx.visibleCatalogCount(), 3, 'shown once its suite is installed');
}

// Ni «vlatest» ni «vv2.0»: la última versión se dice con palabras y un número
// lleva su «v» una sola vez.
function versionLabels() {
  const ctx = vm.createContext({});
  load(ctx, ['versionLabel']);
  assert.equal(ctx.versionLabel('latest'), 'Última versión');
  assert.equal(ctx.versionLabel(''), 'Última versión');
  assert.equal(ctx.versionLabel(undefined), 'Última versión');
  assert.equal(ctx.versionLabel('2.39'), 'v2.39');
  assert.equal(ctx.versionLabel('v2.0'), 'v2.0');
  assert.equal(ctx.versionLabel('V3.1'), 'v3.1');
  assert.equal(ctx.versionLabel('nightly'), 'Nightly');
  assert.equal(ctx.versionLabel('LTS'), 'LTS');
  assert.equal(ctx.versionLabel('Unknown', ''), '', 'WinGet without a version says nothing');
  assert.equal(ctx.versionLabel('latest', 'versión desconocida'), 'versión desconocida');
}

async function splashPresentation() {
  for (const file of ['../src/index.html', 'ui_harness.html']) {
    const html = fs.readFileSync(path.join(__dirname, file), 'utf8');
    if (file === '../src/index.html') {
      assert.ok(
        html.indexOf('window.__bootI18nReady = new Promise') < html.indexOf('id="boot-splash"'),
        'El idioma del arranque debe empezar antes de crear la pantalla de carga',
      );
      assert.match(html, /window\.__i18nReady = setLanguagePreference\(cachedLanguagePreference\(\)\)/);
    }
    const script = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)]
      .map(match => match[1]).find(text => text.includes('window.__bootSplashShown ='));
    assert.ok(script, file);
    for (const fails of [false, true]) {
      const frames = [], events = [], timers = [];
      let languageReady;
      const splash = {
        querySelector: () => ({ complete: true }),
        getBoundingClientRect: () => events.push('layout'),
        classList: { add: name => events.push(name) },
      };
      const window = {
        __bootI18nReady: new Promise(resolve => { languageReady = resolve; }),
        __TAURI__: { core: { invoke: () => {
          events.push('show');
          return fails ? Promise.reject(new Error('fallback')) : Promise.resolve();
        } } },
      };
      vm.runInNewContext(script, { window, document: { getElementById: () => splash, fonts: { load: () => Promise.resolve() } },
        performance: { now: () => 42 }, setTimeout: (callback, delay) => timers.push({ callback, delay }), requestAnimationFrame: callback => frames.push(callback) });
      // Los recursos visuales no bastan: el idioma debe estar preparado.
      for (let turn = 0; turn < 12; turn++) await Promise.resolve();
      assert.deepEqual(events, []);
      assert.equal(frames.length, 0);
      languageReady();
      for (let turn = 0; turn < 12; turn++) await Promise.resolve();
      assert.deepEqual(events, ['layout', 'show'], 'Preparar la capa antes de mostrar la ventana');
      assert.equal(frames.length, 1);
      frames.shift()();
      assert.deepEqual(events, ['layout', 'show']);
      assert.equal(frames.length, 1);
      frames.shift()();
      await window.__bootSplashShown;
      assert.deepEqual(events, ['layout', 'show', 'is-presented']);
      assert.equal(window.__bootSplashShownAt, 42);
      assert.equal(timers.some(timer => timer.delay === 150), false);
    }
  }
}

function modalPresentationBeforeFocus() {
  const classes = new Set(), observed = [];
  const modal = {
    classList: { toggle(name, enabled) { enabled ? classes.add(name) : classes.delete(name); } },
    removeAttribute() {},
    querySelector: selector => selector === 'h2' ? null : { focus: options => observed.push({ classes: [...classes], options }) },
  };
  const backdrop = { classList: { contains: () => true, remove: () => observed.push({ classes: [...classes], shown: true }) } };
  const gear = { expanded: null, setAttribute(name, value) { if (name === 'aria-expanded') this.expanded = value; } };
  const elements = { modal, 'modal-backdrop': backdrop, 'btn-settings': gear };
  const ctx = vm.createContext({ document: { getElementById: id => elements[id] || null, activeElement: {} }, closeShotViewer() {} });
  load(ctx, ['openModal']);
  ctx.openModal('about', false, false, 'about');
  assert.ok(observed[0].shown);
  assert.ok(observed[0].classes.includes('modal-about'));
  assert.ok(observed[0].classes.includes('modal-hero'));
  assert.equal(observed[1].options.preventScroll, true);
  assert.equal(gear.expanded, 'true', 'About opens from the settings, whose button stays active');
  ctx.openModal('settings', false, false, 'settings');
  assert.ok(classes.has('modal-settings') && classes.has('modal-hero'));
  assert.equal(classes.has('modal-about'), false);
  ctx.openModal('plain');
  assert.equal(classes.has('modal-about'), false);
  assert.equal(classes.has('modal-hero'), false);
  assert.equal(classes.has('modal-settings'), false);
  assert.equal(gear.expanded, 'false');
}

function modalBackdropIsStatic() {
  const css = fs.readFileSync(path.join(__dirname, '../src/styles.css'), 'utf8');
  assert.match(
    css,
    /\.modal-backdrop\.hidden\s*\{[^}]*display:\s*grid\s*!important;[^}]*pointer-events:\s*none;/s,
    'El fondo oculto debe seguir montado pero no interceptar clics',
  );
  assert.match(css, /\.modal-backdrop\.hidden\s*>\s*\.modal\s*\{[^}]*display:\s*none;/s);
  assert.match(
    css,
    /\.modal-backdrop\.hidden::before,\s*\.modal-backdrop\.hidden::after\s*\{[^}]*opacity:\s*0;/s,
  );
  assert.doesNotMatch(
    css,
    /\.modal-backdrop(?:\.hidden|::before|::after)?[^\{]*\{[^}]*(?:animation|transition)\s*:/s,
    'El velo y el desenfoque no deben animarse',
  );
}

async function splashFadeCompletesBeforeRemoval() {
  for (const duration of ['320ms', '0.32s']) {
    const timers = [], cleared = [], listeners = new Map();
    let removed = 0;
    const splash = { dataset: {}, classList: { add: name => assert.equal(name, 'done') },
      addEventListener: (name, callback) => listeners.set(name, callback),
      removeEventListener: name => listeners.delete(name), remove: () => removed++ };
    const ctx = vm.createContext({ document: { getElementById: () => splash }, window: {},
      afterFirstPaint: () => Promise.resolve(), performance: { now: () => 0 }, BOOT_SPLASH_MIN_MS: 750,
      getComputedStyle: () => ({ animationDuration: duration }),
      setTimeout: (callback, delay) => { timers.push({ callback, delay }); return timers.length; },
      clearTimeout: id => cleared.push(id) });
    load(ctx, ['dismissBootSplash']);
    ctx.dismissBootSplash();
    ctx.dismissBootSplash();
    for (let turn = 0; turn < 6; turn++) await Promise.resolve();
    assert.equal(timers.length, 1, 'El cierre repetido no reinicia la salida');
    timers[0].callback();
    assert.equal(removed, 0, 'La capa sigue presente durante el fundido');
    const end = listeners.get('animationend');
    end({ target: {}, animationName: 'splashContentIn' });
    assert.equal(removed, 0, 'Una animación del contenido no corta la salida');
    assert.equal(timers[1].delay, 420, 'El respaldo usa la duración CSS más un margen');
    end({ target: splash, animationName: 'bootSplashOut' });
    assert.equal(removed, 1);
    assert.ok(cleared.includes(2));
    assert.equal(listeners.has('animationend'), false);
  }
}

function externalInstalledSearch() {
  const app = { family: 'king.com.CandyCrushSodaSaga_abc', version: '1.329.300.0',
    name: 'king.com.CandyCrushSodaSaga', display_name: 'Candy Crush Soda Saga',
    can_uninstall: true, launch_target: 'shell:AppsFolder/candy' };
  const store = { installed: [app], installedLoaded: true, updates: {}, busy: {}, installing: {} };
  const ctx = vm.createContext({ state: { msstore: store }, String, escapeHtml: String,
    msStoreAppShape: product => ({ name: product.title }), pickAccent: () => '#ccc', avatarBackground: () => '', renderAvatar: () => '',
    MSSTORE_KIND_LABELS: { appx: 'Paquete de Windows' }, versionLabel: value => value,
  });
  load(ctx, ['msStoreInstalledByFamily', 'msStoreInstalledRecord', 'msStoreUpdateOf',
    'msStoreOriginTag', 'msStoreVersionBadge', 'msStoreInstalledActions', 'msStoreCardHtml']);
  const product = { product_id: '9CANDY', title: 'Candy Crush Soda Saga', kind: 'appx',
    installed: false, package_families: ['KING.COM.CANDYCRUSHSODASAGA_ABC'] };
  assert.equal(ctx.msStoreInstalledRecord(product), app, 'Cruzar la familia aunque la marca de búsqueda sea anterior');
  const html = ctx.msStoreCardHtml(product, 0);
  assert.match(html, /Ya instalado/); assert.match(html, /data-ms-launch=/); assert.match(html, /data-ms-uninstall=/);
  assert.doesNotMatch(html, /data-ms-install=/);
  store.updates[app.family.toLowerCase()] = { product_id: '9candy' };
  assert.equal(ctx.msStoreInstalledRecord({ product_id: '9CANDY' }), app, 'Reutilizar la identidad aprendida al comprobar versiones');
  assert.equal(ctx.msStoreInstalledRecord({ product_id: '9OTHER', title: product.title }), null, 'El nombre no prueba identidad');
  store.installed = [];
  assert.equal(ctx.msStoreInstalledRecord({ ...product, installed: true, installed_family: app.family }), null, 'La desinstalación local invalida marcas viejas');
  assert.match(ctx.msStoreCardHtml(product, 0), /data-ms-install=/);
  store.installedLoaded = false;
  assert.equal(ctx.msStoreInstalledRecord({ ...product, installed: true, installed_family: app.family }).family, app.family);
}

async function syncFadeCompletesBeforeRemoval() {
  for (const fallback of [false, true]) {
    let end, removed = 0, timer, cleared = false;
    const overlay = { classList: { add: name => assert.equal(name, 'is-leaving') },
      addEventListener: (name, fn) => { end = fn; }, removeEventListener() {}, remove: () => { removed++; } };
    const ctx = vm.createContext({ syncModalOverlay: overlay, Promise,
      getComputedStyle: () => ({ animationDuration: '0.11s' }),
      setTimeout: (fn, delay) => { assert.equal(delay, 210); timer = fn; return 1; },
      clearTimeout: () => { cleared = true; },
    });
    load(ctx, ['closeSyncModal']); const closing = ctx.closeSyncModal();
    assert.equal(removed, 0, 'La capa sigue visible durante el fundido');
    end({ target: {}, animationName: 'bootSplashOut' }); assert.equal(removed, 0);
    const newerOverlay = {}; ctx.syncModalOverlay = newerOverlay;
    if (fallback) timer(); else end({ target: overlay, animationName: 'bootSplashOut' });
    await closing;
    assert.equal(removed, 1); assert.ok(cleared);
    assert.equal(ctx.syncModalOverlay, newerOverlay, 'La salida no retira una capa posterior');
  }
}

async function startupConnectionGate() {
  const listeners = new Map(), attributes = new Map(), classes = new Set();
  let click, calls = 0, pending, removed = false;
  const subtitle = { textContent: 'Preparando la tienda…' };
  const label = {}, help = {};
  const button = { disabled: false, querySelector: () => label, focus() {},
    addEventListener: (name, fn) => { click = fn; } };
  const panel = { hidden: false, setAttribute() {},
    querySelector: selector => selector === 'button' ? button : help,
    remove() { removed = true; } };
  const copy = { querySelector: () => subtitle, removeAttribute() {} };
  const splash = { querySelector: () => copy, appendChild() {},
    classList: { add: (...names) => names.forEach(name => classes.add(name)), remove: (...names) => names.forEach(name => classes.delete(name)) },
    setAttribute: (name, value) => attributes.set(name, value), removeAttribute: name => attributes.delete(name) };
  const app = { inert: false }, navigator = { onLine: false };
  const ctx = vm.createContext({ Promise, String, navigator,
    document: { getElementById: id => id === 'app' ? app : splash, createElement: () => panel },
    window: { addEventListener: (name, fn) => listeners.set(name, fn), removeEventListener: name => listeners.delete(name) },
    invoke: name => { assert.equal(name, 'check_connection'); calls++; return new Promise(resolve => { pending = resolve; }); },
    clientLog() {},
  });
  load(ctx, ['waitForStartupConnection']);
  let ready = false;
  const gate = ctx.waitForStartupConnection().then(() => { ready = true; });
  await Promise.resolve();
  assert.equal(calls, 0, 'Sin adaptador no hace falta esperar a la red');
  assert.ok(app.inert); assert.ok(classes.has('is-connection-pending'));
  assert.ok(classes.has('is-offline')); assert.equal(panel.hidden, false); assert.equal(ready, false);
  navigator.onLine = true;
  const retry = click();
  click(); listeners.get('online')();
  assert.equal(calls, 1, 'No duplicar peticiones al pulsar repetidamente');
  assert.ok(button.disabled); pending(false); await retry;
  assert.equal(button.disabled, false); assert.equal(ready, false);
  assert.match(help.textContent, /Seguimos sin conexión/);
  const recovery = listeners.get('online')();
  pending(true); await recovery; await gate;
  assert.equal(calls, 2); assert.ok(ready); assert.ok(removed);
  assert.equal(app.inert, false); assert.equal(listeners.size, 0);
  assert.equal(classes.has('is-offline'), false); assert.equal(classes.has('is-connection-pending'), false);
  assert.equal(subtitle.textContent, 'Preparando la tienda…');
}

async function sharedReadRequests() {
  let calls = 0, complete;
  const ctx = vm.createContext({
    Map, JSON, Promise,
    invoke: () => {
      calls += 1;
      if (calls === 1) return new Promise(resolve => { complete = resolve; });
      return Promise.resolve(`respuesta-${calls}`);
    },
  });
  vm.runInContext('const sharedInvokeRequests = new Map();', ctx);
  load(ctx, ['invokeShared']);
  const first = ctx.invokeShared('msstore_search', { query: 'demo' }, 'demo');
  const duplicate = ctx.invokeShared('msstore_search', { query: 'demo' }, 'demo');
  assert.equal(first, duplicate, 'Las lecturas idénticas deben compartir la petición en curso');
  assert.equal(calls, 1);
  complete({ products: [] });
  await Promise.all([first, duplicate]);
  assert.equal(await ctx.invokeShared('msstore_search', { query: 'demo' }, 'demo'), 'respuesta-2');
  assert.equal(calls, 2, 'Al terminar no debe quedar una caché que oculte cambios posteriores');
}

function indexedCatalogCards() {
  let queries = 0;
  const block = {};
  const first = { dataset: { appId: 'demo' }, closest: () => block };
  const repeated = { dataset: { appId: 'demo' }, closest: () => block };
  const other = { dataset: { appId: 'other' }, closest: () => block };
  const content = {
    querySelectorAll(selector) {
      queries += 1;
      if (selector === '.section, .hero, .empty-updates-card') return [block];
      if (selector === '.app-card') return [first, repeated, other];
      if (selector === 'img[data-icon-candidates]') return [];
      return [];
    },
  };
  const ctx = vm.createContext({ Map, Set, WeakMap, document: { getElementById: () => content } });
  vm.runInContext(`
    const ENTERING_BLOCKS = ".section, .hero, .empty-updates-card";
    const contentDomIndex = {
      root: null, blocks: [], cards: [], cardsByBlock: new WeakMap(),
      cardsByAppId: new Map(), icons: [],
    };
  `, ctx);
  load(ctx, ['rebuildContentDomIndex', 'indexedContent', 'indexedAppCards']);
  ctx.rebuildContentDomIndex(content);
  const indexedQueries = queries;
  const found = ctx.indexedAppCards(new Set(['demo']));
  assert.equal(found.length, 2, 'El índice conserva las tarjetas repetidas de Inicio');
  assert.equal(found[0], first);
  assert.equal(found[1], repeated);
  assert.equal(queries, indexedQueries, 'Actualizar un id no debe volver a recorrer el DOM');
}

(async () => {
  await startupConnectionGate();
  externalInstalledSearch();
  await syncFadeCompletesBeforeRemoval();
  await splashFadeCompletesBeforeRemoval();
  await splashPresentation();
  modalPresentationBeforeFocus();
  modalBackdropIsStatic();
  versionLabels();
  visibleCounters();
  await msStoreScanReuse();
  await webBrowserChoice();
  await cancellation('catalog');
  await cancellation('msstore');
  await cancellation('catalog', true);
  await portableUpdate('preserve', false);
  await portableUpdate('delete', true);
  await updaterFailure();
  await updaterWaitsForRunningOperations();
  await startupOfferWaitsForIdle();
  restoredRepositories();
  await queuedInstall();
  queueEligibility();
  await startupVerification();
  await sharedReadRequests();
  indexedCatalogCards();
  console.log('Review regressions: cancellation, portable choices, repository restore, self-update errors and waits, install queue, start-up verification, Microsoft Store scan reuse, shared reads, indexed cards, web-app browser choice, visible counters and version labels passed.');
})().catch(error => { console.error(error); process.exitCode = 1; });
