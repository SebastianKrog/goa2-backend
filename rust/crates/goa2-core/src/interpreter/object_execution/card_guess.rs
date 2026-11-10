//! Hidden card choices and public color-guess reveals.
use super::phases::Field;
use super::*;

impl NativeSession {
    fn standard_card_colors(&self) -> [Symbol; 5] {
        ["BLUE", "GOLD", "GREEN", "RED", "SILVER"]
            .map(|color| self.symbols.lookup(color).expect("color symbol registered"))
    }

    pub(super) fn choose_card_color_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<NativeRequest>> {
        let player_field = self
            .symbols
            .lookup("player_id_key")
            .ok_or_else(|| unsupported("color chooser player key missing"))?;
        let override_player = match objects.arena.field(step, player_field) {
            Some(ObjectValue::Atom(Datum::Text(key))) => self
                .truthy_symbol_bound(objects, objects.arena.context_value(key))?
                .map(|id| self.normalized_owner(id)),
            _ => None,
        };
        let Some(owner) = override_player
            .or(self.planning.actor)
            .filter(|id| !self.symbols.name(*id).is_empty())
        else {
            return Ok(None);
        };
        let colors = self.standard_card_colors();
        if objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|value| objects.arena.truthy(&value))
        {
            if let Some(ObjectValue::Atom(Datum::Text(color))) =
                objects.selected(step, self.selection_key)
                && colors.contains(&color)
            {
                let output = self.passive_text_bound(objects, step, Field::Output)?;
                objects
                    .arena
                    .set_context(output, ObjectValue::Atom(Datum::Text(color)))
                    .map_err(unsupported)?;
                return Ok(None);
            }
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner,
            prompt: self.passive_text_bound(objects, step, Field::Prompt)?,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::CardColors(colors.to_vec()),
            can_skip: false,
            discard: false,
        }))
    }

    fn record_guess_bound(
        &mut self,
        objects: &mut BoundObjects,
        attempt: i64,
        fields: Vec<(&str, Datum)>,
    ) -> Result<()> {
        let actor = self.planning.actor.map_or(Datum::Null, Datum::Text);
        let root = objects.phases.k(Field::Guess);
        let guesser = objects.phases.k(Field::Guesser);
        let attempts_key = self
            .symbols
            .lookup("attempts")
            .expect("guess symbol registered");
        let attempt_key = self
            .symbols
            .lookup("attempt")
            .expect("guess symbol registered");
        let mut attempts = Vec::new();
        if attempt != 1
            && let Some(ObjectValue::Object(guess)) = objects.arena.root_field(root)
            && matches!(objects.arena.field(guess, guesser), Some(ObjectValue::Atom(value)) if value == actor)
            && let Some(ObjectValue::Object(list)) = objects.arena.field(guess, attempts_key)
            && let Some(values) = objects.arena.sequence(list)
        {
            attempts.extend_from_slice(values);
        }
        let mut fields = fields
            .into_iter()
            .map(|(key, value)| {
                (
                    self.symbols.lookup(key).expect("guess symbol registered"),
                    ObjectValue::Atom(value),
                )
            })
            .collect::<Vec<_>>();
        fields.push((attempt_key, ObjectValue::Atom(Datum::Int(attempt))));
        let number = |value: &ObjectValue, arena: &crate::objects::ObjectArena| {
            if let ObjectValue::Object(id) = value
                && let Some(ObjectValue::Atom(Datum::Int(number))) = arena.field(*id, attempt_key)
            {
                return number;
            }
            0
        };
        attempts.retain(|value| number(value, &objects.arena) != attempt);
        attempts.push(objects.arena.dictionary(fields).map_err(unsupported)?);
        attempts.sort_by_key(|value| number(value, &objects.arena));
        let attempts = objects.arena.list(attempts);
        let guess = objects
            .arena
            .dictionary(vec![
                (guesser, ObjectValue::Atom(actor)),
                (attempts_key, attempts),
            ])
            .map_err(unsupported)?;
        objects.arena.set_root(root, guess).map_err(unsupported)
    }

    pub(super) fn bind_sampled_guess(
        &mut self,
        objects: &mut BoundObjects,
        seed: u64,
    ) -> Result<()> {
        let mut frames = self.stack.iter().rev();
        let Some(guess) = frames.next().filter(|frame| {
            matches!(
                self.steps[frame.instruction].instruction,
                Instruction::LiveCardGuess { reveal: false }
            )
        }) else {
            return Ok(());
        };
        let Some((guess, reveal)) = guess.object.zip(
            frames
                .next()
                .filter(|frame| {
                    matches!(
                        self.steps[frame.instruction].instruction,
                        Instruction::LiveCardGuess { reveal: true }
                    )
                })
                .and_then(|frame| frame.object),
        ) else {
            return Err(unsupported("sampled guess requires its paired reveal"));
        };
        let field = |name| self.symbols.lookup(name).expect("guess symbol registered");
        let operand = |step, name| objects.arena.field(step, field(name));
        let key = |name| match operand(guess, name) {
            Some(ObjectValue::Atom(Datum::Text(value))) if matches!(operand(reveal, name), Some(ObjectValue::Atom(Datum::Text(other))) if other == value) => {
                Ok(value)
            }
            _ => Err(unsupported("sampled guess and reveal operands differ")),
        };
        let victim_key = key("victim_key")?;
        let card_key = key("card_key")?;
        let attempt = match operand(guess, "attempt") {
            Some(ObjectValue::Atom(Datum::Int(value))) if matches!(operand(reveal, "attempt"), Some(ObjectValue::Atom(Datum::Int(other))) if other == value) => {
                value
            }
            _ => return Err(unsupported("sampled guess and reveal attempts differ")),
        };
        let Some(ObjectValue::Atom(Datum::Text(victim))) = objects.arena.context_value(victim_key)
        else {
            return Err(unsupported("sampled guess victim missing"));
        };
        let hand = &self
            .world
            .hero(victim)
            .ok_or_else(|| unsupported("sampled guess victim is not a hero"))?
            .hand;
        if hand.is_empty() {
            return Err(unsupported("sampled guess requires a card in hand"));
        }
        let chosen =
            hand[super::super::loadout::sample_index(seed ^ 0xbf58476d1ce4e5b9, hand.len())];
        let card = self
            .world
            .card(chosen)
            .ok_or_else(|| unsupported("sampled guess card missing"))?;
        let printed = self.world.definitions.cards[card.definition].printed_id;
        objects
            .arena
            .set_context(card_key, ObjectValue::Atom(Datum::Text(printed)))
            .map_err(unsupported)?;
        self.record_guess_bound(
            objects,
            attempt,
            vec![
                ("victim_id", Datum::Text(victim)),
                ("card_id", Datum::Text(printed)),
                ("guessed_color", Datum::Null),
                ("actual_color", Datum::Null),
                ("correct", Datum::Null),
            ],
        )?;
        self.sampled_guess_steps = Some([guess, reveal]);
        Ok(())
    }

    pub(super) fn card_guess_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        reveal: bool,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeRequest>> {
        let field = |name: &str, session: &mut Self| {
            session
                .symbols
                .lookup(name)
                .expect("guess symbol registered")
        };
        let victim_key = field("victim_key", self);
        let card_key = field("card_key", self);
        let attempt_key = field("attempt", self);
        let read = |key, objects: &BoundObjects| {
            if let Some(ObjectValue::Atom(Datum::Text(key))) = objects.arena.field(step, key)
                && let Some(ObjectValue::Atom(Datum::Text(value))) =
                    objects.arena.context_value(key)
            {
                return Some(value);
            }
            None
        };
        let victim = read(victim_key, objects);
        let card = read(card_key, objects);
        let attempt = match objects.arena.field(step, attempt_key) {
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            _ => return Err(unsupported("missing guess attempt")),
        };
        let actor = self.planning.actor;
        if reveal {
            let guess_key = field("guess_key", self);
            let (Some(victim), Some(card), Some(guessed)) =
                (victim, card, read(guess_key, objects))
            else {
                return Ok(None);
            };
            let Some(definition) = self.world.hero(victim).and_then(|hero| {
                hero.hand
                    .iter()
                    .filter_map(|id| self.world.card(*id))
                    .map(|card| &self.world.definitions.cards[card.definition])
                    .find(|definition| definition.printed_id == card)
            }) else {
                return Ok(None);
            };
            let (actual, name) = (definition.color, definition.name);
            let correct = actual == Some(guessed);
            self.reveal_knowledge_bound(objects, victim, card)?;
            for (name, active) in [
                ("correct_output_key", correct),
                ("wrong_output_key", !correct),
            ] {
                let key = field(name, self);
                let Some(ObjectValue::Atom(Datum::Text(output))) = objects.arena.field(step, key)
                else {
                    return Err(unsupported("missing guess result output"));
                };
                objects
                    .arena
                    .set_context(
                        output,
                        ObjectValue::Atom(if active {
                            Datum::Bool(true)
                        } else {
                            Datum::Null
                        }),
                    )
                    .map_err(unsupported)?;
            }
            let reanchor = field("rollback_reanchor_pending", self);
            objects
                .arena
                .set_context(reanchor, ObjectValue::Atom(Datum::Bool(true)))
                .map_err(unsupported)?;
            self.record_guess_bound(
                objects,
                attempt,
                vec![
                    ("victim_id", Datum::Text(victim)),
                    ("card_id", Datum::Text(card)),
                    ("guessed_color", Datum::Text(guessed)),
                    ("actual_color", actual.map_or(Datum::Null, Datum::Text)),
                    ("correct", Datum::Bool(correct)),
                ],
            )?;
            events.push(NativeEvent::GuessRevealed {
                actor,
                victim,
                attempt,
                card,
                name,
                actual,
                guessed,
                correct,
            });
            return Ok(None);
        }
        if objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|value| objects.arena.truthy(&value))
        {
            let output = field("output_key", self);
            let Some(ObjectValue::Atom(Datum::Text(output))) = objects.arena.field(step, output)
            else {
                return Err(unsupported("missing guess output"));
            };
            let selection = objects
                .selected(step, self.selection_key)
                .unwrap_or(ObjectValue::Atom(Datum::Null));
            objects
                .arena
                .set_context(output, selection)
                .map_err(unsupported)?;
            return Ok(None);
        }
        let colors = self.standard_card_colors();
        let mut options = Vec::new();
        if let Some(hero) = victim.and_then(|id| self.world.hero(id)) {
            let possible = hero
                .hand
                .iter()
                .copied()
                .map(|id| (id, true))
                .chain(
                    hero.played
                        .iter()
                        .flatten()
                        .chain(&hero.discard)
                        .chain(&hero.current)
                        .chain(&hero.extra)
                        .copied()
                        .map(|id| (id, false)),
                )
                .filter_map(|(id, hand)| self.world.card(id).filter(|card| hand || card.facedown))
                .filter_map(|card| self.world.definitions.cards[card.definition].color)
                .collect::<Vec<_>>();
            options.extend(colors.into_iter().filter(|color| possible.contains(color)));
        }
        if options.is_empty() {
            options.extend(colors);
        }
        let announced = field("selection_announced", self);
        if let (Some(victim), Some(card)) = (victim, card)
            && !matches!(
                objects.arena.field(step, announced),
                Some(ObjectValue::Atom(Datum::Bool(true)))
            )
        {
            self.record_guess_bound(
                objects,
                attempt,
                vec![
                    ("victim_id", Datum::Text(victim)),
                    ("card_id", Datum::Text(card)),
                    ("guessed_color", Datum::Null),
                    ("actual_color", Datum::Null),
                    ("correct", Datum::Null),
                ],
            )?;
            events.push(NativeEvent::GuessSelected {
                actor,
                victim,
                attempt,
            });
            objects
                .arena
                .set_field(step, announced, ObjectValue::Atom(Datum::Bool(true)))
                .map_err(unsupported)?;
        }
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: actor
                .map(|id| self.normalized_owner(id))
                .unwrap_or(objects.phases.k(Field::EmptyOwner)),
            prompt: self
                .symbols
                .lookup("Guess the card's color")
                .expect("guess symbol registered"),
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::CardColors(options),
            can_skip: false,
            discard: false,
        }))
    }
}
