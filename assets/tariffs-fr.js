/* PowerWatch optional French electricity tariffs (beta): standalone shared UI. */
(() => {
  "use strict";
  const root=document.getElementById("tariffs-fr-app");
  if(!root)return;
  const hub=root.dataset.mode==="hub";
  const base=hub?"/api/hub/tariffs-fr":"/api/tariffs-fr";
  const tr=(fr,en)=>document.documentElement.lang==="en"?en:fr;
  const esc=v=>String(v??"").replace(/[&<>"']/g,c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
  let conf=null, nodes=[],cost=null;
  const money=n=>Number(n||0).toLocaleString(document.documentElement.lang==="en"?"en-US":"fr-FR",{style:"currency",currency:"EUR",maximumFractionDigits:4});
  const fields={base:["base"],hphc:["hp","hc"],tempo:["blue_hp","blue_hc","white_hp","white_hc","red_hp","red_hc"],custom:["hp","hc"]};
  const labels={base:"Base",hp:"HP",hc:"HC",blue_hp:"Bleu HP",blue_hc:"Bleu HC",white_hp:"Blanc HP",white_hc:"Blanc HC",red_hp:"Rouge HP",red_hc:"Rouge HC"};
  const toDate=d=>d?String(d).slice(0,10):"";
  const id=()=>"t-"+Date.now().toString(36)+"-"+Math.random().toString(36).slice(2,7);
  const editorMsg=m=>{const el=document.getElementById("tf-status");if(el)el.textContent=m;};
  async function request(path, opts={}) {
    const headers=new Headers(opts.headers||{});
    if(opts.body)headers.set("Content-Type","application/json");
    if(hub){
      const token=sessionStorage.getItem("powerwatchHubAdminToken");
      if(token)headers.set("Authorization","Bearer "+token);
    }
    const response=await fetch(base+path,{...opts,headers,cache:"no-store"});
    if(!response.ok)throw Error(await response.text()||`HTTP ${response.status}`);
    return response.json();
  }
  const editable=(label,value,field,extra="")=>`<label class="tf-label">${esc(label)}<input ${extra} data-field="${esc(field)}" value="${esc(value??"")}"></label>`;
  const inputPrice=(field,value)=>editable(labels[field]||field,value??"",field,'type="number" step="0.0001" min="0" max="10"');
  const option=(value,label,selected)=>`<option value="${esc(value)}" ${value===selected?"selected":""}>${esc(label)}</option>`;
  function siteEditor(){
    if(!hub)return "";
    const available=nodes.map(n=>({id:n.id,name:n.name||n.id}));
    return `<details class="tf-section"><summary>${tr("Sites électriques / affectations","Electric sites / assignments")}</summary>
    <p class="tf-help">${tr("Chaque machine doit appartenir à un seul site. L'abonnement est facturé une seule fois par site.","Each node belongs to one site. Subscription is charged once per site.")}</p>
    <div id="tf-sites">${conf.sites.map((site,i)=>`<div class="tf-item" data-site="${i}">
      ${editable("ID",site.id,"id")}${editable(tr("Nom","Name"),site.name,"name")}
      <div class="tf-label">${tr("Machines du site","Site nodes")}<div>${available.map(n=>`<label class="tf-inline"><input type="checkbox" data-node="${esc(n.id)}" ${site.nodes.includes(n.id)?"checked":""}>${esc(n.name)}</label>`).join(" ")}</div></div>
      <button type="button" data-del-site="${i}">${tr("Supprimer site","Delete site")}</button>
    </div>`).join("")}</div><button type="button" id="tf-add-site">${tr("Ajouter un site","Add site")}</button></details>`;
  }
  function contractEditor(){return `<details class="tf-section" open><summary>${tr("Contrats et grilles tarifaires","Contracts and tariff grids")}</summary>
    <p class="tf-help">${tr("Un contrat par site et période ; plusieurs révisions de prix possibles. Les modifications manuelles ne sont jamais écrasées par l'import CRE.","One contract per site and date range. Manual prices are never replaced by CRE sync.")}</p>
    <div id="tf-contracts">${conf.contracts.map((c,i)=>`<div class="tf-item" data-contract="${i}">
      <div class="tf-row"><strong>${esc(c.supplier)} · ${esc(c.offer)}</strong><button type="button" data-delete="${i}">${tr("Supprimer","Delete")}</button></div>
      <div class="tf-grid">
      ${editable("ID",c.id,"id")}${editable(tr("Fournisseur","Supplier"),c.supplier,"supplier")}
      ${editable("Offre",c.offer,"offer")}
      <label class="tf-label">${tr("Site","Site")}<select data-field="site_id">${conf.sites.map(s=>option(s.id,s.name,c.site_id)).join("")}</select></label>
      <label class="tf-label">${tr("Option","Plan")}<select data-field="option">${["base","hphc","tempo","custom"].map(o=>option(o,o.toUpperCase(),c.option)).join("")}</select></label>
      ${editable("kVA",c.kva,"kva",'type="number" min="1" max="36"')}
      ${editable(tr("Début","Starts"),c.starts_on,"starts_on",'type="date"')}${editable(tr("Fin (facultative)","Ends (optional)"),toDate(c.ends_on),"ends_on",'type="date"')}
      ${editable(tr("Plages HC (ex: 22:30-06:30;13:00-15:00)","Off-peak windows"),c.offpeak.map(w=>w.from+"-"+w.to).join(";"),"offpeak")}
      </div>
      <div class="tf-row"><strong>${tr("Versions de tarifs TTC","Price revisions incl. tax")}</strong>
      <button type="button" data-add-rev="${i}">+ ${tr("Prix","Price")}</button>
      ${c.supplier.trim().toLowerCase()==="edf"&&c.offer.toLowerCase().includes("bleu")?["base","hphc","tempo"].includes(c.option)?`<button type="button" data-sync-cre="${i}">${tr("Importer CRE","Sync CRE")}</button>`:"":""}
      </div>
      ${c.revisions.map((r,j)=>`<div class="tf-rev" data-revision="${j}"><div class="tf-grid">
      ${editable(tr("Tarif du","Price from"),r.from,"from",'type="date"')}${editable(tr("Jusqu'au (facultatif)","Price until"),toDate(r.until),"until",'type="date"')}
      ${fields[c.option].map(key=>inputPrice(key,r.eur_kwh[key])).join("")}
      ${editable(tr("Abonnement annuel TTC","Yearly subscription"),r.annual_subscription_eur??"","annual_subscription_eur",'type="number" step="0.01" min="0"')}
      </div><small>${esc(r.source||"manuel")} ${r.retrieved_at?esc(r.retrieved_at.slice(0,10)):""} · ${r.manual?tr("modifié manuellement","manual override"):tr("source externe","external source")}</small>
      <button type="button" data-delete-rev="${i},${j}">${tr("Supprimer cette grille","Delete revision")}</button></div>`).join("")}
      </div>`).join("")}</div><button type="button" id="tf-add-contract">+ ${tr("Ajouter contrat","Add contract")}</button></details>`}
  function tempoEditor(){const latest=Object.entries(conf.tempo_days).sort((a,b)=>b[0].localeCompare(a[0])).slice(0,16);
    return `<details class="tf-section"><summary>Tempo · ${tr("calendrier des couleurs","day colors")} (${Object.keys(conf.tempo_days).length})</summary>
      <div class="tf-row"><button type="button" id="tf-tempo-sync">${tr("Synchroniser les jours confirmés (365 j)","Sync confirmed Tempo colors (365 days)")}</button>
      ${editable(tr("Date à corriger","Edit date"),"","tempo-date",'type="date"')}
      <label class="tf-label">${tr("Couleur","Color")}<select id="tf-tempo-color"><option value="blue">Bleu</option><option value="white">Blanc</option><option value="red">Rouge</option></select></label>
      <button type="button" id="tf-tempo-save">${tr("Corriger","Override")}</button></div>
      <p class="tf-help">${tr("La journée Tempo commence à 06:00 (Europe/Paris). Les couleurs non confirmées ne sont pas facturées.","Tempo day starts at 06:00 Europe/Paris. Unknown colors are not priced.")}</p>
      <div class="tf-row">${latest.map(([date,t])=>`<span>${esc(date)}: ${esc(t.color)}${t.manual?" ✎":""}</span>`).join("")}</div>
      <small>${esc(conf.tempo_last_sync||tr("Pas encore synchronisé","Not yet synced"))}</small>
    </details>`;
  }
  function costEditor(){return `<details class="tf-section" open><summary>${tr("Estimation de coût","Cost estimate")}</summary>
    <div class="tf-row"><label class="tf-label">${tr("Du","From")}<input id="tf-from" type="datetime-local"></label><label class="tf-label">${tr("Au","To")}<input id="tf-to" type="datetime-local"></label>
    <button type="button" id="tf-calculate">${tr("Calculer","Calculate")}</button></div><div id="tf-cost"></div></details>`;}
  function render(){
    root.innerHTML=`<div class="tf-banner"><strong>${tr("Tarification électrique France","French electricity pricing")} <span class="tf-beta">BÊTA</span></strong>
    <p>${tr("Prix partiels ou périmés possibles. Consommation estimée, non mesurée à la prise. Ne remplace pas une facture.","Prices can be incomplete or outdated. Energy is estimated, not measured at the socket. Not a bill.")}</p></div>
    <div class="tf-row"><label class="tf-inline"><input id="tf-enabled" type="checkbox" ${conf.enabled?"checked":""}> ${tr("Activer la tarification","Enable pricing")}</label>
    <label class="tf-inline"><input id="tf-subscription" type="checkbox" ${conf.include_subscription?"checked":""}> ${tr("Inclure abonnement (une fois par site)","Include standing charge once per site")}</label>
    <button id="tf-save" type="button">${tr("Enregistrer sur le serveur","Save to server")}</button>
    <button id="tf-export" type="button">${tr("Exporter JSON","Export JSON")}</button>
    <button id="tf-import" type="button">${tr("Importer JSON","Import JSON")}</button><input type="file" accept="application/json" id="tf-file" hidden></div>
    <p id="tf-status" class="tf-help" role="status"></p>
    ${siteEditor()}${contractEditor()}${tempoEditor()}${costEditor()}`;
    bind(); if(cost)renderCost();
  }
  function readEdits(){
    conf.enabled=document.getElementById("tf-enabled").checked;
    conf.include_subscription=document.getElementById("tf-subscription").checked;
    root.querySelectorAll("[data-site]").forEach(block=>{
      const x=conf.sites[Number(block.dataset.site)]; if(!x)return;
      x.id=block.querySelector('[data-field="id"]').value.trim();x.name=block.querySelector('[data-field="name"]').value.trim();
      x.nodes=[...block.querySelectorAll("[data-node]:checked")].map(n=>n.dataset.node);
    });
    root.querySelectorAll("[data-contract]").forEach(block=>{
      const c=conf.contracts[Number(block.dataset.contract)];if(!c)return;
      for(const field of ["id","site_id","supplier","offer","option","starts_on","ends_on","kva","offpeak"]){
        const control=block.querySelector(`[data-field="${field}"]`);if(!control)continue;
        if(field==="kva")c.kva=Number(control.value);
        else if(field==="ends_on")c.ends_on=control.value||null;
        else if(field==="offpeak")c.offpeak=control.value.split(";").filter(Boolean).map(v=>{const m=/^(\d\d:\d\d)-(\d\d:\d\d)$/.exec(v.trim());return m?{from:m[1],to:m[2]}:{from:"invalid",to:"invalid"};});
        else c[field]=control.value.trim();
      }
      block.querySelectorAll("[data-revision]").forEach(revBlock=>{
        const r=c.revisions[Number(revBlock.dataset.revision)];if(!r)return;
        const field=n=>revBlock.querySelector(`[data-field="${n}"]`)?.value;
        const from=field("from"),until=field("until");
        const changed=from!==r.from || (until||null)!==(r.until||null);
        r.from=from;r.until=until||null;
        const price={};for(const k of fields[c.option]){
          const input=field(k);if(input!==undefined&&input.trim()!==""){price[k]=Number(input);if(price[k]!==r.eur_kwh[k])r.manual=true;}
        }
        // Do not compare JSON object insertion order: the Rust BTreeMap is sorted
        // alphabetically, whereas HTML fields are in tariff display order.
        const before=r.eur_kwh||{};
        if([...new Set([...Object.keys(price),...Object.keys(before)])].some(k=>price[k]!==before[k]))r.manual=true;
        r.eur_kwh=price;
        const sub=field("annual_subscription_eur");const value=sub?.trim()?Number(sub):null;
        if(value!==r.annual_subscription_eur)r.manual=true;
        r.annual_subscription_eur=value;if(changed)r.manual=true;
      });
    });
  }
  function bind(){
    const act=(selector,handler)=>root.querySelector(selector)?.addEventListener("click",handler);
    act("#tf-save",async()=>{try{readEdits();conf=await request("",{method:"PUT",body:JSON.stringify(conf)});editorMsg(tr("Configuration sauvegardée en SQLite.","Saved to SQLite."));render();}catch(e){editorMsg(e.message);}});
    act("#tf-export",()=>{try{readEdits();const data=JSON.stringify(conf,null,2);const blob=new Blob([data],{type:"application/json"});const url=URL.createObjectURL(blob);const a=document.createElement("a");a.href=url;a.download="powerwatch-tarifs-fr.json";a.click();setTimeout(()=>URL.revokeObjectURL(url),3000);}catch(e){editorMsg(e.message)}});
    act("#tf-import",()=>root.querySelector("#tf-file").click());
    root.querySelector("#tf-file")?.addEventListener("change",async event=>{const file=event.target.files?.[0];if(!file)return;try{const incoming=JSON.parse(await file.text());conf=await request("",{method:"PUT",body:JSON.stringify(incoming)});editorMsg(tr("Import enregistré.","Import saved."));render();}catch(e){editorMsg(e.message)}});
    act("#tf-add-site",()=>{readEdits();conf.sites.push({id:id(),name:tr("Nouveau site","New site"),nodes:[]});render();});
    root.querySelectorAll("[data-del-site]").forEach(btn=>btn.onclick=()=>{readEdits();conf.sites.splice(Number(btn.dataset.delSite),1);render();});
    act("#tf-add-contract",()=>{readEdits();conf.contracts.push({id:id(),site_id:conf.sites[0]?.id||"local",supplier:"EDF",offer:"Tarif Bleu",option:"base",starts_on:new Date().toISOString().slice(0,10),ends_on:null,kva:6,offpeak:[{from:"22:00",to:"06:00"}],revisions:[]});render();});
    root.querySelectorAll("[data-delete]").forEach(btn=>btn.onclick=()=>{readEdits();conf.contracts.splice(Number(btn.dataset.delete),1);render();});
    root.querySelectorAll("[data-add-rev]").forEach(btn=>btn.onclick=()=>{readEdits();const c=conf.contracts[Number(btn.dataset.addRev)];if(!c)return;c.revisions.push({from:c.starts_on,until:null,eur_kwh:{},annual_subscription_eur:null,source:"manual",manual:true,retrieved_at:null});render();});
    root.querySelectorAll("[data-delete-rev]").forEach(btn=>btn.onclick=()=>{readEdits();const [i,j]=btn.dataset.deleteRev.split(",").map(Number);conf.contracts[i].revisions.splice(j,1);render();});
    root.querySelectorAll("[data-sync-cre]").forEach(btn=>btn.onclick=async()=>{try{readEdits();conf=await request("",{method:"PUT",body:JSON.stringify(conf)});const c=conf.contracts[Number(btn.dataset.syncCre)];const data=await request("/sync",{method:"POST",body:JSON.stringify({kind:"cre",contract_id:c.id})});conf=data.config;render();editorMsg(data.message);}catch(e){editorMsg(e.message)}});
    act("#tf-tempo-sync",async()=>{try{readEdits();conf=await request("",{method:"PUT",body:JSON.stringify(conf)});const data=await request("/sync",{method:"POST",body:JSON.stringify({kind:"tempo"})});conf=data.config;render();editorMsg(data.message);}catch(e){editorMsg(e.message)}});
    act("#tf-tempo-save",()=>{try{readEdits();const date=root.querySelector('[data-field="tempo-date"]').value;
      if(!/^\d{4}-\d{2}-\d{2}$/.test(date))throw Error(tr("Date invalide","Invalid date"));
      conf.tempo_days[date]={color:root.querySelector("#tf-tempo-color").value,manual:true,source:"manual"};render();editorMsg(tr("Correction prête : cliquez Enregistrer.","Override ready: click Save."));
    }catch(e){editorMsg(e.message)}});
    act("#tf-calculate",calculate);
    root.querySelectorAll('select[data-field="option"]').forEach(sel=>sel.onchange=()=>{readEdits();render();});
  }
  function renderCost(){
    const target=root.querySelector("#tf-cost");if(!target||!cost)return;
    const summary=part=>`${money((part?.energy_cost_eur||0)+(part?.subscription_eur||0))} · ${(part?.billed_energy_kwh||0).toFixed(3)} kWh ${tr("tarifés","priced")} · ${(part?.unpriced_kwh||0).toFixed(3)} kWh ${tr("sans tarif","unpriced")} · ${(part?.approximate_kwh||0).toFixed(3)} kWh ${tr("approximatifs","approximate")}`;
    const parts=Object.entries(cost.sites||{}).map(([site,v])=>`<li>${esc(site)} : ${esc(summary(v))}</li>`).join("");
    const machine=hub?Object.entries(cost.nodes||{}).map(([node,v])=>`<li>${esc(node)} : ${esc(summary(v))}</li>`).join(""):"";
    target.innerHTML=`<p><strong>${tr("Total", "Total")} : ${esc(summary(cost.global))}</strong></p>
       <small>${tr("Abonnement inclus uniquement si activé ; données manquantes non remplacées.","Subscription only if enabled; missing prices not filled.")}</small>
       <ul>${parts}${machine}</ul><p class="tf-help">${esc((cost.global?.warnings||[]).join(" · "))}</p>`;
  }
  async function calculate(){try{
    const a=root.querySelector("#tf-from").value,b=root.querySelector("#tf-to").value;
    const qs=new URLSearchParams();if(a)qs.set("from",new Date(a).toISOString());if(b)qs.set("to",new Date(b).toISOString());
    cost=await request("/cost?"+qs);renderCost();
  }catch(e){editorMsg(e.message)}}
  async function start(){try{
    conf=await request("");
    if(hub){const response=await fetch("/api/hub/snapshot");if(response.ok){const snap=await response.json();nodes=snap.nodes||[];}}
    render();
  }catch(e){root.textContent=tr("Tarification bêta indisponible : ","Beta tariff configuration unavailable: ")+e.message;}}
  start();
})();
