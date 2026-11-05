import asyncio
import json
import anyio
from typing import TYPE_CHECKING
if TYPE_CHECKING:
    async def exchange(label: str) -> str:
        raise NotImplementedError

async def main():
    try:
        async with anyio.create_task_group() as group:
            assert group.cancel_scope.__exit__(None, None, None) is False
    except RuntimeError as error:
        assert len(error.args[0]) == 1
        assert isinstance(error.args[0][0], RuntimeError)
    else:
        assert False
    await exchange('after')
    return json.dumps('ok')
asyncio.run(main())
