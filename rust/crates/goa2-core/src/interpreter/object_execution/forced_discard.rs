//! Forced discard snapshots the victim into its actual selection/discard steps.
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, EngineError, NativeSession, ObjectId, ObjectValue, Result, Symbol,
    unsupported,
};

pub(super) struct ForcedDiscardPlan {
    victim: Symbol,
    basic: Option<bool>,
    shields: bool,
    allowed: Option<Vec<Symbol>>,
}
pub(super) struct ColorDiscardPlan {
    victim: Symbol,
    victim_key: Symbol,
    color: Symbol,
    output: Symbol,
}

impl NativeSession {
    pub(super) fn preflight_color_discard_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<ColorDiscardPlan>> {
        let victim_key = self.passive_text_bound(objects, step, Field::VictimKey)?;
        let Some(victim) =
            self.truthy_symbol_bound(objects, objects.arena.context_value(victim_key))?
        else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(victim) else {
            return Ok(None);
        };
        let read = |name| {
            self.symbols
                .lookup(name)
                .and_then(|key| objects.arena.field(step, key))
        };
        let color = if let Some(color) = self.truthy_symbol_bound(objects, read("color"))? {
            Some(color)
        } else if let Some(key) = self.truthy_symbol_bound(objects, read("color_key"))? {
            self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
        } else {
            None
        };
        let Some(color) = color else {
            return Ok(None);
        };
        if !matches!(
            self.symbols.name(color),
            "RED" | "BLUE" | "GREEN" | "GOLD" | "SILVER" | "PURPLE"
        ) {
            return Err(EngineError::Rule("invalid forced discard color".into()));
        }
        if !hero.hand.iter().any(|id| {
            self.world.card(*id).is_some_and(|row| {
                self.world.definitions.cards[row.definition].color == Some(color)
            })
        }) {
            return Ok(None);
        }
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        self.phase_generation_check(0, 2)?;
        Ok(Some(ColorDiscardPlan {
            victim,
            victim_key,
            color,
            output,
        }))
    }

