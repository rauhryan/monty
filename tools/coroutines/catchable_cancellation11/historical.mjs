import assert from 'node:assert/strict'
import {readFile,writeFile} from 'node:fs/promises'
import {createHash} from 'node:crypto'
import {WASIShim} from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import {instantiate as current} from '../../../crates/monty-js/dist/worker/component/monty.component.js'
import {admitCancellationContract} from '../../../crates/monty-js/dist/worker/host.js'
const root = new URL('../../../',import.meta.url)
const out = new URL('playground/coroutine-implementation/catchable-cancellation-compatibility11/',root)
const oldDir = new URL('historical-component/',out)
const {instantiate: old} = await import(new URL('monty.component.js',oldDir))
const hash = b => createHash('sha256').update(b).digest('hex')
const identity = JSON.parse(await readFile(new URL('historical-component.json',out)))
const modules = {}
for (const [name,sha] of Object.entries(identity.files)) {
  const bytes = await readFile(new URL(name,oldDir)); assert.equal(hash(bytes),sha)
  if (name.endsWith('.wasm')) modules[name]=await WebAssembly.compile(bytes)
}
const imports = () => new WASIShim({sandbox:{preopens:{},env:{},args:[]}}).getImportObject()
const frozen = {identity,currentGlue:hash(await readFile(new URL('crates/monty-js/dist/worker/component/monty.component.js',root))),source:hash(await readFile(new URL('historical_guest.py',import.meta.url))),expected:{mixed:['LINK_REJECTED','CAPABILITY_REFUSED'],dispatch:0,ready:0,subscription:0,ordinaryFeed:'complete',oldDumpVersion:17}}
await writeFile(new URL('historical-expectations.json',out),JSON.stringify(frozen,null,2),{flag:'wx'})
let mixed
let instance
let dispatches=0
try {
  instance=await current(path=>modules[path.replace(/^\.\//,'')],imports())
  const worker={...instance.worker,dispatch(req){dispatches++; return instance.worker.dispatch(req)}}
  try {admitCancellationContract(worker); mixed={status:'INCORRECT_ADMISSION'}}
  catch(error){mixed={status:'CAPABILITY_REFUSED',code:error.code,error:String(error),cause:String(error.cause)}}
} catch(error){mixed={status:'LINK_REJECTED',error:String(error)}}
// No component-level dispose ABI exists: release every reference to this failed
// instance, without sending reset/configure through the refused dispatcher.
instance=null
mixed={...mixed,dispatches,ready:0,subscription:0,disposal:'references released; process exit disposes Wasm memory'}
assert(frozen.expected.mixed.includes(mixed.status)); assert.equal(dispatches,0)
const producer=await old(path=>modules[path.replace(/^\.\//,'')],imports())
const raw=[]
function dispatch(request){const result=producer.worker.dispatch(request);raw.push({request,result});assert.equal(result.events.length,1);return result.events[0]}
assert.equal(dispatch({tag:'configure',val:{scriptName:'historical-frozen.py',typeCheck:false,typeCheckFormat:'full',typeCheckColor:false}}).tag,'ok')
assert.equal(dispatch({tag:'feed',val:{code:await readFile(new URL('historical_guest.py',import.meta.url),'utf8'),inputs:[],values:{nodes:[]},skipTypeCheck:true,cwd:''}}).tag,'complete')
const snapshot=dispatch({tag:'dump'});assert.equal(snapshot.tag,'dump-result')
assert.equal(Buffer.from(snapshot.val).subarray(0,8).toString('hex'),'4d4f4e5459001100')
await writeFile(new URL('crates/monty/tests/fixtures/compatibility11_frozen_v17.bin',root),snapshot.val,{flag:'wx'})
assert.equal(dispatch({tag:'reset'}).tag,'ok')
await writeFile(new URL('historical-receipt.json',out),JSON.stringify({pid:process.pid,mixed,snapshot:hash(snapshot.val),raw},(_,v)=>typeof v==='bigint'?{bigint:String(v)}:v,2),{flag:'wx'})
console.log(JSON.stringify({mixed,snapshot:hash(snapshot.val)}))
