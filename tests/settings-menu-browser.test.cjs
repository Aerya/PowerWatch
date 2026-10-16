"use strict";
const test=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs");
const os=require("node:os");
const path=require("node:path");
const {execFileSync}=require("node:child_process");
const chromium=["/usr/bin/chromium","/usr/bin/google-chrome"].find(fs.existsSync);
const source=fs.readFileSync(path.join(__dirname,"../assets/tariffs-fr.js"),"utf8");

test("tariff switch persists without erasing contracts and follows the language",{skip:!chromium},()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-settings-"));
  try {
    const html=`<!doctype html><html lang="fr"><body><div id="tariffs-fr-app"></div><pre id="result">pending</pre>
<script>
let config={revision:0,enabled:false,country:"FR",include_subscription:false,sites:[{id:"local",name:"Local",nodes:[]}],contracts:[{id:"saved",site_id:"local",supplier:"EDF",offer:"Tarif Bleu",option:"base",starts_on:"2026-10-16",ends_on:null,kva:6,offpeak:[],revisions:[]}],tempo_days:{}};
let writes=0;
window.fetch=async(url,opts={})=>{if(opts.method==="PUT"){writes++;config=JSON.parse(opts.body);config.revision++;}if(url.includes("/market/suppliers"))return {ok:true,json:async()=>[]};if(url.includes("/cost?"))return {ok:true,json:async()=>({enabled:config.enabled,global:{energy_cost_eur:0,billed_energy_kwh:0,unpriced_kwh:0},sites:{},nodes:{}})};return {ok:true,json:async()=>structuredClone(config)};};
</script><script>${source.replace(/<\/script/gi,"<\\/script")}</script><script>
(async()=>{const wait=ms=>new Promise(r=>setTimeout(r,ms));await wait(180);document.querySelector("#tf-quick-enabled").click();await wait(130);const enabled=config.enabled;document.documentElement.lang="en";await wait(130);const english=document.querySelector("#tf-quick-enabled").parentElement.textContent;document.querySelector("#tf-quick-enabled").click();await wait(130);document.querySelector("#result").textContent=JSON.stringify({enabled,disabled:!config.enabled,writes,contract:config.contracts[0].id,english});})();
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=2500","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);assert.ok(match);
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.equal(result.enabled,true);assert.equal(result.disabled,true);assert.equal(result.writes,2);assert.equal(result.contract,"saved");assert.match(result.english,/Enable pricing and show costs/);
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
