"""Verify hosted Rust routing through a staging frontend without deploying it."""

import argparse
from contextlib import contextmanager
import json
import os
import re
from pathlib import Path
from urllib.parse import urlencode, urlsplit
from uuid import uuid4

import httpx
from websockets.exceptions import WebSocketException
from websockets.sync.client import connect


RUST_ID = re.compile(r"rust-[a-f0-9]{32}")
TIMEOUT = 10


class VerificationError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise VerificationError(message)


def origin(value):
    url = urlsplit(value)
    require(
        url.hostname and not url.username and not url.password
        and url.path in ("", "/") and not url.query and not url.fragment
        and (url.scheme == "https" or
             (url.scheme == "http" and url.hostname in {"localhost", "127.0.0.1", "::1"})),
        "Use an HTTPS origin, or a loopback HTTP origin for a local test",
    )
    return value.rstrip("/")


def access_headers():
    client_id = os.getenv("GOA2_STAGING_ACCESS_CLIENT_ID")
    secret = os.getenv("GOA2_STAGING_ACCESS_CLIENT_SECRET")
    require(bool(client_id) == bool(secret), "Set both staging frontend Access service credentials")
    headers = {}
    if client_id:
        headers.update({"CF-Access-Client-Id": client_id, "CF-Access-Client-Secret": secret})
    if cookie := os.getenv("GOA2_STAGING_ACCESS_COOKIE"):
        headers["Cookie"] = "CF_Authorization=" + cookie
    return headers


def access_denied(response):
    if response.status_code in (401, 403):
        return response.headers.get("cf-mitigated") != "challenge"
    location = urlsplit(response.headers.get("location", ""))
    return response.status_code in (302, 303, 307) and location.path.startswith("/cdn-cgi/access/login")


