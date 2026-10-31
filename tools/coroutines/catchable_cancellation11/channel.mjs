import assert from 'node:assert/strict'
import {Worker} from 'node:worker_threads'
import {readFile,writeFile} from 'node:fs/promises'
import {WorkerChannel} from '../../../crates/monty-js/dist/worker/channel.js'
const out=new URL('../../../playground/coroutine-implementation/catchable-cancellation-compatibility11/',import.meta.url)
const identity=JSON.parse(await readFile(new URL('historical-component.json',out)))
const modules={}
for(const name of Object.keys(identity.files).filter(n=>n.endsWith('.wasm')))modules[name]=await WebAssembly.compile(await readFile(new URL('historical-component/'+name,out)))
await writeFile(new URL('channel-frozen.json',out),JSON.stringify({expected:{code:'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED',posts:0,subscriptions:0,ready:0,terminated:1},identity},null,2),{flag:'wx'})
const worker=new Worker(new URL('channel-worker.mjs',import.meta.url),{workerData:modules})
let posts=0,terminated=0,exitStatus,termination
const messages=[]
const exited=new Promise(resolve=>worker.on('exit',code=>{exitStatus=code;resolve()}))
let code
try{await WorkerChannel.create({post(m){posts++;worker.postMessage(m)},onMessage(fn){worker.on('message',m=>{messages.push(m);fn(m)})},onError(fn){worker.on('error',fn)},terminate(){terminated++;termination=worker.terminate();return termination.then(String)}})}catch(error){code=error.code}
await exited
const receipt={code,posts,terminated,exitStatus,messages}
await writeFile(new URL('channel-receipt.json',out),JSON.stringify(receipt,null,2),{flag:'wx'})
assert.equal(code,'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED');assert.equal(posts,0);assert.equal(terminated,1);assert.equal(messages.length,1);assert.equal(messages[0].subscriptions,0);assert(!messages.some(m=>m.ready))
console.log(JSON.stringify(receipt))
