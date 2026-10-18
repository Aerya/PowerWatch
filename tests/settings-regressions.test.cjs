"use strict";
const test=require("node:test"),assert=require("node:assert/strict"),fs=require("node:fs"),os=require("node:os"),path=require("node:path"),vm=require("node:vm");
const {execFileSync}=require("node:child_process");
const chromium=["/usr/bin/chromium","/usr/bin/google-chrome"].find(fs.existsSync);
const read=file=>fs.readFileSync(path.join(__dirname,"..",file),"utf8");
const script=source=>`<script>${source.replace(/<\/script/gi,"<\\/script")}</script>`;
function browser(body){
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-regression-"));
  try{
    const file=path.join(dir,"index.html");fs.writeFileSync(file,`<!doctype html><html lang="fr"><meta charset="utf-8"><body>${body}</body></html>`);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=3000","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);assert.ok(match,"browser produced a result");
    return JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
  }finally{fs.rmSync(dir,{recursive:true,force:true});}
}

test("toolbar menus close each other, outside click and Escape work, nested sections remain open",{skip:!chromium},()=>{
  const result=browser(`<div class="language-switcher"></div><details class="settings-control" id="tariffs"><summary>Tarifs</summary><div><details id="nested"><summary>Contrat</summary><input></details></div></details><details class="settings-control" id="mqtt"><summary>MQTT</summary><div>MQTT</div></details><button id="outside">Outside</button><pre id="result">pending</pre>`+script(read("assets/theme.js"))+script(`
(async()=>{const wait=()=>new Promise(r=>setTimeout(r,80));await wait();const theme=document.querySelector('.theme-control'),tariffs=document.querySelector('#tariffs'),mqtt=document.querySelector('#mqtt'),nested=document.querySelector('#nested');
theme.open=true;await wait();tariffs.open=true;await wait();const exclusive=!theme.open&&tariffs.open;
nested.open=true;await wait();const nesting=tariffs.open&&nested.open;
mqtt.querySelector('summary').click();await wait();const switched=!tariffs.open&&mqtt.open&&nested.open;
document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));await wait();const escaped=!mqtt.open&&document.activeElement===mqtt.querySelector('summary');
tariffs.open=true;await wait();document.querySelector('#outside').click();await wait();document.querySelector('#result').textContent=JSON.stringify({exclusive,nesting,switched,escaped,outside:!tariffs.open});})();`));
  assert.deepEqual(result,{exclusive:true,nesting:true,switched:true,escaped:true,outside:true});
});

test("tariff setup recovers after admin authentication and preserves sections, dates and scroll on save",{skip:!chromium},()=>{
  const result=browser(`<div style="height:180px;overflow:auto" id="scroller"><div id="tariffs-fr-app" data-mode="hub"></div></div><pre id="result">pending</pre>`+script(`
let authorized=false,writes=0,config={revision:0,enabled:true,country:"FR",include_subscription:false,sites:[{id:"local",name:"Local",nodes:[]}],contracts:[],tempo_days:{}};
window.fetch=async(url,opts={})=>{
 if(!authorized)return {ok:false,text:async()=>"Unauthorized"};
 if(url.includes('/market/suppliers'))return {ok:true,json:async()=>[]};
 if(url.includes('/snapshot'))return {ok:true,json:async()=>({nodes:[]})};
 if(opts.method==='PUT'){writes++;config=JSON.parse(opts.body);config.revision++;}
 return {ok:true,json:async()=>structuredClone(config)};
};`)+script(read("assets/tariffs-fr.js"))+script(`
(async()=>{const wait=()=>new Promise(r=>setTimeout(r,100));await wait();const failed=document.querySelector('#tariffs-fr-app').textContent.includes('Unauthorized');authorized=true;window.dispatchEvent(new Event('powerwatch:hubauthchange'));await wait();
document.querySelectorAll('#tariffs-fr-app details').forEach(el=>el.open=true);await wait();const date=document.querySelector('#tf-from');date.value='2026-10-11T09:30';const scroller=document.querySelector('#scroller');scroller.scrollTop=100;const top=scroller.scrollTop;
document.querySelector('#tf-save').click();await wait();const kept=[...document.querySelectorAll('#tariffs-fr-app details')].every(el=>el.open);const scrollKept=scroller.scrollTop===top;
const dateKept=document.querySelector('#tf-from').value;document.querySelector('.tf-manual').open=false;document.documentElement.lang='en';await wait();
document.querySelector('#result').textContent=JSON.stringify({failed,recovered:!!document.querySelector('#tf-save'),writes,kept,scrollKept,dateKept,manualClosed:!document.querySelector('.tf-manual').open,status:document.querySelector('#tf-status').textContent});})();`));
  assert.equal(result.failed,true);assert.equal(result.recovered,true);assert.equal(result.writes,1);assert.equal(result.kept,true);assert.equal(result.scrollKept,true);assert.equal(result.manualClosed,true);assert.equal(result.dateKept,"2026-10-11T09:30");
});

