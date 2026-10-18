"use strict";
const test=require('node:test'),assert=require('node:assert/strict');
const {browser,chromium,read,script}=require('./helpers/browser.cjs');
function page(mode,scenario){
  const markup=read('assets/tariffs-page.html').replaceAll('__MODE__',mode).replace('__AUTH_SCRIPT__','').replace(/<script src=[^>]*><\/script>/g,'');
  const mock=script(`
let writes=0,posts=0,config={revision:0,enabled:true,country:'FR',include_subscription:false,sites:[{id:'local',name:'Local',nodes:[]}],contracts:[],tempo_days:{}};
window.confirm=()=>true;
window.fetch=async(url,opts={})=>{
if(url.includes('/auth/status'))return {ok:true,json:async()=>({enabled:false})};
if(url.endsWith('/nodes'))return {ok:true,json:async()=>[]};
if(url.includes('/snapshot'))return {ok:true,json:async()=>({nodes:[{id:'a',name:'Atelier'},{id:'b',name:'NAS'}]})};
if(url.includes('/market/suppliers'))return {ok:true,json:async()=>[{slug:'edf',name:'EDF'}]};
if(url.includes('/market/offers'))return {ok:true,json:async()=>[{supplier_slug:'edf',supplier:'EDF',offer:'Catalogue Demo',option:'base',kva:6,eur_kwh:{base:.25},annual_subscription_eur:120,fetched_at:'2026-10-18T00:00:00Z'}]};
if(opts.method==='PUT'){writes++;config=JSON.parse(opts.body);config.revision++;}
if(opts.method==='POST')posts++;
if(url.includes('/cost?'))return {ok:true,json:async()=>({enabled:true,global:{},sites:{},nodes:{}})};
return {ok:true,json:async()=>structuredClone(config)};
};
`);
  return markup.replace('<script>',mock+'<script>').replace('</body>',script(read('assets/tariffs-fr.js'))+'<pre id="result">pending</pre>'+script(scenario)+'</body>');
}
for(const mode of ['local','hub'])test(`${mode}: dedicated tariff form saves, preserves dated history, drafts and language without nested menus`,{skip:!chromium},()=>{
 const result=browser(page(mode,`(async()=>{const wait=()=>new Promise(r=>setTimeout(r,100));await wait();
 const fill=(key,value)=>{const el=document.querySelector('[data-manual="'+key+'"]');el.value=value;el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));};
 for(const [key,value] of Object.entries({supplier:'Fournisseur',offer:'Offre',option:'hphc',kva:'6',starts_on:'2026-10-16',price_from:'2026-10-16',hp:'.3',hc:'.15',offpeak:'22:30-06:30'}))fill(key,value);
 document.querySelector('#tf-manual-save').click();await wait();const first=writes;
 fill('price_from','2026-10-17');fill('hp','.32');document.querySelector('#tf-manual-save').click();await wait();
 const history=config.contracts[0]?.revisions;fill('supplier','Brouillon');document.querySelector('[data-page-tab=history]').click();document.querySelector('[data-page-tab=contract]').click();
 document.querySelector('[data-page-lang=en]').click();await wait();const draft=document.querySelector('[data-manual=supplier]').value;
 fill('option','tempo');const tempo=document.querySelectorAll('[data-manual=blue_hp],[data-manual=blue_hc],[data-manual=white_hp],[data-manual=white_hc],[data-manual=red_hp],[data-manual=red_hc]').length;
 document.querySelector('#result').textContent=JSON.stringify({first,writes,history,draft,tempo,nested:document.querySelectorAll('#tariffs-fr-app details').length,nav:[...document.querySelectorAll('[data-page-tab]')].map(x=>x.textContent),error:document.querySelector('#tf-manual-status').textContent});})();`));
 assert.equal(result.first,1);assert.equal(result.writes,2);assert.equal(result.history.length,2);assert.equal(result.history[0].until,'2026-10-16');assert.equal(result.history[1].eur_kwh.hp,.32);assert.equal(result.draft,'Brouillon');assert.equal(result.tempo,6);assert.equal(result.nested,0);assert.ok(result.nav.includes('My contract'));assert.equal(result.nav.length,mode==='hub'?4:3);
});

