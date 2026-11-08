//! Consensus corrections to live table state.
use super::{ExecutionBudget, GameTransition, LiveGame, Result, SetupTeam, invalid};
use crate::{
    codec::Phase,
    geometry::Hex,
    stats::{MarkerKind, Stat},
    world::CardLifecycle,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CardPatchZone {
    Hand,
    Discard,
    Played,
    Deck,
}

impl CardPatchZone {
    pub(crate) fn lifecycle(self) -> CardLifecycle {
        match self {
            Self::Hand => CardLifecycle::Hand,
            Self::Discard => CardLifecycle::Discard,
            Self::Played => CardLifecycle::Resolved,
            Self::Deck => CardLifecycle::Deck,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Hand => "hand",
            Self::Discard => "discard",
            Self::Played => "played",
            Self::Deck => "deck",
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchCommand {
    AddEffect {
        effect: Value,
    },
    RemoveEffect {
        effect_id: String,
    },
    MoveCard {
        hero_id: String,
        card_id: String,
        zone: CardPatchZone,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<u8>,
    },
    SwapItemCard {
        hero_id: String,
        color: String,
        tier: String,
    },
    MoveEntity {
        entity_id: String,
        hex: Hex,
    },
    RemoveEntity {
        entity_id: String,
    },
    PlaceEntity {
        entity_id: String,
        hex: Hex,
    },
    AddMarker {
        marker_type: MarkerKind,
        target_id: String,
        #[serde(default)]
        value: i64,
        source_id: String,
    },
    RemoveMarker {
        marker_type: MarkerKind,
    },
    SetGold {
        hero_id: String,
        value: i64,
    },
    SetLevel {
        hero_id: String,
        value: i64,
    },
    SetWaveCounter {
        lane_id: String,
        value: i64,
    },
    SetTieBreakerTeam {
        team: SetupTeam,
    },
    SetLifeCounters {
        team: SetupTeam,
        value: i64,
    },
    SetItem {
        hero_id: String,
        stat: Stat,
        value: i64,
    },
}

fn coordinates(hex: Hex) -> String {
    let (q, r, s) = hex.coordinates();
    format!("({q}, {r}, {s})")
}

impl PatchCommand {
    pub fn summary(&self) -> String {
        match self {
            Self::AddEffect { effect } => {
                format!("Add effect {}", effect["id"].as_str().unwrap_or_default())
            }
            Self::RemoveEffect { effect_id } => format!("Remove effect {effect_id}"),
            Self::MoveCard {
                hero_id,
                card_id,
                zone,
                ..
            } => {
                format!("Move card {card_id} of {hero_id} to {}", zone.name())
            }
            Self::SwapItemCard {
                hero_id,
                color,
                tier,
            } => {
                format!("Swap {hero_id}'s {color} tier {tier} upgrade with its item")
            }
            Self::MoveEntity { entity_id, hex } => {
                format!("Move {entity_id} to {}", coordinates(*hex))
            }
            Self::RemoveEntity { entity_id } => format!("Remove {entity_id} from the board"),
            Self::PlaceEntity { entity_id, hex } => {
                format!("Place {entity_id} at {}", coordinates(*hex))
            }
            Self::AddMarker {
                marker_type,
                target_id,
                ..
            } => {
                format!("Place {} marker on {target_id}", marker_type.name())
            }
            Self::RemoveMarker { marker_type } => {
                format!("Return {} marker to supply", marker_type.name())
            }
            Self::SetGold { hero_id, value } => format!("Set {hero_id} gold to {value}"),
            Self::SetLevel { hero_id, value } => format!("Set {hero_id} level to {value}"),
            Self::SetWaveCounter { lane_id, value } => {
                format!("Set wave counter of {lane_id} to {value}")
            }
            Self::SetTieBreakerTeam { team } => format!("Set tie-breaker team to {}", team.name()),
            Self::SetLifeCounters { team, value } => {
                format!("Set {} life counters to {value}", team.name())
            }
            Self::SetItem {
                hero_id,
                stat,
                value,
            } => format!("Set {hero_id} {} items to {value}", stat.name()),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::AddEffect { .. } => "add_effect",
            Self::RemoveEffect { .. } => "remove_effect",
            Self::MoveCard { .. } => "move_card",
            Self::SwapItemCard { .. } => "swap_item_card",
            Self::MoveEntity { .. } => "move_entity",
            Self::RemoveEntity { .. } => "remove_entity",
            Self::PlaceEntity { .. } => "place_entity",
            Self::AddMarker { .. } => "add_marker",
            Self::RemoveMarker { .. } => "remove_marker",
            Self::SetGold { .. } => "set_gold",
            Self::SetLevel { .. } => "set_level",
            Self::SetWaveCounter { .. } => "set_wave_counter",
            Self::SetTieBreakerTeam { .. } => "set_tie_breaker_team",
            Self::SetLifeCounters { .. } => "set_life_counters",
            Self::SetItem { .. } => "set_item",
        }
    }
}

impl LiveGame {
    pub fn validate_patch_proposer(&self, command: &PatchCommand, proposer: &str) -> Result<()> {
        let allowed = match command {
            PatchCommand::MoveCard {
                hero_id, card_id, ..
            } if hero_id != proposer => self
                .session
                .patch_card_is_public(self.hero(hero_id)?, card_id)?,
            PatchCommand::SwapItemCard { hero_id, .. } => hero_id == proposer,
            _ => true,
        };
        if !allowed {
            return Err(invalid(
                "Only the owner can propose a correction to a private card",
            ));
        }
        Ok(())
    }

    pub fn validate_patch(&self, command: &PatchCommand) -> Result<()> {
        if self.clock_paused() {
            return Err(invalid("The match is paused"));
        }
        match command {
            PatchCommand::AddEffect { effect } => {
                self.session.validate_effect_patch(effect)?;
            }
            PatchCommand::MoveCard {
                zone,
                slot: Some(slot),
                ..
            } if *zone != CardPatchZone::Played || !(1..=4).contains(slot) => {
                return Err(invalid(
                    "A played slot must be between 1 and 4 and only applies to the played zone",
                ));
            }
            PatchCommand::SwapItemCard { color, tier, .. }
                if !matches!(color.as_str(), "RED" | "BLUE" | "GREEN")
                    || !matches!(tier.as_str(), "II" | "III") =>
            {
                return Err(invalid(
                    "Choose a red, blue or green tier II or III upgrade",
                ));
            }
            PatchCommand::SetLevel { value, .. } if !(1..=8).contains(value) => {
                return Err(invalid("Level must be between 1 and 8"));
            }
            PatchCommand::SetGold { value, .. }
            | PatchCommand::SetWaveCounter { value, .. }
            | PatchCommand::SetLifeCounters { value, .. }
            | PatchCommand::SetItem { value, .. }
                if *value < 0 =>
            {
                return Err(invalid("Patch values must be non-negative"));
            }
            _ => {}
        }
        match command {
            PatchCommand::MoveCard { hero_id, .. }
            | PatchCommand::SwapItemCard { hero_id, .. }
            | PatchCommand::SetGold { hero_id, .. }
            | PatchCommand::SetLevel { hero_id, .. }
            | PatchCommand::SetItem { hero_id, .. } => {
                self.hero(hero_id)?;
            }
            PatchCommand::SetWaveCounter { lane_id, .. } => {
                if !self
                    .session
                    .symbols
                    .lookup(lane_id)
                    .is_some_and(|lane| self.session.world.waves.iter().any(|(id, _)| *id == lane))
                {
                    return Err(invalid("Unknown lane"));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn prepare_patch(
        &self,
        command: &PatchCommand,
        now: u64,
        budget: &ExecutionBudget<'_>,
    ) -> Result<(Self, GameTransition)> {
        budget.check(0)?;
        self.validate_patch(command)?;
        let mut next = self.table_edit_candidate(now)?;
        let previous_request = next
            .session
            .last_request
            .as_ref()
            .map(|request| next.session.request_token(request.id));
        next.session.invalidate_table_prompt()?;
        let event = match command {
            PatchCommand::AddEffect { effect } => {
                next.session.patch_add_effect(effect)?;
                None
            }
            PatchCommand::RemoveEffect { effect_id } => {
                next.session.patch_remove_effect(effect_id)?;
                None
            }
            PatchCommand::MoveCard {
                hero_id,
                card_id,
                zone,
                slot,
            } => {
                let hero = next.hero(hero_id)?;
                next.session.patch_move_card(hero, card_id, *zone, *slot)?;
                next.upgrade_history.invalidate_edited_hero(hero_id);
                None
            }
            PatchCommand::SwapItemCard {
                hero_id,
                color,
                tier,
            } => {
                let hero = next.hero(hero_id)?;
                next.session.patch_swap_item_card(hero, color, tier)?;
                next.upgrade_history.invalidate_edited_hero(hero_id);
                None
            }
            PatchCommand::MoveEntity { entity_id, hex } => {
                next.session
                    .patch_entity_position(entity_id, Some(*hex), true)?;
                None
            }
            PatchCommand::RemoveEntity { entity_id } => {
                next.session.patch_entity_position(entity_id, None, true)?;
                None
            }
            PatchCommand::PlaceEntity { entity_id, hex } => {
                next.session
                    .patch_entity_position(entity_id, Some(*hex), false)?;
                None
            }
            PatchCommand::AddMarker {
                marker_type,
                target_id,
                value,
                source_id,
            } => {
                next.session
                    .patch_marker(*marker_type, Some((target_id, source_id, *value)))?;
                None
            }
            PatchCommand::RemoveMarker { marker_type } => {
                next.session.patch_marker(*marker_type, None)?;
                None
            }
            PatchCommand::SetGold { hero_id, value } => {
                let hero = next.hero(hero_id)?;
                next.session.patch_hero_gold(hero, *value)?;
                None
            }
            PatchCommand::SetLevel { hero_id, value } => {
                let hero = next.hero(hero_id)?;
                next.session.patch_hero_level(hero, *value)?;
                next.upgrade_history.invalidate_edited_hero(hero_id);
                None
            }
            PatchCommand::SetItem {
                hero_id,
                stat,
                value,
            } => {
                let hero = next.hero(hero_id)?;
                next.session.patch_hero_items(hero, *stat, *value)?;
                next.upgrade_history.invalidate_edited_hero(hero_id);
                None
            }
            PatchCommand::SetWaveCounter { lane_id, value } => {
                next.session.patch_wave(lane_id, *value)?;
                None
            }
            PatchCommand::SetTieBreakerTeam { team } => {
                next.session.patch_tie_breaker(team.name())?;
                None
            }
            PatchCommand::SetLifeCounters { team, value } => {
                next.session.patch_life(team.name(), *value)?
            }
        };
        let mut transition = next.advance(budget)?;
        transition.previous_phase = self.session.planning.phase;
        if let Some(event) = event {
            transition.events.insert(0, next.session.event_json(&event));
        }
        if let (Some(previous), Some(request)) =
            (previous_request, next.session.last_request.as_ref())
        {
            next.preserve_response_credit(&previous, next.session.request_token(request.id));
        }
        if self.session.planning.phase == Phase::GameOver
            && next.session.planning.phase != Phase::GameOver
        {
            next.ready_after_reviving();
        }
        next.update_rollback()?;
        next.reconcile_clock(now)?;
        Ok((next, transition))
    }
}
