// One fresh Node process and WASI component per advance. No instance survives exit.
import { readFile, writeFile } from 'node:fs/promises'
import { WASIShim } from '../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import { instantiate } from '../../crates/monty-js/dist/worker/component/monty.component.js'

const [mode, source, replyText, output] = process.argv.slice(2)
const componentDir = new URL('../../crates/monty-js/dist/worker/component/', import.meta.url)
const imports = new WASIShim({ sandbox: { preopens: {}, env: {}, args: [] } }).getImportObject()
imports['wasi:cli/exit'] = { exit() { throw new Error('guest exit') }, exitWithCode() { throw new Error('guest exit') } }
const component = await instantiate(async path => WebAssembly.compile(await readFile(new URL(path, componentDir))), imports)

/** Require the guest text boundary without converting JSON number tokens. */
function textNode(values, index) {
  const node = Number.isInteger(index) && index >= 0 && values?.nodes?.[index]
  if (node?.tag !== 'text' || typeof node.val !== 'string') throw new Error('expected guest JSON text')
  return node.val
}

/** Both fresh and restored calls must have one positional text argument. */
function effectText(call) {
  if (call.functionName !== 'exchange' || !(call.args instanceof Uint32Array) || call.args.length !== 1 ||
      !Array.isArray(call.kwargs) || call.kwargs.length !== 0) throw new Error('invalid exchange arguments')
  return textNode(call.values, call.args[0])
}

/** Require precisely one terminal event; any print or unexpected event is a failure. */
function dispatch(request) {
  const result = component.worker.dispatch(request)
  if (result.status !== 'continue' || result.events.length !== 1) throw new Error(JSON.stringify(result))
  return result.events[0]
}

const configured = dispatch({ tag: 'configure', val: { scriptName: 'baseline.py', typeCheck: false, typeCheckFormat: 'full', typeCheckColor: false } })
if (configured.tag !== 'ok') throw new Error(JSON.stringify(configured))
let event
if (mode === 'start') {
  event = dispatch({ tag: 'feed', val: {
    code: await readFile(source, 'utf8'), inputs: [{ name: 'exchange', value: 0 }],
    values: { nodes: [{ tag: 'function', val: { name: 'exchange' } }] }, skipTypeCheck: true, cwd: '',
  } })
} else {
  event = dispatch({ tag: 'load', val: new Uint8Array(await readFile(source)) })
  if (event.tag !== 'function-call') throw new Error(JSON.stringify(event))
  effectText(event.val)
  if (mode === 'dispose') {
    if (dispatch({ tag: 'reset' }).tag !== 'ok') throw new Error('reset failed')
    console.log(JSON.stringify({ disposed: true }))
    process.exit(0)
  }
  const reply = JSON.parse(replyText)
  event = dispatch({ tag: 'resume-call', val: {
    callId: event.val.callId,
    outcome: 'raise' in reply ? { tag: 'error', val: { excType: 'ValueError', message: reply.raise } } : { tag: 'return-value', val: 0 },
    values: { nodes: 'raise' in reply ? [] : [{ tag: 'text', val: replyText }] },
  } })
}
if (event.tag === 'function-call') {
  const effect = effectText(event.val)
  const snapshot = dispatch({ tag: 'dump' })
  if (snapshot.tag !== 'dump-result') throw new Error(JSON.stringify(snapshot))
  await writeFile(mode === 'start' ? replyText : output, snapshot.val)
  if (dispatch({ tag: 'reset' }).tag !== 'ok') throw new Error('original disposal failed')
  // Forward guest JSON verbatim. The strict host decoder checks syntax and duplicates.
  console.log('{"effect":' + effect + '}')
} else if (event.tag === 'complete') {
  console.log('{"complete":' + textNode(event.val.values, event.val.value) + '}')
} else if (event.tag === 'error') {
  console.log(JSON.stringify({ error: { type: event.val.excType, message: event.val.message, diagnostic: event.val.traceback } }))
  process.exitCode = 1
} else {
  throw new Error(JSON.stringify(event))
}
