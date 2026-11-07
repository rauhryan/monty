import collections
import dataclasses
import datetime
import functools
import itertools
import pathlib
import random
import re

big = 2**100
neg = -7
f = 3.5
s = 'héllo'
b = b'\x00\x01bytes'
xs = [1, 2.5, 'three', None, True]
t = (1, 'two')
d = {'a': 1, 'b': [2, 3], 3: 'int key'}
st = {1, 2, 3}
fs = frozenset({'x', 'y'})
dq = collections.deque([1, 2, 3], maxlen=5)
Point = collections.namedtuple('Point', ['x', 'y'])
p = Point(1, 2)


@dataclasses.dataclass
class Item:
    name: str
    qty: int = 0


item = Item('widget', 3)


class Counter:
    total = 0

    def __init__(self, start):
        self.value = start

    def bump(self):
        self.value += 1
        return self.value


c = Counter(10)
c.bump()


def make_adder(n):
    def add(x):
        return x + n

    return add


add5 = make_adder(5)


it = iter([1, 2])
next(it)

try:
    raise ValueError('boom')
except ValueError as exc:
    err = exc

dt = datetime.datetime(2024, 1, 2, 3, 4, 5, 678901)
dlt = datetime.timedelta(days=1, seconds=2)
pat = re.compile(r'(\d+)-(\w+)')
m = pat.match('12-ab')
part = functools.partial(add5, 1)
cycle = []
cycle.append(cycle)


def generator(value):
    try:
        received = yield value
        assert received is value
    finally:
        yield value


new_generator = generator(cycle)
dormant_generator = generator(cycle)
assert next(dormant_generator) is cycle


def delegating():
    yield from generator(cycle)
    return cycle


delegated_generator = delegating()
assert next(delegated_generator) is cycle
owned_completion = StopIteration(cycle)
cycle.append(owned_completion)

day = datetime.date(2024, 5, 6)
tz = datetime.timezone(datetime.timedelta(hours=2), 'X')
tm = datetime.time(1, 2, 3)
raw = b'\x07\x08' + b'\x09'
rg = range(2, 10, 3)
sl = slice(1, 5, 2)
bound = c.bump
kv, iv, vv = d.keys(), d.items(), d.values()
rnd = random.Random(42)
rnd.random()
dq_it = iter(dq)
next(dq_it)
t_it = iter(t)
next(t_it)
b_it = iter(raw)
next(b_it)
di_it = iter(d.items())
next(di_it)
s_it = iter(st)
next(s_it)
call_it = iter(lambda: 7, 0)
joined = '-'.join(['x', 'y'])
str_it = iter('abc')
next(str_it)
range_it = iter(range(3))
next(range_it)
dk_it = iter(d)
next(dk_it)
dv_it = iter(d.values())
next(dv_it)
counter = itertools.count(5)
next(counter)
alias = list[int]
opt = int | None
where = pathlib.Path('/data') / 'file.txt'


def with_default(x, y=[1]):
    return x + len(y)


def exec_host(xs, d, b):
    g = {'host_call': host_call, 'xs': xs, 'd': d, 'b': b}
    exec('out = host_call(xs, d, b)', g)
    return g['out']


pending_log = []


class Pending:
    def __init__(self, xs, d, b):
        try:
            raise KeyError('pending')
        except KeyError:
            try:
                self.result = exec_host(xs, d, b)
            finally:
                pending_log.append('finally')
