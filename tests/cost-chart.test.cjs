"use strict";
const test=require('node:test'),assert=require('node:assert/strict'),vm=require('node:vm');
const {read}=require('./helpers/browser.cjs');
function setup(){
 const calls=[],requests=[],events={};
 const context={document:{documentElement:{lang:'fr'},createElement:()=>({dataset:{},classList:{toggle(){}},setAttribute(){}})},window:{addEventListener:(name,handler)=>events[name]=handler},URLSearchParams,Date,Number,AbortController,
  getComputedStyle:()=>({getPropertyValue:()=> '#58a6ff'}),fetch:(url,opts)=>new Promise(resolve=>requests.push({url,opts,resolve})),};
 vm.createContext(context);vm.runInContext(read('assets/cost-chart.js'),context);
 const curve=context.window.PowerWatchCostChart.create({endpoint:'/api/hub/tariffs-fr/series',redraw:()=>calls.push('redraw')});
 return {curve,requests,calls,events};
}
const point=(second,cost,extra={})=>({timestamp:new Date(second*1000).toISOString(),cost_eur:cost,partial:false,estimated:false,...extra});
const respond=(request,data)=>request.resolve({ok:true,json:async()=>data});

test('cost curve ignores stale ranges, aborts requests and encodes an explicit empty Hub selection',async()=>{
 const {curve,requests}=setup();
 const old=curve.refresh(1000,4000);const latest=curve.refresh(5000,9000,[]);
 assert.equal(requests[0].opts.signal.aborted,true);assert.match(requests[1].url,/nodes=$/);
 respond(requests[1],{enabled:true,points:[point(5,0),point(9,2)]});await latest;
 respond(requests[0],{enabled:true,points:[point(1,0),point(4,99)]});await old;
 assert.equal(curve.active(),true);assert.match(curve.tooltip(8000),/2,000 €/);assert.doesNotMatch(curve.tooltip(8000),/99/);
});

test('cost curve draws gaps for unpriced energy and preserves partial and estimate labels',async()=>{
 const {curve,requests}=setup();const pending=curve.refresh(1000,5000);
 respond(requests[0],{enabled:true,points:[point(1,0),point(2,.2),point(3,null,{partial:true}),point(5,.4,{partial:true,estimated:true})]});await pending;
 assert.match(curve.tooltip(2500),/période non chiffrée/);assert.match(curve.tooltip(4500),/0,400 €.*partiel.*estimé/);
 const moves=[],lines=[],ctx={save(){},restore(){},fillText(){},beginPath(){},rect(){},clip(){},setLineDash(){},moveTo:(...v)=>moves.push(v),lineTo:(...v)=>lines.push(v),stroke(){}};
 curve.draw(ctx,{min:1000,max:5000,left:40,right:80,top:16,bottom:32,width:800,height:300});
 assert.equal(moves.length,2);assert.equal(lines.length,1,'the missing bucket is never connected across');
 const elements=[];curve.legend({appendChild:el=>elements.push(el)});assert.match(elements[0].textContent,/partiel/);elements[0].onclick({stopPropagation(){}});assert.equal(curve.active(),false);
});

test('disabled pricing and failed requests clear old costs and settings changes refresh the curve',async()=>{
 const {curve,requests,events}=setup();let pending=curve.refresh(1000,5000);
 respond(requests[0],{enabled:true,points:[point(1,0),point(5,1)]});await pending;assert.equal(curve.active(),true);
 pending=curve.refresh(1000,5000);respond(requests[1],{enabled:false,points:[]});await pending;assert.equal(curve.active(),false);assert.equal(curve.tooltip(4000),null);
 pending=curve.refresh(1000,5000);requests[2].resolve({ok:false,status:503});await pending;assert.equal(curve.active(),undefined);assert.match(curve.tooltip(4000),/indisponible/);
 events['storage']({key:'powerwatchTariffsChanged'});assert.equal(requests.length,4);respond(requests[3],{enabled:false,points:[]});
});

test('large cost histories draw without an argument overflow and retain the final maximum',async()=>{
 const {curve,requests}=setup();const pending=curve.refresh(1000,300000000);
 const points=Array.from({length:200000},(_,i)=>point(i+1,i===199999?100:1));
 respond(requests[0],{enabled:true,points});await pending;
 const labels=[];let lines=0;
 const ctx={save(){},restore(){},fillText:t=>labels.push(t),beginPath(){},rect(){},clip(){},setLineDash(){},moveTo(){},lineTo(x,y){assert.ok(Number.isFinite(x)&&Number.isFinite(y));lines++;},stroke(){}};
 curve.draw(ctx,{min:1000,max:300000000,left:40,right:80,top:16,bottom:32,width:800,height:300});
 assert.equal(lines,199999);assert.ok(labels.includes('100,000 €'));
});
