from collections import deque, namedtuple

for source, expected in [
    ([1, 2, 3], [3, 2, 1]),
    ((1, 2), [2, 1]),
    ('a🐍é', ['é', '🐍', 'a']),
    (b'ab', [98, 97]),
    (range(2, 10, 3), [8, 5, 2]),
    (range(9, 1, -3), [3, 6, 9]),
    (deque([1, 2, 3]), [3, 2, 1]),
    (namedtuple('Point', 'x y')(1, 2), [2, 1]),
]:
    r = reversed(source)
    assert iter(r) is r
    assert list(r) == expected
    assert next(r, None) is None

# A reverse list iterator starts at the original end but reads live elements.
items = [1, 2, 3]
r = reversed(items)
items.append(4)
items[2] = 30
assert next(r) == 30
items.pop()
items.pop()
assert list(r) == [2, 1]
items = [1, 2, 3]
r = reversed(items)
items.clear()
assert next(r, None) is None
items.extend([4, 5, 6])
assert next(r, None) is None

# Large ranges do not materialize their contents.
r = reversed(range(10**12))
assert next(r) == 10**12 - 1
assert next(r) == 10**12 - 2

for kind in ['keys', 'values', 'items', 'dict']:
    d = {'a': 1, 'b': 2, 'c': 3}
    d.pop('b')
    source = {'dict': d, 'keys': d.keys(), 'values': d.values(), 'items': d.items()}[kind]
    r = reversed(source)
    d['c'] = 30
    expected = {'keys': ['c', 'a'], 'dict': ['c', 'a'], 'values': [30, 1], 'items': [('c', 30), ('a', 1)]}
    assert list(r) == expected[kind]
    d['z'] = 4
    assert next(r, None) is None

    r = reversed(source)
    d['new'] = 5
    for _ in range(2):
        try:
            next(r)
        except RuntimeError as error:
            assert str(error) == 'dictionary changed size during iteration'
        else:
            assert False

q = deque([1, 2, 3])
r = reversed(q)
q[2] = 30
assert next(r) == 30
q.append(4)
try:
    next(r)
except RuntimeError as error:
    assert str(error) == 'deque mutated during iteration'
else:
    assert False

assert next(r, None) is None

for unsupported in [iter([1]), {1, 2}, 42]:
    try:
        reversed(unsupported)
    except TypeError:
        pass
    else:
        assert False

# Exact range bounds and a minimum-i64 step must not overflow reverse arithmetic.
r = reversed(range(-(2**63), 2**63 - 1))
assert next(r) == 2**63 - 2
assert next(r) == 2**63 - 3
assert list(reversed(range(0, -(2**63), -(2**63)))) == [0]
assert list(reversed(range(2**63 - 1, -(2**63), -(2**63)))) == [-1, 2**63 - 1]
assert list(reversed(range(3, 3))) == []
