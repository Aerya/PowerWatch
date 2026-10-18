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
const partial={...rated,unpriced_kwh:1,warnings:["meter off-peak hours are missing"]};
const absent={...rated,energy_cost_eur:0,billed_energy_kwh:0,unpriced_kwh:1,warnings:["no contract for this historical period"]};
const projected={...rated,energy_cost_eur:0,subscription_eur:0,billed_energy_kwh:0,projected_energy_cost_eur:0.18,projected_kwh:1,unpriced_kwh:0};
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
    assert.match(cost(energy,partial),/0,25/);
    assert.match(cost(energy,partial),/partiel/);
    assert.match(cost(energy,absent),/aucun contrat pour cette période historique/);
    assert.match(cost(energy,projected),/0,18.*estimation historique/);
    assert.match(cost({...energy,estimated_kwh:0.001},rated),/partiel/);
    assert.equal(cost({first_seen_epoch:null},rated),"");
    context[label==="local"?"powerwatchCosts":"hubEnergyCosts"]={enabled:false};
    assert.equal(cost(energy,rated),"", "disabled pricing hides the cost line");
    context[label==="local"?"powerwatchCosts":"hubEnergyCosts"]={enabled:true};
    context[label==="local"?"pwCostFailed":"hubCostFailed"]=true;
    assert.equal(cost(energy,rated),"Coût indisponible · API");
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

test('infrastructure costs use the normal total independently of counter filters and clear stale prices',async()=>{
  const element={hidden:false,replaceChildren(...rows){this.rows=rows;}};
  let response={enabled:true,windows:Object.fromEntries(['24h','7d','30d','all'].map(period=>[period,{...rated,observed_seconds:3600}]))};
  const requests=[];
  const context={lang:'fr',hubEnergyCosts:null,hubCostFailed:false,tr:key=>({energyCostUnavailable:'Coût indisponible',energyCostPartial:'partiel'})[key],
    document:{getElementById:()=>element,createElement:()=>({textContent:''})},
    api:async url=>{requests.push(url);if(response instanceof Error)throw response;return response;}};
  context.hubCostText=getFunction(hub,'hubCostText',context);
  const start=hub.indexOf('let infrastructureCosts=');
  vm.createContext(context);
  vm.runInContext(hub.slice(start,hub.indexOf('async function refreshHubCosts()',start)),context);
  await vm.runInContext('refreshInfrastructureCosts()',context);
  assert.equal(requests[0],'/api/hub/tariffs-fr/overview');
  assert.equal(element.rows.length,4);assert.match(element.rows[0].textContent,/24 h.*0,25/);assert.match(element.rows[3].textContent,/Total cumulé.*0,25/);
  context.lang='en';vm.runInContext('renderInfrastructureCosts()',context);assert.match(element.rows[3].textContent,/Cumulative total \(since first reading\)/);
  response={enabled:false,windows:{}};await vm.runInContext('refreshInfrastructureCosts()',context);assert.equal(element.hidden,true);
  response=new Error('HTTP 503');await vm.runInContext('refreshInfrastructureCosts()',context);assert.equal(element.hidden,false);assert.match(element.rows[0].textContent,/indisponible/);assert.doesNotMatch(element.rows[0].textContent,/0,25/);
  response={enabled:true,windows:{'24h':{...rated,observed_seconds:0}}};await vm.runInContext('refreshInfrastructureCosts()',context);assert.match(element.rows[0].textContent,/indisponible/);
});

test('dashboards no longer load or request the cumulative cost curve',()=>{
  for(const html of [local,hub]){assert.doesNotMatch(html,/costCurve|src="\/static\/cost-chart.js"/);new vm.Script([...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)].map(match=>match[1]).join('\n'));}
});

test('PowerWatch Total shows current cost windows and lifetime total, translates and hides disabled costs',()=>{
  const element={replaceChildren(...rows){this.rows=rows;}};
  const context={PWI18n:{language:'fr'},pwCostFailed:false,powerwatchCosts:{enabled:true,windows:Object.fromEntries(['24h','7d','30d','all'].map((period,i)=>[period,{...rated,energy_cost_eur:i+1,observed_seconds:3600}]))},
    pwEnergyText:key=>({costUnavailable:'Coût indisponible',costPartial:'partiel'})[key],
    document:{getElementById:()=>element,createElement:()=>({textContent:''})}};
  context.pwCostText=getFunction(local,'pwCostText',context);
  const render=getFunction(local,'pwTotalCostsRender',context);render();
  assert.equal(element.rows.length,5);assert.equal(element.rows[0].textContent,'Coûts estimés');
  assert.match(element.rows[1].textContent,/24 h.*1,05/);assert.match(element.rows[4].textContent,/Total cumulé.*4,05/);
  context.PWI18n.language='en';render();assert.match(element.rows[4].textContent,/Cumulative total/);
  context.powerwatchCosts={enabled:false};render();assert.equal(element.hidden,true);
  context.powerwatchCosts=null;context.pwCostFailed=true;render();assert.equal(element.hidden,false);assert.match(element.rows[1].textContent,/indisponible/);assert.doesNotMatch(element.rows[4].textContent,/4,05/);
});
