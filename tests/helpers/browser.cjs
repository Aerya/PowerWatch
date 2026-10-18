"use strict";
const fs=require("node:fs"),os=require("node:os"),path=require("node:path");
const {execFileSync}=require("node:child_process");
const chromium=["/usr/bin/chromium","/usr/bin/google-chrome"].find(fs.existsSync);
const read=file=>fs.readFileSync(path.join(__dirname,"../..",file),"utf8");
const script=source=>`<script>${source.replace(/<\/script/gi,"<\\/script")}</script>`;
function browser(html){
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),"powerwatch-page-"));
  try{
    const file=path.join(dir,"index.html");fs.writeFileSync(file,html);
    const output=execFileSync(chromium,["--headless","--no-sandbox","--disable-gpu","--disable-dev-shm-usage","--disable-background-networking","--no-first-run","--no-default-browser-check","--virtual-time-budget=4000","--dump-dom","file://"+file],{encoding:"utf8",timeout:60000,stdio:["ignore","pipe","ignore"]});
    const match=output.match(/<pre id="result">([^<]+)<\/pre>/);if(!match)throw Error("Browser did not complete the scenario");
    return JSON.parse(match[1].replaceAll("&quot;",'"').replaceAll("&amp;","&"));
  }finally{fs.rmSync(dir,{recursive:true,force:true});}
}
module.exports={browser,chromium,read,script};