test('catalogue rates prefill the single contract form without saving or importing implicitly',{skip:!chromium},()=>{
 const result=browser(page('local',`(async()=>{const wait=()=>new Promise(r=>setTimeout(r,100));await wait();document.querySelector('#tf-catalog-toggle').click();document.querySelector('#tf-market-search').click();await wait();document.querySelector('#tf-market-import').click();await wait();document.querySelector('#result').textContent=JSON.stringify({writes,posts,supplier:document.querySelector('[data-manual=supplier]').value,offer:document.querySelector('[data-manual=offer]').value,base:document.querySelector('[data-manual=base]').value,contracts:config.contracts.length});})();`));
 assert.deepEqual(result,{writes:0,posts:0,supplier:'EDF',offer:'Catalogue Demo',base:'0.25',contracts:0});
});

test('Hub site assignment uses one choice per node and saves mutually exclusive assignments',{skip:!chromium},()=>{
 const result=browser(page('hub',`(async()=>{const wait=()=>new Promise(r=>setTimeout(r,100));await wait();document.querySelector('[data-page-tab=sites]').click();document.querySelector('#tf-add-site').click();const selects=[...document.querySelectorAll('[data-node-site]')];selects[0].value='local';selects[1].selectedIndex=2;selects.forEach(el=>el.dispatchEvent(new Event('input',{bubbles:true})));document.querySelector('#tf-save').click();await wait();document.querySelector('#result').textContent=JSON.stringify({writes,nodes:config.sites.map(site=>site.nodes),selects:document.querySelectorAll('[data-node-site]').length,nested:document.querySelectorAll('#tariffs-fr-app details').length});})();`));
 assert.equal(result.writes,1);assert.deepEqual(result.nodes,[['a'],['b']]);assert.equal(result.selects,2);assert.equal(result.nested,0);
});

test('changing contract keeps the previous contract and its price history',{skip:!chromium},()=>{
 const result=browser(page('local',`(async()=>{const wait=()=>new Promise(r=>setTimeout(r,100));await wait();const fill=(key,value)=>{const el=document.querySelector('[data-manual="'+key+'"]');el.value=value;el.dispatchEvent(new Event('input',{bubbles:true}));el.dispatchEvent(new Event('change',{bubbles:true}));};for(const [key,value] of Object.entries({supplier:'Ancien fournisseur',offer:'Base',base:'.2',starts_on:'2026-10-16',price_from:'2026-10-16'}))fill(key,value);document.querySelector('#tf-manual-save').click();await wait();document.querySelector('#tf-new-contract').click();for(const [key,value] of Object.entries({supplier:'Nouveau fournisseur',offer:'Nouvelle offre',base:'.25',starts_on:'2026-10-17',price_from:'2026-10-17'}))fill(key,value);document.querySelector('#tf-manual-save').click();await wait();document.querySelector('#result').textContent=JSON.stringify({writes,contracts:config.contracts.map(c=>({supplier:c.supplier,start:c.starts_on,end:c.ends_on,prices:c.revisions.map(r=>r.eur_kwh.base)}))});})();`));
 assert.equal(result.writes,2);assert.deepEqual(result.contracts,[{supplier:'Ancien fournisseur',start:'2026-10-16',end:'2026-10-16',prices:[.2]},{supplier:'Nouveau fournisseur',start:'2026-10-17',end:null,prices:[.25]}]);
});

test('required tariff fields show inline errors before any server write',{skip:!chromium},()=>{
 const result=browser(page('local',`(async()=>{await new Promise(r=>setTimeout(r,100));document.querySelector('#tf-manual-save').click();const error=document.querySelector('[data-error-for=supplier]').textContent,invalid=document.querySelector('[data-manual=supplier]').getAttribute('aria-invalid');const supplier=document.querySelector('[data-manual=supplier]');supplier.value='Fournisseur';supplier.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('#result').textContent=JSON.stringify({writes,error,invalid,cleared:document.querySelector('[data-error-for=supplier]').textContent});})();`));
 assert.equal(result.writes,0);assert.equal(result.invalid,'true');assert.match(result.error,/obligatoire/);assert.equal(result.cleared,'');
});
