//! Ephemeral messages for authenticated connections at the same table.
use crate::{ApiError, Result, blocking, game::Game};
use goa2_core::{api::TableCardSlot, geometry::Hex};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Notice {
    pub body: Arc<Value>,
    exclude_hero: Option<String>,
}

impl Notice {
    pub fn visible_to(&self, viewer: Option<&str>) -> bool {
        self.exclude_hero
            .as_deref()
            .is_none_or(|actor| viewer != Some(actor))
    }
}

#[derive(Default)]
pub struct Limits {
    last_ping: Option<Instant>,
    last_pointer: Option<Instant>,
}

impl Limits {
    pub async fn send(
        &mut self,
        game: &Arc<Game>,
        actor: &str,
        kind: &str,
        body: Value,
    ) -> Result<()> {
        let notice = match kind {
            "PING" if allowed(&mut self.last_ping, 450) => {
                let game = game.clone();
                let actor = actor.to_owned();
                blocking(move || ping(&game, &actor, &body["target"])).await?
            }
            "POINTER" if allowed(&mut self.last_pointer, 60) => pointer(game, actor, &body),
            _ => None,
        };
        if let Some(notice) = notice {
            let _ = game.presence.send(notice);
        }
        Ok(())
    }
}

fn allowed(last: &mut Option<Instant>, interval_ms: u64) -> bool {
    let now = Instant::now();
    if last.is_some_and(|at| now.duration_since(at) < Duration::from_millis(interval_ms)) {
        return false;
    }
    *last = Some(now);
    true
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
enum PingTarget {
    Hex {
        hex: Coordinates,
    },
    Card {
        hero_id: String,
        #[serde(flatten)]
        slot: TableCardSlot,
    },
}

#[derive(Deserialize)]
struct Coordinates {
    q: i32,
    r: i32,
    s: i32,
}

fn ping(game: &Game, actor: &str, raw_target: &Value) -> Result<Option<Notice>> {
    let snapshot = game.snapshot()?;
    if !snapshot.accepts_table_ping() {
        return Ok(None);
    }
    let target = PingTarget::deserialize(raw_target)
        .map_err(|_| ApiError::invalid("Invalid table ping target"))?;
    let target = match target {
        PingTarget::Hex {
            hex: Coordinates { q, r, s },
        } => {
            let hex = Hex::new(q, r, s).map_err(ApiError::invalid)?;
            if !snapshot.table_contains_hex(hex) {
                return Err(ApiError::invalid("PING hex is not on the board"));
            }
            json!({"kind": "HEX", "hex": hex})
        }
        PingTarget::Card { hero_id, slot } => {
            if !snapshot.table_contains_card(&hero_id, &slot) {
                return Err(ApiError::invalid(
                    "PING card target is not currently on the table",
                ));
            }
            let mut target = serde_json::to_value(slot).map_err(ApiError::internal)?;
            target["kind"] = json!("CARD");
            target["hero_id"] = json!(hero_id);
            target
        }
    };
    Ok(Some(Notice {
        body: Arc::new(
            json!({"type": "PING", "ping_id": Uuid::new_v4().simple().to_string(),
                              "hero_id": actor, "target": target}),
        ),
        exclude_hero: None,
    }))
}

fn pointer(game: &Game, actor: &str, input: &Value) -> Option<Notice> {
    let mut body = json!({"type": "POINTER", "hero_id": actor});
    if input["hidden"] == true {
        body["hidden"] = json!(true);
    } else if !input["x"].is_null() || !input["z"].is_null() {
        let (x, z) = (input["x"].as_f64()?, input["z"].as_f64()?);
        if !(-100.0..=100.0).contains(&x) || !(-100.0..=100.0).contains(&z) {
            return None;
        }
        body["x"] = input["x"].clone();
        body["z"] = input["z"].clone();
    } else {
        let (hero_id, zone) = (input["target_hero_id"].as_str()?, input["zone"].as_str()?);
        if !matches!(
            zone,
            "COMMIT"
                | "PLAYED"
                | "ULTIMATE"
                | "DECK"
                | "DISCARD"
                | "SPELLBOOK"
                | "STATUS"
                | "WISH"
                | "HAND"
                | "SHEET"
        ) || !game.has_seat(hero_id)
        {
            return None;
        }
        body["target_hero_id"] = json!(hero_id);
        body["zone"] = json!(zone);
    }
    Some(Notice {
        body: Arc::new(body),
        exclude_hero: Some(actor.to_owned()),
    })
}
