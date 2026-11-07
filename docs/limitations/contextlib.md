# `contextlib`

`ExitStack`, `AsyncExitStack`, `aclosing`, `suppress`, `nullcontext`, redirect helpers and public context-manager base classes are absent.

Decorated functions and manager-as-decorator wrappers do not copy function metadata or expose `__wrapped__`.
Manager instances do not inherit from `ContextDecorator` or `AsyncContextDecorator` and their private attributes differ from CPython: entering clears argument references to `None` rather than deleting attributes.
Reusing a manager instance after entry is unsupported; call the decorated factory again.
Traceback objects remain unavailable and exception chaining attributes are read-only; see [with.md](with.md) and [exceptions.md](exceptions.md).

`closing` is a callable factory rather than a public subclassable class; type identity and constructor error wording differ.
As with other synchronous context managers, `close` can suspend on a host call.

`asynccontextmanager` supports async generator bodies through the generator continuation used by `contextmanager`.
The underlying object exposes synchronous generator methods rather than `__anext__`, `asend`, `athrow` and `aclose`.
Use it through the decorated manager; general async iteration and event-loop finalizer hooks are unavailable.
`StopAsyncIteration` is unavailable, and returning a value from an async generator is accepted rather than rejected at compile time.
