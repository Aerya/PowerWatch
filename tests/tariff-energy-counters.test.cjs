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

test('dashboards no longer load or request the cumulative cost curve',()=>{
  for(const html of [local,hub]){assert.doesNotMatch(html,/costCurve|src="\/static\/cost-chart.js"/);new vm.Script([...html.matchAll(/<script(?:\s[^>]*)?>([\s\S]*?)<\/script>/g)].map(match=>match[1]).join('\n'));}
});

const {browser,chromium,script}=require('./helpers/browser.cjs');
for(const [name,html] of [['PowerWatch',local],['Hub',hub]])test(name+': compact costs show lifetime, change periods, preserve selection and expose the tooltip',{skip:!chromium},()=>{
  const start=html.indexOf('function renderCostSummary('),end=html.indexOf('\n}',start)+2;
  const result=browser('<div id="costs"></div><pre id="result">pending</pre>'+script(html.slice(start,end))+script(`
const element=document.getElementById('costs');
const rated={energy_cost_eur:1.04,subscription_eur:0,observed_seconds:3600,billed_energy_kwh:1,unpriced_kwh:1,estimated_kwh:1};
const data={enabled:true,windows:{'24h':rated,'7d':{...rated,energy_cost_eur:6.39},'30d':{...rated,energy_cost_eur:12.59},all:{...rated,energy_cost_eur:12.59}}};
renderCostSummary(element,data,false,false);
const first=element.textContent,tooltip=element.querySelector('[role=tooltip]').textContent;
const select=element.querySelector('select');select.value='7d';select.dispatchEvent(new Event('change'));const seven=element.querySelector('[data-period-cost]').textContent;
renderCostSummary(element,data,false,true);const selected=element.querySelector('select').value,english=element.textContent;
renderCostSummary(element,{enabled:false},false,true);const hidden=element.hidden;
renderCostSummary(element,null,true,false);const failed=element.textContent;
document.querySelector('#result').textContent=JSON.stringify({first,tooltip,seven,selected,english,hidden,failed});`));
  assert.match(result.first,/Coût estimé cumulé.*12,59/);assert.doesNotMatch(result.first,/partiel|≈|estimation historique/);
  assert.equal(result.tooltip,"Ces coûts sont à titre indicatifs et en partie dérivés d'estimations de consommation électrique.");
  assert.match(result.seven,/6,39/);assert.equal(result.selected,'7d');assert.match(result.english,/Estimated cumulative cost/);assert.equal(result.hidden,true);assert.match(result.failed,/Indisponible/);assert.doesNotMatch(result.failed,/12,59/);
});

test('Hub machine cards request their own costs, keep independent periods and do not duplicate site standing charges',{skip:!chromium},()=>{
 const extract=name=>{const start=hub.indexOf(name);return hub.slice(start,hub.indexOf('\n}',start)+2);};
 const result=browser('<div id="cost-a" data-cost-node="a"></div><div id="cost-b" data-cost-node="b"></div><pre id="result">pending</pre>'+script(extract('function renderCostSummary('))+script(extract('function renderNodeCosts('))+script(extract('async function refreshNodeCosts('))+script(`
 const nodeCostData=new Map(),nodeCostPeriods=new Map();let nodeCostSequence=0,lang='fr';const snapshot={nodes:[{id:'a'},{id:'b'}]},requests=[];
 async function api(url){requests.push(url);const amount=url.endsWith('=a')?2:3;return {enabled:true,windows:Object.fromEntries(['24h','7d','30d','all'].map(key=>[key,{energy_cost_eur:amount,subscription_eur:8,projected_subscription_eur:1,observed_seconds:3600}]))};}
 (async()=>{await refreshNodeCosts();const a=document.querySelector('#cost-a'),b=document.querySelector('#cost-b');const amounts=[a.querySelector('.cost-summary-amount').textContent,b.querySelector('.cost-summary-amount').textContent];const select=a.querySelector('select');select.value='7d';select.dispatchEvent(new Event('change'));select.focus();renderNodeCosts();const stable=select===a.querySelector('select');a.innerHTML='';renderNodeCosts();document.querySelector('#result').textContent=JSON.stringify({requests,amounts,stable,periods:[a.querySelector('select').value,b.querySelector('select').value]});})();`));
 assert.deepEqual(result.requests,['/api/hub/tariffs-fr/overview?nodes=a','/api/hub/tariffs-fr/overview?nodes=b']);
 assert.match(result.amounts[0],/2,00/);assert.match(result.amounts[1],/3,00/);assert.deepEqual(result.periods,['7d','24h']);assert.equal(result.stable,true);
});
