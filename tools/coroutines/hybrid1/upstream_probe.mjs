// Actual upstream Task/future scheduling, not host-coroutine mode or a new runtime.
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { WASIShim } from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
import { instantiate } from '../../../crates/monty-js/dist/worker/component/monty.component.js'

const directory = new URL('../../../crates/monty-js/dist/worker/component/', import.meta.url)
const hashes = {}
async function instance() {
  const imports = new WASIShim({ sandbox: { preopens: {}, env: {}, args: [] } }).getImportObject()
  imports['wasi:cli/exit'] = { exit() { throw Error('guest exit') }, exitWithCode() { throw Error('guest exit') } }
  const component = await instantiate(async path => {
    const bytes = await readFile(new URL(path, directory))
    hashes[path] = createHash('sha256').update(bytes).digest('hex')
    return WebAssembly.compile(bytes)
  }, imports)
  const dispatch = request => {
    const result = component.worker.dispatch(request)
    assert.equal(result.status, 'continue')
    assert.equal(result.events.length, 1)
    return result.events[0]
  }
  assert.equal(dispatch({ tag: 'configure', val: {
    scriptName: 'hybrid-upstream.py', typeCheck: false, typeCheckFormat: 'full', typeCheckColor: false,
  } }).tag, 'ok')
  return dispatch
}

const code = `
import asyncio
import json
async def read(name):
    try:
        return await exchange('read:' + name)
    finally:
        await exchange('close:' + name)
async def main():
    try:
        return await asyncio.gather(read('A'), read('B'), read('C'))
    except ValueError as error:
        return 'failed:' + str(error)
json.dumps(asyncio.run(main()))
`

async function probe(failed) {
  let dispatch = await instance()
  let event = dispatch({ tag: 'feed', val: {
    code, inputs: [{ name: 'exchange', value: 0 }],
    values: { nodes: [{ tag: 'function', val: { name: 'exchange' } }] }, skipTypeCheck: true, cwd: '',
  } })
  const pending = new Map()
  const trace = []
  let restores = 0
  async function restore() {
    const dump = dispatch({ tag: 'dump' })
    assert.equal(dump.tag, 'dump-result')
    assert.equal(dispatch({ tag: 'reset' }).tag, 'ok')
    dispatch = await instance()
    const loaded = dispatch({ tag: 'load', val: dump.val })
    assert.deepEqual(loaded, event)
    // A second independent load/reset proves disposal does not advance authored code.
    const disposal = await instance()
    assert.deepEqual(disposal({ tag: 'load', val: dump.val }), event)
    assert.equal(disposal({ tag: 'reset' }).tag, 'ok')
    restores++
  }
  async function drain() {
    while (event.tag === 'function-call') {
      const call = event.val
      const name = call.values.nodes[call.args[0]].val
      assert.equal(call.functionName, 'exchange')
      trace.push(name)
      pending.set(name, call.callId)
      await restore()
      event = dispatch({ tag: 'resume-call', val: {
        callId: call.callId, outcome: { tag: 'pending-future', val: call.callId }, values: { nodes: [] },
      } })
    }
    if (event.tag === 'resolve-futures') {
      assert.deepEqual([...event.val.pendingCallIds].sort(), [...pending.values()].sort())
      await restore()
    }
  }
  async function settle(name, value, error = false) {
    const callId = pending.get(name)
    assert.notEqual(callId, undefined)
    pending.delete(name)
    event = dispatch({ tag: 'resume-futures', val: {
      results: [{ callId, outcome: error ? { tag: 'error', val: { excType: 'ValueError', message: value } }
        : { tag: 'return-value', val: 0 } }],
      values: { nodes: error ? [] : [{ tag: 'text', val: value }] },
    } })
    await drain()
  }
  await drain()
  assert.deepEqual(trace, ['read:A', 'read:B', 'read:C'])
  await settle('read:B', failed ? 'B-failed' : 'b', failed)
  await settle('close:B', 'closed')
  if (!failed) {
    await settle('read:C', 'c')
    await settle('close:C', 'closed')
    await settle('read:A', 'a')
    await settle('close:A', 'closed')
  }
  assert.equal(event.tag, 'complete')
  const result = JSON.parse(event.val.values.nodes[event.val.value].val)
  assert.deepEqual(result, failed ? 'failed:B-failed' : ['a', 'b', 'c'])
  assert.deepEqual(trace, failed ? ['read:A', 'read:B', 'read:C', 'close:B'] :
    ['read:A', 'read:B', 'read:C', 'close:B', 'close:C', 'close:A'])
  assert.equal(dispatch({ tag: 'reset' }).tag, 'ok')
  return { failed, trace, result, restores, abandoned: [...pending.keys()] }
}
console.log(JSON.stringify({ cases: [await probe(false), await probe(true)], wasm_sha256: hashes }, null, 2))
