# ruff: noqa: F821
import json

assert big == 2**100 and item == Item('widget', 3)
assert add5(2) == 7 and part() == 6 and with_default(1) == 2
assert cycle[0] is cycle and cycle[1] is owned_completion
assert owned_completion.value is cycle
assert next(new_generator) is cycle
assert new_generator.send(cycle) is cycle
assert dormant_generator.send(cycle) is cycle
assert next(new_generator, 'done') == 'done'
assert next(dormant_generator, 'done') == 'done'
assert delegated_generator.send(cycle) is cycle
try:
    next(delegated_generator)
except StopIteration as done:
    assert done.value is cycle
else:
    assert False
assert next(it) == 2 and list(t_it) == ['two']
assert next(counter) == 6
assert bound() == 12
assert str(err) == 'boom'
assert m.groups() == ('12', 'ab')
json.dumps('ok')
