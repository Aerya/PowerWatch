'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const root=path.resolve(__dirname,'..');
const read=name=>fs.readFileSync(path.join(root,name),'utf8');

test('shared Home Assistant JavaScript parses and has French/English translations',()=>{
  const script=read('assets/home-assistant.js');
  assert.doesNotThrow(()=>new vm.Script(script));
  for(const text of ['Home Assistant · MQTT','Découverte automatique','Automatically discover','clear_password','/test','publish_components','publish_energy']){
    assert.ok(script.includes(text),text);
  }
});

test('both interfaces expose the same optional settings panel and assets',()=>{
  for(const name of ['powerwatch-web/static/index.html','powerwatch-hub/static/index.html']){
    const html=read(name);
    for(const token of ['id="pw-ha-mount"','/static/home-assistant.js','/static/home-assistant.css','PowerWatchHA?.init'])
      assert.ok(html.includes(token),`${name}: ${token}`);
  }
});

test('Hub MQTT administrative paths are included in its Bearer auth wrapper',()=>{
  const hub=read('powerwatch-hub/static/index.html');
  assert.ok(hub.includes('path.startsWith("/api/hub/home-assistant")'));
  const backend=read('powerwatch-hub/src/main.rs');
  assert.match(backend,/\/api\/hub\/home-assistant.*get\(ha_handlers::get_config\)\.put\(ha_handlers::put_config\)/);
  const local=read('powerwatch-web/src/app.rs');
  assert.match(local,/\/api\/home-assistant.*get\(crate::ha_handlers::get_config\)\.put\(crate::ha_handlers::put_config\)/);
});

test('FR/EN docs explain independent monotonic global Hub meter',()=>{
  assert.match(read('README.md'),/Cumul kWh global Home Assistant du Hub/);
  assert.match(read('README.en.md'),/Hub-wide Home Assistant kWh meter/);
  assert.match(read('README.md'),/Le cumul historique du Hub reste inchangé/);
  assert.match(read('README.en.md'),/existing Hub historical totals remain unchanged/);
  assert.match(read('powerwatch-core/src/home_assistant.rs'),/total_increasing/);
});

test('Hub maintains separate persistent ledger, even on offline ticks',()=>{
  const hub=read('powerwatch-hub/src/main.rs');
  const ledger=read('powerwatch-hub/src/ha_energy.rs');
  const exportBackend=read('powerwatch-hub/src/ha_handlers.rs');
  assert.ok(hub.includes('ha_energy::init(&storage.conn)'));
  assert.ok(hub.includes('ha_energy::record(&tx, timestamp.timestamp(), totals, included_nodes)'));
  assert.ok(hub.includes('storage.write_totals(now, &totals, &included_nodes)'));
  assert.ok(ledger.includes('MAX_SAMPLE_GAP_SECONDS: i64 = 180'));
  assert.match(ledger,/DELETE FROM ha_hub_energy_last/);
  assert.match(ledger,/UPDATE ha_hub_energy_total SET wh=wh\+\?1/);
  assert.ok(exportBackend.includes('ha_energy::total_kwh'));
});

test('MQTT 0.24 client ID set at construction',()=>{
  const core=read('powerwatch-core/src/home_assistant.rs');
  assert.ok(!core.includes('set_client_id('));
});
