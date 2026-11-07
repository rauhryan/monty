# `contextlib`

Only synchronous `contextmanager` is available.
`asynccontextmanager`, `ExitStack`, `AsyncExitStack`, `closing`, `aclosing`, `suppress`, `nullcontext`, redirect helpers and public context-manager base classes are absent.

Decorated functions and manager-as-decorator wrappers do not copy function metadata or expose `__wrapped__`.
Manager instances do not inherit from `ContextDecorator` and their private attributes differ from CPython: entering clears argument references to `None` rather than deleting attributes.
Reusing a manager instance after entry is unsupported; call the decorated factory again.
Traceback objects and public exception chaining attributes remain unavailable; see [with.md](with.md) and [exceptions.md](exceptions.md).
