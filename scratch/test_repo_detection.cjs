const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');

const source = fs.readFileSync(path.join(__dirname, '../src/main.js'), 'utf8');
const context = vm.createContext({
  REPOS: { choco: { prefix: 'choco-', source_type: 'choco', package_field: 'choco_id' } },
  state: { catalog: [], statuses: {}, busy: {}, repos: { choco: { results: [] } } },
  derived: { catalogById: new Map(), repoPackages: new Map(), taskByAppId: new Map() },
  escapeHtml: String,
  avatarBackground: () => '',
  renderAvatar: () => '',
});
vm.runInContext('function appStatus(id) { return state.statuses[id] || {}; }', context);
for (const name of ['repoAppId', 'repoAppShape', 'repoCatalogEntry', 'repoPackageByAppId',
  'installedRepoApp', 'actionButtons', 'repoCardHtml', 'findApp']) {
  const start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, name);
  const end = source.indexOf('\n}', start) + 2;
  vm.runInContext(source.slice(start, end), context);
}

const pkg = { key: 'cheatengine', name: 'Cheat Engine', version: '7.7', publisher: 'Dark Byte' };
context.state.repos.choco.results = [pkg];
const render = () => context.repoCardHtml('choco', pkg, 0);
assert.match(render(), /data-repo-install=/);

context.state.busy['choco-cheatengine'] = 'installing';
assert.match(render(), /disabled aria-busy="true"/);
delete context.state.busy['choco-cheatengine'];
context.state.statuses['choco-cheatengine'] = { installed: true, can_launch: true };
assert.match(render(), /data-launch="choco-cheatengine"/);
assert.match(render(), /data-uninstall="choco-cheatengine"/);
assert.doesNotMatch(render(), /data-repo-install=/);
assert.equal(context.findApp('choco-cheatengine').choco_id, 'cheatengine');

// A refresh/restart can already know the curated card for this same product.
delete context.state.statuses['choco-cheatengine'];
context.state.catalog = [{ id: 'cheat_engine', name: 'Cheat Engine' }];
context.state.statuses.cheat_engine = { installed: true, can_launch: true };
assert.match(render(), /data-launch="cheat_engine"/);

// A similar product must not hide the Install action for this one.
context.state.catalog[0].name = 'Cheat Engine Tutorial';
assert.match(render(), /data-repo-install=/);

// A confirmed uninstall must restore the install action.
context.state.catalog[0].name = 'Cheat Engine';
context.state.statuses.cheat_engine.installed = false;
assert.match(render(), /data-repo-install=/);
console.log('Repository detection UI: all regression checks passed.');