class Staging:
    def __init__(self, http, frontend, rust, access):
        self.http = http
        self.frontend = frontend
        self.rust = rust
        self.access = access

    def request(self, method, path, status=200, *, token=None, body=None):
        headers = dict(self.access)
        if token:
            headers["Authorization"] = "Bearer " + token
        try:
            response = self.http.request(method, self.frontend + "/api" + path, headers=headers, json=body)
        except httpx.HTTPError as error:
            raise VerificationError(f"Frontend HTTP request failed ({type(error).__name__})") from None
        require(response.status_code == status,
                f"Frontend {method} returned HTTP {response.status_code}, expected {status}")
        if status >= 400:
            return None
        require("application/json" in response.headers.get("content-type", ""),
                "Frontend returned a non-JSON response")
        try:
            return response.json()
        except ValueError:
            raise VerificationError("Frontend returned invalid JSON") from None

    def check_ingress(self):
        try:
            # Do not forward frontend Access credentials or player tokens to the tunnel.
            with httpx.Client(timeout=TIMEOUT, follow_redirects=False, trust_env=False) as probe:
                denied = probe.get(self.rust + "/overrides/schema")
        except httpx.HTTPError as error:
            raise VerificationError(f"Rust origin probe failed ({type(error).__name__})") from None
        require(access_denied(denied), "The direct Rust origin did not deny an unauthenticated public route")
        for path in ("/__lab/health", "/__hosted/replays", "/%5f_hosted/replays"):
            self.request("GET", path, 404)

    def create_canary(self):
        game = self.request("POST", "/games", 201, body={
            "red_heroes": ["Arien"], "blue_heroes": ["Brogan"], "map_name": "forgotten_island",
        })
        require(RUST_ID.fullmatch(game.get("game_id", "")),
                "Create Game did not return a Rust game; check the staging creation flag")
        seats = {seat["hero_id"]: seat["token"] for seat in game["player_tokens"]}
        require(set(seats) == {"hero_arien", "hero_brogan"}, "The canary has the wrong seats")
        return game["game_id"], seats["hero_arien"], game["spectator_token"], seats["hero_brogan"]

    def create_draft_canary(self):
        host = self.request("POST", "/drafts", 201, body={
            "host_name": "Staging Red", "map_name": "forgotten_island",
        })
        require(re.fullmatch(r"[a-f0-9]{12}", host["draft_id"]), "The draft ID is invalid")
        path = "/drafts/" + host["draft_id"]
        guest = self.request("POST", path + "/join", body={"display_name": "Staging Blue"})
        seats = {seat["player_id"]: seat["player_token"] for seat in (host, guest)}
        red = host["player_token"]
        blue = guest["player_token"]
        self.request("POST", path + "/team", token=red, body={"team": "RED"})
        self.request("POST", path + "/team", token=blue, body={"team": "BLUE"})
        draft = self.request("POST", path + "/start", token=red)["draft"]
        captains = {player["team"]: seats[player["id"]] for player in draft["players"] if player["is_captain"]}
        names = iter(self.request("GET", "/heroes"))
        for step in draft["sequence"]:
            draft = self.request("POST", path + "/action", token=captains[step["team"]],
                                 body={"hero": next(names)})["draft"]
        self.request("POST", path + "/claim", token=red, body={"hero": draft["picks"]["RED"][0]})
        completed = self.request("POST", path + "/claim", token=blue, body={"hero": draft["picks"]["BLUE"][0]})
        game = completed["game_id"]
        require(completed["draft"]["status"] == "COMPLETE" and RUST_ID.fullmatch(game or ""),
                "The draft did not complete to a Rust game; check the staging draft flag")
        links = [self.request("GET", path, token=token) for token in (red, host["spectator_token"], blue)]
        require(all(link["game_id"] == game for link in links), "Draft roles received different game IDs")
        player, spectator, opponent = (link["game_token"] for link in links)
        require(all((player, spectator, opponent)) and len({player, spectator, opponent}) == 3
                and opponent == completed["game_token"], "The draft did not issue distinct, stable role links")
        return game, player, spectator, opponent

    @contextmanager
    def socket(self, path, token, label):
        url = "ws" + self.frontend[4:] + "/api" + path + "/ws?" + urlencode({"token": token})
        try:
            with connect(url, origin=self.frontend, additional_headers=self.access, proxy=None,
                         open_timeout=TIMEOUT, close_timeout=2) as socket:
                yield socket
        except (OSError, TimeoutError, WebSocketException) as error:
            raise VerificationError(f"{label} WebSocket failed ({type(error).__name__})") from None

    def verify_game(self, game, seat, spectator, opponent=None, *, exercise=False):
        require(RUST_ID.fullmatch(game), "Use a Rust game ID")
        require(seat and spectator and seat != spectator, "Set distinct player and spectator tokens")
        path = "/games/" + game
        schema = self.request("GET", path + "/overrides/schema")
        require({"move_card", "add_effect", "end_turn"} <= {op["name"] for op in schema["ops"]},
                "The Rust override catalogue is incomplete")
        self.request("GET", path, 401)
        self.request("GET", path, 401, token="invalid-staging-probe")
        owner = self.request("GET", path, token=seat)
        public = self.request("GET", path, token=spectator)
        require(owner["view"].get("clock") is None, "Use an untimed staging game")
        require(owner["revision"] == public["revision"], "The staging game changed during verification")
        own = [hero for hero in heroes(owner) if isinstance(hero["deck"], list)]
        require(len(own) == 1 and own[0]["deck"], "The player view does not expose exactly one private deck")
        actor = own[0]["id"]
        require(all(hidden_cards(hero) for hero in heroes(owner) if hero["id"] != actor),
                "The player received another hero's private cards")
        check_public(public)
        with self.socket(path, seat, "Player") as player, self.socket(path, spectator, "Spectator") as watcher:
            first = packet(player, "STATE_UPDATE")
            watched = packet(watcher, "STATE_UPDATE")
            require(same_view(first, owner) and same_view(watched, public),
                    "The WebSocket and REST views disagree")
            if exercise:
                require(first["revision"] == 0 and own[0]["hand"], "The canary was already changed")
                card = own[0]["hand"][0]
                action_id = uuid4().hex
                player.send(json.dumps({
                    "type": "COMMIT_CARD", "card_id": card["id"],
                    "expected_revision": 0, "client_action_id": action_id,
                }))
                receipt = packet(player, "ACTION_RESULT")
                first = packet(player, "STATE_UPDATE")
                watched = packet(watcher, "STATE_UPDATE")
                require(receipt["revision"] == first["revision"] == watched["revision"] == 1,
                        "The canary command did not publish exactly one revision")
                own_commit = next(hero for hero in heroes(first) if hero["id"] == actor)["current_turn_card"]
                require(receipt.get("client_action_id") == action_id and own_commit["id"] == card["id"],
                        "The player did not receive their own commitment and receipt")
                check_public(watched)
                hidden = next(hero for hero in heroes(watched) if hero["id"] == actor)["current_turn_card"]
                require(hidden["is_facedown"] and "id" not in hidden and "client_action_id" not in watched,
                        "A spectator received private commitment data")
                if opponent:
                    peer = self.request("GET", path, token=opponent)
                    peer_private = [hero for hero in heroes(peer) if isinstance(hero["deck"], list)]
                    require(peer["revision"] == 1 and len(peer_private) == 1 and peer_private[0]["id"] != actor,
                            "The opposing seat did not receive its own private view at the new revision")
                    peer_actor = next(hero for hero in heroes(peer) if hero["id"] == actor)
                    require(hidden_cards(peer_actor) and peer_actor["current_turn_card"] == hidden,
                            "The opposing seat received private commitment data")
                history = self.request("GET", path + "/overrides/history", token=spectator)
                require(history["total"] == 1 and history["decisions"][0]["type"] == "commit"
                        and history["decisions"][0]["label"].endswith("committed a card"),
                        "Public history exposed a commitment or missed the canary decision")
        with self.socket(path, seat, "Reconnecting player") as resumed:
            require(same_view(packet(resumed, "STATE_UPDATE"), first), "Reconnect did not restore the same revision")
        require(same_view(self.request("GET", path, token=seat), first),
                "REST did not return the same revision after reconnect")
        return first["revision"]

    def verify_identity(self, game, manifest):
        require(RUST_ID.fullmatch(game), "Use a Rust game ID")
        require(manifest["format_version"] == 1, "Unsupported release manifest version")
        running = self.request("GET", f"/games/{game}/health")
        require(running.pop("ok", False) and running == manifest["service"],
                "Running service identity does not match the release manifest")

    def verify_share(self, token):
        require(RUST_ID.fullmatch(token), "Use a Rust replay share token")
        path = "/shared/" + token
        metadata = self.request("GET", path)
        total = len(metadata["decisions"])
        require(total > 0, "Use a published replay containing decisions")
        for decision in (0, total):
            group = self.request("GET", path + f"/state?decision={decision}")
            require(isinstance(group["keyframe"], dict)
                    and group["start"] <= decision <= group["start"] + len(group["patches"]),
                    "Published replay did not cover the requested decision")


