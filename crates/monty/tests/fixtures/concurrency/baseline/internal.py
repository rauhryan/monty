# ruff: noqa: F821
import annotationlib
import asyncio
import json


async def main():
    pending = []
    attempts = []

    class Target:
        pass

    def annotate(format):
        attempts.append(format)
        if format == 4:
            raise NotImplementedError('request fake globals')
        assert format == 2
        pending.append(exchange('internal'))
        return {'value': 'int'}

    Target.__annotate__ = annotate
    assert annotationlib.get_annotations(Target, format=annotationlib.Format.STRING) == {'value': 'int'}
    assert attempts == [4, 2]
    assert await pending.pop() == 'ok'
    return json.dumps('ok')


asyncio.run(main())
