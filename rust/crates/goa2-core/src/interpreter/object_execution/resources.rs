//! Resource effects stage live operands before consuming the executing step.
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, EngineError, GameOutcome, NativeEvent, NativeSession, ObjectId,
    ObjectValue, Result, Symbol, unsupported,
};

pub(super) enum ResourcePlan {
    Done,
    SpendLife {
        model: ObjectId,
        team: Symbol,
        target: Symbol,
        total: i64,
        change: i64,
        outcome: Option<GameOutcome>,
    },
    Item {
        items: ObjectId,
        owner: Symbol,
        stat: crate::stats::Stat,
        key: Symbol,
        total: i64,
        amount: i64,
    },
    Gold {
        model: ObjectId,
        owner: Symbol,
        total: i64,
        output: Option<Symbol>,
        event: NativeEvent,
    },
    Transfer {
        from: (ObjectId, Symbol, i64),
        to: (ObjectId, Symbol, i64),
        output: Option<Symbol>,
        event: NativeEvent,
    },
    Life {
        model: ObjectId,
        team: Symbol,
        total: i64,
        event: NativeEvent,
    },
}
impl NativeSession {
    pub(super) fn resource_identity_bound(
        &mut self,
        objects: &BoundObjects,
        value: ObjectValue,
    ) -> Result<Symbol> {
        let value = objects
            .arena
            .typed_value(&value, Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let text = value
            .python_scalar_text(&self.symbols)
            .ok_or_else(|| unsupported("resource identity aggregate coercion not bound"))?;
        Ok(Arc::make_mut(&mut self.symbols).intern(&text))
    }
    fn resource_integer_bound(&self, _objects: &BoundObjects, value: &ObjectValue) -> Result<i64> {
        match value {
            ObjectValue::Atom(Datum::Int(value)) => Ok(*value),
            ObjectValue::Atom(Datum::Bool(value)) => Ok(i64::from(*value)),
            _ => Err(unsupported("resource integer operand not bound")),
        }
    }
    pub(super) fn preflight_resource_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<ResourcePlan> {
        let kind = objects
            .arena
            .field(step, objects.phases.k(Field::FilterType));
        let kind = match kind {
            Some(ObjectValue::Atom(Datum::Text(value))) => self.symbols.name(value),
            _ => return Err(unsupported("resource step type missing")),
        };
        if kind == "steal_coins" {
            return self.preflight_steal_bound(objects, step);
        }
        let item = kind == "gain_item";
        let spend_life = kind == "spend_additional_life_counter";
        let life = kind == "restore_spent_life_counter";
        let lost = kind == "lose_coins";
        let value = if life {
            let mut value = objects
                .arena
                .field(step, objects.phases.k(Field::HeroId))
                .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)));
            if let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::HeroKey)?
                .filter(|key| !self.symbols.name(*key).is_empty())
                && let Some(found) = objects
                    .arena
                    .context_value(key)
                    .filter(|v| objects.arena.truthy(v))
            {
                value = Some(found);
            }
            value.or_else(|| {
                self.planning
                    .actor
                    .filter(|id| !self.symbols.name(*id).is_empty())
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
            })
        } else {
            let key = self.passive_text_bound(
                objects,
                step,
                if lost || spend_life {
                    Field::VictimKey
                } else {
                    Field::HeroKey
                },
            )?;
            objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v))
        };
        let Some(value) = value else {
            return Ok(ResourcePlan::Done);
        };
        let raw = self.resource_identity_bound(objects, value)?;
        let Some(owner) = self.world.hero(raw).map(|hero| hero.id) else {
            return Ok(ResourcePlan::Done);
        };
        let model = self
            .hero_model_bound(objects, owner)
            .ok_or_else(|| unsupported("resource hero model missing"))?;
        let amount = objects
            .arena
            .field(step, objects.phases.k(Field::Amount))
            .ok_or_else(|| unsupported("resource amount missing"))?;
        if item {
            let stat_field = self.cleanup_key("stat_type");
            let Some(ObjectValue::Atom(Datum::Text(key))) = objects.arena.field(step, stat_field)
            else {
                return Err(unsupported("item stat missing"));
            };
            let stat = crate::stats::Stat::parse(self.symbols.name(key)).map_err(unsupported)?;
            let Some(ObjectValue::Object(items)) =
                objects.arena.field(model, self.cleanup_key("items"))
            else {
                return Err(unsupported("hero items mapping missing"));
            };
            let amount = self.resource_integer_bound(objects, &amount)?;
            let current = objects
                .arena
                .field(items, key)
                .map(|value| self.resource_integer_bound(objects, &value))
                .transpose()?
                .unwrap_or(0);
            let total = current
                .checked_add(amount)
                .ok_or_else(|| unsupported("item total outside i64"))?;
            return Ok(ResourcePlan::Item {
                items,
                owner,
                stat,
                key,
                total,
                amount,
            });
        }
        if spend_life {
            let team = self.world.hero(owner).unwrap().team;
            let teams = objects.phases.root(&objects.arena, Field::Teams)?;
            let Some(ObjectValue::Object(model)) = objects.arena.field(teams, team) else {
                return Ok(ResourcePlan::Done);
            };
            let current = objects
                .arena
                .field(model, objects.phases.k(Field::Life))
                .ok_or_else(|| unsupported("life counters missing"))?;
            let amount = self.resource_integer_bound(objects, &amount)?;
            let change = amount
                .checked_neg()
                .ok_or_else(|| unsupported("life penalty overflow"))?;
            let total = self
                .resource_integer_bound(objects, &current)?
                .checked_add(change)
                .ok_or_else(|| unsupported("life counter overflow"))?
                .max(0);
            let outcome = if total == 0 {
                self.phase_generation_check(0, 1)?;
                let winner = if self.symbols.name(team) == "RED" {
                    "BLUE"
                } else {
                    "RED"
                };
                Some(GameOutcome {
                    team: self.symbols.lookup(winner),
                    hero: None,
                    condition: self
                        .symbols
                        .lookup("ANNIHILATION")
                        .ok_or_else(|| unsupported("annihilation condition missing"))?,
                })
            } else {
                None
            };
            return Ok(ResourcePlan::SpendLife {
                model,
                team,
                target: raw,
                total,
                change,
                outcome,
            });
        }
        if life {
            let Some(team) =
                self.passive_optional_text_bound(objects, model, Field::ResourceTeam)?
            else {
                return Ok(ResourcePlan::Done);
            };
            let teams = objects.phases.root(&objects.arena, Field::Teams)?;
            let Some(ObjectValue::Object(model)) = objects.arena.field(teams, team) else {
                return Ok(ResourcePlan::Done);
            };
            let Some(starting) = objects
                .arena
                .field(model, objects.phases.k(Field::StartingLife))
                .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)))
            else {
                return Ok(ResourcePlan::Done);
            };
            let starting = self.resource_integer_bound(objects, &starting)?;
            let current = objects
                .arena
                .field(model, objects.phases.k(Field::Life))
                .ok_or_else(|| unsupported("resource life missing"))?;
            let current = self.resource_integer_bound(objects, &current)?;
            let gap = starting
                .checked_sub(current)
                .ok_or_else(|| unsupported("life gap outside i64"))?;
            let restored = self.resource_integer_bound(objects, &amount)?.min(gap);
            if restored <= 0 {
                return Ok(ResourcePlan::Done);
            }
            let total = current
                .checked_add(restored)
                .ok_or_else(|| unsupported("life total outside i64"))?;
            return Ok(ResourcePlan::Life {
                model,
                team,
                total,
                event: NativeEvent::LifeRestored {
                    hero: owner,
                    team,
                    change: restored,
                    remaining: total,
                },
            });
        }
        let amount = if let Some(key) = self
            .passive_optional_text_bound(objects, step, Field::AmountKey)?
            .filter(|key| !self.symbols.name(*key).is_empty())
        {
            objects.arena.context_value(key).unwrap_or(amount)
        } else {
            amount
        };
        let event_amount = if lost {
            None
        } else {
            match &amount {
                ObjectValue::Atom(value) => Some(value.clone()),
                _ => None,
            }
        };
        let current = objects
            .arena
            .field(model, objects.phases.k(Field::Gold))
            .ok_or_else(|| unsupported("resource gold missing"))?;
        let current = self.resource_integer_bound(objects, &current)?;
        let requested = if lost {
            let amount = objects
                .arena
                .typed_value(&amount, Arc::make_mut(&mut self.symbols))
                .map_err(unsupported)?;
            amount
                .to_integer(&self.symbols)
                .map_err(unsupported)?
                .ok_or_else(|| unsupported("coin loss integer conversion failed"))?
        } else {
            self.resource_integer_bound(objects, &amount)?
        };
        let amount = if lost {
            requested.max(0).min(current)
        } else {
            requested
        };
        let total = if lost {
            current.checked_sub(amount)
        } else {
            current.checked_add(amount)
        }
        .ok_or_else(|| unsupported("gold total outside i64"))?;
        let output = if lost {
            self.passive_optional_text_bound(objects, step, Field::Output)?
                .filter(|key| !self.symbols.name(*key).is_empty())
        } else {
            None
        };
        Ok(ResourcePlan::Gold {
            model,
            owner,
            total,
            output,
            event: NativeEvent::GoldEffect {
                actor: if lost {
                    self.planning
                        .actor
                        .filter(|id| !self.symbols.name(*id).is_empty())
                } else {
                    Some(raw)
                },
                target: lost.then_some(raw),
                owner,
                amount: event_amount.unwrap_or(Datum::Int(amount)),
                lost,
            },
        })
    }
    fn preflight_steal_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<ResourcePlan> {
        let key = self.passive_text_bound(objects, step, Field::VictimKey)?;
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(ResourcePlan::Done);
        };
        let victim_raw = self.resource_identity_bound(objects, value)?;
        let Some(victim) = self.world.hero(victim_raw).map(|h| h.id) else {
            return Ok(ResourcePlan::Done);
        };
        let actor = objects
            .arena
            .context_value(objects.actor)
            .filter(|v| objects.arena.truthy(v))
            .or_else(|| {
                self.planning
                    .actor
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
            });
        let Some(actor) = actor else {
            return Ok(ResourcePlan::Done);
        };
        let actor_raw = self.resource_identity_bound(objects, actor)?;
        let Some(actor) = self.world.hero(actor_raw).map(|h| h.id) else {
            return Ok(ResourcePlan::Done);
        };
        let from = self
            .hero_model_bound(objects, victim)
            .ok_or_else(|| unsupported("steal victim model missing"))?;
        let to = self
            .hero_model_bound(objects, actor)
            .ok_or_else(|| unsupported("steal actor model missing"))?;
        let base = objects
            .arena
            .field(step, objects.phases.k(Field::Amount))
            .ok_or_else(|| unsupported("steal amount missing"))?;
        let requested = if let Some(key) = self
            .passive_optional_text_bound(objects, step, Field::AmountKey)?
            .filter(|id| !self.symbols.name(*id).is_empty())
        {
            objects.arena.context_value(key).unwrap_or(base)
        } else {
            base
        };
        let from_value = objects
            .arena
            .field(from, objects.phases.k(Field::Gold))
            .ok_or_else(|| unsupported("steal victim gold missing"))?;
        let available = self.resource_integer_bound(objects, &from_value)?;
        let requested_int = self.resource_integer_bound(objects, &requested)?;
        let actual = requested_int.min(available);
        if actual <= 0 {
            return Ok(ResourcePlan::Done);
        }
        let amount = if requested_int <= available {
            requested
        } else {
            from_value
        };
        let ObjectValue::Atom(amount) = amount else {
            return Err(unsupported("steal amount is not scalar"));
        };
        let current = objects
            .arena
            .field(to, objects.phases.k(Field::Gold))
            .ok_or_else(|| unsupported("steal actor gold missing"))?;
        let current = self.resource_integer_bound(objects, &current)?;
        let remaining = available
            .checked_sub(actual)
            .ok_or_else(|| unsupported("steal victim gold outside i64"))?;
        let total = if from == to {
            current
        } else {
            current
                .checked_add(actual)
                .ok_or_else(|| unsupported("steal actor gold outside i64"))?
        };
        let output = self
            .passive_optional_text_bound(objects, step, Field::Output)?
            .filter(|id| !self.symbols.name(*id).is_empty());
        Ok(ResourcePlan::Transfer {
            from: (from, victim, remaining),
            to: (to, actor, total),
            output,
            event: NativeEvent::CoinsStolen {
                actor: actor_raw,
                victim: victim_raw,
                amount,
            },
        })
    }
    pub(in crate::interpreter) fn grant_test_gold(
        &mut self,
        hero: Symbol,
        amount: i64,
    ) -> Result<()> {
        let total = self
            .world
            .hero(hero)
            .and_then(|row| row.gold.checked_add(amount))
            .ok_or_else(|| {
                EngineError::InvalidInput("Gold total is outside the supported range".into())
            })?;
        self.patch_hero_gold(hero, total)
    }

    pub(super) fn write_resource_gold_bound(
        &mut self,
        objects: &mut BoundObjects,
        model: ObjectId,
        owner: Symbol,
        total: i64,
    ) -> Result<()> {
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::Gold),
                ObjectValue::Atom(Datum::Int(total)),
            )
            .map_err(unsupported)?;
        let row = self
            .world
            .hero_index(owner)
            .ok_or_else(|| unsupported("resource cached hero missing"))?;
        self.world.heroes[row].gold = total;
        Ok(())
    }
    pub(super) fn resource_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: ResourcePlan,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeEvent>> {
        objects.arena.symbols = self.symbols.clone();
        match plan {
            ResourcePlan::Done => Ok(None),
            ResourcePlan::SpendLife {
                model,
                team,
                target,
                total,
                change,
                outcome,
            } => {
                objects
                    .arena
                    .set_field(
                        model,
                        objects.phases.k(Field::Life),
                        ObjectValue::Atom(Datum::Int(total)),
                    )
                    .map_err(unsupported)?;
                self.world
                    .combat
                    .set_team_life(team, total)
                    .map_err(unsupported)?;
                let penalty = NativeEvent::AdditionalLifeSpent {
                    team,
                    target,
                    total,
                    change,
                };
                if let Some(outcome) = outcome {
                    self.solo_win_bound(objects, outcome)?;
                    events.push(penalty);
                    Ok(Some(NativeEvent::LifePenaltyAnnihilation {
                        winner: outcome
                            .team
                            .ok_or_else(|| unsupported("annihilation winner missing"))?,
                    }))
                } else {
                    Ok(Some(penalty))
                }
            }

            ResourcePlan::Item {
                items,
                owner,
                stat,
                key,
                total,
                amount,
            } => {
                objects
                    .arena
                    .set_field(items, key, ObjectValue::Atom(Datum::Int(total)))
                    .map_err(unsupported)?;
                let hero = self.world.hero_row_mut(owner).unwrap();
                hero.items[stat.index()] = total;
                if !hero.items_order.contains(&stat) {
                    hero.items_order.push(stat);
                }
                Ok(Some(NativeEvent::ItemGranted {
                    owner,
                    stat,
                    amount,
                }))
            }

            ResourcePlan::Gold {
                model,
                owner,
                total,
                output,
                event,
            } => {
                self.write_resource_gold_bound(objects, model, owner, total)?;
                if let Some(key) = output {
                    let NativeEvent::GoldEffect { amount, .. } = &event else {
                        unreachable!()
                    };
                    objects
                        .arena
                        .set_context(key, ObjectValue::Atom(amount.clone()))
                        .map_err(unsupported)?;
                }
                Ok(Some(event))
            }
            ResourcePlan::Transfer {
                from,
                to,
                output,
                event,
            } => {
                self.write_resource_gold_bound(objects, from.0, from.1, from.2)?;
                self.write_resource_gold_bound(objects, to.0, to.1, to.2)?;
                if let Some(key) = output {
                    objects.write(key, Datum::Bool(true))?;
                }
                Ok(Some(event))
            }
            ResourcePlan::Life {
                model,
                team,
                total,
                event,
            } => {
                objects
                    .arena
                    .set_field(
                        model,
                        objects.phases.k(Field::Life),
                        ObjectValue::Atom(Datum::Int(total)),
                    )
                    .map_err(unsupported)?;
                self.world
                    .combat
                    .set_team_life(team, total)
                    .map_err(unsupported)?;
                Ok(Some(event))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Value;
    use super::*;
    fn session() -> NativeSession {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let index: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join("object-execution.json")).unwrap(),
        ))
        .unwrap();
        let entry = index["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "object-execution-resource-gain-plain")
            .unwrap();
        let case: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .unwrap();
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn gains_read_live_amounts_and_preserve_boolean_event_metadata() {
        let root = session();
        let before = root.export_envelope().unwrap();
        let mut changed = root.clone();
        let step = changed.stack.last().unwrap().object.unwrap();
        let objects = changed.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::Amount),
                ObjectValue::Atom(Datum::Bool(true)),
            )
            .unwrap();
        let result = changed.advance_prefix(1).unwrap();
        assert_eq!(
            changed.event_json(&result.events[0])["metadata"]["amount"],
            Value::Bool(true)
        );
        let envelope = changed.export_envelope().unwrap();
        assert_eq!(envelope.state["execution_context"]["hero_alias"]["gold"], 6);
        let hero = changed.symbols.lookup("hero_arien").unwrap();
        assert_eq!(changed.world.hero(hero).unwrap().gold, 6);
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn unsupported_gain_amount_is_guarded_before_step_or_arena_mutation() {
        let mut worker = session();
        let step = worker.stack.last().unwrap().object.unwrap();
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::Amount),
                ObjectValue::Atom(Datum::Float(1.5)),
            )
            .unwrap();
        let before = worker.export_envelope().unwrap();
        let depth = worker.stack.len();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("resource integer operand not bound")
        );
        assert_eq!(worker.stack.len(), depth);
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
}