    pub(super) fn color_discard_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<ColorDiscardPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        let prompt = format!(
            "{}, select a {} card to discard.",
            self.symbols.name(plan.victim),
            self.symbols.name(plan.color)
        );
        let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
        objects.arena.symbols = self.symbols.clone();
        let select = self.phase_step(objects, "select_card")?;
        let discard = self.phase_step(objects, "discard_card")?;
        let p = &objects.phases;
        for (field, value) in [
            (Field::Prompt, prompt),
            (Field::Output, plan.output),
            (Field::ContextHeroKey, plan.victim_key),
            (Field::OverridePlayerKey, plan.victim_key),
            (Field::DecisionPurpose, p.k(Field::DiscardPurpose)),
            (Field::Container, p.k(Field::HandState)),
        ] {
            objects
                .arena
                .set_field(select, p.k(field), ObjectValue::Atom(Datum::Text(value)))
                .map_err(unsupported)?;
        }
        objects
            .arena
            .set_field(
                select,
                p.k(Field::Mandatory),
                ObjectValue::Atom(Datum::Bool(true)),
            )
            .map_err(unsupported)?;
        let colors = objects
            .arena
            .list(vec![ObjectValue::Atom(Datum::Text(plan.color))]);
        objects
            .arena
            .set_field(select, p.k(Field::CardColors), colors)
            .map_err(unsupported)?;
        for (field, value) in [
            (Field::HeroKey, plan.victim_key),
            (Field::CardKey, plan.output),
        ] {
            objects
                .arena
                .set_field(discard, p.k(field), ObjectValue::Atom(Datum::Text(value)))
                .map_err(unsupported)?;
        }
        self.push_live_steps_bound(objects, &[select, discard])
    }

    pub(super) fn preflight_forced_discard_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<ForcedDiscardPlan>> {
        let mut victim = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::Victim)),
        )?;
        if victim.is_none()
            && let Some(key) = self.truthy_symbol_bound(
                objects,
                objects
                    .arena
                    .field(step, objects.phases.k(Field::VictimKey)),
            )?
        {
            victim = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let Some(victim) = victim else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(victim) else {
            return Ok(None);
        };
        if let Some(source) = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::Immunity)),
        )? && self
            .world
            .actor_immune(hero.id, source)
            .map_err(unsupported)?
        {
            return Ok(None);
        }
        let basic = match objects.arena.field(step, objects.phases.k(Field::Basic)) {
            None | Some(ObjectValue::Atom(Datum::Null)) => None,
            Some(ObjectValue::Atom(Datum::Bool(value))) => Some(value),
            _ => return Err(unsupported("discard basic filter must be boolean or null")),
        };
        let eligible = hero
            .hand
            .iter()
            .copied()
            .filter(|id| {
                self.world.card(*id).is_some_and(|c| {
                    basic.is_none_or(|b| self.world.definitions.cards[c.definition].basic == b)
                })
            })
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            return Ok(None);
        }
        let shields = self.world.active_shield_cards(hero.id);
        let allowed = if !shields.is_empty() && basic.is_some() {
            Some(
                eligible
                    .iter()
                    .chain(shields.iter())
                    .map(|id| {
                        self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                            .printed_id
                    })
                    .collect(),
            )
        } else {
            None
        };
        self.phase_generation_check(0, 2)?;
        Ok(Some(ForcedDiscardPlan {
            victim,
            basic,
            shields: !shields.is_empty(),
            allowed,
        }))
    }
    pub(super) fn forced_discard_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<ForcedDiscardPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        let prompt = format!(
            "{}, select a card to discard.",
            self.symbols.name(plan.victim)
        );
        let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
        objects.arena.symbols = self.symbols.clone();
        let select = self.phase_step(objects, "select_card")?;
        for (field, value) in [
            (Field::Prompt, Datum::Text(prompt)),
            (
                Field::Output,
                Datum::Text(objects.phases.k(Field::CardToDiscard)),
            ),
            (Field::ContextHero, Datum::Text(plan.victim)),
            (Field::OverridePlayer, Datum::Text(plan.victim)),
            (
                Field::DecisionPurpose,
                Datum::Text(objects.phases.k(Field::DiscardPurpose)),
            ),
        ] {
            objects
                .arena
                .set_field(select, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        if plan.shields {
            let containers = objects.arena.list(
                [Field::HandState, Field::PlayedContainer]
                    .into_iter()
                    .map(|field| ObjectValue::Atom(Datum::Text(objects.phases.k(field))))
                    .collect(),
            );
            objects
                .arena
                .set_field(select, objects.phases.k(Field::Containers), containers)
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(
                    select,
                    objects.phases.k(Field::ShieldsOnly),
                    ObjectValue::Atom(Datum::Bool(true)),
                )
                .map_err(unsupported)?;
            if let Some(allowed) = plan.allowed {
                let list = objects.arena.list(
                    allowed
                        .into_iter()
                        .map(|id| ObjectValue::Atom(Datum::Text(id)))
                        .collect(),
                );
                objects
                    .arena
                    .set_field(select, objects.phases.k(Field::AllowedCards), list)
                    .map_err(unsupported)?;
            }
        } else {
            objects
                .arena
                .set_field(
                    select,
                    objects.phases.k(Field::Basic),
                    ObjectValue::Atom(plan.basic.map_or(Datum::Null, Datum::Bool)),
                )
                .map_err(unsupported)?;
        }
        let discard = self.phase_step(objects, "discard_card")?;
        objects
            .arena
            .set_field(
                discard,
                objects.phases.k(Field::HeroId),
                ObjectValue::Atom(Datum::Text(plan.victim)),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(
                discard,
                objects.phases.k(Field::CardKey),
                ObjectValue::Atom(Datum::Text(objects.phases.k(Field::CardToDiscard))),
            )
            .map_err(unsupported)?;
        self.push_live_steps_bound(objects, &[select, discard])
    }
}

pub(super) enum DiscardOrDefeatPlan {
    Defeat {
        victim: Symbol,
        killer: Option<Symbol>,
    },
    Discard {
        victim: Symbol,
        key: Symbol,
    },
}
impl NativeSession {
    pub(super) fn preflight_discard_or_defeat_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<DiscardOrDefeatPlan>> {
        let key = self.passive_text_bound(objects, step, Field::VictimKey)?;
        let Some(victim) = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
        else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(victim) else {
            return Ok(None);
        };
        if let Some(source) = self.spatial_text_bound(objects, step, Field::Immunity)?
            && self
                .world
                .actor_immune(hero.id, source)
                .map_err(unsupported)?
        {
            return Ok(None);
        }
        if self.world.position(victim).is_none()
            && !(hero.id == victim
                && self.world.entity(hero.id).is_some_and(|e| e.multi_piece)
                && self.world.has_board_presence(hero.id))
        {
            return Ok(None);
        }
        if !hero.hand.is_empty() {
            self.phase_generation_check(0, 2)?;
            return Ok(Some(DiscardOrDefeatPlan::Discard { victim, key }));
        }
        let mut killer = self.spatial_text_bound(objects, step, Field::KillerId)?;
        if killer.is_none()
            && let Some(key) = self.spatial_text_bound(objects, step, Field::KillerKey)?
        {
            killer = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        if killer.is_none() {
            killer = self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty());
        }
        self.phase_generation_check(0, 1)?;
        Ok(Some(DiscardOrDefeatPlan::Defeat { victim, killer }))
    }
    pub(super) fn discard_or_defeat_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<DiscardOrDefeatPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        let children = match plan {
            DiscardOrDefeatPlan::Defeat { victim, killer } => {
                let step = self.phase_step(objects, "defeat")?;
                for (field, value) in [
                    (Field::Victim, Datum::Text(victim)),
                    (Field::KillerId, killer.map_or(Datum::Null, Datum::Text)),
                ] {
                    objects
                        .arena
                        .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                vec![step]
            }
            DiscardOrDefeatPlan::Discard { victim, key } => {
                let prompt = format!(
                    "{}, select a card to discard (or be Defeated).",
                    self.symbols.name(victim)
                );
                let prompt = self
                    .symbols
                    .lookup(&prompt)
                    .unwrap_or_else(|| Arc::make_mut(&mut self.symbols).intern(&prompt));
                objects.arena.symbols = self.symbols.clone();
                let select = self.phase_step(objects, "select_card")?;
                for (field, value) in [
                    (Field::Prompt, Datum::Text(prompt)),
                    (
                        Field::Output,
                        Datum::Text(objects.phases.k(Field::CardToDiscard)),
                    ),
                    (
                        Field::DecisionPurpose,
                        Datum::Text(objects.phases.k(Field::DiscardPurpose)),
                    ),
                    (Field::ContextHeroKey, Datum::Text(key)),
                    (Field::OverridePlayerKey, Datum::Text(key)),
                ] {
                    objects
                        .arena
                        .set_field(select, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                let discard = self.phase_step(objects, "discard_card")?;
                for (field, value) in [
                    (
                        Field::CardKey,
                        Datum::Text(objects.phases.k(Field::CardToDiscard)),
                    ),
                    (Field::HeroKey, Datum::Text(key)),
                ] {
                    objects
                        .arena
                        .set_field(discard, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                vec![select, discard]
            }
        };
        self.push_live_steps_bound(objects, &children)
    }
}
