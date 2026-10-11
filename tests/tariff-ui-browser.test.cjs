"use strict";
const test=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs");
const os=require("node:os");
const path=require("node:path");
const {execFileSync}=require("node:child_process");
const chromium=["/usr/bin/chromium","/usr/bin/google-chrome"].find(fs.existsSync);
const source=fs.readFileSync(path.join(__dirname,"../assets/tariffs-fr.js"),"utf8");

test("catalogue selection survives language change and atomic HP/HC import",{skip:!chromium},()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-tariff-ui-"));
  try {
    const html=`<!doctype html><html lang="fr"><body><div id="tariffs-fr-app"></div><pre id="result">pending</pre>
<script>
let config={enabled:false,country:"FR",include_subscription:false,sites:[{id:"local",name:"Local",nodes:[]}],contracts:[{id:"old",site_id:"local",supplier:"EDF",offer:"Tarif Bleu",option:"hphc",kva:6,starts_on:"2026-10-11",ends_on:null,offpeak:[],revisions:[]}],tempo_days:{},cre_last_sync:null,tempo_last_sync:null};
const offers=[{supplier:"TotalEnergies",supplier_slug:"totalenergies",offer:"Access Électricité",option:"base",kva:3,eur_kwh:{base:0.22},annual_subscription_eur:150,source_url:"https://example.test",fetched_at:"2026-10-11T10:00:00Z"},{supplier:"TotalEnergies",supplier_slug:"totalenergies",offer:"Access Électricité",option:"hphc",kva:6,eur_kwh:{hp:0.24,hc:0.17},annual_subscription_eur:190,source_url:"https://example.test",fetched_at:"2026-10-11T10:00:00Z"}];
let putCount=0,postCount=0,replacedId=null;
window.fetch=async(url,opts={})=>{let data;if(url.endsWith("/market/suppliers"))data=[{slug:"totalenergies",name:"TotalEnergies"},{slug:"edf",name:"EDF"}];else if(url.includes("/market/offers"))data=offers;else if(url.endsWith("/market/import")){postCount++;const req=JSON.parse(opts.body);replacedId=req.replace_contract_id;const o=offers.find(x=>x.offer===req.offer&&x.option===req.option&&x.kva===req.kva);config={...config,enabled:true,include_subscription:req.include_subscription,contracts:[{id:req.contract_id,site_id:"local",supplier:o.supplier,offer:o.offer,option:o.option,kva:o.kva,starts_on:"2026-10-11",ends_on:null,offpeak:req.offpeak||[],revisions:[{from:"2026-10-11",until:null,eur_kwh:o.eur_kwh,annual_subscription_eur:o.annual_subscription_eur,source:"Kelwatt",retrieved_at:o.fetched_at,manual:false}]}]};data={changed:true,config};}else if(opts.method==="PUT"){putCount++;data=config;}else if(url.includes("/cost?"))data={enabled:true,global:{energy_cost_eur:0,billed_energy_kwh:0,unpriced_kwh:0},sites:{},nodes:{}};else data=config;return {ok:true,json:async()=>structuredClone(data)};};
window.confirm=()=>true;
</script><script>${source.replace(/<\/script/gi,"<\\/script")}</script><script>
(async()=>{const wait=ms=>new Promise(r=>setTimeout(r,ms));await wait(250);const initial=document.querySelector(".tf-main-contract")?.textContent;document.querySelector("#tf-enabled").click();document.querySelector("#tf-subscription").click();const supplier=document.querySelector("#tf-market-supplier");supplier.value="totalenergies";supplier.dispatchEvent(new Event("change"));document.querySelector("#tf-market-search").click();await wait(250);let select=document.querySelector("#tf-market-offer");select.selectedIndex=1;select.dispatchEvent(new Event("change"));const before=select.value;document.documentElement.lang="en";await wait(100);const after=document.querySelector("#tf-market-offer").value;document.querySelector("#tf-market-offpeak").value="22:30-06:30";document.querySelector("#tf-market-import").click();await wait(250);document.querySelector("#result").textContent=JSON.stringify({initial,before,after,saved:document.querySelector("#tf-market-offer").value,option:config.contracts[0]?.option,kva:config.contracts[0]?.kva,offpeak:config.contracts[0]?.offpeak,includeSubscription:config.include_subscription,replacedId,putCount,postCount,summary:document.querySelector(".tf-main-contract")?.textContent});})();
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=3000","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);
    assert.ok(match,"browser completed the import flow");
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.match(result.before,/hphc.*6$/);assert.equal(result.after,result.before);assert.equal(result.saved,result.before);
    assert.equal(result.option,"hphc");assert.equal(result.kva,6);assert.equal(result.offpeak[0].from,"22:30");
    assert.match(result.initial,/EDF.*Tarif Bleu/);assert.equal(result.replacedId,"old");assert.equal(result.includeSubscription,true);
    assert.equal(result.putCount,0,"import never saves unrelated editor state");assert.equal(result.postCount,1);
    assert.match(result.summary,/Access Électricité.*6 kVA/);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});

test("catalogue outage leaves saved configuration untouched and shows an inline error",{skip:!chromium},()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-tariff-outage-"));
  try {
    const html=`<!doctype html><html lang="fr"><body><div id="tariffs-fr-app"></div><pre id="result">pending</pre>
<script>
let writes=0;const config={enabled:false,country:"FR",include_subscription:false,sites:[{id:"local",name:"Local",nodes:[]}],contracts:[],tempo_days:{}};
window.fetch=async(url,opts={})=>{if(opts.method==="PUT"||opts.method==="POST")writes++;if(url.includes("/market/offers"))return {ok:false,text:async()=>"source indisponible"};if(url.endsWith("/market/suppliers"))return {ok:true,json:async()=>[{slug:"totalenergies",name:"TotalEnergies"}]};return {ok:true,json:async()=>config};};
</script><script>${source.replace(/<\/script/gi,"<\\/script")}</script><script>
(async()=>{await new Promise(r=>setTimeout(r,200));document.querySelector("#tf-market-search").click();await new Promise(r=>setTimeout(r,250));document.querySelector("#result").textContent=JSON.stringify({writes,status:document.querySelector("#tf-market-status").textContent,importDisabled:document.querySelector("#tf-market-import").disabled});})();
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=3000","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);assert.ok(match);
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.equal(result.writes,0);assert.equal(result.importDisabled,true);assert.match(result.status,/source indisponible/);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});