test("Hub token is checked before activation and invalid tokens preserve the working session",async()=>{
  const source=read("powerwatch-hub/static/index.html");
  const start=source.indexOf('document.getElementById("save-hub-admin-token").onclick=');
  const end=source.indexOf('\ndocument.getElementById("history-range").onchange',start);
  const input={value:" rejected "},button={},status={},storage=new Map([["powerwatchHubAdminToken","previous"]]),events=[],calls=[];
  const elements={"hub-admin-token":input,"save-hub-admin-token":button,"hub-admin-status":status};
  const context={document:{getElementById:id=>elements[id]},hubAdminToken:"previous",lang:"fr",tr:key=>key,Headers,Event,Error,
    sessionStorage:{setItem:(k,v)=>storage.set(k,v),removeItem:k=>storage.delete(k)},window:{dispatchEvent:event=>events.push(event.type)},
    fetch:async(url,opts)=>{calls.push([url,opts.headers.get("Authorization")]);return {ok:opts.headers.get("Authorization")==="Bearer valid",status:401};}};
  vm.createContext(context);vm.runInContext(source.slice(start,end),context);
  await button.onclick();assert.equal(context.hubAdminToken,"previous");assert.equal(storage.get("powerwatchHubAdminToken"),"previous");assert.equal(events.length,0);assert.match(status.textContent,/Jeton refusé/);assert.equal(input.value," rejected ");assert.equal(button.disabled,false);
  input.value=" valid ";await button.onclick();assert.equal(context.hubAdminToken,"valid");assert.equal(storage.get("powerwatchHubAdminToken"),"valid");assert.equal(input.value,"");assert.equal(status.textContent,"adminReady");assert.deepEqual(events,["powerwatch:hubauthchange"]);assert.deepEqual(calls,[["/api/hub/nodes","Bearer rejected"],["/api/hub/nodes","Bearer valid"]]);
});

test("MQTT recovers after authentication and reopening its section preserves unsaved credentials",{skip:!chromium},()=>{
  const result=browser(`<div id="pw-ha-mount"></div><pre id="result">pending</pre>`+script(`
let authorized=false,reads=0;
window.fetch=async()=>{reads++;return {ok:authorized,status:authorized?200:401,json:async()=>({enabled:true,host:'saved-broker',port:1883,tls:false,interval_seconds:30}),text:async()=>"Unauthorized"};};`)+script(read("assets/home-assistant.js"))+script(`
window.PowerWatchHA.init({endpoint:'/api/hub/home-assistant',hub:true,request:window.fetch});
(async()=>{const wait=()=>new Promise(r=>setTimeout(r,100));await wait();const initial=document.querySelector('#pw-ha-message').textContent;authorized=true;window.dispatchEvent(new Event('powerwatch:hubauthchange'));await wait();const recovered=document.querySelector('#pw-ha-host').value;const cleared=!document.querySelector('#pw-ha-message').textContent;
document.querySelector('#pw-ha-host').value='draft-broker';document.querySelector('#pw-ha-password').value='draft-secret';const panel=document.querySelector('#pw-ha-panel');panel.open=true;await wait();panel.open=false;await wait();panel.open=true;await wait();
window.dispatchEvent(new Event('powerwatch:hubauthchange'));await wait();document.querySelector('#result').textContent=JSON.stringify({initial,recovered,cleared,host:document.querySelector('#pw-ha-host').value,passwordKept:document.querySelector('#pw-ha-password').value==='draft-secret',reads});})();`));
  assert.match(result.initial,/jeton administrateur/);assert.equal(result.recovered,"saved-broker");assert.equal(result.cleared,true);assert.equal(result.host,"draft-broker");assert.equal(result.passwordKept,true);assert.equal(result.reads,2);
});

test("settings panels fit within the viewport and scroll vertically",{skip:!chromium},()=>{
  const result=browser(`<style>${read("assets/theme.css")}</style><details class="settings-control" open><summary>Tarifs</summary><div class="settings-panel"><div style="height:2000px">Form</div></div></details><pre id="result">pending</pre>`+script(`const panel=document.querySelector('.settings-panel'),rect=panel.getBoundingClientRect();document.querySelector('#result').textContent=JSON.stringify({inside:rect.left>=0&&rect.right<=innerWidth,bounded:rect.top>=0&&rect.bottom<=innerHeight,scrollable:panel.scrollHeight>panel.clientHeight});`));
  assert.deepEqual(result,{inside:true,bounded:true,scrollable:true});
});
