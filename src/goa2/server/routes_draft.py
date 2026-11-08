"""REST endpoints under /drafts (pre-game draft lobby)."""

from __future__ import annotations

import asyncio
import os
import random
import time
import uuid
from dataclasses import dataclass
from typing import Annotated

from fastapi import APIRouter, Depends, HTTPException, Request

from goa2.data.heroes.registry import HeroRegistry, get_hero_difficulty_stars
from goa2.domain.models import TeamColor
from goa2.draft import service
from goa2.draft.errors import InvalidTeamError, NotHostError
from goa2.draft.models import DraftStatus
from goa2.draft.modes import DRAFT_MODES
from goa2.engine.session import GameSession
from goa2.engine.setup import GameSetup
from goa2.server.draft_registry import DraftRegistry, ManagedDraft
from goa2.server.draft_ws import broadcast_draft, draft_view_payload
from goa2.server.map_paths import MapFileNotFoundError, maps_dir, resolve_map_path
from goa2.server.models import (
    ClaimHeroRequest,
    CreateDraftRequest,
    CreateDraftResponse,
    DraftActionRequest,
    DraftModeInfo,
    DraftViewResponse,
    JoinDraftRequest,
    JoinDraftResponse,
    SetCaptainRequest,
    SetTeamRequest,
    UpdateDraftSettingsRequest,
)
from goa2.server.player_names import resolve_player_names
from goa2.server.rate_limit import check_creation_budget, spend_creation_budget
from goa2.server.rust_hosted import hosted_games

router = APIRouter(prefix="/drafts", tags=["drafts"])

# Exact total player counts the engine supports as match brackets.
VALID_TOTALS = frozenset({2, 4, 5, 6})


def get_draft_registry(request: Request) -> DraftRegistry:
    return request.app.state.draft_registry


DraftRegistryDep = Annotated[DraftRegistry, Depends(get_draft_registry)]


@dataclass
class DraftContext:
    draft_id: str
    player_id: str
    is_spectator: bool
    is_host: bool


def get_draft_player(request: Request, registry: DraftRegistryDep) -> DraftContext:
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer "):
        raise HTTPException(status_code=401, detail="Missing or invalid Authorization header")
    token = auth[len("Bearer ") :]
    resolved = registry.resolve_token(token)
    if resolved is None:
        raise HTTPException(status_code=401, detail="Invalid token")
    draft_id, player_id, is_spectator, is_host = resolved
    path_draft_id = request.path_params.get("draft_id")
    if path_draft_id and path_draft_id != draft_id:
        raise HTTPException(status_code=403, detail="Token does not match this draft")
    return DraftContext(draft_id, player_id, is_spectator, is_host)


DraftPlayerDep = Annotated[DraftContext, Depends(get_draft_player)]


def _maps_dir() -> str:
    return str(maps_dir())


def _map_path(map_name: str) -> str:
    try:
        return resolve_map_path(map_name)
    except MapFileNotFoundError:
        raise HTTPException(status_code=404, detail=f"Map '{map_name}' not found") from None


def _list_maps() -> list[str]:
    base = _maps_dir()
    if not os.path.isdir(base):
        return []
    return sorted(f[:-5] for f in os.listdir(base) if f.endswith(".json"))


def _draft_view(md: ManagedDraft, player_id: str, is_spectator: bool) -> DraftViewResponse:
    return DraftViewResponse(**draft_view_payload(md, player_id, is_spectator))


def _reject_spectator(player: DraftContext) -> None:
    if player.is_spectator:
        raise HTTPException(status_code=403, detail="Spectators cannot modify the draft")


