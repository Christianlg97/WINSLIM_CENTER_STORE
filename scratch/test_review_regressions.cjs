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
  load(ctx, ['isQueueable', 'isBulkUpdateRunning', 'appStatus']);
  assert.deepEqual(catalog.filter(app => ctx.isQueueable(app)).map(app => app.id), ['steam']);
  assert.equal(ctx.isQueueable({ id: 'winget-demo', source_type: 'winget' }), false, 'repository results have no catalog card');
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

(async () => {
  await cancellation('catalog');
  await cancellation('msstore');
  await cancellation('catalog', true);
  await portableUpdate('preserve', false);
  await portableUpdate('delete', true);
  await updaterFailure();
  restoredRepositories();
  await queuedInstall();
  queueEligibility();
  console.log('Review regressions: cancellation, portable choices, repository restore, self-update errors and install queue passed.');
})().catch(error => { console.error(error); process.exitCode = 1; });
