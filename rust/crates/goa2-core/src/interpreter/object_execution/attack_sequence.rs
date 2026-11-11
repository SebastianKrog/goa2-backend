//! Attack macro expansion with live history/filter aliases and source model copies.
use super::phases::Field;
use super::{
    BoundObjects, Datum, NativeSession, ObjectId, ObjectKind, ObjectValue, Result, Symbol,
    unsupported,
};
use crate::{
    attack::{AttackSpec, AttackValues},
    objects::ObjectSlot,
    stats::Stat,
};

pub(super) struct AttackPlan {
    expansion: Option<AttackExpansionPlan>,
}
struct AttackExpansionPlan {
    key: Symbol,
    select: bool,
    marks: Option<ObjectId>,
    mark: i64,
    values: AttackValues,
    ranged: bool,
    initiative: bool,
    source_card: Option<Datum>,
    filters: Vec<ObjectValue>,
}
impl NativeSession {
    pub(super) fn preflight_attack_sequence_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<AttackPlan> {
        let text = |field| -> Result<Option<Symbol>> {
            Ok(self
                .passive_optional_text_bound(objects, step, field)?
                .filter(|key| !self.symbols.name(*key).is_empty()))
        };
        let stat = |field| -> Result<Option<Stat>> {
            text(field)?
                .map(|id| Stat::parse(self.symbols.name(id)).map_err(unsupported))
                .transpose()
        };
        let spec = AttackSpec {
            damage: self
                .spatial_int_bound(objects, step, Field::Damage)?
                .ok_or_else(|| unsupported("attack damage missing"))?,
            range: self
                .spatial_int_bound(objects, step, Field::Range)?
                .ok_or_else(|| unsupported("attack range missing"))?,
            ranged: self.spatial_bool_bound(objects, step, Field::Ranged)?,
            card_ranged: false,
            initiative: self.spatial_bool_bound(objects, step, Field::DefenseInitiative)?,
            target: text(Field::AttackTargetKey)?,
            output: text(Field::AttackOutputKey)?,
            damage_bonus: text(Field::DamageBonusKey)?,
            range_bonus: text(Field::RangeBonusKey)?,
            damage_stat: stat(Field::DamageStat)?,
            range_stat: stat(Field::RangeStat)?,
        };
        let keys = [Field::PerformingCard, Field::PerformingOwner].map(|f| objects.phases.k(f));
        let inputs = self.spatial_inputs_bound(
            objects,
            keys.into_iter()
                .chain(spec.target)
                .chain(spec.damage_bonus)
                .chain(spec.range_bonus),
        );
        let Some((key, select)) = self
            .attack_rules
            .target(&inputs, &self.symbols, &spec, None)
        else {
            return Ok(AttackPlan { expansion: None });
        };
        let marks = match objects
            .arena
            .context_value(objects.phases.k(Field::DefeatMarks))
        {
            None => None,
            Some(ObjectValue::Object(id)) if objects.arena.kind(id) == ObjectKind::Dict => Some(id),
            _ => return Err(unsupported("attack marks require a dictionary")),
        };
        let mark = match objects
            .arena
            .context_value(objects.phases.k(Field::DefeatLog))
        {
            None => 0,
            Some(ObjectValue::Atom(Datum::Text(text))) => self.symbols.name(text).chars().count(),
            Some(ObjectValue::Object(id)) => {
                if let Some(values) = objects.arena.sequence(id) {
                    values.len()
                } else if objects.arena.kind(id) == ObjectKind::Dict {
                    objects
                        .arena
                        .fields(id)
                        .ok_or_else(|| unsupported("attack log dictionary missing"))?
                        .len()
                } else {
                    return Err(unsupported(
                        "attack log requires a sized sequence or dictionary",
                    ));
                }
            }
            _ => {
                return Err(unsupported(
                    "attack log requires a sized sequence or dictionary",
                ));
            }
        };
        let mark = i64::try_from(mark).map_err(|_| unsupported("attack log length outside i64"))?;
        // Source writes the dictionary before coercing bonuses. A bonus read of
        // that dictionary cannot be represented as an integer, even if absent
        // before expansion. Reject it before mutating the authoritative arena.
        if [spec.damage_bonus, spec.range_bonus]
            .contains(&Some(objects.phases.k(Field::DefeatMarks)))
        {
            return Err(unsupported("attack bonus reads the defeat-mark dictionary"));
        }
        self.sync_card_collections_bound(objects)?;
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty());
        let values = self
            .attack_rules
            .values(&self.world, &inputs, &self.symbols, actor, &spec)
            .map_err(unsupported)?;
        let source_card = self
            .track_attack_source
            .then(|| {
                self.attack_rules
                    .source_card(&self.world, &inputs, &self.symbols, actor)
            })
            .transpose()
            .map_err(unsupported)?;
        let filters = if select {
            self.unit_filter_list_bound(objects, step, Field::TargetFilters)?
        } else {
            Vec::new()
        };
        // Imported filters are referenced, never reconstructed or deep-copied.
        if filters.iter().any(|v| !matches!(v, ObjectValue::Object(_))) {
            return Err(unsupported("attack target filter model missing"));
        }
        self.phase_generation_check(0, if select { 11 } else { 10 })?;
        Ok(AttackPlan {
            expansion: Some(AttackExpansionPlan {
                key,
                select,
                marks,
                mark,
                values,
                ranged: spec.ranged,
                initiative: spec.initiative,
                source_card,
                filters,
            }),
        })
    }
    pub(super) fn attack_sequence_bound(
        &mut self,
        objects: &mut BoundObjects,
        original: ObjectId,
        plan: AttackPlan,
        mandatory: bool,
    ) -> Result<()> {
        let Some(plan) = plan.expansion else {
            if mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(());
        };
        let marks = match plan.marks {
            Some(id) => id,
            None => {
                let ObjectValue::Object(id) =
                    objects.arena.dictionary(Vec::new()).map_err(unsupported)?
                else {
                    unreachable!()
                };
                objects
                    .arena
                    .set_context(
                        objects.phases.k(Field::DefeatMarks),
                        ObjectValue::Object(id),
                    )
                    .map_err(unsupported)?;
                id
            }
        };
        objects
            .arena
            .set_field(marks, plan.key, ObjectValue::Atom(Datum::Int(plan.mark)))
            .map_err(unsupported)?;
        for (field, value) in [
            (Field::AttackRanged, Datum::Bool(plan.ranged)),
            (
                Field::Attacker,
                self.planning
                    .actor
                    .filter(|id| !self.symbols.name(*id).is_empty())
                    .map_or(Datum::Null, Datum::Text),
            ),
            (Field::AttackDamage, Datum::Int(plan.values.damage)),
            (Field::DefenseInitiative, Datum::Bool(plan.initiative)),
            (Field::AttackBasic, Datum::Bool(plan.values.basic)),
        ] {
            objects.write(objects.phases.k(field), value)?;
        }
        if let Some(source_card) = plan.source_card {
            objects.write(objects.phases.k(Field::AttackSourceCard), source_card)?;
        }
        let mut children = Vec::new();
        if plan.select {
            let select = self.phase_step(objects, "attack_select")?;
            objects
                .arena
                .set_field(
                    select,
                    objects.phases.k(Field::Output),
                    ObjectValue::Atom(Datum::Text(plan.key)),
                )
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(
                    select,
                    objects.phases.k(Field::Mandatory),
                    ObjectValue::Atom(Datum::Bool(mandatory)),
                )
                .map_err(unsupported)?;
            let ObjectValue::Object(filters) = objects
                .arena
                .field(select, objects.phases.k(Field::Filters))
                .ok_or_else(|| unsupported("attack selection filters missing"))?
            else {
                return Err(unsupported("attack selection filters missing"));
            };
            let range = objects
                .arena
                .classify_recipe_object(
                    select,
                    &[
                        ObjectSlot::Field(objects.phases.k(Field::Filters)),
                        ObjectSlot::Index(0),
                    ],
                    ObjectKind::Model,
                )
                .map_err(unsupported)?;
            objects
                .arena
                .classify_recipe_object(
                    select,
                    &[
                        ObjectSlot::Field(objects.phases.k(Field::Filters)),
                        ObjectSlot::Index(1),
                    ],
                    ObjectKind::Model,
                )
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(
                    range,
                    objects.phases.k(Field::MaxRange),
                    ObjectValue::Atom(Datum::Int(plan.values.range)),
                )
                .map_err(unsupported)?;
            for filter in plan.filters {
                objects.arena.append(filters, filter).map_err(unsupported)?;
            }
            children.push(select);
            if !mandatory {
                let copy = objects.arena.shallow_step(original).map_err(unsupported)?;
                let instruction = objects
                    .program(original)
                    .ok_or_else(|| unsupported("attack program missing"))?;
                objects.bind_copied_program(copy, instruction);
                for (field, value) in [
                    (Field::AttackTargetKey, Datum::Text(plan.key)),
                    (Field::AttackOutputKey, Datum::Null),
                    (Field::ActiveIf, Datum::Text(plan.key)),
                ] {
                    objects
                        .arena
                        .set_field(copy, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                self.push_live_steps_bound(objects, &[select, copy])?;
                return Ok(());
            }
        }
        for name in [
            "reaction",
            "set_actor",
            "passive",
            "pre_movement",
            "pre_discard",
            "set_actor",
            "defense_text",
            "resolve_combat",
            "on_block",
            "restore_action",
        ] {
            let child = self.phase_step(objects, name)?;
            let k = |field| objects.phases.k(field);
            let mut fields = Vec::new();
            match name {
                "reaction" => fields.push((k(Field::TargetPlayerKey), Datum::Text(plan.key))),
                "set_actor" => {
                    let defender = children.len() == usize::from(plan.select) + 1;
                    let actor_key = k(if defender {
                        Field::DefenderId
                    } else {
                        Field::PrePamActor
                    });
                    let save_key = k(if defender {
                        Field::PrePamActor
                    } else {
                        Field::DiscardPam
                    });
                    fields.extend([
                        (k(Field::ActorKey), Datum::Text(actor_key)),
                        (k(Field::SaveKey), Datum::Text(save_key)),
                    ]);
                }
                "pre_movement" | "pre_discard" => {
                    fields.extend([
                        (k(Field::HeroId), Datum::Null),
                        (k(Field::HeroKey), Datum::Text(k(Field::DefenderId))),
                        (k(Field::ActiveIf), Datum::Text(k(Field::PrimaryDefense))),
                    ]);
                    if name == "pre_discard" {
                        fields.push((k(Field::AbortOnDefeat), Datum::Bool(false)));
                    }
                }
                "resolve_combat" => fields.extend([
                    (k(Field::Damage), Datum::Int(plan.values.damage)),
                    (k(Field::TargetKey), Datum::Text(plan.key)),
                ]),
                _ => {}
            }
            for (key, value) in fields {
                objects
                    .arena
                    .set_field(child, key, ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
            children.push(child);
        }
        self.push_live_steps_bound(objects, &children)
    }
}