async def _maybe_create_game(request: Request, md: ManagedDraft) -> None:
    if not service.is_ready_to_create_game(md.state):
        return
    state = md.state
    red_heroes, blue_heroes = service.team_hero_lists(state)
    player_names = {
        player.claimed_hero: player.display_name
        for player in state.players
        if player.claimed_hero
    }
    use_rust = request.app.state.rust_new_drafts and request.app.state.rust_hosted is not None
    if md.hosted_game_id is not None or use_rust:
        rust = hosted_games(request)
        if md.hosted_game_id is None:
            md.hosted_game_id = f"rust-{uuid.uuid4().hex}"
        game_id = md.hosted_game_id
        name_to_id = {}
        for name in red_heroes + blue_heroes:
            hero = HeroRegistry.get(name)
            if hero is None:
                raise HTTPException(503, "Draft hero is unavailable")
            name_to_id[name] = hero.id
        opened = await rust.create(
            game_id,
            {
                "map_name": state.map_name,
                "red_heroes": red_heroes,
                "blue_heroes": blue_heroes,
                "game_type": state.game_type,
                "cheats_enabled": state.cheats,
                "seed": int(game_id[-16:], 16),
                "tie_breaker_team": state.first_team.value if state.first_team else None,
                "time_control": state.time_control.model_dump(mode="json") if state.time_control else None,
                "player_names": resolve_player_names(player_names, name_to_id),
            },
            set(name_to_id.values()),
        )
        token_by_hero = {seat.hero_id: seat.token for seat in opened.player_tokens}
        _finish_game(md, game_id, name_to_id, token_by_hero, opened.spectator_token)
        return
    game_seed_id = uuid.uuid4().hex
    game_id = game_seed_id[:12]
    seed = int(game_seed_id, 16)
    game_state = GameSetup.create_game(
        _map_path(state.map_name),
        red_heroes,
        blue_heroes,
        state.cheats,
        state.game_type,
        seed=seed,
        time_control=state.time_control,
        tie_breaker_team=state.first_team,
    )
    session = GameSession(game_state)
    hero_ids = [h.id for team in game_state.teams.values() for h in team.heroes]
    name_to_id = {h.name: h.id for team in game_state.teams.values() for h in team.heroes}
    hero_names = resolve_player_names(player_names, name_to_id)
    game_registry = request.app.state.registry
    game = game_registry.create_game(session, hero_ids, game_id=game_id, hero_names=hero_names)
    if game.replay_recorder:
        game.replay_recorder.record_setup(
            map_name=state.map_name,
            red_heroes=red_heroes,
            blue_heroes=blue_heroes,
            game_type=state.game_type,
            cheats=state.cheats,
            seed=seed,
            time_control=state.time_control,
            tie_breaker_team=game_state.tie_breaker_team.value,
            player_names=hero_names,
            rules_version=game_state.rules_version,
        )
    _finish_game(md, game.game_id, name_to_id, game.hero_to_token, game.spectator_token)


def _finish_game(
    md: ManagedDraft,
    game_id: str,
    name_to_id: dict[str, str],
    tokens: dict[str, str],
    spectator_token: str,
) -> None:
    md.player_game_tokens = {
        player.id: tokens[name_to_id[player.claimed_hero]]
        for player in md.state.players
        if player.claimed_hero
    }
    md.game_spectator_token = spectator_token
    md.state.game_id = game_id
    md.state.status = DraftStatus.COMPLETE


# ---- Endpoints ----


@router.get("/modes", response_model=list[DraftModeInfo])
async def list_modes() -> list[DraftModeInfo]:
    return [DraftModeInfo(name=m.name, description=m.description) for m in DRAFT_MODES.values()]


@router.get("/maps", response_model=list[str])
async def list_maps() -> list[str]:
    """Available map names for the in-lobby map picker."""
    return _list_maps()


@router.post("", response_model=CreateDraftResponse, status_code=201)
async def create_draft(
    body: CreateDraftRequest, registry: DraftRegistryDep, request: Request
) -> CreateDraftResponse:
    ip = check_creation_budget(request)
    # Match settings can be tweaked in the lobby, but seed/validate the initial values now.
    if body.game_type not in ("QUICK", "LONG"):
        raise HTTPException(status_code=400, detail="game_type must be QUICK or LONG")
    if body.draft_mode not in DRAFT_MODES:
        raise HTTPException(status_code=400, detail=f"Unknown draft_mode '{body.draft_mode}'")
    if not (1 <= body.max_hero_stars <= 4):
        raise HTTPException(status_code=400, detail="max_hero_stars must be between 1 and 4")
    _map_path(body.map_name)
    draft_id = uuid.uuid4().hex[:12]
    state = service.create_draft(
        draft_id,
        body.map_name,
        body.game_type,
        body.draft_mode,
        body.host_name,
        now=time.time(),
        cheats=body.cheats_enabled,
        max_hero_stars=body.max_hero_stars,
        time_control=body.time_control,
    )
    md = registry.create(state)
    spend_creation_budget(request, ip)
    return CreateDraftResponse(
        draft_id=draft_id,
        player_id="p1",
        player_token=md.host_token,
        spectator_token=md.spectator_token,
    )


@router.patch("/{draft_id}/settings", response_model=DraftViewResponse)
async def update_settings(
    draft_id: str,
    body: UpdateDraftSettingsRequest,
    player: DraftPlayerDep,
    registry: DraftRegistryDep,
) -> DraftViewResponse:
    if not player.is_host:
        raise NotHostError("Only the host may change draft settings")
    if body.game_type is not None and body.game_type not in ("QUICK", "LONG"):
        raise HTTPException(status_code=400, detail="game_type must be QUICK or LONG")
    if body.draft_mode is not None and body.draft_mode not in DRAFT_MODES:
        raise HTTPException(status_code=400, detail=f"Unknown draft_mode '{body.draft_mode}'")
    if body.max_hero_stars is not None and not (1 <= body.max_hero_stars <= 4):
        raise HTTPException(status_code=400, detail="max_hero_stars must be between 1 and 4")
    if body.map_name is not None:
        _map_path(body.map_name)  # 404 if missing
    md = registry.get(draft_id)
    async with md.lock:
        time_control = (
            body.time_control if "time_control" in body.model_fields_set else service.UNSET
        )
        service.update_settings(
            md.state,
            map_name=body.map_name,
            game_type=body.game_type,
            draft_mode=body.draft_mode,
            cheats=body.cheats_enabled,
            max_hero_stars=body.max_hero_stars,
            time_control=time_control,
        )
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/join", response_model=JoinDraftResponse)
async def join_draft(
    draft_id: str, body: JoinDraftRequest, registry: DraftRegistryDep
) -> JoinDraftResponse:
    md = registry.get(draft_id)
    async with md.lock:
        player = service.join(md.state, body.display_name)
        token = registry.add_player_token(draft_id, player.id)
    await broadcast_draft(md, registry)
    return JoinDraftResponse(draft_id=draft_id, player_id=player.id, player_token=token)


