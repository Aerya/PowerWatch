"use strict";
const test=require('node:test'),assert=require('node:assert/strict'),vm=require('node:vm');
const {read}=require('./helpers/browser.cjs');
const html=read('powerwatch-hub/static/index.html');
function setup(hidden=[]){
  const labels=[],coordinates=[];let strokes=0;
  const ctx={setTransform(){},clearRect(){},beginPath(){},moveTo:(...p)=>coordinates.push(p),lineTo:(...p)=>coordinates.push(p),stroke(){strokes++;},fillText:t=>labels.push(t)};
  const context={document:{getElementById:()=>({getBoundingClientRect:()=>({width:1000,height:300}),getContext:()=>ctx})},window:{devicePixelRatio:1},hiddenSeries:new Set(hidden),historyHover:null,
    nodeName:k=>k,renderLegend(){},varColor:()=> '#fff',timeAxisTicks:(a,b)=>[a,b],formatAxisTime:()=> 'date',seriesColor:()=> '#fff',tr:k=>k};
  vm.createContext(context);
  for(const [start,end] of [['function allSeries(', 'function seriesColor('],['function valueFor(', 'function renderLegend('],['function drawHistory(', 'function nearestHistoryIndex(']])
    vm.runInContext(html.slice(html.indexOf(start),html.indexOf(end)),context);
  return {draw:context.drawHistory,labels,coordinates,strokes:()=>strokes};
}
test('Hub draws 30 days for nine machines without overflowing the argument limit',()=>{
  const points=Array.from({length:30*1440},(_,i)=>({timestamp:new Date(1700000000000+i*60000).toISOString(),global_watts:45,nodes:Object.fromEntries(Array.from({length:9},(_,j)=>['n'+j,5]))}));
  points.at(-1).global_watts=90;
  const chart=setup();chart.draw(points);
  assert.ok(chart.strokes()>=10,'all power series are drawn');
  assert.ok(chart.labels.includes('90'),'the last point contributes to the axis maximum');
  assert.ok(chart.coordinates.every(p=>p.every(Number.isFinite)));
});
test('Hub scale ignores hidden and non-finite readings and keeps a nonzero empty scale',()=>{
  const points=[{timestamp:'2026-10-01T00:00:00Z',global_watts:1000,nodes:{a:12,b:NaN}},{timestamp:'2026-10-02T00:00:00Z',global_watts:1000,nodes:{a:8,b:Infinity}}];
  const chart=setup(['__global__']);chart.draw(points);
  assert.ok(chart.labels.includes('12'));assert.ok(!chart.labels.includes('1000'));
  assert.ok(chart.coordinates.every(p=>p.every(Number.isFinite)));
  const hidden=setup(['__global__','a','b']);hidden.draw(points);
  assert.ok(hidden.labels.includes('1'));assert.ok(hidden.coordinates.every(p=>p.every(Number.isFinite)));
  const empty=setup();empty.draw([]);assert.ok(empty.labels.includes('noHistory'));
});
