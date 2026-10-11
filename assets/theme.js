/* Shared local appearance preferences for PowerWatch and Hub. */
(() => {
  "use strict";
  const root=document.documentElement;
  const prefix="powerwatchThemeV1:";
  const light=["default","cat-latte","nord-snow","solarized-light"];
  const dark=["default","cat-mocha","nord","dracula","gruvbox","tokyo-night","solarized-dark","rose-pine"];
  const names={default:"PowerWatch", "cat-latte":"Catppuccin Latte","nord-snow":"Nord Snow","solarized-light":"Solarized Light","cat-mocha":"Catppuccin Mocha",nord:"Nord",dracula:"Dracula",gruvbox:"Gruvbox","tokyo-night":"Tokyo Night","solarized-dark":"Solarized Dark","rose-pine":"Rosé Pine"};
  const read=(key,fallback)=>{try{return localStorage.getItem(prefix+key)||fallback;}catch{return fallback;}};
  const write=(key,value)=>{try{localStorage.setItem(prefix+key,value);}catch{ /* Appearance remains available for this tab. */ }};
  const media=matchMedia("(prefers-color-scheme: dark)");
  let mode=read("mode","auto"),day=read("day","default"),night=read("night","default");
  if(!["auto","light","dark"].includes(mode))mode="auto";
  if(!light.includes(day))day="default";
  if(!dark.includes(night))night="default";
  const tone=()=>mode==="auto"?(media.matches?"dark":"light"):mode;
  const apply=()=>{const active=tone();root.dataset.themeTone=active;root.dataset.themePalette=active==="dark"?night:day;root.style.colorScheme=active;window.dispatchEvent(new Event("powerwatch:themechange"));};
  apply();
  media.addEventListener?.("change",()=>{if(mode==="auto")apply();});
  const text=(fr,en)=>root.lang==="fr"?fr:en;
  function mount(){
    const language=document.querySelector(".language-switcher");if(!language)return;
    const control=document.createElement("details");control.className="theme-control";
    language.insertAdjacentElement("afterend",control);
    const render=()=>{
      const opened=control.open;
      const options=(items,selected)=>items.map(value=>`<option value="${value}" ${value===selected?"selected":""}>${names[value]}${value==="default"?` (${text("par défaut","default")})`:""}</option>`).join("");
      control.innerHTML=`<summary aria-label="${text("Apparence","Appearance")}">◐ ${text("Thème","Theme")}</summary><div class="theme-control-panel">
        <div class="theme-modes" role="group" aria-label="${text("Mode","Mode")}">${[["light",text("Clair","Light")],["dark",text("Sombre","Dark")],["auto","Auto"]].map(([value,label])=>`<button type="button" data-theme-mode="${value}" aria-pressed="${mode===value}">${label}</button>`).join("")}</div>
        <label>${text("Palette de jour","Day palette")}<select data-theme-day>${options(light,day)}</select></label>
        <label>${text("Palette de nuit","Night palette")}<select data-theme-night>${options(dark,night)}</select></label>
        <small>${text("Auto suit les préférences du système.","Auto follows your system preference.")}</small>
      </div>`;
      control.open=opened;
      control.querySelectorAll("[data-theme-mode]").forEach(button=>button.addEventListener("click",()=>{mode=button.dataset.themeMode;write("mode",mode);apply();render();}));
      control.querySelector("[data-theme-day]").addEventListener("change",event=>{day=event.target.value;write("day",day);apply();});
      control.querySelector("[data-theme-night]").addEventListener("change",event=>{night=event.target.value;write("night",night);apply();});
    };
    render();
    new MutationObserver(render).observe(root,{attributes:true,attributeFilter:["lang"]});
  }
  if(document.readyState==="loading")document.addEventListener("DOMContentLoaded",mount,{once:true});else mount();
})();
