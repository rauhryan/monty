// Actual component execution. No substituted dispatch results or patched core bytes.
import assert from 'node:assert/strict'
import {readFile, writeFile} from 'node:fs/promises'
import {WASIShim} from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import {instantiate} from '../../../crates/monty-js/dist/worker/component/monty.component.js'

const [action, mode, input, output, ordinalText = '0'] = process.argv.slice(2)
const ordinal = Number(ordinalText)
const directory = new URL('../../../crates/monty-js/dist/worker/component/', import.meta.url)
const fault = ['uncaught','later'].includes(mode) ? '' : mode
const imports = new WASIShim({sandbox:{preopens:{},env:{MONTY_REPAIR10_FAULT:fault},args:[]}}).getImportObject()
const component = await instantiate(async path => WebAssembly.compile(await readFile(new URL(path,directory))),imports)
assert.equal(component.worker.cancellationContract(),7)
const raw = []
function dispatch(request) {
  const result = component.worker.dispatch(request)
  raw.push({request,result})
  assert.equal(result.events.length,1)
  return result.events[0]
}
function boundary(event) {
  if (event.tag !== 'function-call') return event
  assert.equal(event.val.functionName,'exchange')
  const phase = event.val.values.nodes[event.val.args[0]]
  assert.equal(phase.tag,'text')
  const pending = dispatch({tag:'resume-call',val:{callId:event.val.callId,outcome:{tag:'pending-future',val:event.val.callId},values:{nodes:[]}}})
  assert.equal(pending.tag,'resolve-futures')
  return {...pending, phase:phase.val}
}
function resume(event, cut) {
  return boundary(dispatch({tag:'resume-futures',val:{nativeCancel:cut === 0 ? 'FIRST' : 'REPEAT',results:[{callId:event.val.pendingCallIds[0],outcome:{tag:'return-value',val:0}}],values:{nodes:[{tag:'none'}]}}}))
}
assert.equal(dispatch({tag:'configure',val:{scriptName:'repair10.py',typeCheck:false,typeCheckFormat:'full',typeCheckColor:false}}).tag,'ok')
let event
if (action === 'start' || action === 'run') {
  const code = await readFile(input,'utf8') + `\nrepair10('${mode}')._native1()\n`
  event = boundary(dispatch({tag:'feed',val:{code,inputs:[{name:'exchange',value:0}],values:{nodes:[{tag:'function',val:{name:'exchange'}}]},skipTypeCheck:true,cwd:''}}))
} else {
  event = dispatch({tag:'load',val:new Uint8Array(await readFile(input))})
  assert.equal(event.tag,'resolve-futures')
}
const phases = []
if (action === 'run') {
  for (let cut=0; event.tag === 'resolve-futures'; cut++) {
    assert(cut < 2)
    phases.push(event.phase)
    event = resume(event,cut)
  }
} else if (action === 'resume') {
  event = resume(event,ordinal)
}
if (action !== 'dispose' && event.tag === 'resolve-futures') {
  const dumped = dispatch({tag:'dump'})
  assert.equal(dumped.tag,'dump-result')
  await writeFile(output,dumped.val,{flag:'wx'})
}
// Reset is real physical disposal; it must not emit a guest effect.
assert.equal(dispatch({tag:'reset'}).tag,'ok')
console.log(JSON.stringify({event,phases,disposed:true,raw}, (_, value) => typeof value === 'bigint' ? {bigint:value.toString()} : value))
