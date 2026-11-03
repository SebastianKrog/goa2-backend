//! Live defense windows: shared menus, physical-card equality and arena action stacks.
use super::phases::Field;
use super::*;
use crate::reactions::{DefenseCard, DefenseMenu};

const RESET: [Field; 5] = [
    Field::AutoBlock,
    Field::DefenseInvalid,
    Field::DefenseBonus,
    Field::IgnoreMinionDefense,
    Field::SnorriRuneDefense,
];
pub(super) enum ReactionPlan {
    Missing,
    Pass(Symbol),
    Selected {
        target: Symbol,
        owner: Symbol,
        card: DefenseCard,
        stack: Option<ObjectId>,
        create_stack: bool,
        current: Option<ObjectValue>,
    },
    Wait {
        menu: DefenseMenu,
        clear_pending: bool,
    },
}
impl NativeSession {
    pub(super) fn preflight_reaction_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<ReactionPlan> {
        let key = self.passive_text_bound(objects, step, Field::TargetPlayerKey)?;
        // Source removes flags before reading the target, including a target
        // key that aliases one of those flags.
        if RESET.iter().any(|field| objects.phases.k(*field) == key) {
            return Ok(ReactionPlan::Missing);
        }
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|value| objects.arena.truthy(value))
        else {
            return Ok(ReactionPlan::Missing);
        };
        let target = match value {
            ObjectValue::Atom(Datum::Text(id)) => id,
            ObjectValue::Atom(value) => {
                let text = value
                    .python_scalar_text(&self.symbols)
                    .ok_or_else(|| unsupported("reaction target scalar not bound"))?;
                Arc::make_mut(&mut self.symbols).intern(&text)
            }
            _ => return Err(unsupported("reaction aggregate target coercion not bound")),
        };
        self.sync_card_collections_bound(objects)?;
        let Some(owner) = self.world.hero(target).map(|hero| hero.id) else {
            return Ok(ReactionPlan::Pass(target));
        };
        let keys = [
            Field::BlockPrimaryDefense,
            Field::DefenseInitiative,
            Field::AttackDamage,
            Field::PerformingCard,
            Field::PerformingOwner,
        ]
        .map(|f| objects.phases.k(f));
        let inputs = self.spatial_inputs_bound(objects, keys);
        let (cards, initiative) = self
            .reaction_rules
            .cards(&self.world, &inputs, &self.symbols, target)
            .map_err(unsupported)?;
        let selected = match objects.arena.field(step, objects.pending) {
            None | Some(ObjectValue::Atom(Datum::Null)) => None,
            Some(ObjectValue::Object(id)) if objects.arena.kind(id) == ObjectKind::Dict => {
                objects.arena.field(id, self.selection_key)
            }
            _ => return Err(unsupported("reaction pending input requires a dictionary")),
        };
        if matches!(selected, Some(ObjectValue::Atom(Datum::Text(id))) if id==objects.phases.k(Field::PassChoice))
        {
            return Ok(ReactionPlan::Pass(target));
        }
        if let Some(ObjectValue::Atom(Datum::Text(id))) = selected.as_ref()
            && let Some(card) = cards.iter().find(|card| card.id == *id)
        {
            let current = objects
                .arena
                .context_value(objects.phases.k(Field::CurrentAction))
                .filter(|value| objects.arena.truthy(value));
            let existing = objects.arena.context_value(objects.type_stack);
            let create_stack = existing.is_none();
            let stack = if let Some(current) = &current {
                match existing {
                    None => None,
                    Some(ObjectValue::Object(id)) if !objects.protected(id) => {
                        objects
                            .arena
                            .check_append(id, current)
                            .map_err(unsupported)?;
                        Some(id)
                    }
                    _ => return Err(unsupported("reaction action stack is not a writable list")),
                }
            } else {
                None
            };
            self.phase_generation_check(0, 2)?;
            return Ok(ReactionPlan::Selected {
                target,
                owner,
                card: card.clone(),
                stack,
                create_stack,
                current,
            });
        }
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        let clear_pending = selected
            .as_ref()
            .is_some_and(|value| objects.arena.truthy(value));
        let menu = self
            .reaction_rules
            .menu(
                &self.world,
                &inputs,
                target,
                self.planning.actor,
                cards,
                initiative,
            )
            .map_err(unsupported)?;
        Ok(ReactionPlan::Wait {
            menu,
            clear_pending,
        })
    }
    fn reaction_pass_bound(&self, objects: &mut BoundObjects, target: Symbol) -> Result<()> {
        for (field, value) in [
            (Field::DefenseValue, Datum::Null),
            (Field::DefenseCardId, Datum::Null),
            (Field::DefenseCardColor, Datum::Null),
            (Field::DefenderId, Datum::Text(target)),
            (Field::PrimaryDefense, Datum::Bool(false)),
        ] {
            objects.write(objects.phases.k(field), value)?;
        }
        Ok(())
    }
    pub(super) fn reaction_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: ReactionPlan,
    ) -> Result<Option<NativeRequest>> {
        objects.arena.symbols = self.symbols.clone();
        for field in RESET {
            objects
                .arena
                .remove_context(objects.phases.k(field))
                .map_err(unsupported)?;
        }
        match plan {
            ReactionPlan::Missing => {}
            ReactionPlan::Pass(target) => self.reaction_pass_bound(objects, target)?,
            ReactionPlan::Selected {
                target,
                owner,
                card,
                stack,
                create_stack,
                current,
            } => {
                for (field, value) in [
                    (Field::DefenseValue, Datum::Int(card.total)),
                    (Field::DefenseCardId, Datum::Text(card.id)),
                    (
                        Field::DefenseCardColor,
                        card.color.map_or(Datum::Null, Datum::Text),
                    ),
                    (Field::DefenderId, Datum::Text(target)),
                    (Field::PrimaryDefense, Datum::Bool(card.primary)),
                ] {
                    objects.write(objects.phases.k(field), value)?;
                }
                let stack = if create_stack {
                    let ObjectValue::Object(list) = objects.arena.list(Vec::new()) else {
                        unreachable!()
                    };
                    objects
                        .arena
                        .set_context(objects.type_stack, ObjectValue::Object(list))
                        .map_err(unsupported)?;
                    Some(list)
                } else {
                    stack
                };
                if let Some(current) = current {
                    objects
                        .arena
                        .append(
                            stack.ok_or_else(|| unsupported("reaction stack plan missing"))?,
                            current,
                        )
                        .map_err(unsupported)?;
                }
                objects.write(
                    objects.phases.k(Field::CurrentAction),
                    Datum::Text(objects.phases.k(Field::DefenseKind)),
                )?;
                let discard = self.phase_step(objects, "discard_card")?;
                let passive = self.phase_step(objects, "passive")?;
                for (child, field, value) in [
                    (discard, Field::CardId, Datum::Text(card.id)),
                    (discard, Field::HeroId, Datum::Text(owner)),
                    (passive, Field::HeroId, Datum::Text(owner)),
                    (
                        passive,
                        Field::Trigger,
                        Datum::Text(objects.phases.k(Field::BeforeDefense)),
                    ),
                ] {
                    objects
                        .arena
                        .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                self.push_live_steps_bound(objects, &[discard, passive])?;
            }
            ReactionPlan::Wait {
                menu,
                clear_pending,
            } => {
                if clear_pending {
                    objects
                        .arena
                        .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                objects.write(
                    objects.phases.k(Field::MinionDefenseModifier),
                    Datum::Int(menu.modifier),
                )?;
                return Ok(Some(NativeRequest {
                    controlled: None,
                    id: RequestToken::Generated(0),
                    owner: menu.owner,
                    prompt: objects.phases.k(Field::TargetPlayerKey),
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::Defense(menu),
                    can_skip: false,
                    discard: false,
                }));
            }
        }
        Ok(None)
    }
}
