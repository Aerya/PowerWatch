"use strict";
const test=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs");
const path=require("node:path");
const vm=require("node:vm");
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

test("history duration options follow FR and EN language changes",()=>{
  const historySelect=dashboard.match(/<select id="history-range"[^>]*>([\s\S]*?)<\/select>/)[1];
  const options=[...historySelect.matchAll(/<option value="([^"]+)"(?: selected)?(?: data-i18n="([^"]+)")?[^>]*>([^<]+)<\/option>/g)]
    .filter((match)=>["7d","30d","180d","400d","custom"].includes(match[1]))
    .map((match)=>({value:match[1],dataset:{i18n:match[2]},textContent:match[3]}));
  assert.equal(options.length,5);
  const document={documentElement:{lang:""},body:{dataset:{page:"dashboard"}},title:"",
    querySelectorAll:(selector)=>selector==="[data-i18n]"?options:[],addEventListener:()=>{}};
  const storage=new Map();
  const context={document,navigator:{language:"en"},localStorage:{getItem:key=>storage.get(key),setItem:(key,value)=>storage.set(key,value)},
    window:{dispatchEvent:()=>{}},CustomEvent:class {}};
  vm.runInNewContext(i18n,context);
  context.window.setPowerWatchLanguage("fr");
  assert.deepEqual(options.map((option)=>option.textContent),["7 j","30 j","180 j","400 j","Période personnalisée"]);
  context.window.setPowerWatchLanguage("en");
  assert.deepEqual(options.map((option)=>option.textContent),["7 d","30 d","180 d","400 d","Custom range"]);
  assert.equal(document.documentElement.lang,"en");
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
