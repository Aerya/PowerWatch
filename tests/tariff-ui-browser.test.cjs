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
let config={enabled:false,country:"FR",include_subscription:false,sites:[{id:"local",name:"Local",nodes:[]}],contracts:[],tempo_days:{},cre_last_sync:null,tempo_last_sync:null};
const offers=[{supplier:"TotalEnergies",supplier_slug:"totalenergies",offer:"Access Électricité",option:"base",kva:3,eur_kwh:{base:0.22},annual_subscription_eur:150,source_url:"https://example.test",fetched_at:"2026-10-11T10:00:00Z"},{supplier:"TotalEnergies",supplier_slug:"totalenergies",offer:"Access Électricité",option:"hphc",kva:6,eur_kwh:{hp:0.24,hc:0.17},annual_subscription_eur:190,source_url:"https://example.test",fetched_at:"2026-10-11T10:00:00Z"}];
let putCount=0,postCount=0;
window.fetch=async(url,opts={})=>{let data;if(url.endsWith("/market/suppliers"))data=[{slug:"totalenergies",name:"TotalEnergies"}];else if(url.includes("/market/offers"))data=offers;else if(url.endsWith("/market/import")){postCount++;const req=JSON.parse(opts.body);const o=offers.find(x=>x.offer===req.offer&&x.option===req.option&&x.kva===req.kva);config={...config,enabled:true,contracts:[{id:req.contract_id,site_id:"local",supplier:o.supplier,offer:o.offer,option:o.option,kva:o.kva,starts_on:"2026-10-11",ends_on:null,offpeak:req.offpeak||[],revisions:[{from:"2026-10-11",until:null,eur_kwh:o.eur_kwh,annual_subscription_eur:o.annual_subscription_eur,source:"Kelwatt",retrieved_at:o.fetched_at,manual:false}]}]};data={changed:true,config};}else if(opts.method==="PUT"){putCount++;data=config;}else if(url.includes("/cost?"))data={enabled:true,global:{energy_cost_eur:0,billed_energy_kwh:0,unpriced_kwh:0},sites:{},nodes:{}};else data=config;return {ok:true,json:async()=>structuredClone(data)};};
window.prompt=()=>"22:30-06:30";window.confirm=()=>true;
</script><script>${source.replace(/<\/script/gi,"<\\/script")}</script><script>
(async()=>{const wait=ms=>new Promise(r=>setTimeout(r,ms));await wait(250);document.querySelector("#tf-market-search").click();await wait(250);let select=document.querySelector("#tf-market-offer");select.selectedIndex=1;select.dispatchEvent(new Event("change"));const before=select.value;document.documentElement.lang="en";await wait(100);const after=document.querySelector("#tf-market-offer").value;document.querySelector("#tf-market-import").click();await wait(250);document.querySelector("#result").textContent=JSON.stringify({before,after,saved:document.querySelector("#tf-market-offer").value,option:config.contracts[0]?.option,kva:config.contracts[0]?.kva,offpeak:config.contracts[0]?.offpeak,putCount,postCount,summary:document.querySelector(".tf-main-contract")?.textContent});})();
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--virtual-time-budget=3000","--dump-dom","file://"+file],{encoding:"utf8",timeout:15000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);
    assert.ok(match,"browser completed the import flow");
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.match(result.before,/hphc.*6$/);assert.equal(result.after,result.before);assert.equal(result.saved,result.before);
    assert.equal(result.option,"hphc");assert.equal(result.kva,6);assert.equal(result.offpeak[0].from,"22:30");
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
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--virtual-time-budget=3000","--dump-dom","file://"+file],{encoding:"utf8",timeout:15000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);assert.ok(match);
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.equal(result.writes,0);assert.equal(result.importDisabled,true);assert.match(result.status,/source indisponible/);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
