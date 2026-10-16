"use strict";
const test=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs");
const os=require("node:os");
const path=require("node:path");
const {execFileSync}=require("node:child_process");
const vm=require("node:vm");
const chromium=["/usr/bin/chromium","/usr/bin/google-chrome"].find(fs.existsSync);
const dashboard=fs.readFileSync(path.join(__dirname,"../powerwatch-web/static/index.html"),"utf8");
const i18n=fs.readFileSync(path.join(__dirname,"../powerwatch-web/static/i18n.js"),"utf8");

test("local chart precedes energy and sensors, with the Hub's preset durations",()=>{
  const chart=dashboard.indexOf('<section class="history-stack">');
  assert.ok(chart>dashboard.indexOf('id="total"'));
  assert.ok(chart<dashboard.indexOf('class="energy-panel"'));
  assert.ok(chart<dashboard.indexOf('id="sensors"'));
  for(const duration of ["1h","6h","24h","7d","30d","180d","400d"])
    assert.match(dashboard,new RegExp(`<option value="${duration}"`));
  assert.match(dashboard,/<option value="24h" selected>/);
  assert.match(dashboard,/id="history-custom"/);
});

test("history duration options follow FR and EN language changes",{skip:!chromium},()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-history-menu-"));
  try {
    const options=dashboard.match(/<select id="history-range"[\s\S]*?<\/select>/)?.[0];
    assert.ok(options);
    const html=`<!doctype html><html lang="en"><body data-page="dashboard">${options}<pre id="result">pending</pre>
<script>${i18n.replace(/<\/script/gi,"<\\/script")}</script><script>
document.addEventListener("DOMContentLoaded",()=>{setPowerWatchLanguage("fr");const select=document.querySelector("#history-range");const fr=[...select.options].map(x=>x.textContent);const selected=select.value;setPowerWatchLanguage("en");document.querySelector("#result").textContent=JSON.stringify({fr,en:[...select.options].map(x=>x.textContent),selected});});
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=1500","--dump-dom","file://"+file],{encoding:"utf8",timeout:30000,stdio:["ignore","pipe","ignore"]});
    const result=JSON.parse(output.match(/<pre id="result">([^<]+)<\/pre>/)[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.equal(result.selected,"24h");
    assert.deepEqual(result.fr.slice(4,8),["7 j","30 j","180 j","400 j"]);
    assert.deepEqual(result.en.slice(4,8),["7 d","30 d","180 d","400 d"]);
    assert.equal(result.fr[8],"Période personnalisée");
    assert.equal(result.en[8],"Custom range");
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});


test("duration picker requests the matching history range and preserves Live",async()=>{
  const start=dashboard.indexOf("function setupHistoryControls() {");
  const end=dashboard.indexOf("function formatBucket(",start);
  assert.ok(start>=0 && end>start);
  const listeners={};const calls=[];
  const picker={value:"24h",setAttribute(){},addEventListener:(event,handler)=>{listeners.picker=handler;}};
  const custom={open:false};
  const form={addEventListener:(event,handler)=>{listeners.form=handler;}};
  const amount={value:"12",setAttribute(){}};const unit={value:"hours",setAttribute(){}};
  const context={document:{getElementById:id=>({"history-range":picker,"history-custom":custom,"history-form":form,"history-amount":amount,"history-unit":unit})[id]},t:()=>"History duration",PWI18n:{rangeLabel:(n,u)=>`${n} ${u}`},loadHistory:async(n,u,label)=>{calls.push([n,u,label]);},enterLiveMode:()=>{calls.push(["live"]);}};
  vm.runInNewContext(dashboard.slice(start,end)+"setupHistoryControls();",context);
  assert.equal(JSON.stringify(calls[0]),JSON.stringify([24,"hours","24 hours"]));
  picker.value="6h";await listeners.picker();assert.equal(JSON.stringify(calls.at(-1)),JSON.stringify([6,"hours","6 hours"]));
  picker.value="400d";await listeners.picker();assert.equal(JSON.stringify(calls.at(-1)),JSON.stringify([400,"days","400 days"]));
  picker.value="custom";await listeners.picker();assert.equal(custom.open,true);
  await listeners.form({preventDefault(){}});assert.equal(JSON.stringify(calls.at(-1)),JSON.stringify([12,"hours","12 hours"]));
  picker.value="live";await listeners.picker();assert.equal(JSON.stringify(calls.at(-1)),JSON.stringify(["live"]));
});
