//! CreateEffectStep bindings. All model/JSON work occurs during compilation.
use crate::{
    data::{Datum, RegisterRead, Symbol, Symbols},
    effects::{CreationContext, EffectCreateSpec},
    world::NativeWorld,
};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Debug)]
pub struct EffectStepSpec {
    base: EffectCreateSpec,
    use_card: bool,
    token: bool,
    except_key: Option<Symbol>,
    origin_key: Option<Symbol>,
    color_key: Option<Symbol>,
    current_card: Symbol,
    current_action: Symbol,
    override_key: Symbol,
    substitution: Symbol,
    system: Symbol,
    colors: Vec<Symbol>,
    actions: Vec<Symbol>,
    body_keys: [Symbol; 6],
    split: Option<Symbol>,
    split_value: i64,
}
#[derive(Clone, Debug)]
pub struct EffectCreated {
    pub actor: Symbol,
    pub kind: Symbol,
    pub duration: Symbol,
    pub split: Option<Symbol>,
    pub split_value: i64,
    pub color: Option<Symbol>,
}
fn defaults() -> Result<&'static Value> {
    static DATA: OnceLock<Result<Value>> = OnceLock::new();
    DATA.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!(
            "../../../content/catalogs/effect-defaults.json"
        ))
        .map_err(|e| e.to_string())?;
        let manifest = crate::codec::oracle_manifest()?;
        if data["schema_version"] != 1 || data["source_sha256"] != manifest["source_sha256"] {
            return Err("effect defaults identity mismatch".into());
        }
        Ok(data)
    })
    .as_ref()
    .map_err(Clone::clone)
}
/// Expand the Python ActiveEffect wire defaults without applying gameplay
/// creation semantics (automatic ID, source, round or activation changes).
pub(crate) fn table_effect_wire(raw: &Value) -> Result<Value> {
    fn expand(defaults: &Value, raw: &Value) -> Result<serde_json::Map<String, Value>> {
        let mut out = defaults
            .as_object()
            .ok_or("effect defaults missing")?
            .clone();
        for (key, value) in raw.as_object().ok_or("effect payload must be an object")? {
            let Some(default) = out.get_mut(key) else {
                return Err(format!("unknown effect field {key}"));
            };
            let valid = match default {
                Value::Bool(_) => value.is_boolean(),
                Value::Number(_) => value.as_i64().is_some(),
                Value::String(_) => value.is_string(),
                Value::Array(_) => value.is_array(),
                Value::Object(_) => value.is_object(),
                Value::Null => {
                    value.is_null()
                        || match key.as_str() {
                            "origin_hex" | "isolated_hex" => {
                                serde_json::from_value::<crate::geometry::Hex>(value.clone())
                                    .is_ok()
                            }
                            "max_value"
                            | "apply_stat_value_only_if_result_at_least"
                            | "direction" => value.as_i64().is_some(),
                            "origin_attack_is_basic" => value.is_boolean(),
                            _ => value.is_string(),
                        }
                }
            };
            if !valid {
                return Err(format!("invalid effect field {key}"));
            }
            *default = value.clone();
        }
        Ok(out)
    }

    let catalog = defaults()?;
    if [
        "id",
        "source_id",
        "effect_type",
        "scope",
        "duration",
        "created_at_turn",
        "created_at_round",
    ]
    .iter()
    .any(|key| raw.get(*key).is_none())
        || raw["scope"].get("shape").is_none()
    {
        return Err("effect identity, scope, duration and creation time are required".into());
    }
    if raw["id"].as_str().is_none_or(str::is_empty)
        || !catalog["effect_types"]
            .as_array()
            .is_some_and(|kinds| kinds.contains(&raw["effect_type"]))
    {
        return Err("invalid effect ID or type".into());
    }
    let mut fields = expand(&catalog["prototype"], raw)?;
    fields.insert(
        "scope".into(),
        Value::Object(expand(&catalog["prototype"]["scope"], &raw["scope"])?),
    );
    let effect = Value::Object(fields);
    for (name, choices) in [
        ("named_color", &catalog["colors"]),
        ("origin_action_type", &catalog["actions"]),
        (
            "marker_type",
            &serde_json::json!(["venom", "poison", "bounty"]),
        ),
    ] {
        if !effect[name].is_null()
            && !choices
                .as_array()
                .is_some_and(|items| items.contains(&effect[name]))
        {
            return Err(format!("invalid effect {name}"));
        }
    }
    for name in ["except_card_colors", "allowed_discard_colors"] {
        if effect[name]
            .as_array()
            .unwrap()
            .iter()
            .any(|color| !catalog["colors"].as_array().unwrap().contains(color))
        {
            return Err(format!("invalid effect {name}"));
        }
    }
    Ok(effect)
}

