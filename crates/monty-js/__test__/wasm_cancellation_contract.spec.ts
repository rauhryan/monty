import { expect, test } from 'vitest'
import { admitCancellationContract } from '../dist/worker/host.js'
import { WorkerChannel, type WorkerMessage } from '../dist/worker/channel.js'

test.each([
  undefined,
  null,
  7,
  () => {
    throw new Error('broken')
  },
  () => false,
  () => '7',
  () => 0,
  () => 6,
  () => 8,
  () => 7.5,
  () => NaN,
])('rejects malformed capability %s', (cancellationContract) => {
  expect(() => admitCancellationContract({ cancellationContract })).toThrow()
  try {
    admitCancellationContract({ cancellationContract })
  } catch (error) {
    expect(error).toHaveProperty('code', 'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED')
  }
})
test('admission belongs to each actual instance', () => {
  let calls = 0
  admitCancellationContract({
    cancellationContract: () => {
      calls++
      return 7
    },
  })
  expect(calls).toBe(1)
  expect(() => admitCancellationContract({})).toThrow()
})
test('WorkerChannel preserves startup admission code and sends no request', async () => {
  let posts = 0
  let terminated = 0
  const channel = WorkerChannel.create({
    post() {
      posts++
    },
    onMessage(handler: (reply: WorkerMessage) => void) {
      queueMicrotask(() =>
        handler({ startupError: 'unsupported', startupCode: 'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED' }),
      )
    },
    onError() {},
    terminate() {
      terminated++
    },
  })
  await expect(channel).rejects.toHaveProperty('code', 'MONTY_CANCELLATION_CONTRACT_UNSUPPORTED')
  expect(posts).toBe(0)
  expect(terminated).toBe(1)
})