test("manual Online contract replaces an unpriced Tarif Bleu draft only after bill-rate confirmation",{skip:!chromium},()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-manual-tariff-"));
  try {
    const html=`<!doctype html><html lang="fr"><body><div id="tariffs-fr-app"></div><pre id="result">pending</pre>
<script>
let writes=0,config={revision:0,enabled:false,country:"FR",include_subscription:false,sites:[{id:"local",name:"Local",nodes:[]}],contracts:[{id:"old",site_id:"local",supplier:"EDF",offer:"Tarif Bleu",option:"hphc",starts_on:"2026-10-11",ends_on:null,kva:6,offpeak:[],revisions:[]}],tempo_days:{}};
window.fetch=async(url,opts={})=>{if(opts.method==="PUT"){writes++;config=JSON.parse(opts.body);config.revision++;return {ok:true,json:async()=>structuredClone(config)};}if(url.endsWith("/market/suppliers"))return {ok:true,json:async()=>[{slug:"edf",name:"EDF"}]};if(url.includes("/cost?"))return {ok:true,json:async()=>({enabled:true,global:{energy_cost_eur:0,billed_energy_kwh:0,unpriced_kwh:0},sites:{},nodes:{}})};return {ok:true,json:async()=>structuredClone(config)};};
window.confirm=()=>true;
</script><script>${source.replace(/<\/script/gi,"<\\/script")}</script><script>
(async()=>{const wait=ms=>new Promise(r=>setTimeout(r,ms));await wait(200);document.querySelector(".tf-manual").open=true;
const set=(key,value)=>{const input=document.querySelector('[data-manual="'+key+'"]');input.value=value;input.dispatchEvent(new Event("input"));};
set("supplier","TotalEnergies");set("offer","Online Électricité");set("starts_on","2019-07-02");
document.querySelector("#tf-manual-identity").click();await wait(100);const identity={writes,offer:config.contracts[0].offer,revisions:config.contracts[0].revisions.length,status:document.querySelector("#tf-manual-status").textContent};
set("price_from","2026-10-05");set("hp","0.2150");set("subscription_monthly","12.345");set("offpeak","22:30-06:30");
document.querySelector("#tf-manual-save").click();await wait(40);const refused={writes,status:document.querySelector("#tf-manual-status").textContent,old:config.contracts[0].offer};
set("hc","0.1600");document.querySelector("#tf-manual-save").click();await wait(120);
const saved={writes,contracts:structuredClone(config.contracts),summary:document.querySelector(".tf-main-contract").textContent,status:document.querySelector("#tf-manual-status").textContent};
set("price_from","2026-10-10");set("hp","0.2200");document.querySelector("#tf-manual-save").click();await wait(90);
const revision={writes,contract:structuredClone(config.contracts[0])};
set("offer","Autre offre");document.querySelector("#tf-manual-identity").click();await wait(40);
document.querySelector("#result").textContent=JSON.stringify({identity,refused,saved,revision,writes,blockedStatus:document.querySelector("#tf-manual-status").textContent});})();
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=3000","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);assert.ok(match);
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.equal(result.identity.writes,1);assert.equal(result.identity.offer,"Online Électricité");assert.equal(result.identity.revisions,0);assert.match(result.identity.status,/sans prix/);
    assert.equal(result.refused.writes,1);assert.equal(result.refused.old,"Online Électricité");assert.match(result.refused.status,/prix TTC/);
    assert.equal(result.saved.writes,2);assert.equal(result.saved.contracts.length,1);assert.equal(result.saved.contracts[0].offer,"Online Électricité");
    assert.equal(result.saved.contracts[0].starts_on,"2019-07-02");assert.equal(result.saved.contracts[0].revisions[0].from,"2026-10-05");
    assert.equal(result.saved.contracts[0].revisions[0].eur_kwh.hc,0.16);assert.equal(result.saved.contracts[0].offpeak[0].from,"22:30");
    assert.equal(result.saved.contracts[0].revisions[0].annual_subscription_eur,148.14);
    assert.match(result.saved.summary,/TotalEnergies.*Online/);assert.match(result.saved.status,/enregistrés sur le serveur/);
    assert.equal(result.revision.writes,3);assert.equal(result.revision.contract.revisions.length,2);
    assert.equal(result.revision.contract.revisions[0].until,"2026-10-09");assert.equal(result.revision.contract.revisions[1].from,"2026-10-10");
    assert.equal(result.writes,3);assert.match(result.blockedStatus,/prix historiques/);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
