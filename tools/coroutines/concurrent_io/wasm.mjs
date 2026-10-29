// Actual Rust Wasm worker, fresh instance in every process; no guest scheduler.
import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { WASIShim } from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import { instantiate } from '../../../crates/monty-js/dist/worker/component/monty.component.js'

const [rawMode, source, replyText, output] = process.argv.slice(2)
const step = rawMode.endsWith('-step')
const mode = rawMode.replace(/-step$/, '')
const directory = new URL('../../../crates/monty-js/dist/worker/component/', import.meta.url)
const imports = new WASIShim({ sandbox: { preopens: {}, env: {}, args: [] } }).getImportObject()
const component = await instantiate(async path => WebAssembly.compile(await readFile(new URL(path, directory))), imports)
assert.equal(component.worker.cancellationContract(), 7)
function decodeRecord(values, index, summary) {
  function node(id) {
    assert(Number.isInteger(id) && id >= 0 && id < values.nodes.length, 'invalid reference')
    return values.nodes[id]
  }
  const record = node(index)
  assert.equal(record.tag, 'dict')
  const keys = summary ? ['type','message','message_truncated','representation']
    : ['status','reason','value','error','interrupted_error','value_export_error']
  const seen = new Set()
  for (const pair of record.val) {
    const key = node(pair.key)
    assert.equal(key.tag, 'text')
    assert(keys.includes(key.val) && !seen.has(key.val), 'unknown or duplicate key')
    seen.add(key.val)
    node(pair.value)
  }
  assert.equal(seen.size, keys.length)
  return Object.fromEntries(record.val.map(pair => {
    const key = node(pair.key).val
    const value = node(pair.value)
    if (!summary && key === 'interrupted_error' && value.tag !== 'none') return [key,decodeRecord(values,pair.value,true)]
    if (summary && key === 'message_truncated') {
      assert.equal(value.tag,'boolean'); assert.equal(typeof value.val,'boolean'); return [key,value.val]
    }
    if (value.tag === 'none') {
      assert(['message','value','error','interrupted_error','value_export_error'].includes(key)); return [key,null]
    }
    assert.equal(value.tag,'text'); assert.equal(typeof value.val,'string'); return [key,value.val]
  }))
}
function dispatch(request) {
  const result = component.worker.dispatch(request)
  assert.equal(result.status, 'continue')
  assert.equal(result.events.length, 1)
  return result.events[0]
}
function feed(code, inputs = [], nodes = []) {
  return dispatch({tag:'feed', val:{code, inputs, values:{nodes}, skipTypeCheck:true, cwd:''}})
}
function resume(event, batch) {
  if (batch.ack) {
    assert.equal(event.tag, 'function-call')
    const id = event.val.callId
    return dispatch({tag:'resume-call',val:{callId:id,outcome:{tag:'pending-future',val:id},values:{nodes:[]}}})
  }
  assert.equal(event.tag, 'resolve-futures')
  const nativeCancel = batch.cancel
  batch = batch.results ?? batch
  const results = batch.map((reply, index) => ({callId:reply.id, outcome:{tag:'return-value', val:index}}))
  return dispatch({tag:'resume-futures', val:{nativeCancel, results, values:{nodes:batch.map(reply => ({tag:'text', val:reply.value}))}}})
}
function boundary(event) {
  if (step && event.tag === 'function-call') {
    return [event, {call:{id:event.val.callId,label:event.val.values.nodes[event.val.args[0]].val}}]
  }
  const calls = []
  while (event.tag === 'function-call') {
    assert.equal(event.val.functionName, 'exchange')
    const node = event.val.values.nodes[event.val.args[0]]
    assert.equal(node.tag, 'text')
    calls.push({id:event.val.callId, label:node.val})
    const id = event.val.callId
    event = dispatch({tag:'resume-call',val:{callId:id,outcome:{tag:'pending-future',val:id},values:{nodes:[]}}})
  }
  if (event.tag === 'resolve-futures') return [event, {calls, pending:[...event.val.pendingCallIds].sort((a,b) => a-b)}]
  assert.equal(event.tag, 'complete', JSON.stringify(event))
  const node = event.val.values.nodes[event.val.value]
  return [event, {complete:node.tag === 'text' ? JSON.parse(node.val) : decodeRecord(event.val.values,event.val.value,false)}]
}
assert.equal(dispatch({tag:'configure',val:{scriptName:'concurrent_io.py',typeCheck:false,typeCheckFormat:'full',typeCheckColor:false}}).tag,'ok')
let event
if (mode === 'start' || mode === 'run') {
  assert.equal(feed(await readFile(new URL('wrappers.py', import.meta.url), 'utf8')).tag, 'complete')
  assert.equal(feed(await readFile(new URL('scope.py', import.meta.url), 'utf8')).tag, 'complete')
  event = feed(await readFile(source,'utf8'), [{name:'exchange',value:0}], [{tag:'function',val:{name:'exchange'}}])
} else {
  event = dispatch({tag:'load',val:new Uint8Array(await readFile(source))})
  assert(['resolve-futures','function-call'].includes(event.tag))
  if (mode === 'dispose') {
    assert.equal(dispatch({tag:'reset'}).tag,'ok')
    console.log('{"disposed":true}')
    process.exit(0)
  }
  if (mode === 'reject') {
    const batch = JSON.parse(replyText)
    const results = batch.map((reply,index) => ({callId:reply.id,outcome:reply.future
      ? {tag:'pending-future',val:0} : {tag:'return-value',val:index}}))
    const rejected = dispatch({tag:'resume-futures',val:{results,values:{nodes:batch.map(reply => ({tag:'text',val:reply.value ?? ''}))}}})
    assert.equal(rejected.tag,'error',JSON.stringify(rejected))
    const snapshot = dispatch({tag:'dump'})
    assert.equal(snapshot.tag,'dump-result')
    await writeFile(output,snapshot.val,{flag:'wx'})
    console.log(JSON.stringify({rejected:rejected.val}))
    process.exit(0)
  }
  event = resume(event, JSON.parse(replyText))
}
let view
;[event, view] = boundary(event)
if (mode === 'run') {
  const views = [view]
  for (const batch of JSON.parse(await readFile(replyText,'utf8'))) {
    ;[event, view] = boundary(resume(event,batch))
    views.push(view)
  }
  assert.equal(event.tag, 'complete')
  console.log(JSON.stringify(views))
} else {
  if (event.tag === 'resolve-futures' || event.tag === 'function-call') {
    const snapshot = dispatch({tag:'dump'})
    assert.equal(snapshot.tag,'dump-result')
    await writeFile(mode === 'start' ? replyText : output,snapshot.val,{flag:'wx'})
  }
  console.log(JSON.stringify(view))
}
assert.equal(dispatch({tag:'reset'}).tag,'ok')
