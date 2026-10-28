import assert from 'node:assert/strict'
import {readFile,writeFile} from 'node:fs/promises'
import {createHash} from 'node:crypto'
import {WASIShim} from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import {instantiate} from '../../../crates/monty-js/dist/worker/component/monty.component.js'
const [route,mode,restoreText,output]=process.argv.slice(2)
const restore=Number(restoreText)
const directory=new URL('../../../crates/monty-js/dist/worker/component/',import.meta.url)
const raw=[]
let component
async function fresh(){component=await instantiate(async path=>WebAssembly.compile(await readFile(new URL(path,directory))),new WASIShim({sandbox:{preopens:{},env:{},args:[]}}).getImportObject());assert.equal(component.worker.cancellationContract(),7);assert.equal(dispatch({tag:'configure',val:{scriptName:'compatibility11.py',typeCheck:false,typeCheckFormat:'full',typeCheckColor:false}}).tag,'ok')}
function dispatch(request){const result=component.worker.dispatch(request);raw.push({request,result});assert.equal(result.events.length,1);return result.events[0]}
function feed(code){return dispatch({tag:'feed',val:{code,inputs:[{name:'exchange',value:0}],values:{nodes:[{tag:'function',val:{name:'exchange'}}]},skipTypeCheck:true,cwd:''}})}
function pending(event){assert.equal(event.tag,'function-call');assert.equal(event.val.functionName,'exchange');return dispatch({tag:'resume-call',val:{callId:event.val.callId,outcome:{tag:'pending-future',val:event.val.callId},values:{nodes:[]}}})}
const snapshots=[]
async function reload(event,cut){const snap=dispatch({tag:'dump'});assert.equal(snap.tag,'dump-result');const bytes=Buffer.from(snap.val);await writeFile(output+`.cut-${cut}.bin`,bytes,{flag:'wx'});snapshots.push({cut,sha256:createHash('sha256').update(bytes).digest('hex')});assert.equal(dispatch({tag:'reset'}).tag,'ok');component=null;await fresh();const loaded=dispatch({tag:'load',val:snap.val});assert.equal(loaded.tag,event.tag);return loaded}
function resume(event,reason){assert.equal(event.tag,'resolve-futures');return dispatch({tag:'resume-futures',val:{nativeCancel:reason,results:[{callId:event.val.pendingCallIds[0],outcome:{tag:'return-value',val:0}}],values:{nodes:[{tag:'none'}]}}})}
try {
await fresh()
assert.equal(dispatch({tag:'load',val:new Uint8Array(await readFile(new URL('../../../crates/monty/tests/fixtures/compatibility11_frozen_v17.bin',import.meta.url)))}).tag,'ok')
assert.equal(feed(await readFile(new URL('activation.py',import.meta.url),'utf8')).tag,'complete')
let event=pending(feed(`from __monty_host__ import host_coroutines\ncurrent('${mode}', '${route}')._native1()`))
if(restore===1)event=await reload(event,0)
event=resume(event,mode==='ordinary'?undefined:'FIRST')
if(mode==='delivered'){event=pending(event);if(restore===2)event=await reload(event,1);event=resume(event,'REPEAT')}
if(mode==='pending'){
 assert.equal(event.tag,'error');assert.equal(event.val.excType,'RuntimeError');assert.equal(event.val.message,'native cancellation requires catchable cancellation policy 7')
 assert.equal(feed('assert MARKERS == []').tag,'complete')
 const checks={frozen:"assert functools.reduce(fold, [TOKEN, TOKEN]) is TOKEN\nassert MARKERS == ['fold']",coroutine:"from __monty_host__ import host_coroutines\nassert legacy_coroutine(TOKEN)._native1() == 'legacy-ok'\nassert MARKERS == ['legacy-coroutine']",new:"assert next(legacy_new) is TOKEN\nassert MARKERS == ['legacy-generator']",dormant:"assert legacy_dormant.send(TOKEN) is TOKEN\nassert MARKERS == ['legacy-resumed']",mixed:"assert mixed_generator.send(TOKEN) is TOKEN\nassert MARKERS == ['legacy-resumed']"}
 assert.equal(feed(checks[route]).tag,'complete')
}else{
 assert.equal(event.tag,'complete')
 const nodes=event.val.values.nodes
 if(mode==='ordinary')assert.equal(nodes[event.val.value].val,'ok')
 else{const fields=Object.fromEntries(nodes[event.val.value].val.map(p=>[nodes[p.key].val,nodes[p.value]]));assert.equal(fields.status.val,'cancelled');assert.equal(fields.reason.val,'FIRST');assert.equal(fields.interrupted_error.tag,'none');assert.equal(fields.error.tag,'none')}
 const marker={frozen:'fold',coroutine:'legacy-coroutine',new:'legacy-generator',dormant:'legacy-resumed',mixed:'legacy-resumed'}[route]
 assert.equal(feed(`assert MARKERS == ['${marker}', 'ran']`).tag,'complete')
}
assert.equal(dispatch({tag:'reset'}).tag,'ok')
await writeFile(output,JSON.stringify({passed:true,pid:process.pid,route,mode,restore,snapshots,raw},(_,v)=>typeof v==='bigint'?{bigint:String(v)}:v,2),{flag:'wx'})
console.log(JSON.stringify({passed:true,route,mode,restore}))
}catch(error){await writeFile(output,JSON.stringify({passed:false,pid:process.pid,error:String(error),stack:error.stack,raw},(_,v)=>typeof v==='bigint'?{bigint:String(v)}:v,2),{flag:'wx'});throw error}
