"use strict";
const test=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs");
const vm=require("node:vm");
const path=require("node:path");
const read=name=>fs.readFileSync(path.join(__dirname,"..",name),"utf8");
const local=read("powerwatch-web/static/index.html");
const hub=read("powerwatch-hub/static/index.html");
function getFunction(html,name,context){
  const start=html.indexOf(`function ${name}(`);
  assert.ok(start>=0,`${name} exists`);
  const after=html.indexOf("\n}",start);
  assert.ok(after>start);
  return vm.runInNewContext(`${html.slice(start,after+2)}\n${name}`,context);
}
const rated={energy_cost_eur:0.2,subscription_eur:0.05,billed_energy_kwh:1,unpriced_kwh:0};
const partial={...rated,unpriced_kwh:1};
const absent={...rated,energy_cost_eur:0,billed_energy_kwh:0,unpriced_kwh:1};
const energy={first_seen_epoch:1750000,energy_kwh:1};
for(const [label,html,prefix] of [["local",local,"energy"],["hub",hub,"hub-energy"]]){
  test(label+": each existing energy counter has its own cost line",()=>{
    for(const id of ["24h","7d","30d","all"]){
      assert.match(html,new RegExp(`id="${prefix}-${id}"`));
      assert.match(html,new RegExp(`id="${prefix}-cost-${id}"`));
    }
    assert.match(html,/\.energy-tile-cost \{/);
    assert.match(html,/\.selected/);
  });
}
for(const label of ["local","hub"]){
  test(label+": valid, partial, disabled and unpriced costs",()=>{
    const target=label==="local"?local:hub;
    const data={};
    const context=label==="local"?{
      PWI18n:{language:"fr"},pwCostFailed:false,powerwatchCosts:{enabled:true},
      pwEnergyText:key=>({costUnavailable:"Coût indisponible",costDisabled:"Tarif non configuré",costPartial:"partiel"})[key]
    }:{
      lang:"fr",hubCostFailed:false,hubEnergyCosts:{enabled:true},
      tr:key=>({energyCostUnavailable:"Coût indisponible",energyCostDisabled:"Tarif non configuré",energyCostPartial:"partiel"})[key]
    };
    const cost=getFunction(target,label==="local"?"pwCostText":"hubCostText",context);
    assert.match(cost(energy,rated),/0,25/);
    assert.match(cost(energy,partial),/partiel/);
    assert.equal(cost(energy,absent),"Coût indisponible");
    assert.equal(cost({first_seen_epoch:null},rated),"");
    context[label==="local"?"powerwatchCosts":"hubEnergyCosts"]={enabled:false};
    assert.equal(cost(energy,rated),"Tarif non configuré");
    context[label==="local"?"pwCostFailed":"hubCostFailed"]=true;
    assert.equal(cost(energy,rated),"Coût indisponible");
  });
}
test("Hub selection is sent to pricing API, separate from energy query",()=>{
  assert.match(hub,/qs\.set\("nodes",hubEnergyChosenIds\(\)\.join\(","\)\)/);
  assert.match(hub,/api\/hub\/tariffs-fr\/overview/);
  assert.match(hub,/void refreshHubCosts\(\)/);
});
test("Tariff editor invalidates displayed costs after server writes",()=>{
  assert.match(read("assets/tariffs-fr.js"),/powerwatch:tariffschange/);
  assert.match(local,/addEventListener\("powerwatch:tariffschange"/);
  assert.match(hub,/addEventListener\("powerwatch:tariffschange"/);
});
