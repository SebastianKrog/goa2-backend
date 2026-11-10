//! Run with `cargo run --locked -p goa2-core --example local_game`.
use goa2_core::api::{ExecutionBudget, GameContent, GameMode, GameSetup, LiveGame, PlayerCommand};
use std::{error::Error, fs::File, io::BufReader, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/smoke-game.json")
        });
    let content = GameContent::read(BufReader::new(File::open(path)?))?;
    let setup = GameSetup {
        map: "forgotten_island".into(),
        red_heroes: vec!["Arien".into()],
        blue_heroes: vec!["Brogan".into()],
        seed: 29,
        mode: GameMode::Quick,
        cheats_enabled: false,
        tie_breaker: None,
    };
    let budget = ExecutionBudget::default();
    let (mut game, _) = LiveGame::create(&content, &setup, &budget)?;
    let view = game.client_view(Some("hero_arien"))?;
    let card_id = view["view"]["teams"]["RED"]["heroes"][0]["hand"][0]["id"]
        .as_str()
        .ok_or("Opening hand unavailable")?
        .to_owned();
    game.submit(
        "hero_arien",
        game.position().revision,
        PlayerCommand::Commit { card_id },
        &budget,
    )?;
    // Scope every response. Private checkpoints never belong in a player response.
    println!("{}", game.client_view(Some("hero_arien"))?);
    Ok(())
}
