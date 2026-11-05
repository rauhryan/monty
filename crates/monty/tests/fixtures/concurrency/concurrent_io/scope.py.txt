from __monty_host__ import host_coroutines
import asyncio


class Scope:
    def __init__(self):
        self.errors = []
        self.cleanups = []

    async def __aenter__(self):
        asyncio._scope()
        return self

    def spawn(self, continuation):
        return asyncio._spawn(child(continuation))

    def cancel(self, handle):
        asyncio._cancel(handle)

    def defer(self, cleanup):
        self.cleanups.append(cleanup)

    async def __aexit__(self, kind, value, traceback):
        self.errors = await asyncio._drain()
        while self.cleanups:
            cleanup = self.cleanups.pop()
            try:
                await cleanup()
            except Exception as error:
                self.errors.append(error)
