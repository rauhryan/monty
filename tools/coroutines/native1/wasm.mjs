// The transport mirrors native.rs; all execution/scheduling is in actual Rust Wasm.
import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { WASIShim } from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import { instantiate } from '../../../crates/monty-js/dist/worker/component/monty.component.js'

const [mode, source, replyText, output] = process.argv.slice(2)
const directory = new URL('../../../crates/monty-js/dist/worker/component/', import.meta.url)
const imports = new WASIShim({ sandbox: { preopens: {}, env: {}, args: [] } }).getImportObject()
const component = await instantiate(async path => WebAssembly.compile(await readFile(new URL(path, directory))), imports)
// Each instance must prove admission before even configure.
try {
  const capability = component.worker.cancellationContract
  if (typeof capability !== 'function') throw new Error('missing export')
  const revision = capability()
  if (typeof revision !== 'number' || !Number.isInteger(revision) || revision !== 7) throw new Error('wrong revision')
} catch (cause) {
  throw Object.assign(new Error('MONTY_CANCELLATION_CONTRACT_UNSUPPORTED', {cause}), {code:'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED'})
}
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
function text(values, index) {
  assert.equal(values.nodes[index].tag, 'text')
  return values.nodes[index].val
}
function resume(event, replyText) {
  assert.equal(event.tag, 'resolve-futures')
  assert.equal(event.val.pendingCallIds.length, 1)
  const reply = JSON.parse(replyText)
  if (reply !== null && typeof reply === 'object' && ('native_cancel' in reply || 'native_results' in reply)) {
    const nodes = []
    const results = (reply.native_results ?? []).map(result => {
      const answer = result.reply
      const throwing = answer !== null && typeof answer === 'object' && 'raise' in answer
      const index = nodes.length
      if (!throwing) nodes.push({tag:'text',val:JSON.stringify(answer)})
      return {callId:result.id ?? event.val.pendingCallIds[0],outcome:throwing
        ? {tag:'error',val:{excType:'ValueError',message:answer.raise}} : {tag:'return-value',val:index}}
    })
    return dispatch({tag:'resume-futures',val:{nativeCancel:reply.native_cancel,results,values:{nodes}}})
  }
  const throwing = reply !== null && typeof reply === "object" && "raise" in reply
  return dispatch({ tag: 'resume-futures', val: {
    results: [{callId: event.val.pendingCallIds[0], outcome: throwing
      ? {tag:'error', val:{excType:'ValueError', message:reply.raise}} : {tag:'return-value', val:0}}],
    values: {nodes: throwing ? [] : [{tag:'text',val:replyText}]},
  }})
}
function boundary(event) {
  if (event.tag === 'resolve-futures') return [event, {waiting:true,raw_pending_ids:event.val.pendingCallIds}]
  if (event.tag === 'complete') {
    const node = event.val.values.nodes[event.val.value]
    if (node.tag === 'text') return [event, {complete:JSON.parse(node.val)}]
    assert.equal(node.tag,'dict')
    const view = decodeRecord(event.val.values, event.val.value, false)
    return [event,{complete:view,raw_envelope:event.val}]
  }
  assert.equal(event.tag, 'function-call', JSON.stringify(event))
  assert.equal(event.val.functionName, 'exchange')
  assert.equal(event.val.args.length, 1)
  assert.equal(event.val.kwargs.length, 0)
  const view = JSON.parse(text(event.val.values,event.val.args[0]))
  const id = event.val.callId
  const pending = dispatch({tag:'resume-call',val:{callId:id,outcome:{tag:'pending-future',val:id},values:{nodes:[]}}})
  assert.equal(pending.tag, 'resolve-futures')
  return [pending, {effect:view,raw_pending_ids:pending.val.pendingCallIds}]
}
assert.equal(dispatch({tag:'configure',val:{scriptName:'native1.py',typeCheck:false,typeCheckFormat:'full',typeCheckColor:false}}).tag,'ok')
let event
if (mode === 'start' || mode === 'run') {
  event = dispatch({tag:'feed',val:{code:await readFile(source,'utf8'),inputs:[{name:'exchange',value:0}],values:{nodes:[{tag:'function',val:{name:'exchange'}}]},skipTypeCheck:true,cwd:''}})
} else {
  event = dispatch({tag:'load',val:new Uint8Array(await readFile(source))})
  assert.equal(event.tag,'resolve-futures')
  if (mode === 'dispose') {
    assert.equal(dispatch({tag:'reset'}).tag,'ok')
    console.log('{"disposed":true}')
    process.exit(0)
  }
  event = resume(event, replyText)
}
let view
;[event,view] = boundary(event)
if (mode === 'run') {
  const raw = [view]
  const views = [view.effect]
  for (const batch of JSON.parse(await readFile(replyText,'utf8'))) {
    ;[event,view] = boundary(resume(event,JSON.stringify(batch)))
    raw.push(view)
    views.push(view.effect ?? view.complete ?? views.at(-1))
  }
  assert.equal(event.tag,'complete')
  console.log(JSON.stringify({views,raw}))
} else {
  if (event.tag === 'resolve-futures') {
    const snapshot = dispatch({tag:'dump'})
    assert.equal(snapshot.tag,'dump-result')
    await writeFile(mode === 'start' ? replyText : output,snapshot.val,{flag:'wx'})
  }
  console.log(JSON.stringify(view))
}
assert.equal(dispatch({tag:'reset'}).tag,'ok')
