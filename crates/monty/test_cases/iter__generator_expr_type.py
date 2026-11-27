# Generator expressions are lazy, single-use generator objects.
gen_result = (x * 2 for x in range(5))
assert type(gen_result).__name__ == 'generator'
assert list(gen_result) == [0, 2, 4, 6, 8]
assert list(gen_result) == []
