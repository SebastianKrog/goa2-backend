"""Private connection to the hosted Rust game service."""

from __future__ import annotations

import asyncio
import http.client
import json
import os
import re
from dataclasses import dataclass
from urllib.parse import urlencode, urlsplit

from fastapi import HTTPException, Request
from pydantic import BaseModel

from goa2.server.models import CreateGameResponse, ShareLinkResponse

_MAX_REPLY_BYTES = 64 * 1024
_MAX_REPLAY_BYTES = 16 * 1024 * 1024
_RUST_GAME = re.compile(r"rust-[a-f0-9]{32}")


def is_rust_game(game_id: str) -> bool:
    return _RUST_GAME.fullmatch(game_id) is not None


def hosted_games(request: Request) -> RustHostedGames:
    service = request.app.state.rust_hosted
    if service is None:
        raise HTTPException(503, "Rust game service is unavailable")
    return service


class ReportPosition(BaseModel):
    reporter_hero: str | None
    decision_index: int
    round: int
    turn: int


class RejectedSetup(ValueError):
    """The native service explicitly rejected the supplied setup."""


@dataclass(frozen=True)
class RustHostedGames:
    origin: str
    token: str

    @classmethod
    def from_environment(cls) -> RustHostedGames | None:
        origin = os.environ.get("GOA2_RUST_DRAFT_ORIGIN", "").strip()
        token = os.environ.get("GOA2_GAME_PROVISION_TOKEN", "").strip()
        if not origin and not token:
            return None
        if not origin or not token:
            raise ValueError("Set both GOA2_RUST_DRAFT_ORIGIN and GOA2_GAME_PROVISION_TOKEN")
        url = urlsplit(origin)
        if (
            url.scheme not in {"http", "https"}
            or not url.hostname
            or url.username is not None
            or url.path not in {"", "/"}
            or url.query
            or url.fragment
        ):
            raise ValueError("GOA2_RUST_DRAFT_ORIGIN must be a private HTTP(S) origin")
        return cls(origin, token)

    async def create(
        self, game_id: str, setup: dict, hero_ids: set[str], *, direct: bool = False,
    ) -> CreateGameResponse:
        for _ in range(2 if direct else 1):
            try:
                result = await asyncio.to_thread(self._provision, game_id, setup)
                issued = [seat.token for seat in result.player_tokens] + [result.spectator_token]
                if (
                    result.game_id != game_id
                    or len(result.player_tokens) != len(hero_ids)
                    or {seat.hero_id for seat in result.player_tokens} != hero_ids
                    or len(set(issued)) != len(issued)
                    or not all(issued)
                ):
                    raise ValueError("Provisioning reply did not match the game roster")
                return result
            except RejectedSetup as error:
                if direct:
                    raise HTTPException(422, str(error)) from None
                break
            except (OSError, http.client.HTTPException, ValueError):
                # A connection failure can follow a committed creation. Retry
                # the same ID and setup, or let the draft retry its final claim.
                pass
        detail = "Could not open the Rust game."
        if not direct:
            detail += " Retry the final claim."
        raise HTTPException(503, detail)

    def _provision(self, game_id: str, setup: dict) -> CreateGameResponse:
        status, data = self._request("PUT", f"/__hosted/games/{game_id}", setup=setup)
        if status == 422:
            error = json.loads(data)
            detail = error.get("detail") if isinstance(error, dict) else None
            raise RejectedSetup(detail if isinstance(detail, str) else "Invalid game setup")
        if status != 201:
            raise ValueError("Rust service rejected game provisioning")
        return CreateGameResponse.model_validate_json(data)

    def report_position(self, game_id: str, authorization: str) -> ReportPosition:
        try:
            status, data = self._request(
                "POST", f"/__hosted/games/{game_id}/report-position", authorization=authorization,
            )
            if status == 401:
                raise HTTPException(401, "Invalid game token")
            if status == 404:
                raise HTTPException(404, "Game not found")
            if status != 200:
                raise ValueError("Rust report position unavailable")
            return ReportPosition.model_validate_json(data)
        except (OSError, http.client.HTTPException, ValueError):
            raise HTTPException(503, "Could not record the Rust game position. Retry the report.") from None

    def replay(self, game_id: str, seek: dict[str, int] | None = None) -> dict:
        endpoint = f"/__hosted/games/{game_id}/replay"
        if seek is not None:
            endpoint += "/state?" + urlencode(seek)
        try:
            status, data = self._request("GET", endpoint, limit=_MAX_REPLAY_BYTES, timeout=75)
            if status == 404:
                raise HTTPException(404, "Replay not found")
            if status == 422:
                raise HTTPException(422, "The Rust replay could not be reconstructed")
            if status != 200:
                raise ValueError("Rust replay unavailable")
            result = json.loads(data)
            if not isinstance(result, dict):
                raise ValueError("Invalid Rust replay reply")
            return result
        except (OSError, http.client.HTTPException, ValueError):
            raise HTTPException(503, "Rust replay service is unavailable") from None

    def replay_catalog(self) -> list[dict]:
        return self._catalog("/__hosted/replays")

    def share_catalog(self) -> list[dict]:
        return self._catalog("/__hosted/shares")

    def _catalog(self, endpoint: str) -> list[dict]:
        try:
            data = self._staff_request("GET", endpoint, 200)
            result = json.loads(data)
            if not isinstance(result, list) or any(not isinstance(row, dict) for row in result):
                raise ValueError("Invalid Rust catalogue reply")
            return result
        except ValueError:
            raise HTTPException(503, "Rust replay service is unavailable") from None

    async def create_share(self, game_id: str) -> ShareLinkResponse:
        try:
            data = await asyncio.to_thread(
                self._staff_request, "POST", f"/__hosted/games/{game_id}/share", 201, timeout=120,
            )
            return ShareLinkResponse.model_validate_json(data)
        except ValueError:
            raise HTTPException(503, "Rust replay service is unavailable") from None

    def revoke_share(self, token: str) -> None:
        self._staff_request("DELETE", f"/__hosted/shares/{token}", 204)

    def _staff_request(
        self, method: str, endpoint: str, expected: int, *, timeout: int = 8,
    ) -> bytes:
        try:
            status, data = self._request(method, endpoint, limit=_MAX_REPLAY_BYTES, timeout=timeout)
            if status == expected:
                return data
            if status == 404:
                raise HTTPException(404, "Replay or share not found")
            if status == 409:
                raise HTTPException(409, "Only finished games can be shared")
            if status == 422:
                raise HTTPException(422, "The Rust replay could not be shared")
        except (OSError, http.client.HTTPException, ValueError):
            pass
        raise HTTPException(503, "Rust replay service is unavailable")

    def _request(
        self, method: str, endpoint: str, *, setup: dict | None = None,
        authorization: str | None = None, limit: int = _MAX_REPLY_BYTES, timeout: int = 8,
    ) -> tuple[int, bytes]:
        url = urlsplit(self.origin)
        connection_type = (
            http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
        )
        connection = connection_type(url.hostname, url.port, timeout=timeout)
        headers = {"X-Game-Provision-Token": self.token}
        if authorization:
            headers["Authorization"] = authorization
        if setup is not None:
            headers["Content-Type"] = "application/json"
        try:
            connection.request(method, endpoint, body=json.dumps(setup) if setup is not None else None, headers=headers)
            response = connection.getresponse()
            data = response.read(limit + 1)
            if len(data) > limit:
                raise ValueError("Oversized Rust service reply")
            return response.status, data
        finally:
            connection.close()
