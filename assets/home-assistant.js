/* Home Assistant MQTT setup. No broker credentials are ever returned by GET. */
(() => {
  "use strict";
  const copy = {
    fr: {
      title:"Home Assistant · MQTT",intro:"Découverte automatique des équipements et capteurs dans Home Assistant",off:"Désactivé",online:"Connecté",connecting:"Connexion…",error:"Indisponible",enable:"Activer l'export MQTT",enableHelp:"Désactivé par défaut. Nécessite un broker MQTT accessible depuis ce conteneur et l'intégration MQTT dans Home Assistant.",
      connection:"Connexion au broker",host:"Adresse du broker (DNS ou IP)",port:"Port MQTT",tls:"Connexion chiffrée TLS (certificat valide)",user:"Identifiant MQTT (facultatif)",password:"Nouveau mot de passe MQTT",keep:"Laisser vide pour conserver le mot de passe enregistré",clear:"Effacer le mot de passe enregistré",
      publication:"Données publiées",interval:"Intervalle",components:"Capteurs CPU, GPU, RAM et disques",energy:"Compteurs kWh (machines + compteur global Hub indépendant)",costs:"Coûts estimés en euros (tarification France configurée)",scope:"Portée (Hub uniquement)",all:"Hub + toutes les machines",global:"Hub uniquement",nodes:"Machines uniquement",advanced:"Préfixes MQTT",prefix:"Préfixe des topics",discovery:"Préfixe Home Assistant Discovery",
      warning:"Les coûts sont des estimations recalculées à partir des tarifs configurés (ils peuvent évoluer si les tarifs ou la sélection changent) ; ils ne sont pas des relevés de facture. Les périodes non chiffrées ne produisent pas de valeur en euros. Les valeurs sont partielles ou estimées selon les capteurs. Sur le Hub, le compteur MQTT global est indépendant du cumul historique filtrable : il conserve les kWh déjà acquis et n’ajoute que les consommations des machines incluses et disponibles.",save:"Enregistrer",test:"Tester la connexion",saving:"Enregistrement…",testing:"Test MQTT…",saved:"Paramètres enregistrés. Le service applique les changements automatiquement.",success:"Connexion MQTT et authentification validées.",loadError:"Impossible de charger les paramètres",auth:"Le Hub demande un jeton administrateur pour ce réglage.",testHint:"Enregistre les réglages avant de tester.",invalid:"Renseigne une adresse de broker valide." },
    en: {
      title:"Home Assistant · MQTT",intro:"Automatically discover devices and sensors in Home Assistant",off:"Disabled",online:"Connected",connecting:"Connecting…",error:"Unavailable",enable:"Enable MQTT export",enableHelp:"Disabled by default. Requires a reachable MQTT broker and the Home Assistant MQTT integration.",
      connection:"Broker connection",host:"Broker address (DNS or IP)",port:"MQTT port",tls:"Encrypted TLS connection (valid certificate)",user:"MQTT username (optional)",password:"New MQTT password",keep:"Leave empty to keep the stored password",clear:"Clear stored password",
      publication:"Published data",interval:"Interval",components:"CPU, GPU, RAM and disk sensors",energy:"kWh counters (machines + independent Hub-wide meter)",costs:"Estimated costs in EUR (requires French tariff configuration)",scope:"Scope (Hub only)",all:"Hub + all machines",global:"Hub only",nodes:"Machines only",advanced:"MQTT prefixes",prefix:"Topic prefix",discovery:"Home Assistant Discovery prefix",
      warning:"EUR costs are recalculated tariff estimates (they may change when rates or selected machines change), not billed amounts. Unpriced history does not produce a monetary sensor. Values can be partial or estimated. The Hub-wide MQTT kWh meter is independent of the selectable historical total: it preserves recorded energy and adds only new consumption from included, online nodes.",save:"Save settings",test:"Test connection",saving:"Saving…",testing:"Testing MQTT…",saved:"Settings saved. The service applies changes automatically.",success:"MQTT connection and authentication verified.",loadError:"Failed to load settings",auth:"This Hub setting requires an administrator token.",testHint:"Save the settings before testing.",invalid:"Enter a valid broker address." }
  };
  function language(hub){
    const key=hub?"powerwatchHubLang":"powerwatch-language";
    return (localStorage.getItem(key)||document.documentElement.lang||"en").startsWith("fr")?"fr":"en";
  }
  function init({endpoint,hub=false,request=fetch}) {
    const mount=document.getElementById("pw-ha-mount");if(!mount)return;
    const label=(key)=>copy[language(hub)][key];
    mount.innerHTML=`<details class="pw-ha" id="pw-ha-panel"><summary><span class="pw-ha-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10.5 12 3l9 7.5V21h-6v-7H9v7H3z"/><path d="M17.5 3.5a7 7 0 0 1 3 4M15.5 6a4 4 0 0 1 2 2"/></svg></span><span class="pw-ha-head"><strong data-ha="title"></strong><small data-ha="intro"></small></span><span class="pw-ha-tag" id="pw-ha-status" data-status="off"></span><span class="pw-ha-chevron" aria-hidden="true">⌄</span></summary>
      <div class="pw-ha-body"><div class="pw-ha-switch-row"><div><strong data-ha="enable"></strong><p class="pw-ha-desc" data-ha="enableHelp"></p></div><input type="checkbox" class="pw-ha-toggle" id="pw-ha-enabled" aria-label="Enable MQTT"></div>
      <div class="pw-ha-settings" id="pw-ha-settings" hidden><h3 class="pw-ha-subtitle" data-ha="connection"></h3><div class="pw-ha-grid">
      <label class="pw-ha-field pw-ha-span"><span class="pw-ha-label" data-ha="host"></span><input id="pw-ha-host" autocomplete="off" placeholder="192.168.0.20" maxlength="253"></label>
      <label class="pw-ha-field"><span class="pw-ha-label" data-ha="port"></span><input id="pw-ha-port" type="number" min="1" max="65535" value="1883"></label>
      <label class="pw-ha-field"><span class="pw-ha-label" data-ha="interval"></span><select id="pw-ha-interval"><option value="10">10 s</option><option value="30">30 s</option><option value="60">60 s</option><option value="120">120 s</option><option value="300">300 s</option><option value="600">600 s</option></select></label>
      <label class="pw-ha-option pw-ha-span"><input id="pw-ha-tls" type="checkbox"><span data-ha="tls"></span></label>
      <label class="pw-ha-field"><span class="pw-ha-label" data-ha="user"></span><input id="pw-ha-user" autocomplete="username" maxlength="256"></label>
      <label class="pw-ha-field"><span class="pw-ha-label" data-ha="password"></span><input id="pw-ha-password" type="password" autocomplete="new-password" placeholder="••••••••"><small class="pw-ha-desc" data-ha="keep"></small></label>
      <label class="pw-ha-option pw-ha-span"><input id="pw-ha-clear" type="checkbox"><span data-ha="clear"></span></label></div>
      <h3 class="pw-ha-subtitle" data-ha="publication"></h3><div class="pw-ha-grid">
      <label class="pw-ha-option"><input type="checkbox" id="pw-ha-components" checked><span data-ha="components"></span></label>
      <label class="pw-ha-option"><input type="checkbox" id="pw-ha-energy" checked><span data-ha="energy"></span></label>
      <label class="pw-ha-option pw-ha-span"><input type="checkbox" id="pw-ha-costs" checked><span data-ha="costs"></span></label>
      ${hub?`<label class="pw-ha-field pw-ha-span"><span class="pw-ha-label" data-ha="scope"></span><select id="pw-ha-scope"><option value="all" data-ha="all"></option><option value="global" data-ha="global"></option><option value="nodes" data-ha="nodes"></option></select></label>`:""}
      </div><h3 class="pw-ha-subtitle" data-ha="advanced"></h3><div class="pw-ha-grid">
      <label class="pw-ha-field"><span class="pw-ha-label" data-ha="prefix"></span><input id="pw-ha-prefix" value="powerwatch" maxlength="60"></label>
      <label class="pw-ha-field"><span class="pw-ha-label" data-ha="discovery"></span><input id="pw-ha-discovery" value="homeassistant" maxlength="60"></label>
      </div><p class="pw-ha-desc" data-ha="warning"></p></div>
      <div class="pw-ha-actions"><span class="pw-ha-feedback" id="pw-ha-message" aria-live="polite"></span><button type="button" id="pw-ha-test" data-ha="test"></button><button type="button" class="pw-ha-primary" id="pw-ha-save" data-ha="save"></button></div></div></details>`;
    const $=(id)=>mount.querySelector("#pw-ha-"+id);
    const renderLanguage=()=>mount.querySelectorAll("[data-ha]").forEach(el=>{el.textContent=label(el.dataset.ha);});
    const say=(message,kind="")=>{const el=$("message");el.textContent=message;el.dataset.kind=kind;};
    const refreshStatus=(data)=>{const el=$("status");el.textContent=data.enabled?(data.connected?label("online"):(data.status==="connecting"?label("connecting"):label("error"))):label("off");el.dataset.status=data.enabled&&data.connected?"online":"off";el.title=data.status||"";};
    function expanded(){ $("settings").hidden=!$("enabled").checked; }
    $("enabled").addEventListener("change",expanded);
    const fetchJson=async(url,init={})=>{const response=await request(url,{...init,cache:"no-store"});if(response&&typeof response.json==="function"&&typeof response.ok==="boolean"){if(!response.ok){if(response.status===401)throw new Error(label("auth"));throw new Error((await response.text()).slice(0,220));}return response.json();}return response;};
    const state={enabled:false,has_password:false};
    let loaded=false;
    async function load(){
      try { const cfg=await fetchJson(endpoint);Object.assign(state,cfg);
        $("enabled").checked=cfg.enabled;$("host").value=cfg.host||"";$("port").value=cfg.port||1883;$("tls").checked=cfg.tls;
        $("user").value=cfg.username||"";$("interval").value=cfg.interval_seconds||30;
        $("prefix").value=cfg.topic_prefix||"powerwatch";$("discovery").value=cfg.discovery_prefix||"homeassistant";
        $("components").checked=cfg.publish_components;$("energy").checked=cfg.publish_energy;$("costs").checked=cfg.publish_costs!==false;
        if(hub)$("scope").value=cfg.scope||"global";
        $("password").value="";$("clear").checked=false;expanded();refreshStatus(cfg);say("");loaded=true;
      }catch(err){say(`${label("loadError")}: ${err.message}`,"error");}
    }
    const busy=(value)=>{for(const el of [$("test"),$("save")])el.disabled=value;};
    $("save").addEventListener("click",async()=>{
      const config={enabled:$("enabled").checked,host:$("host").value.trim(),port:Number($("port").value),tls:$("tls").checked,
        username:$("user").value,password:$("password").value||null,clear_password:$("clear").checked,
        interval_seconds:Number($("interval").value),publish_components:$("components").checked,publish_energy:$("energy").checked,publish_costs:$("costs").checked,
        scope:hub?$("scope").value:"global",topic_prefix:$("prefix").value.trim(),discovery_prefix:$("discovery").value.trim()};
      if(config.enabled&&!config.host){say(label("invalid"),"error");return;}
      busy(true);say(label("saving"));
      try{const data=await fetchJson(endpoint,{method:"PUT",headers:{"Content-Type":"application/json"},body:JSON.stringify(config)});
        $("password").value="";$("clear").checked=false;Object.assign(state,data);refreshStatus(data);say(label("saved"),"success");
      }catch(err){say(err.message,"error");}finally{busy(false);}
    });
    $("test").addEventListener("click",async()=>{
      busy(true);say(label("testing"));try{await fetchJson(endpoint+"/test",{method:"POST"});say(label("success"),"success");}
      catch(err){say(err.message,"error");}finally{busy(false);}
    });
    renderLanguage();load();
    if(hub)window.addEventListener("powerwatch:hubauthchange",()=>{if(!loaded)void load();});
    window.addEventListener("powerwatch:languagechange",()=>{renderLanguage();refreshStatus(state);});
    // Hub uses its own language switcher rather than the local i18n event.
    if(hub){document.querySelectorAll("#lang-fr,#lang-en").forEach(el=>el.addEventListener("click",()=>setTimeout(()=>{renderLanguage();refreshStatus(state);},0)));}
    mount.querySelector("details").addEventListener("toggle",async e=>{if(e.target.open&&!loaded)await load();});
    setInterval(async()=>{if(!mount.querySelector("details").open)return;try{const cfg=await fetchJson(endpoint);Object.assign(state,cfg);refreshStatus(cfg);}catch(_){}},15000);
  }
  window.PowerWatchHA={init};
})();
