"""Unreachable yield still makes a generator; nested yield does not taint its parent."""
import json


def dead():
    return
    yield 1


def outer():
    def inner():
        yield 1
    return inner


g = dead()
assert next(g, 9) == 9
factory = outer()
assert next(factory()) == 1
json.dumps({'dead': 'exhausted', 'nested': 1})
