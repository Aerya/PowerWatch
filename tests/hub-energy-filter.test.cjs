'use strict';
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const path=require('node:path');

const html=fs.readFileSync(path.join(__dirname,'../powerwatch-hub/static/index.html'),'utf8');
const match=html.match(/\/\* hub-energy-aggregation-start \*\/([\s\S]*?)\/\* hub-energy-aggregation-end \*\//);
assert.ok(match,'hub energy sum helpers must be embedded in the UI');
const {hubSelectEnergyGroup}=vm.runInNewContext(`${match[1]}\n({hubSelectEnergyGroup})`,{});

function sample(kwh,coverage,first){return {
 energy_kwh:kwh, estimated_kwh:kwh/2, coverage_seconds:coverage,
 first_seen_epoch:first, from_epoch:100, to_epoch:1000
};}
const group=kwh=>({windows:{'24h':sample(kwh,600,100),'7d':sample(kwh*7,4200,100),'30d':sample(kwh*30,18000,100),'all':sample(kwh*90,54000,100)},selected:sample(kwh*2,1200,100)});
const response={
 global:group(3),
 nodes:{'node-a':group(1),'node-b':group(2),'node-c':group(3)}
};

test('normal total stays the exact server global (respects Hub flags)',()=>{
 assert.equal(hubSelectEnergyGroup(response,['node-a','node-b'],true),response.global);
});
test('one node computes every timeframe and custom range from that node only',()=>{
 const data=hubSelectEnergyGroup(response,['node-b'],false);
 assert.equal(data.windows['24h'].energy_kwh,2);
 assert.equal(data.windows['7d'].energy_kwh,14);
 assert.equal(data.windows['30d'].energy_kwh,60);
 assert.equal(data.windows.all.energy_kwh,180);
 assert.equal(data.selected.energy_kwh,4);
 assert.equal(data.windows['24h'].coverage_seconds,600);
});
test('multiple nodes sum kWh and node-hours without adding server global twice',()=>{
 const data=hubSelectEnergyGroup(response,['node-a','node-b'],false);
 assert.equal(data.windows['24h'].energy_kwh,3);
 assert.equal(data.windows['24h'].estimated_kwh,1.5);
 assert.equal(data.windows['24h'].coverage_seconds,1200);
 assert.equal(data.selected.energy_kwh,6);
});
test('all configured nodes includes otherwise-excluded nodes',()=>{
 const data=hubSelectEnergyGroup(response,Object.keys(response.nodes),false);
 assert.equal(data.windows['24h'].energy_kwh,6);
 assert.equal(data.selected.energy_kwh,12);
});
test('empty selection is not confused with the server global',()=>{
 const data=hubSelectEnergyGroup(response,[],false);
 assert.equal(data.windows['24h'].energy_kwh,0);
 assert.equal(data.windows['24h'].first_seen_epoch,null);
 assert.equal(data.selected.first_seen_epoch,null);
});
test('missing and data-less nodes are not counted as observed energy',()=>{
 const empty=group(0);
 for(const item of [...Object.values(empty.windows),empty.selected])item.first_seen_epoch=null;
 const data=hubSelectEnergyGroup({...response,nodes:{...response.nodes,empty}},['unknown','empty','node-a'],false);
 assert.equal(data.windows['24h'].energy_kwh,1);
 assert.equal(data.windows['24h'].first_seen_epoch,100);
});
test('the interface includes compact controls and instance picker',()=>{
 assert.match(html,/id="hub-energy-preset-all"/);
 assert.match(html,/id="hub-energy-preset-included"/);
 assert.match(html,/font-size:\.78rem;line-height:1\.2;min-height:30px/);
 assert.match(html,/data-energy-id/);
});