def packet(socket, kind):
    value = json.loads(socket.recv(timeout=TIMEOUT))
    require(value.get("type") == kind, f"Expected WebSocket {kind}")
    return value


def heroes(frame):
    return [hero for team in frame["view"]["teams"].values() for hero in team["heroes"]]


def hidden_cards(hero):
    deck = hero["deck"]
    return not hero["hand"] and isinstance(deck, dict) and set(deck) == {"count"} and isinstance(deck["count"], int)


def check_public(frame):
    require(not frame.get("input_request") and all(map(hidden_cards, heroes(frame))),
            "A spectator received a private prompt, hand or deck")


def same_view(first, second):
    return all(first.get(key) == second.get(key)
               for key in ("revision", "view", "input_request", "awaiting_input", "winner", "hero_names"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frontend-origin", required=True)
    parser.add_argument("--rust-origin", required=True, help="Direct, Access-protected Rust tunnel origin")
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--game-id", help="Read-only checks against an existing untimed Rust staging game")
    mode.add_argument("--exercise", action="store_true", help="Create one disposable game and commit one card")
    mode.add_argument("--exercise-draft", action="store_true",
                      help="Complete one disposable draft into a Rust game and commit one card")
    parser.add_argument("--share-token", help="Also seek an existing published Rust replay")
    parser.add_argument("--manifest", type=Path, help="Match the running identity to a packaged release")
    args = parser.parse_args()
    try:
        frontend, rust = origin(args.frontend_origin), origin(args.rust_origin)
        require(frontend != rust, "Use separate frontend and Rust origins")
        with httpx.Client(timeout=TIMEOUT, follow_redirects=False, trust_env=False) as http:
            stage = Staging(http, frontend, rust, access_headers())
            stage.check_ingress()
            if args.manifest:
                # This route is static. Check a new deployment before the
                # optional canary writes anything to its database.
                manifest_game = args.game_id or "rust-" + "0" * 32
                stage.verify_identity(manifest_game, json.loads(args.manifest.read_text(encoding="utf-8")))
            if args.exercise_draft:
                game, seat, spectator, opponent = stage.create_draft_canary()
            elif args.exercise:
                game, seat, spectator, opponent = stage.create_canary()
            else:
                game = args.game_id
                seat = os.getenv("GOA2_STAGING_SEAT_TOKEN")
                spectator = os.getenv("GOA2_STAGING_SPECTATOR_TOKEN")
                opponent = None
            revision = stage.verify_game(game, seat, spectator, opponent,
                                         exercise=args.exercise or args.exercise_draft)
            if args.share_token:
                stage.verify_share(args.share_token)
    except VerificationError as error:
        parser.exit(1, f"Staging verification failed: {error}\n")
    except OSError:
        parser.exit(1, "Staging verification failed: cannot read the release manifest\n")
    except (KeyError, TypeError, ValueError, AttributeError, StopIteration):
        parser.exit(1, "Staging verification failed: unexpected response format\n")
    print(f"Staging Access, scoped REST and WebSocket reconnect passed for {game} at revision {revision}")
    if args.exercise_draft:
        print("Draft roles received distinct links to the same Rust game")
    if args.manifest:
        print("The running binary and content match the packaged release")
    if args.share_token:
        print("Published replay opened and sought at the first and final decision")


if __name__ == "__main__":
    main()