fn optional(value: &Value, key: &str, s: &mut Symbols) -> Result<Option<Symbol>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(s.intern(v))),
        _ => Err(format!("invalid effect {key}")),
    }
}
fn flag(value: &Value, key: &str, fallback: bool) -> Result<bool> {
    value.get(key).map_or(Ok(fallback), |v| {
        v.as_bool().ok_or(format!("invalid effect flag {key}"))
    })
}
fn field(fields: &[(Symbol, Datum)], key: Symbol) -> Option<&Datum> {
    fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}
fn put(fields: &mut Vec<(Symbol, Datum)>, key: Symbol, value: Datum) {
    if let Some((_, v)) = fields.iter_mut().find(|(k, _)| *k == key) {
        *v = value;
    } else {
        fields.push((key, value));
    }
}
fn context_text(value: Option<&Datum>, s: &Symbols) -> Result<Option<Symbol>> {
    match value.filter(|v| v.truthy(s)) {
        None => Ok(None),
        Some(Datum::Text(v)) => Ok(Some(*v)),
        _ => Err("non-string effect identity requires source model coercion".into()),
    }
}
type LiveEffectCatalog = (Datum, Vec<Symbol>, Vec<Symbol>, Vec<Symbol>);
pub(crate) fn live_catalog(s: &mut Symbols) -> Result<LiveEffectCatalog> {
    let data = defaults()?;
    let prototype = Datum::from_json(&data["prototype"], s)?;
    let mut ids = |name: &str| {
        data[name]
            .as_array()
            .ok_or("invalid effect catalog".to_owned())?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|name| s.intern(name))
                    .ok_or("invalid effect catalog ID".to_owned())
            })
            .collect::<Result<Vec<_>>>()
    };
    let kinds = ids("effect_types")?;
    let actions = ids("actions")?;
    let colors = ids("colors")?;
    Ok((prototype, kinds, actions, colors))
}
impl EffectStepSpec {
    pub fn compile(raw: &Value, s: &mut Symbols) -> Result<Self> {
        let catalog = defaults()?;
        let mut value = catalog["prototype"].clone();
        let fields = value.as_object_mut().unwrap();
        let source = raw.as_object().ok_or("effect step must be an object")?;
        for (key, target) in fields.iter_mut() {
            if !matches!(
                key.as_str(),
                "id" | "source_id"
                    | "created_at_turn"
                    | "created_at_round"
                    | "token_id"
                    | "subject_id"
                    | "named_color"
                    | "origin_attack_is_basic"
            ) && let Some(v) = source.get(key)
            {
                *target = v.clone();
            }
        }
        let kind = raw["effect_type"].as_str().ok_or("effect type missing")?;
        if !catalog["effect_types"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == kind)
        {
            return Err("unknown effect type".into());
        }
        let scope = raw["scope"].as_object().ok_or("effect scope missing")?;
        let mut expanded = catalog["prototype"]["scope"].as_object().unwrap().clone();
        for (k, v) in scope {
            expanded.insert(k.clone(), v.clone());
        }
        fields.insert("scope".into(), Value::Object(expanded));
        let colors = catalog["colors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| s.intern(v.as_str().unwrap()))
            .collect();
        let actions = catalog["actions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| s.intern(v.as_str().unwrap()))
            .collect();
        let mut key = |name: &str| -> Result<Option<Symbol>> {
            Ok(optional(raw, name, s)?.filter(|id| !s.name(*id).is_empty()))
        };
        let except_key = key("except_attacker_key")?;
        let origin_key = key("origin_id_key")?;
        let color_key = key("named_color_key")?;
        Ok(Self {
            base: EffectCreateSpec::compile(&value, s)?,
            use_card: flag(raw, "use_context_card", true)?,
            token: flag(raw, "is_token_effect", false)?,
            except_key,
            origin_key,
            color_key,
            current_card: s.intern("current_card_id"),
            current_action: s.intern("current_action_type"),
            override_key: s.intern("token_type_override"),
            substitution: s.intern("substitution_actor_id"),
            system: s.intern("system"),
            colors,
            actions,
            body_keys: [
                "except_attacker_ids",
                "named_color",
                "finishing_steps",
                "color_key",
                "color",
                "steps_template",
            ]
            .map(|v| s.intern(v)),
            split: optional(raw, "split_axis", s)?,
            split_value: raw
                .get("split_value")
                .map_or(Ok(0), |v| v.as_i64().ok_or("invalid effect split value"))?,
        })
    }
    pub fn bind_primary(&mut self, mut bind: impl FnMut(&mut Datum) -> Result<()>) -> Result<()> {
        let def = Arc::make_mut(&mut self.base.template.definition);
        def.behavior.bind_primary(&mut bind)?;
        for (_, v) in &mut def.fields {
            bind(v)?;
        }
        for (_, v) in &mut def.scope {
            bind(v)?;
        }
        for v in &mut def.finishing {
            bind(v)?;
        }
        for id in [
            &mut self.base.template.card,
            &mut self.base.template.scope_origin,
            &mut self.base.template.barrier_origin,
        ]
        .into_iter()
        .flatten()
        {
            let mut v = Datum::Text(*id);
            bind(&mut v)?;
            if let Datum::Text(bound) = v {
                *id = bound;
            } else {
                return Err("effect identity binding changed type".into());
            }
        }
        let mut split = Datum::Int(self.split_value);
        bind(&mut split)?;
        self.split_value = match split {
            Datum::Int(v) => v,
            _ => return Err("invalid bound split value".into()),
        };
        Ok(())
    }
    pub fn execute(
        &self,
        world: &mut NativeWorld,
        context: &CreationContext<'_>,
        s: &mut Symbols,
    ) -> Result<Option<EffectCreated>> {
        let ctx = context.registers;
        if self.token
            && ctx.get(self.override_key).is_some_and(|v| v.truthy(s))
            && context.actor.is_some_and(|a| {
                ctx.get(self.substitution) == Some(&Datum::Text(world.hero(a).map_or(a, |h| h.id)))
            })
        {
            return Ok(None);
        }
        let mut bound = self.base.clone();
        let e = &mut bound.template;
        e.source = context
            .actor
            .filter(|id| !s.name(*id).is_empty())
            .unwrap_or(self.system);
        e.card = if self.token {
            None
        } else if let Some(card) = e.card {
            Some(card)
        } else if self.use_card {
            context_text(ctx.get(self.current_card), s)?
        } else {
            None
        };
        e.linked_card = e.card.filter(|id| !s.name(*id).is_empty());
        if e.origin_action.is_none() {
            e.origin_action = context_text(ctx.get(self.current_action), s)?;
        }
        if e.origin_action
            .is_some_and(|id| !self.actions.contains(&id))
        {
            return Err("invalid effect origin action".into());
        }
        if let Some(key) = self.origin_key
            && let Some(value) = ctx.get(key).filter(|v| v.truthy(s))
        {
            e.scope_origin = Some(match value {
                Datum::Text(v) => *v,
                Datum::Int(v) => s.intern(&v.to_string()),
                Datum::Bool(v) => s.intern(if *v { "True" } else { "False" }),
                _ => return Err("effect origin Python string conversion not compiled".into()),
            });
        }
        e.token = if self.token {
            e.scope_origin.filter(|id| !s.name(*id).is_empty())
        } else {
            None
        };
        let color = self
            .color_key
            .and_then(|key| ctx.get(key))
            .filter(|v| v.truthy(s));
        let color = match color {
            None => None,
            Some(Datum::Text(id)) if self.colors.contains(id) => Some(*id),
            _ => return Err("invalid named effect color".into()),
        };
        let mut extra = None;
        if let Some(key) = self.except_key {
            extra = context_text(ctx.get(key), s)?;
        }
        let finishing = self.resolve_finishing(&e.definition.finishing, ctx, s)?;
        if color.is_some() || extra.is_some() || finishing != e.definition.finishing {
            let def = Arc::make_mut(&mut e.definition);
            if let Some(color) = color {
                put(&mut def.fields, self.body_keys[1], Datum::Text(color));
            }
            if let Some(extra) = extra {
                let mut values = match field(&def.fields, self.body_keys[0]) {
                    Some(Datum::List(v)) => v.clone(),
                    _ => return Err("invalid effect exceptions".into()),
                };
                if !values.contains(&Datum::Text(extra)) {
                    values.push(Datum::Text(extra));
                }
                if !def.behavior.except_attackers.contains(&extra) {
                    def.behavior.except_attackers.push(extra);
                }
                put(&mut def.fields, self.body_keys[0], Datum::List(values));
            }
            def.finishing = finishing;
            put(
                &mut def.fields,
                self.body_keys[2],
                Datum::List(def.finishing.clone()),
            );
        }
        let old_ids = world
            .effects
            .instances
            .iter()
            .map(|e| e.id)
            .collect::<Vec<_>>();
        let index = world.create_effect(&bound, context, s)?;
        let effect = &world.effects.instances[index];
        if old_ids.contains(&effect.id) {
            return Ok(None);
        }
        let duration = field(&effect.definition.fields, s.lookup("duration").unwrap())
            .and_then(|d| {
                if let Datum::Text(id) = d {
                    Some(*id)
                } else {
                    None
                }
            })
            .ok_or("effect duration missing")?;
        Ok(Some(EffectCreated {
            actor: effect.source,
            kind: effect.definition.kind,
            duration,
            split: self.split,
            split_value: self.split_value,
            color,
        }))
    }
    fn resolve_finishing(
        &self,
        steps: &[Datum],
        ctx: &dyn RegisterRead,
        s: &Symbols,
    ) -> Result<Vec<Datum>> {
        let mut result = steps.to_vec();
        for step in &mut result {
            let Datum::Object(fields) = step else {
                return Err("effect finishing step must be typed record".into());
            };
            if let Some(Datum::Text(key)) = field(fields, self.body_keys[3])
                && !s.name(*key).is_empty()
                && field(fields, self.body_keys[4]).is_none_or(|v| matches!(v, Datum::Null))
                && let Some(value) = ctx.get(*key).filter(|v| v.truthy(s))
            {
                let Datum::Text(color) = value else {
                    return Err("invalid finishing color".into());
                };
                if !self.colors.contains(color) {
                    return Err("invalid finishing color".into());
                }
                if field(fields, self.body_keys[4]).is_some() {
                    put(fields, self.body_keys[4], Datum::Text(*color));
                }
            }
            for name in ["steps_template", "new_steps"] {
                if let Some(key) = s.lookup(name)
                    && let Some(Datum::List(children)) = field(fields, key)
                {
                    let resolved = self.resolve_finishing(children, ctx, s)?;
                    put(fields, key, Datum::List(resolved));
                }
            }
        }
        Ok(result)
    }
}
