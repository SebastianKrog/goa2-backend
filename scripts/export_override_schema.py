"""Export the shared consensus override catalogue for the native game server."""

import asyncio
import json

from goa2.server.routes_overrides import get_override_schema


if __name__ == "__main__":
    schema = asyncio.run(get_override_schema())
    print(json.dumps(schema.model_dump(mode="json"), indent=2, ensure_ascii=False))
