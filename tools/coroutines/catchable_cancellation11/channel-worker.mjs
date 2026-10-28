import {parentPort,workerData} from 'node:worker_threads'
import {serveDispatch} from '../../../crates/monty-js/dist/worker/serve.js'
let subscriptions=0
await serveDispatch(workerData,reply=>parentPort.postMessage({...reply,subscriptions}),handler=>{subscriptions++;parentPort.on('message',handler)})
