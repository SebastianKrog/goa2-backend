//! Compiled unit predicates and ordered candidate pools, with live effect rules.
use crate::{
    data::{Datum, Symbol, Symbols},
    spatial::{HexFilter, SpatialContext, compile_filter},
    world::{EntityKind, MinionKind, NativeWorld},
};
use serde_json::Value;
#[derive(Clone, Copy, Debug)]
pub enum UnitCandidates {
    Units,
    UnitsWithIllusions,
    UnitsAndTokens,
}
#[derive(Clone, Debug)]
pub(crate) struct TargetingKeys {
    action: Symbol,
    attack: Symbol,
}
impl TargetingKeys {
    pub(crate) fn compile(s: &mut Symbols) -> Self {
        Self {
            action: s.intern("current_action_type"),
            attack: s.intern("ATTACK"),
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Relation {
    Friendly,
    Enemy,
    SelfOnly,
}
#[derive(Clone, Debug)]
pub enum UnitFilter {
    CountMatch(CandidateCount),
    Cards {
        container: Symbol,
        min: Option<i64>,
        max: Option<i64>,
        visible: bool,
    },
    HasTeam,
    PreviousSlotCard,
    Terrain(bool),
    BatchRemoval(Box<crate::batch::RemovalSearch>),
    Or(Vec<UnitFilter>),
    And(Vec<UnitFilter>),
    BehindEmpty(crate::spatial::Origin),
    Spatial(HexFilter),
    AdjacentContext(Symbol),
    EmptyNeighbor,
    Displacement(crate::validation::DisplacementKind),
    Team(Relation),
    Kind(EntityKind),
    Exclude {
        self_: bool,
        keys: Vec<Symbol>,
    },
    Immunity,
    Range(HexFilter),
    ContextIds(Symbol),
    Piece {
        exclude_acting: bool,
    },
    TokenType(Option<Symbol>),
    MinionTypes(Vec<MinionKind>),
    PlayedCard {
        action: Option<crate::world::ActionKind>,
        color: Option<Symbol>,
    },
}
#[derive(Clone, Debug)]
pub struct CandidateCount {
    pub filters: Vec<UnitFilter>,
    pub min: i64,
    pub max: Option<i64>,
    pub tokens: bool,
    pub origin: Symbol,
}
impl CandidateCount {
    pub(crate) fn accepts(
        &self,
        hex: crate::geometry::Hex,
        context: &SpatialContext<'_>,
    ) -> Result<bool, String> {
        if let Some(observer) = context.count_observer {
            observer.record();
        }
        struct Overlay<'a> {
            base: &'a dyn crate::data::RegisterRead,
            key: Symbol,
            value: Datum,
        }
        impl crate::data::RegisterRead for Overlay<'_> {
            fn get(&self, key: Symbol) -> Option<&Datum> {
                if key == self.key {
                    Some(&self.value)
                } else {
                    self.base.get(key)
                }
            }
            fn excludes_hex(&self, key: Symbol, hex: crate::geometry::Hex) -> Option<bool> {
                if key == self.key {
                    None
                } else {
                    self.base.excludes_hex(key, hex)
                }
            }
        }
        let (q, r, s) = hex.coordinates();
        let inputs = Overlay {
            base: context.registers,
            key: self.origin,
            value: Datum::Object(
                context
                    .coordinates
                    .into_iter()
                    .zip([q, r, s])
                    .map(|(k, v)| (k, Datum::Int(i64::from(v))))
                    .collect(),
            ),
        };
        let nested = SpatialContext {
            registers: &inputs,
            ..*context
        };
        let mut count = 0usize;
        for (id, _) in &context.world.locations {
            let Some(entity) = context.world.entity(*id) else {
                continue;
            };
            if !matches!(
                entity.kind,
                EntityKind::Hero | EntityKind::Minion | EntityKind::Piece
            ) && !(self.tokens && entity.kind == EntityKind::Token)
            {
                continue;
            }
            let mut accepted = true;
            for filter in &self.filters {
                if !filter.accepts(*id, &nested)? {
                    accepted = false;
                    break;
                }
            }
            count += usize::from(accepted);
        }
        let count = i64::try_from(count).map_err(|_| "candidate count outside i64")?;
        Ok(count >= self.min && self.max.is_none_or(|max| count <= max))
    }
}

impl NativeWorld {
    fn controller(&self, id: Symbol) -> Symbol {
        self.entity(id).and_then(|e| e.owner).unwrap_or(id)
    }
    pub fn ordinary_immune(&self, id: Symbol, actor: Option<Symbol>) -> bool {
        let Some(target) = self.entity(id) else {
            return false;
        };
        if actor.is_some_and(|a| self.controller(a) == self.controller(id)) {
            return false;
        }
        if target.minion_kind != Some(MinionKind::Heavy) {
            return false;
        }
        let Some(team) = target.team else {
            return false;
        };
        let zone = target
            .lane
            .and_then(|lane| self.battle_zones.iter().find(|(l, _)| *l == lane))
            .and_then(|(_, zone)| self.definitions.zones.zones.iter().find(|z| z.id == *zone));
        let Some(zone) = zone else {
            return false;
        };
        self.definitions.entities.iter().any(|m| {
            m.kind == EntityKind::Minion
                && m.roster == Some(team)
                && m.id != id
                && m.lane == target.lane
                && self.position(m.id).is_some_and(|h| zone.hexes.contains(&h))
        })
    }
    pub fn immune_filter(&self, id: Symbol, context: &SpatialContext<'_>) -> Result<bool, String> {
        let actor = context
            .actor
            .filter(|id| *id != self.definitions.effect_empty);
        let Some(target) = self.entity(id) else {
            return Ok(false);
        };
        if target.kind == EntityKind::Token && target.immune_enemy_actions {
            let actor_team = actor.and_then(|a| self.entity(a)).and_then(|a| a.team);
            let owner_team = target
                .owner
                .and_then(|h| self.hero(h))
                .and_then(|h| self.entity(h.id))
                .and_then(|e| e.team);
            if actor_team.is_some() && owner_team.is_some() && actor_team != owner_team {
                return Ok(false);
            }
        }
        if matches!(
            target.kind,
            EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
        ) {
            let immune = if let Some(actor) = actor {
                self.actor_immune(id, actor)?
            } else {
                self.ordinary_immune(id, None)
            };
            if immune {
                return Ok(false);
            }
        }
        if context.registers.get(self.definitions.targeting.action)
            == Some(&Datum::Text(self.definitions.targeting.attack))
        {
            let basic = match actor.filter(|id| self.hero(*id).is_some()) {
                Some(actor) => self
                    .performing_card(actor, context.registers)?
                    .and_then(|card| self.card(card))
                    .is_some_and(|card| self.definitions.cards[card.definition].basic),
                None => false,
            };
            if self.attack_immune(id, actor, basic) {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub fn select_units(
        &self,
        pool: UnitCandidates,
        target_actor: Option<Symbol>,
        filters: &[UnitFilter],
        context: &SpatialContext<'_>,
    ) -> Result<Vec<Symbol>, String> {
        let actual_unit = |id| {
            self.entity(id).is_some_and(|e| {
                matches!(
                    e.kind,
                    EntityKind::Hero | EntityKind::Minion | EntityKind::Piece
                )
            })
        };
        let mut candidates = self
            .locations
            .iter()
            .filter_map(|(id, _)| {
                (actual_unit(*id)
                    || matches!(pool, UnitCandidates::UnitsAndTokens)
                        && self
                            .entity(*id)
                            .is_some_and(|e| e.kind == EntityKind::Token))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        if matches!(pool, UnitCandidates::UnitsWithIllusions) {
            candidates.extend(
                self.locations.iter().filter_map(|(id, _)| {
                    self.equivalent_illusion(*id, context.actor).then_some(*id)
                }),
            );
        }
        let mut visible = Vec::with_capacity(candidates.len());
        for id in candidates {
            if actual_unit(id)
                && target_actor.is_some_and(|actor| actor != self.definitions.effect_empty)
                && !self.can_target(target_actor, Some(id))?.allowed
            {
                continue;
            }
            visible.push(id);
        }
        let mut candidates = visible;
        for filter in filters {
            let mut next = Vec::new();
            for id in candidates {
                if filter.accepts(id, context)? {
                    next.push(id);
                }
            }
            candidates = next;
        }
        Ok(candidates)
    }
}
impl UnitFilter {
    pub(crate) fn accepts(&self, id: Symbol, context: &SpatialContext<'_>) -> Result<bool, String> {
        let world = context.world;
        let entity = world.entity(id);
        Ok(match self {
            Self::CountMatch(count) => world
                .position(id)
                .map(|hex| count.accepts(hex, context))
                .transpose()?
                .unwrap_or(false),
            Self::Cards {
                container,
                min,
                max,
                visible,
            } => {
                if let Some(hero) = world.hero(id) {
                    let visible_card =
                        |id| !*visible || world.card(id).is_some_and(|c| !c.facedown);
                    let count = match context.symbols.name(*container) {
                        "HAND" => Some(if *visible {
                            hero.hand.iter().filter(|id| visible_card(**id)).count()
                        } else {
                            hero.hand.len()
                        }),
                        "DISCARD" => Some(if *visible {
                            hero.discard.iter().filter(|id| visible_card(**id)).count()
                        } else {
                            hero.discard.len()
                        }),
                        "DECK" => Some(if *visible {
                            hero.deck.iter().filter(|id| visible_card(**id)).count()
                        } else {
                            hero.deck.len()
                        }),
                        "PLAYED" => Some(
                            hero.played
                                .iter()
                                .flatten()
                                .filter(|id| visible_card(**id))
                                .count(),
                        ),
                        _ => None,
                    };
                    if let Some(count) = count {
                        let count =
                            i64::try_from(count).map_err(|_| "card filter count outside i64")?;
                        min.is_none_or(|v| count >= v) && max.is_none_or(|v| count <= v)
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Self::Terrain(wanted) => *wanted,
            Self::HasTeam => entity.is_some_and(|e| e.has_team && e.team.is_some()),
            Self::PreviousSlotCard => world.hero(id).is_some_and(|hero| {
                world
                    .combat
                    .turn
                    .checked_sub(1)
                    .and_then(|turn| hero.card_in_turn_slot(turn))
                    .is_some()
            }),
            Self::BatchRemoval(search) => search.accepts(id, context)?,
            Self::And(filters) => {
                let mut found = true;
                for filter in filters {
                    if !filter.accepts(id, context)? {
                        found = false;
                        break;
                    }
                }
                found
            }
            Self::Or(filters) => {
                let mut found = false;
                for filter in filters {
                    if filter.accepts(id, context)? {
                        found = true;
                        break;
                    }
                }
                found
            }
            Self::BehindEmpty(origin) => {
                let from = context
                    .origin(origin, true)?
                    .and_then(|id| world.position(id));
                from.zip(world.position(id)).is_some_and(|(from, target)| {
                    let (a, b, c) = from.coordinates();
                    let (q, r, s) = target.coordinates();
                    crate::displacement::offset(
                        target,
                        (
                            i64::from(q) - i64::from(a),
                            i64::from(r) - i64::from(b),
                            i64::from(s) - i64::from(c),
                        ),
                        1,
                    )
                    .is_ok_and(|behind| world.on_map(behind) && !world.obstacle(behind))
                })
            }
            Self::Spatial(filter) => {
                if let Some(hex) = world.position(id) {
                    let mut accepted = [true];
                    filter.apply_scan(context, &[hex], &mut accepted)?;
                    accepted[0]
                } else {
                    false
                }
            }
            Self::AdjacentContext(key) => {
                HexFilter::AdjacentContext(*key).accept_unit(context, id)?
            }
            Self::EmptyNeighbor => world.position(id).is_some_and(|h| {
                h.neighbors()
                    .iter()
                    .any(|n| world.on_map(*n) && !world.obstacle(*n))
            }),
            Self::Displacement(kind) => {
                if let Some(actor) = context
                    .actor
                    .filter(|id| *id != world.definitions.effect_empty)
                {
                    world.can_displace(id, Some(actor), *kind, None)?.allowed
                } else {
                    true
                }
            }
            Self::Team(relation) => {
                let actor = context.actor.and_then(|a| world.entity(a));
                let team = entity.and_then(|e| e.team).or_else(|| {
                    world
                        .equivalent_illusion(id, context.actor)
                        .then(|| world.illusion_minion_team(context.actor))
                        .flatten()
                });
                if let Some(actor) = actor.filter(|a| a.team.is_some() && team.is_some()) {
                    match relation {
                        Relation::SelfOnly => actor.id == id,
                        Relation::Enemy => actor.team != team,
                        Relation::Friendly => {
                            actor.team == team
                                && actor.id != id
                                && !(world.acting_piece == Some(id)
                                    && entity.is_some_and(|e| e.owner == Some(actor.id)))
                        }
                    }
                } else {
                    false
                }
            }
            Self::Kind(kind) => {
                if *kind == EntityKind::Hero {
                    entity.is_some_and(|e| matches!(e.kind, EntityKind::Hero | EntityKind::Piece))
                } else {
                    entity.is_some_and(|e| e.kind == *kind)
                        || *kind == EntityKind::Minion
                            && world.equivalent_illusion(id, context.actor)
                }
            }
            Self::Exclude { self_, keys } => {
                let own = *self_
                    && (context.actor == Some(id)
                        || world.acting_piece == Some(id)
                            && entity.and_then(|e| e.owner) == context.actor
                            && context.actor.is_some());
                !own && !keys.iter().any(|key| match context.registers.get(*key) {
                    Some(Datum::Text(value)) => *value == id,
                    Some(Datum::List(items)) => items.contains(&Datum::Text(id)),
                    _ => false,
                })
            }
            Self::PlayedCard { action, color } => world.hero(id).is_some_and(|hero| {
                let resolved = hero.card_in_turn_slot(world.combat.turn);
                resolved.into_iter().chain(hero.current).any(|card| {
                    world.card(card).is_some_and(|card| {
                        let definition = &world.definitions.cards[card.definition];
                        action.is_none_or(|a| !card.facedown && definition.primary == Some(a))
                            && color.is_none_or(|c| !card.facedown && definition.color == Some(c))
                    })
                })
            }),
            Self::Immunity => world.immune_filter(id, context)?,
            Self::Range(filter) => filter.accept_unit(context, id)?,
            Self::ContextIds(key) => {
                if let Some(Datum::List(items)) = context.registers.get(*key) {
                    // Context ID lists are normally strings. Reject aggregate/string
                    // coercion rather than silently choosing a different rule branch.
                    if items.iter().any(|v| !matches!(v, Datum::Text(_))) {
                        return Err("non-string context ID coercion not compiled".into());
                    }
                    items.contains(&Datum::Text(id))
                } else {
                    false
                }
            }
            Self::Piece { exclude_acting } => {
                entity.is_some_and(|e| e.kind == EntityKind::Piece)
                    && context
                        .actor
                        .and_then(|a| world.hero(a))
                        .is_some_and(|a| entity.is_some_and(|e| e.owner == Some(a.id)))
                    && (!exclude_acting || world.acting_piece != Some(id))
            }
            Self::TokenType(kind) => entity.is_some_and(|e| {
                e.kind == EntityKind::Token && kind.is_none_or(|k| e.token_type == Some(k))
            }),
            Self::MinionTypes(kinds) => entity.and_then(|e| e.minion_kind).map_or_else(
                || {
                    kinds.contains(&MinionKind::Melee)
                        && world.equivalent_illusion(id, context.actor)
                },
                |kind| kinds.contains(&kind),
            ),
        })
    }
}
pub fn compile_unit_filter(raw: &Value, symbols: &mut Symbols) -> Result<UnitFilter, String> {
    let keys = |name: &str, symbols: &mut Symbols| -> Result<Vec<Symbol>, String> {
        raw[name]
            .as_array()
            .ok_or_else(|| format!("missing filter keys {name}"))?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| symbols.intern(s))
                    .ok_or("non-string filter key".into())
            })
            .collect()
    };
    Ok(match raw["type"].as_str().ok_or("missing filter type")? {
        "and_filter" => UnitFilter::And(
            raw["filters"]
                .as_array()
                .ok_or("AND filters missing")?
                .iter()
                .map(|f| compile_unit_filter(f, symbols))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        "or_filter" => UnitFilter::Or(
            raw["filters"]
                .as_array()
                .ok_or("OR filters missing")?
                .iter()
                .map(|f| compile_unit_filter(f, symbols))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        "space_behind_empty_filter" => UnitFilter::BehindEmpty(crate::spatial::Origin {
            literal: raw["origin_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| symbols.intern(s)),
            key: raw["origin_key"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| symbols.intern(s)),
            frame: false,
        }),
        "adjacency_filter"
        | "line_behind_target_filter"
        | "between_hexes_filter"
        | "relative_distance_filter"
        | "in_straight_line_filter"
        | "not_in_straight_line_filter" => UnitFilter::Spatial(compile_filter(raw, symbols)?),
        "adjacency_to_context_filter" => UnitFilter::AdjacentContext(
            symbols.intern(
                raw["target_key"]
                    .as_str()
                    .ok_or("adjacency context key missing")?,
            ),
        ),
        "has_empty_neighbor_filter" => UnitFilter::EmptyNeighbor,
        "can_be_moved_filter" => {
            UnitFilter::Displacement(crate::validation::DisplacementKind::Move)
        }
        "count_match_filter" => UnitFilter::CountMatch(CandidateCount {
            filters: raw["sub_filters"]
                .as_array()
                .ok_or("count subfilters missing")?
                .iter()
                .map(|f| compile_unit_filter(f, symbols))
                .collect::<Result<Vec<_>, _>>()?,
            min: raw["min_count"].as_i64().unwrap_or(1),
            max: raw["max_count"].as_i64(),
            tokens: raw["include_tokens"].as_bool().unwrap_or(false),
            origin: symbols.intern("_cmf_origin_hex"),
        }),
        "cards_in_container_filter" => UnitFilter::Cards {
            container: symbols.intern(raw["container"].as_str().unwrap_or("HAND")),
            min: raw["min_cards"].as_i64(),
            max: raw["max_cards"].as_i64(),
            visible: raw["exclude_facedown"].as_bool().unwrap_or(false),
        },
        "can_be_placed_filter" => {
            UnitFilter::Displacement(crate::validation::DisplacementKind::Place)
        }
        "team_filter" => UnitFilter::Team(match raw["relation"].as_str() {
            Some("FRIENDLY") => Relation::Friendly,
            Some("ENEMY") => Relation::Enemy,
            Some("SELF") => Relation::SelfOnly,
            _ => return Err("invalid team relation".into()),
        }),
        "unit_type_filter" => UnitFilter::Kind(match raw["unit_type"].as_str() {
            Some("HERO") => EntityKind::Hero,
            Some("MINION") => EntityKind::Minion,
            Some("TOKEN") => EntityKind::Token,
            _ => return Err("invalid unit type".into()),
        }),
        "exclude_identity_filter" => UnitFilter::Exclude {
            self_: raw["exclude_self"].as_bool().unwrap_or(true),
            keys: keys("exclude_keys", symbols)?,
        },
        "terrain_filter" => UnitFilter::Terrain(raw["is_terrain"].as_bool().unwrap_or(true)),
        "has_previous_slot_card_filter" => UnitFilter::PreviousSlotCard,
        "played_card_filter" => {
            let action = raw
                .get("action_type")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                .transpose()?;
            let color = match raw.get("card_color") {
                None | Some(Value::Null) => None,
                Some(Value::String(color)) => {
                    crate::card_selection::validate_color(color)?;
                    Some(symbols.intern(color))
                }
                _ => return Err("invalid played-card color".into()),
            };
            UnitFilter::PlayedCard { action, color }
        }
        "immunity_filter" => UnitFilter::Immunity,
        "range_filter" => UnitFilter::Range(compile_filter(raw, symbols)?),
        "context_ids_filter" => UnitFilter::ContextIds(
            symbols.intern(raw["ids_key"].as_str().ok_or("IDs key missing")?),
        ),
        "hero_piece_filter" => UnitFilter::Piece {
            exclude_acting: raw["exclude_acting"].as_bool().unwrap_or(true),
        },
        "token_type_filter" => {
            UnitFilter::TokenType(raw["token_type"].as_str().map(|s| symbols.intern(s)))
        }
        "minion_type_filter" => UnitFilter::MinionTypes(
            raw["minion_types"]
                .as_array()
                .ok_or("minion types missing")?
                .iter()
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        _ => return Err("unit filter closure not compiled".into()),
    })
}

/// Ordered implicit filters shared by selection, counting and collection.
pub fn compile_effective_unit_filters(
    raw: &Value,
    symbols: &mut Symbols,
    default_skip_immunity: bool,
) -> Result<Vec<UnitFilter>, String> {
    let flag = |name: &str, default: bool| -> Result<bool, String> {
        match raw.get(name) {
            None => Ok(default),
            Some(v) => v
                .as_bool()
                .ok_or_else(|| format!("invalid unit filter flag: {name}")),
        }
    };
    let mut filters = raw["filters"]
        .as_array()
        .ok_or("unit filters missing")?
        .iter()
        .map(|f| compile_unit_filter(f, symbols))
        .collect::<Result<Vec<_>, _>>()?;
    if !flag("skip_self_filter", false)?
        && !filters
            .iter()
            .any(|f| matches!(f, UnitFilter::Exclude { self_: true, .. }))
    {
        filters.push(UnitFilter::Exclude {
            self_: true,
            keys: Vec::new(),
        });
    }
    if !flag("skip_immunity_filter", default_skip_immunity)?
        && !filters.iter().any(|f| matches!(f, UnitFilter::Immunity))
    {
        filters.push(UnitFilter::Immunity);
    }
    Ok(filters)
}
