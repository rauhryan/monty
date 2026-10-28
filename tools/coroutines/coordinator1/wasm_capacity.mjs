// Linear-memory capacity only: this does NOT expose allocator live bytes/refcounts.
import { readFile } from 'node:fs/promises'
import { WASIShim } from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import { instantiate } from '../../../crates/monty-js/dist/worker/component/monty.component.js'

const memories = new Set()
const directory = new URL('../../../crates/monty-js/dist/worker/component/', import.meta.url)
const imports = new WASIShim({ sandbox: { preopens: {}, env: {}, args: [] } }).getImportObject()
imports['wasi:cli/exit'] = { exit() { throw new Error('guest exit') }, exitWithCode() { throw new Error('guest exit') } }
const component = await instantiate(
  async path => WebAssembly.compile(await readFile(new URL(path, directory))),
  imports,
  async (module, values) => {
    const instance = await WebAssembly.instantiate(module, values)
    for (const value of Object.values(instance.exports)) {
      if (value instanceof WebAssembly.Memory) memories.add(value)
    }
    return instance
  },
)
function capacities() { return [...memories].map(memory => memory.buffer.byteLength) }
function dispatch(request, expected) {
  const response = component.worker.dispatch(request)
  if (response.status !== 'continue' || response.events.length !== 1 || response.events[0].tag !== expected) {
    throw new Error(JSON.stringify(response))
  }
  return response.events[0]
}
dispatch({ tag: 'configure', val: { scriptName: 'capacity.py', typeCheck: false, typeCheckFormat: 'full', typeCheckColor: false } }, 'ok')
const baseline = capacities()
const loaded = dispatch({ tag: 'load', val: new Uint8Array(await readFile(process.argv[2])) }, 'function-call')
const afterLoad = capacities()
dispatch({ tag: 'resume-call', val: {
  callId: loaded.val.callId,
  outcome: { tag: 'return-value', val: 0 },
  values: { nodes: [{ tag: 'text', val: '[]' }] },
} }, 'function-call')
const afterResume = capacities()
dispatch({ tag: 'reset' }, 'ok')
console.log(JSON.stringify({ snapshot: process.argv[2], unit: 'bytes', metric: 'exported linear-memory capacity, not live ownership', baseline, afterLoad, afterResume, afterReset: capacities() }))
