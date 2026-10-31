// Hold genuine historical modules in memory, never copy binaries or alter headers.
import { readFile, writeFile } from 'node:fs/promises'
import { createHash } from 'node:crypto'
import { createInterface } from 'node:readline'
import { WASIShim } from '../../../crates/monty-js/node_modules/@bytecodealliance/preview2-shim/dist/common/instantiation.js'
const directory = new URL('../../../crates/monty-js/dist/worker/component/', import.meta.url)
const old = await import(new URL('monty.component.js', directory))
const modules = new Map()
const hashes = {}
const hash = bytes => createHash('sha256').update(bytes).digest('hex')
hashes.glue = hash(await readFile(new URL('monty.component.js', directory)))
const imports = () => new WASIShim({sandbox:{preopens:{},env:{},args:[]}}).getImportObject()
const instance = await old.instantiate(async path => {
  const bytes = await readFile(new URL(path,directory)); hashes[path] = hash(bytes)
  const module = await WebAssembly.compile(bytes); modules.set(path,module); return module
}, imports())
let dispatches = 0
const worker = { ...instance.worker, dispatch(request) { dispatches++; return instance.worker.dispatch(request) } }
let admission
try {
  if (typeof worker.cancellationContract !== 'function') throw new Error('missing export')
  throw new Error('unexpected current component')
} catch (error) { admission = {code:'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED',message:error.message,dispatches} }
const output = process.argv[2]
await writeFile(output, JSON.stringify({hashes,admission},null,2), {flag:'wx'})
console.log('Historical instance recorded; retained old modules await regenerated glue on stdin')
for await (const line of createInterface({input:process.stdin})) {
  if (line !== 'generated') continue
  let link
  try {
    const fresh = await import(new URL('monty.component.js?policy7',directory))
    await fresh.instantiate(path => modules.get(path),imports())
    link = {status:'UNEXPECTED_SUCCESS'}
  } catch (error) { link = {status:'LINK_REJECTED',message:String(error)} }
  await writeFile(output+'.link.json',JSON.stringify({historical:hashes,link},null,2),{flag:'wx'})
  console.log(JSON.stringify(link)); break
}
