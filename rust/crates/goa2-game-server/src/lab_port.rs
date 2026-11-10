//! One-time offline conversion of trusted Lab fixtures into native checkpoints.
use crate::{
    ApiError, Result,
    game::{Server, portable_checkpoint},
};
use goa2_core::{
    api::{ExecutionBudget, GameCheckpoint, LiveGame, PlayerCommand},
    codec::StateEnvelope,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    envelope: StateEnvelope,
    hero_names: BTreeMap<String, String>,
    rollback: Option<StateEnvelope>,
}

fn port(fixture: Fixture) -> Result<Value> {
    let budget = ExecutionBudget::default();
    let (game, _) = LiveGame::import_with_rollback(fixture.envelope, fixture.rollback, &budget)?;
    let checkpoint = portable_checkpoint(&game, &fixture.hero_names)?;
    let native: GameCheckpoint = serde_json::from_str(
        checkpoint["native_checkpoint_json"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Missing native checkpoint"))?,
    )
    .map_err(ApiError::internal)?;
    let restored = Server::restore_engine(native)?;
    if portable_checkpoint(&restored, &fixture.hero_names)? != checkpoint {
        return Err(ApiError::invalid(
            "The native checkpoint changed after restoring",
        ));
    }
    let mut projections = BTreeMap::new();
    for viewer in std::iter::once(None).chain(game.players().map(|(id, _)| Some(id))) {
        let mut view = game.client_view(viewer)?;
        if restored.client_view(viewer)? != view {
            return Err(ApiError::invalid("A client view changed after restoring"));
        }
        view["hero_names"] = json!(fixture.hero_names);
        projections.insert(viewer.unwrap_or("spectator").to_owned(), view);
    }
    let rollback_projections = match game.decision_players()?.into_iter().find(|id| {
        game.request_for(id)
            .is_some_and(|value| value["can_rollback"] == true)
    }) {
        Some(actor) => {
            let (rewound, _) = restored.prepare_submission(
                actor,
                restored.position().revision,
                PlayerCommand::Rollback,
                &budget,
            )?;
            let mut rows = BTreeMap::new();
            for viewer in std::iter::once(None).chain(rewound.players().map(|(id, _)| Some(id))) {
                let mut view = rewound.client_view(viewer)?;
                view["hero_names"] = json!(fixture.hero_names);
                rows.insert(viewer.unwrap_or("spectator").to_owned(), view);
            }
            Some(rows)
        }
        None => None,
    };
    let mut upgrade_previews = Vec::new();
    for actor in restored.decision_players()? {
        let Some(offer) = restored
            .request_for(actor)
            .filter(|row| row["request_type"] == "UPGRADE_PHASE")
        else {
            continue;
        };
        let request_id = offer["id"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Missing upgrade request ID"))?;
        let mut cards = Vec::new();
        loop {
            let preview = restored.preview_upgrades(actor, request_id, &cards)?;
            let next = preview["options"][0]["pair"][0].as_str().map(str::to_owned);
            upgrade_previews.push(json!({"actor": actor, "preview": preview}));
            let Some(card) = next else { break };
            cards.push(card);
        }
    }
    if portable_checkpoint(&restored, &fixture.hero_names)? != checkpoint {
        return Err(ApiError::invalid(
            "Read-only verification changed the native checkpoint",
        ));
    }
    Ok(json!({"checkpoint": checkpoint, "projections": projections,
              "rollback_projections": rollback_projections, "upgrade_previews": upgrade_previews}))
}

pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        let reply = match serde_json::from_str::<Fixture>(&line) {
            Ok(fixture) => match port(fixture) {
                Ok(result) => json!({"result": result}),
                Err(error) => json!({"error": error.to_string()}),
            },
            Err(error) => json!({"error": format!("Invalid fixture: {error}")}),
        };
        serde_json::to_writer(&mut output, &reply)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}
