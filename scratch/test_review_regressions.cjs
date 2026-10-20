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

(async () => {
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
  restoredRepositories();
  await queuedInstall();
  queueEligibility();
  await startupVerification();
  console.log('Review regressions: cancellation, portable choices, repository restore, self-update errors, install queue, start-up verification, Microsoft Store scan reuse, web-app browser choice, visible counters and version labels passed.');
})().catch(error => { console.error(error); process.exitCode = 1; });
