// Generate separate historical component artifacts; never overwrite production glue.
import { readFile, mkdir, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { createHash } from 'node:crypto'
import { componentNew, preview1AdapterReactorPath, transpile } from '../../../crates/monty-js/node_modules/@bytecodealliance/jco/src/api.js'
const root = new URL('../../../', import.meta.url)
const out = new URL('playground/coroutine-implementation/catchable-cancellation-compatibility11/historical-component/', root)
const core = await readFile(new URL('playground/coroutine-baseline/target/wasm32-wasip1/release/monty_wasm_runtime.wasm',root))
const adapter = await readFile(preview1AdapterReactorPath())
const hash = b => createHash('sha256').update(b).digest('hex')
const component = await componentNew(core, [['wasi_snapshot_preview1', adapter]])
const { files } = await transpile(component, {name:'monty.component',instantiation:'async'})
const hashes = {}
for (const [name, bytes] of Object.entries(files)) {
  const path = new URL(name,out)
  await mkdir(dirname(path.pathname),{recursive:true})
  await writeFile(path,bytes,{flag:'wx'})
  hashes[name] = hash(bytes)
}
await writeFile(new URL('../historical-component.json',out),JSON.stringify({core:hash(core),adapter:hash(adapter),files:hashes},null,2),{flag:'wx'})
console.log(JSON.stringify(hashes))
