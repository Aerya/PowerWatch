/* Energy-only cumulative cost, shared by the standalone and Hub canvases. */
(() => {
  "use strict";
  const tr=(fr,en)=>document.documentElement.lang==="en"?en:fr;
  const money=value=>Number(value).toLocaleString(document.documentElement.lang==="en"?"en-US":"fr-FR",{minimumFractionDigits:3,maximumFractionDigits:3})+" €";
  function create({endpoint,redraw}){
    let data=null,failed=false,loading=false,shown=true,sequence=0,controller=null,lastRequest=null;
    const label=()=>tr("Coût cumulé (€)","Cumulative cost (€)");
    const status=()=>failed?tr("indisponible","unavailable"):loading?tr("chargement…","loading…"):data?.enabled===false?tr("désactivé","disabled"):data&&!data.points?.some(p=>p.cost_eur!==null&&p.timestamp!==data.points[0]?.timestamp)?tr("sans données tarifées","no priced data"):data?.points?.at(-1)?.partial?tr("partiel","partial"):data?.points?.at(-1)?.estimated?tr("estimé","estimated"):"";
    async function refresh(from,to,nodes){
      if(!Number.isFinite(from)||!Number.isFinite(to)||to<=from)return;
      lastRequest=[from,to,nodes];const seq=++sequence;
      controller?.abort();controller=new AbortController();data=null;failed=false;loading=true;redraw();
      const params=new URLSearchParams({from:new Date(from).toISOString(),to:new Date(to).toISOString()});
      if(nodes!==undefined)params.set("nodes",nodes.join(','));
      try{const response=await fetch(endpoint+'?'+params,{cache:'no-store',signal:controller.signal});if(!response.ok)throw Error('HTTP '+response.status);const next=await response.json();if(seq===sequence){data=next;failed=false;}}
      catch(error){if(seq===sequence&&error.name!=="AbortError"){data=null;failed=true;}}
      finally{if(seq===sequence){loading=false;redraw();}}
    }
    function legend(container){
      const button=document.createElement('button');button.type='button';button.dataset.costCurve='';button.classList.toggle('hidden-series',!shown);button.setAttribute('aria-pressed',String(shown));
      const extra=status();button.textContent='┄ '+label()+(extra?' · '+extra:'');button.title=tr('Coût des kWh consommés depuis le début de la période, hors abonnement. Sur le Hub : total habituel.','Cost of energy consumed since the start of the period, excluding standing charge. Hub: normal total.');
      button.onclick=event=>{event.stopPropagation();shown=!shown;redraw();};container.appendChild(button);
    }
    function active(){return shown&&data?.enabled&&data.points?.some((p,i)=>i>0&&Number.isFinite(p.cost_eur));}
    function draw(ctx,{min,max,left,right,top,bottom,width,height}){
      if(!active())return;
      const points=data.points,values=points.map(p=>p.cost_eur).filter(Number.isFinite),ceiling=Math.max(...values,0.001);
      const x=t=>left+(t-min)/Math.max(1,max-min)*(width-left-right),y=v=>height-bottom-v/ceiling*(height-top-bottom);
      const color=getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()||'#58a6ff';
      ctx.save();ctx.font='10px monospace';ctx.textAlign='left';ctx.fillStyle=color;
      for(let i=0;i<=4;i++)ctx.fillText(money(ceiling*i/4),width-right+5,y(ceiling*i/4)+3);
      ctx.beginPath();ctx.rect(left,top,width-left-right,height-top-bottom);ctx.clip();ctx.lineWidth=2;ctx.strokeStyle=color;ctx.setLineDash([6,4]);ctx.beginPath();
      let started=false;for(const point of points){if(!Number.isFinite(point.cost_eur)){started=false;continue;}const px=x(Date.parse(point.timestamp)),py=y(point.cost_eur);if(started)ctx.lineTo(px,py);else{ctx.moveTo(px,py);started=true;}}
      ctx.stroke();ctx.restore();
    }
    function tooltip(time){
      if(!shown)return null;if(!data?.enabled)return failed?label()+' · '+status():null;
      const points=data.points||[];if(!points.length)return null;
      // Use the containing bucket's endpoint; never bridge an unpriced bucket.
      const point=points.find(p=>Date.parse(p.timestamp)>=time)||points.at(-1);
      if(!Number.isFinite(point.cost_eur))return label()+' : '+tr('période non chiffrée','unpriced period');
      return label()+' : '+money(point.cost_eur)+(point.partial?' · '+tr('partiel','partial'):'')+(point.estimated?' · '+tr('estimé','estimated'):'');
    }
    window.addEventListener('powerwatch:tariffschange',()=>{if(lastRequest)void refresh(...lastRequest);});
    window.addEventListener('storage',event=>{if(event.key==='powerwatchTariffsChanged'&&lastRequest)void refresh(...lastRequest);});
    return {refresh,legend,draw,tooltip,active};
  }
  window.PowerWatchCostChart={create};
})();