@router.get("/{draft_id}", response_model=DraftViewResponse)
async def get_draft(
    draft_id: str, player: DraftPlayerDep, registry: DraftRegistryDep
) -> DraftViewResponse:
    md = registry.get(draft_id)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/team", response_model=DraftViewResponse)
async def set_team(
    draft_id: str,
    body: SetTeamRequest,
    player: DraftPlayerDep,
    registry: DraftRegistryDep,
) -> DraftViewResponse:
    _reject_spectator(player)
    md = registry.get(draft_id)
    try:
        team = TeamColor(body.team)
    except ValueError as exc:
        raise InvalidTeamError(f"Invalid team '{body.team}'") from exc
    async with md.lock:
        service.set_team(md.state, player.player_id, team)
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/leave-team", response_model=DraftViewResponse)
async def leave_team(
    draft_id: str,
    player: DraftPlayerDep,
    registry: DraftRegistryDep,
) -> DraftViewResponse:
    _reject_spectator(player)
    md = registry.get(draft_id)
    async with md.lock:
        service.leave_team(md.state, player.player_id)
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/randomize-teams", response_model=DraftViewResponse)
async def randomize_teams(
    draft_id: str, player: DraftPlayerDep, registry: DraftRegistryDep
) -> DraftViewResponse:
    if not player.is_host:
        raise NotHostError("Only the host may randomize teams")
    md = registry.get(draft_id)
    async with md.lock:
        service.randomize_teams(md.state, random.Random())
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/captain", response_model=DraftViewResponse)
async def set_captain(
    draft_id: str,
    body: SetCaptainRequest,
    player: DraftPlayerDep,
    registry: DraftRegistryDep,
) -> DraftViewResponse:
    if not player.is_host:
        raise NotHostError("Only the host may set captains")
    md = registry.get(draft_id)
    async with md.lock:
        service.set_captain(md.state, body.player_id)
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/start", response_model=DraftViewResponse)
async def start_draft(
    draft_id: str, player: DraftPlayerDep, registry: DraftRegistryDep
) -> DraftViewResponse:
    if not player.is_host:
        raise NotHostError("Only the host may start the draft")
    md = registry.get(draft_id)
    async with md.lock:
        # Team sizes emerge from membership; the total must be an exact engine bracket.
        total = sum(1 for p in md.state.players if p.team is not None)
        if total not in VALID_TOTALS:
            raise HTTPException(
                status_code=400,
                detail=f"{total} assigned players is not a valid match size "
                f"(must be one of {sorted(VALID_TOTALS)}).",
            )
        # Only rated heroes (1..4 stars) are draftable. A 0-star result means the
        # hero is unrated (e.g. legacy/test-only heroes not in the difficulty
        # table) and must never appear in a difficulty-capped pool, regardless of
        # the cap.
        eligible = [
            h
            for h in HeroRegistry.list_heroes()
            if 1 <= get_hero_difficulty_stars(h) <= md.state.max_hero_stars
        ]
        service.start_draft(md.state, eligible, random.Random())
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/action", response_model=DraftViewResponse)
async def draft_action(
    draft_id: str,
    body: DraftActionRequest,
    player: DraftPlayerDep,
    registry: DraftRegistryDep,
) -> DraftViewResponse:
    _reject_spectator(player)
    md = registry.get(draft_id)
    async with md.lock:
        service.apply_action(md.state, player.player_id, body.hero)
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)


@router.post("/{draft_id}/claim", response_model=DraftViewResponse)
async def claim_hero(
    draft_id: str,
    body: ClaimHeroRequest,
    player: DraftPlayerDep,
    registry: DraftRegistryDep,
    request: Request,
) -> DraftViewResponse:
    _reject_spectator(player)
    md = registry.get(draft_id)
    async with md.lock:
        claimant = service.get_player(md.state, player.player_id)
        previous_claim = claimant.claimed_hero
        if md.hosted_game_id and md.state.status is DraftStatus.COMPLETE and previous_claim == body.hero:
            return _draft_view(md, player.player_id, player.is_spectator)
        service.claim_hero(md.state, player.player_id, body.hero)
        try:
            await _maybe_create_game(request, md)
        except (HTTPException, asyncio.CancelledError):
            if md.hosted_game_id:
                # Leave the last selection available to retry after a refresh.
                claimant.claimed_hero = previous_claim
            raise
    await broadcast_draft(md, registry)
    return _draft_view(md, player.player_id, player.is_spectator)
