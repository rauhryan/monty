// Compile the unchanged historical TS client source, then exercise its real
// WorkerTransport codec and configure/feed methods against the current component.
import assert from 'node:assert/strict'
import {readFile,writeFile,mkdir,readdir,symlink} from 'node:fs/promises'
import {dirname} from 'node:path'
import {createHash} from 'node:crypto'
import ts from '../../../crates/monty-js/node_modules/typescript/lib/typescript.js'
const root=new URL('../../../',import.meta.url)
const out=new URL('playground/coroutine-implementation/catchable-cancellation-compatibility11/',root)
const old=new URL('historical-source/crates/monty-js/ts/',out)
const dest=new URL('historical-client/',out)
const hash=b=>createHash('sha256').update(b).digest('hex')
const sources={},outputs={}
async function compile(dir,relative=''){
 for(const item of await readdir(dir,{withFileTypes:true})){
  const name=relative+item.name
  if(item.isDirectory())await compile(new URL(item.name+'/',dir),name+'/')
  else if(name.endsWith('.ts')&&!name.endsWith('.d.ts')){
   const source=await readFile(new URL(item.name,dir),'utf8');sources[name]=hash(source)
   const result=ts.transpileModule(source,{fileName:name,compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.ES2022}}).outputText
   const path=new URL(name.replace(/\.ts$/,'.js'),dest);await mkdir(dirname(path.pathname),{recursive:true});await writeFile(path,result,{flag:'wx'});outputs[name]=hash(result)
  }
 }
}
await compile(old)
await writeFile(new URL('package.json',dest),'{"type":"module"}\n',{flag:'wx'})
await symlink(new URL('crates/monty-js/node_modules',root).pathname,new URL('node_modules',dest))
await symlink(new URL('crates/monty-js/dist/worker/component',root).pathname,new URL('worker/component',dest))
await writeFile(new URL('client-frozen.json',out),JSON.stringify({sources,outputs,typescript:ts.version,expected:{requests:['configure','feed','reset'],value:42},componentGlue:hash(await readFile(new URL('crates/monty-js/dist/worker/component/monty.component.js',root)))},null,2),{flag:'wx'})
const {instantiateWorker}=await import(new URL('worker/host.js',dest))
const {WorkerTransport}=await import(new URL('worker/transport.js',dest))
const dir=new URL('crates/monty-js/dist/worker/component/',root)
const modules={}
for(const name of await readdir(dir))if(name.endsWith('.wasm'))modules[name]=await WebAssembly.compile(await readFile(new URL(name,dir)))
const actual=await instantiateWorker(modules)
const raw=[]
const dispatch=async request=>{const result=actual(request);raw.push({request,result});return result}
const transport=await WorkerTransport.create(dispatch,{})
const event=await transport.feed('6 * 7',null,[],{skipTypeCheck:true},()=>{})
await dispatch({tag:'reset'})
await writeFile(new URL('client-receipt.json',out),JSON.stringify({pid:process.pid,event,raw},(_,v)=>typeof v==='bigint'?{bigint:String(v)}:v,2),{flag:'wx'})
assert.deepEqual(raw.map(r=>r.request.tag),['configure','feed','reset'])
assert.equal(raw[1].result.events[0].tag,'complete')
assert.equal(raw[1].result.events[0].val.values.nodes[0].val,42n)
console.log(JSON.stringify(event,(_,v)=>typeof v==='bigint'?String(v):v))
