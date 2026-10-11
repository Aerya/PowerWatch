"use strict";
const test=require("node:test");
const assert=require("node:assert/strict");
const fs=require("node:fs");
const os=require("node:os");
const path=require("node:path");
const {execFileSync}=require("node:child_process");
const chromium=["/usr/bin/chromium","/usr/bin/google-chrome"].find(fs.existsSync);
const source=fs.readFileSync(path.join(__dirname,"../assets/theme.js"),"utf8");
const style=fs.readFileSync(path.join(__dirname,"../assets/theme.css"),"utf8");

test("appearance controls persist mode and day/night palettes across both dashboards",{skip:!chromium},()=>{
  for(const page of ["powerwatch-web/static/index.html","powerwatch-hub/static/index.html"]){
    const markup=fs.readFileSync(path.join(__dirname,"..",page),"utf8");
    assert.match(markup,/\/static\/theme\.js/);assert.match(markup,/\/static\/theme\.css/);
  }
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-theme-"));
  try {
    const html=`<!doctype html><html lang="fr"><head><style>:root{--bg:#0d1117}</style><style>${style}</style><script>${source.replace(/<\/script/gi,"<\\/script")}</script></head><body><header><div class="language-switcher"></div></header><pre id="result">pending</pre><script>
document.addEventListener("DOMContentLoaded",()=>{const root=document.documentElement;
const tone=()=>root.dataset.themeTone,palette=()=>root.dataset.themePalette,bg=()=>getComputedStyle(root).getPropertyValue("--bg").trim();
const initial={tone:tone(),palette:palette()};
document.querySelector('[data-theme-mode="light"]').click();document.querySelector('[data-theme-day]').value="cat-latte";document.querySelector('[data-theme-day]').dispatchEvent(new Event("change"));
const light={tone:tone(),palette:palette(),bg:bg()};
document.querySelector('[data-theme-mode="dark"]').click();document.querySelector('[data-theme-night]').value="tokyo-night";document.querySelector('[data-theme-night]').dispatchEvent(new Event("change"));
const dark={tone:tone(),palette:palette(),bg:bg()};
document.querySelector('[data-theme-mode="auto"]').click();
document.querySelector("#result").textContent=JSON.stringify({initial,light,dark,stored:localStorage.getItem("powerwatchThemeV1:mode"),day:localStorage.getItem("powerwatchThemeV1:day"),night:localStorage.getItem("powerwatchThemeV1:night"),controls:document.querySelectorAll("[data-theme-mode]").length});});
</script></body></html>`;
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=2000","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);assert.ok(match);
    const result=JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
    assert.equal(result.controls,3);assert.equal(result.light.tone,"light");assert.equal(result.light.palette,"cat-latte");assert.equal(result.light.bg,"#eff1f5");
    assert.equal(result.dark.tone,"dark");assert.equal(result.dark.palette,"tokyo-night");assert.equal(result.dark.bg,"#16161e");
    assert.equal(result.stored,"auto");assert.equal(result.day,"cat-latte");assert.equal(result.night,"tokyo-night");
  } finally {fs.rmSync(dir,{recursive:true,force:true});}
});
