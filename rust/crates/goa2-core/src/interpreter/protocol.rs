use super::NativeSession;
use crate::{
    actions::ActionMenu,
    combat::CombatEvent,
    data::{Datum, Symbol},
    geometry::Hex,
    markers::MarkerEvent,
    reactions::DefenseMenu,
    stats::Stat,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestToken {
    Imported(Symbol),
    Generated(u64),
}

#[derive(Clone, Debug, Hash, PartialEq)]
pub enum RequestChoices {
    SpellCards {
        values: Vec<crate::cards::CardId>,
        cast: bool,
        removed: usize,
        max: i64,
    },
    Minions(Vec<(Symbol, crate::world::MinionKind)>),
    Protection(Vec<crate::cards::CardId>),
    Mines(Vec<crate::mines::MinePath>),
    Numbers {
        values: Vec<i64>,
        labels: Vec<(i64, Symbol)>,
    },
    Units {
        tokens: bool,
        values: Vec<(Symbol, Option<(Symbol, Hex)>)>,
    },
    YesNo,
    Repeat {
        done: i64,
        max: i64,
    },
    Runes(Vec<Symbol>),
    CardColors(Vec<Symbol>),
    RuneInscription {
        hero: Symbol,
        placed: Vec<(Symbol, Symbol)>,
        remaining: Vec<Symbol>,
    },
    Passive {
        card: Symbol,
        name: Symbol,
    },
    ConfirmAction,
    Laugh,
    MirrorDistances(Vec<(i64, i64)>),
    Upgrades(Vec<crate::round::UpgradePlayer>),
    Actors {
        team: Symbol,
        values: Vec<Symbol>,
    },
    Respawn {
        hexes: Vec<Hex>,
        location: bool,
    },
    Cards(Vec<(Symbol, Symbol, i64)>),
    CardIds {
        values: Vec<Symbol>,
        forced_victim: Option<Symbol>,
        defeat_victim: Option<Symbol>,
    },
    Multiple {
        values: Vec<Symbol>,
        count: usize,
        max: i64,
    },
    PrimaryReplacement {
        actor: Symbol,
        card: Symbol,
        primary: crate::world::ActionKind,
        replacement: crate::world::ActionKind,
    },
    Action(ActionMenu),
    Defense(DefenseMenu),
    Hexes(Vec<Hex>),
    ZoneHexes {
        values: Vec<Hex>,
        unit: Option<Symbol>,
    },
}
#[derive(Clone, Debug, Hash, PartialEq)]
pub struct NativeRequest {
    pub id: RequestToken,
    pub owner: Symbol,
    /// Original prompt actor; ownership is remapped only when the request is issued.
    pub(super) controlled: Option<Symbol>,
    pub(super) prompt: Symbol,
    pub(super) movement_range: Option<i32>,
    pub(super) pre_action_range: Option<i32>,
    pub choices: RequestChoices,
    pub(super) can_skip: bool,
    pub(super) discard: bool,
}
#[derive(Clone, Debug)]
pub enum NativeEvent {
    AdditionalLifeSpent {
        team: Symbol,
        target: Symbol,
        total: i64,
        change: i64,
    },
    LifePenaltyAnnihilation {
        winner: Symbol,
    },
    DirectionalMove {
        unit: Symbol,
        from: Hex,
        to: Hex,
        direction: i64,
    },
    ItemGranted {
        owner: Symbol,
        stat: crate::stats::Stat,
        amount: i64,
    },
    GuessSelected {
        actor: Option<Symbol>,
        victim: Symbol,
        attempt: i64,
    },
    HeroLaughed(Symbol),
    HeroPieceRemoved {
        owner: Symbol,
        piece: Symbol,
        from: Option<Hex>,
    },
    HeroPieceSpawned {
        owner: Symbol,
        piece: Symbol,
        to: Hex,
    },
    ResolvedCardsSwapped {
        actor: Option<Symbol>,
        target: Symbol,
        a: Symbol,
        b: Symbol,
    },
    HandCardRevealed {
        actor: Option<Symbol>,
        target: Symbol,
        owner: Symbol,
        card: Symbol,
        name: Symbol,
        color: Option<Symbol>,
        tier: Symbol,
        tier_value: i64,
    },
    GuessRevealed {
        actor: Option<Symbol>,
        victim: Symbol,
        attempt: i64,
        card: Symbol,
        name: Symbol,
        actual: Option<Symbol>,
        guessed: Symbol,
        correct: bool,
    },

    DeckCardSwapped {
        actor: Symbol,
        outgoing: Symbol,
        incoming: Symbol,
        state: &'static str,
        facedown: bool,
    },
    RunesPlaced {
        hero: Symbol,
        placed: Vec<(Symbol, Symbol)>,
    },
    JourneyScheduled {
        actor: Symbol,
        target: Symbol,
        kind: Symbol,
    },
    ActionControlScheduled {
        actor: Symbol,
        target: Symbol,
        card: Symbol,
    },
    InitiativeOverride {
        actor: Option<Symbol>,
        target: Symbol,
        card: Symbol,
        value: i64,
    },
    MineTriggered {
        actor: Option<Symbol>,
        target: Symbol,
        from: Hex,
        kind: Option<Symbol>,
        blast: bool,
    },
    EffectCreated(crate::effect_steps::EffectCreated),
    GoldEffect {
        actor: Option<Symbol>,
        target: Option<Symbol>,
        owner: Symbol,
        amount: Datum,
        lost: bool,
    },
    Spell(crate::spells::SpellEvent),
    CoinsStolen {
        actor: Symbol,
        victim: Symbol,
        amount: Datum,
    },
    LifeRestored {
        hero: Symbol,
        team: Symbol,
        change: i64,
        remaining: i64,
    },
    Round(crate::round::RoundEvent),
    TurnEnded(Symbol),
    RespawnCleared {
        hero: Symbol,
        token: Symbol,
        from: Hex,
    },
    Pushed {
        actor: Option<Symbol>,
        target: Symbol,
        token: bool,
        from: Hex,
        to: Hex,
        distance: usize,
        collision: bool,
    },
    BoardEntityPlaced {
        actor: Option<Symbol>,
        entity: Symbol,
        from: Option<Hex>,
        to: Hex,
        obstacle: bool,
    },
    BoardEntityRemoved {
        actor: Option<Symbol>,
        entity: Symbol,
        from: Hex,
    },
    TokenPlaced {
        actor: Option<Symbol>,
        token: Symbol,
        to: Hex,
        kind: Symbol,
    },
    TokenReplaced {
        actor: Option<Symbol>,
        token: Symbol,
        from: Hex,
        to: Hex,
        kind: Symbol,
    },
    TokenWalked {
        actor: Option<Symbol>,
        token: Symbol,
        from: Hex,
        to: Hex,
        range: i64,
    },
    TokenMoved {
        actor: Option<Symbol>,
        token: Symbol,
        from: Option<Hex>,
        to: Hex,
    },
    Swapped {
        a: Symbol,
        b: Symbol,
        from: Hex,
        to: Hex,
    },
    TokenRemoved {
        actor: Option<Symbol>,
        token: Symbol,
        from: Hex,
        effects_removed: usize,
    },
    Marker(MarkerEvent),
    Combat(CombatEvent),
    UnitPlaced {
        unit: Symbol,
        from: Option<Hex>,
        to: Hex,
    },
    ItemGained {
        actor: Symbol,
        stat: Stat,
        card: Symbol,
    },
    CardRetrieved {
        hero: Symbol,
        card: Symbol,
        name: Symbol,
        kept: Option<Symbol>,
    },
    TieBreakerFlipped {
        blue: bool,
    },
    UnitMoved {
        unit: Symbol,
        from: Hex,
        to: Hex,
        range: i32,
    },
}
#[derive(Clone, Debug, Default)]
pub struct AdvanceResult {
    pub request: Option<NativeRequest>,
    pub events: Vec<NativeEvent>,
    pub steps: usize,
}

use serde_json::{Value, json};

impl NativeSession {
    pub fn request_json(&self, request: &NativeRequest) -> Value {
        let mut wire = self.request_json_base(request);
        if let Some(controlled) = request.controlled {
            wire["context"]["controlled_hero_id"] = json!(self.symbols.name(controlled));
            wire["player_id"] = json!(self.symbols.name(request.owner));
        }
        wire
    }
    fn request_json_base(&self, request: &NativeRequest) -> Value {
        let owner = request.controlled.unwrap_or(request.owner);
        if let RequestChoices::Minions(values) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),"options":values.iter().map(|(id,kind)|json!({"id":self.symbols.name(*id),"text":format!("{} Minion",match kind {crate::world::MinionKind::Melee=>"MELEE",crate::world::MinionKind::Ranged=>"RANGED",crate::world::MinionKind::Heavy=>"HEAVY"}),"metadata":{}})).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{},"decision_context":null});
        }
        if let RequestChoices::SpellCards {
            values,
            cast,
            removed,
            max,
        } = &request.choices
        {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_CARD","player_id":self.symbols.name(owner),
                "prompt":if *cast {"Choose a prepared spell to cast".to_owned()}else{format!("Find Familiar: remove up to three other prepared spells ({removed}/{max})")},
                "options":values.iter().map(|id|{let d=&self.world.definitions.cards[self.world.card(*id).unwrap().definition];json!({"id":self.symbols.name(d.printed_id),"text":self.symbols.name(d.name),"metadata":{}})}).collect::<Vec<_>>(),
                "can_skip":!*cast,"can_rollback":false,"context":{},"decision_context":if *cast {json!({"purpose":"cast_spell","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}})}else{Value::Null}});
        }
        if let RequestChoices::Protection(cards) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_CARD","player_id":self.symbols.name(owner),
                "prompt":format!("Discard a card to protect {}?",self.symbols.name(request.prompt)),
                "options":cards.iter().map(|id| {let d=&self.world.definitions.cards[self.world.card(*id).unwrap().definition]; json!({"id":self.symbols.name(d.printed_id),"text":self.symbols.name(d.name),"metadata":{"card_id":self.symbols.name(d.printed_id),"color":d.color.map(|id| self.symbols.name(id))}})}).collect::<Vec<_>>(),
                "can_skip":true,"can_rollback":false,"context":{},"decision_context":{"purpose":"discard","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}}});
        }
        if let RequestChoices::RuneInscription {
            hero,
            placed,
            remaining,
        } = &request.choices
        {
            let slot = placed.len() + 1;
            let count = remaining.len();
            let mut counts = vec![1, count - 1, count];
            counts.dedup();
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":format!("Place a rune below turn slot {slot}"),
                "options":remaining.iter().map(|id| {let name=self.symbols.name(*id);let mut label=name.to_owned();label[..1].make_ascii_uppercase();json!({"id":name,"text":label,"metadata":{}})}).collect::<Vec<_>>(),
                "can_skip":false,"can_rollback":false,"context":{"supports_batch":true,"next_slot":slot,"placed_runes":self.runes_json(placed),"max_selections":count,"selection_counts":counts},"decision_context":{"purpose":"rune_inscription","actor_id":self.symbols.name(*hero),"source_card_id":null,"choice_facts":{}}});
        }
        if let RequestChoices::CardColors(values) = &request.choices {
            let options = values
                .iter()
                .map(|id| {
                    let name = self.symbols.name(*id);
                    json!({"id":name,"text":name,"metadata":{}})
                })
                .collect::<Vec<_>>();
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION",
                "player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),
                "options":options,"can_skip":false,"can_rollback":false,"context":{},"decision_context":null});
        }
        if let RequestChoices::Runes(values) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),"options":values.iter().map(|id| {let name=self.symbols.name(*id);let label=match name {"axe"=>"Axe","bird"=>"Bird","anvil"=>"Anvil","horn"=>"Horn",_=>name};json!({"id":name,"text":label,"metadata":{}})}).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{},"decision_context":{"purpose":"rune_selection","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}}});
        }
        if let RequestChoices::Mines(options) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":"Choose which mines to move through","options":options.iter().enumerate().map(|(i,path)|json!({"id":i.to_string(),"text":format!("Path through {} mine(s)",path.mines.len()),"metadata":{"metadata":{"mine_count":path.mines.len(),"mine_hexes":path.mines.iter().filter_map(|id|self.world.position(*id)).collect::<Vec<_>>(),"path":path.path}}})).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{},"decision_context":null});
        }
        if let RequestChoices::PrimaryReplacement {
            actor,
            card,
            primary,
            replacement,
        } = &request.choices
        {
            let mut facts = serde_json::Map::new();
            let options = [
                (*primary, "Primary", "primary"),
                (*replacement, "Replacement", "replacement"),
            ]
            .into_iter()
            .map(|(kind, label, role)| {
                let name = kind.name();
                facts.insert(
                    name.into(),
                    json!({"action_role":role,"defense_stat":null,"defense_kind":null}),
                );
                json!({"id":name,"text":format!("{label}: {name}"),"metadata":{"type":name}})
            })
            .collect::<Vec<_>>();
            return json!({"id":self.request_token(request.id),"request_type":"CHOOSE_ACTION","player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),"options":options,"can_skip":false,"can_rollback":false,"context":{},"decision_context":{"purpose":"action","actor_id":self.symbols.name(*actor),"source_card_id":self.symbols.name(*card),"choice_facts":facts}});
        }
        if let RequestChoices::Action(menu) = &request.choices {
            return menu.to_json(self.request_token(request.id), &self.symbols);
        }
        if let RequestChoices::Defense(menu) = &request.choices {
            return menu.to_json(self.request_token(request.id), &self.symbols);
        }
        if let RequestChoices::Actors { team, values } = &request.choices {
            let names = values
                .iter()
                .map(|h| format!("'{}'", self.symbols.name(*h)))
                .collect::<Vec<_>>()
                .join(", ");
            return json!({"id":self.request_token(request.id),"request_type":"CHOOSE_ACTOR","player_id":self.symbols.name(owner),
                "prompt":format!("Team {}, choose who acts first between [{names}].",self.symbols.name(*team)),
                "options":values.iter().map(|h|json!({"id":self.symbols.name(*h),"text":self.symbols.name(*h),"metadata":{}})).collect::<Vec<_>>(),
                "can_skip":false,"can_rollback":false,"context":{"team":self.symbols.name(*team)},"decision_context":null});
        }
        if let RequestChoices::Respawn { hexes, location } = &request.choices {
            let options = if *location {
                hexes.iter().map(|h|{let(q,r,s)=h.coordinates();json!({"id":format!("hex_{q}_{r}_{s}"),"text":format!("({q},{r},{s})"),"metadata":{"hex":h,"raw":h}})}).collect::<Vec<_>>()
            } else {
                ["RESPAWN", "PASS"]
                    .iter()
                    .map(|id| json!({"id":id,"text":id,"metadata":{}}))
                    .collect()
            };
            return json!({"id":self.request_token(request.id),"request_type":if *location{"CHOOSE_RESPAWN_HEX"}else{"CHOOSE_RESPAWN"},"player_id":self.symbols.name(owner),
                "prompt":self.symbols.name(request.prompt),"options":options,"can_skip":false,"can_rollback":false,"context":{"valid_hexes":hexes},"decision_context":null});
        }
        if let RequestChoices::Upgrades(players) = &request.choices {
            let mut envelope = (*self.source).clone();
            self.world.export_cards(&mut envelope, &self.symbols);
            let players=Value::Object(players.iter().map(|player|{
                let options=player.options.iter().map(|option|json!({"color":self.symbols.name(option.color),"tier":self.symbols.name(option.tier),
                    "pair":option.pair.iter().map(|id|self.symbols.name(self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id)).collect::<Vec<_>>(),
                    "card_details":option.pair.iter().map(|id|envelope.expand(&crate::cards::wire(Some(*id))).expect("compiled card record")).collect::<Vec<_>>() })).collect::<Vec<_>>();
                let mut counts=vec![1,player.remaining];counts.sort();counts.dedup();
                (self.symbols.name(player.hero).into(),json!({"remaining":player.remaining,"options":options,"selection_counts":counts}))
            }).collect());
            return json!({"id":self.request_token(request.id),"request_type":"UPGRADE_PHASE","player_id":"simultaneous","prompt":"Mandatory Upgrade Phase",
                "options":[],"can_skip":false,"can_rollback":false,"context":{"players":players,"supports_batch":true,"supports_preview":true},"decision_context":null});
        }
        let (kind,options)=match &request.choices {
            RequestChoices::Minions(_) | RequestChoices::SpellCards { .. } | RequestChoices::PrimaryReplacement { .. } | RequestChoices::Runes(_) | RequestChoices::CardColors(_) | RequestChoices::RuneInscription { .. } | RequestChoices::Protection(_) | RequestChoices::Mines(_) | RequestChoices::Defense(_) | RequestChoices::Action(_)|RequestChoices::Actors{..}|RequestChoices::Respawn{..}|RequestChoices::Upgrades(_)=>unreachable!(),
            RequestChoices::ConfirmAction=>("CHOOSE_ACTION",vec![json!({"id":"CONFIRM","text":"Confirm","metadata":{}})]),
            RequestChoices::Multiple{values,..}=>("SELECT_UNIT",values.iter().map(|id|json!({"id":self.symbols.name(*id),"text":self.symbols.name(*id),"metadata":{}})).collect()),
            RequestChoices::Units {tokens,values}=>(if *tokens {"SELECT_UNIT_OR_TOKEN"}else{"SELECT_UNIT"},values.iter().map(|(id,label)|{
                let text=label.map_or_else(||if *id == self.skip { "Keep all pieces".to_owned() } else { self.symbols.name(*id).to_owned() },|(name,hex)|{let(q,r,s)=hex.coordinates();format!("{} at ({q}, {r}, {s})",self.symbols.name(name))});
                json!({"id":self.symbols.name(*id),"text":text,"metadata":{}})
            }).collect()),
            RequestChoices::Hexes(values)|RequestChoices::ZoneHexes{values,..}=>("SELECT_HEX",values.iter().map(|hex|{
                let (q,r,s)=hex.coordinates();json!({"id":format!("hex_{q}_{r}_{s}"),"text":format!("({q},{r},{s})"),"metadata":{"hex":hex,"raw":hex}})
            }).collect()),
            RequestChoices::CardIds{values,..}=>("SELECT_CARD",values.iter().map(|id|json!({"id":self.symbols.name(*id),"text":self.symbols.name(*id),"metadata":{}})).collect()),
            RequestChoices::Cards(values)=>("SELECT_CARD",values.iter().map(|(id,name,initiative)|json!({"id":self.symbols.name(*id),"text":format!("{} (initiative {initiative})",self.symbols.name(*name)),"metadata":{}})).collect()),
            RequestChoices::Passive{..} | RequestChoices::Laugh=>("CONFIRM_PASSIVE",vec![json!({"id":"YES","text":"Yes","metadata":{}}),json!({"id":"NO","text":"No","metadata":{}})]),
            RequestChoices::Repeat{..} | RequestChoices::YesNo=>("SELECT_OPTION",vec![json!({"id":"YES","text":"Yes","metadata":{}}),json!({"id":"NO","text":"No","metadata":{}})]),
            RequestChoices::MirrorDistances(values)=>("SELECT_NUMBER", values.iter().map(|(distance, actual)|
                json!({"id":distance.to_string(),"text":distance.to_string(),"metadata":{"actual_moved":actual}})
            ).collect()),
            RequestChoices::Numbers {values,labels}=>("SELECT_NUMBER",values.iter().map(|n|{
                let label=labels.iter().find(|(number,_)|number==n).map(|(_,id)|self.symbols.name(*id).to_owned()).unwrap_or_else(||n.to_string());
                json!({"id":n.to_string(),"text":label,"metadata":if labels.is_empty(){json!({})}else{json!({"raw":n})}})
            }).collect()),
        };
        let prompt = if let Some(range) = request.pre_action_range {
            format!(
                "Pre-action movement (up to {range} space{})",
                if range > 1 { "s" } else { "" }
            )
        } else if let RequestChoices::Repeat { done, max } = &request.choices {
            format!("{} ({done}/{max} done)", self.symbols.name(request.prompt))
        } else if let RequestChoices::Multiple { count, max, .. } = &request.choices {
            format!("{} ({count}/{max})", self.symbols.name(request.prompt))
        } else if let RequestChoices::CardIds {
            defeat_victim: Some(v),
            ..
        } = &request.choices
        {
            format!(
                "{}, select a card to discard (or be Defeated).",
                self.symbols.name(*v)
            )
        } else if let RequestChoices::CardIds {
            forced_victim: Some(v),
            ..
        } = &request.choices
        {
            format!("{}, select a card to discard.", self.symbols.name(*v))
        } else {
            request.movement_range.map_or_else(
                || self.symbols.name(request.prompt).to_owned(),
                |range| format!("Select Movement Destination (Range {range})"),
            )
        };
        json!({"id":self.request_token(request.id),"request_type":kind,"player_id":self.symbols.name(owner),
            "prompt":prompt,"options":options,"can_skip":request.can_skip,"can_rollback":false,"context":match &request.choices {RequestChoices::Passive{card,name}=>json!({"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name)}),RequestChoices::ZoneHexes{unit:Some(id),..}=>json!({"context_unit_id":self.symbols.name(*id)}),_=>json!({})},
            "decision_context":if request.discard {json!({"purpose":"discard","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}})}else{Value::Null}})
    }
    fn runes_json(&self, placed: &[(Symbol, Symbol)]) -> Value {
        Value::Object(
            placed
                .iter()
                .map(|(slot, rune)| {
                    (
                        self.symbols.name(*slot).into(),
                        json!(self.symbols.name(*rune)),
                    )
                })
                .collect(),
        )
    }
    pub fn event_json(&self, event: &NativeEvent) -> Value {
        match event {
            NativeEvent::RunesPlaced { hero, placed } => {
                json!({"event_type":"RUNES_PLACED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"rune_slots":self.runes_json(placed)}})
            }
            NativeEvent::JourneyScheduled {
                actor,
                target,
                kind,
            } => {
                json!({"event_type":"EFFECT_CREATED","actor_id":self.symbols.name(*actor),"target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"effect_type":self.symbols.name(*kind)}})
            }
            NativeEvent::ActionControlScheduled {
                actor,
                target,
                card,
            } => {
                json!({"event_type":"EFFECT_CREATED","actor_id":self.symbols.name(*actor),"target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"effect":"action_control","controller_id":self.symbols.name(*actor),"card_id":self.symbols.name(*card)}})
            }
            NativeEvent::InitiativeOverride {
                actor,
                target,
                card,
                value,
            } => {
                json!({"event_type":"EFFECT_CREATED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"effect":"initiative_override","card_id":self.symbols.name(*card),"value":value}})
            }
            NativeEvent::EffectCreated(event) => {
                let mut metadata = serde_json::Map::new();
                metadata.insert("effect_type".into(), json!(self.symbols.name(event.kind)));
                metadata.insert("duration".into(), json!(self.symbols.name(event.duration)));
                if let Some(axis) = event.split {
                    metadata.insert("split_axis".into(), json!(self.symbols.name(axis)));
                    metadata.insert("split_value".into(), json!(event.split_value));
                }
                if let Some(color) = event.color {
                    metadata.insert("named_color".into(), json!(self.symbols.name(color)));
                }
                json!({"event_type":"EFFECT_CREATED","actor_id":self.symbols.name(event.actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":metadata})
            }
            NativeEvent::Round(event) => event.to_json(&self.symbols),
            NativeEvent::TurnEnded(hero) => {
                json!({"event_type":"TURN_ENDED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{}})
            }
            NativeEvent::RespawnCleared { hero, token, from } => {
                json!({"event_type":"TOKEN_REMOVED","actor_id":self.symbols.name(*hero),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":null,"metadata":{"reason":"respawn"}})
            }
            NativeEvent::MineTriggered {
                actor,
                target,
                from,
                kind,
                blast,
            } => {
                json!({"event_type":"MINE_TRIGGERED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*target),"from_hex":from,"to_hex":null,"metadata":{"token_type":kind.map(|id|self.symbols.name(id)),"is_blast":blast}})
            }
            NativeEvent::TokenRemoved {
                actor,
                token,
                from,
                effects_removed,
            } => {
                json!({"event_type":"TOKEN_REMOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":null,"metadata":{"effects_removed":effects_removed}})
            }
            NativeEvent::BoardEntityPlaced {
                actor,
                entity,
                from,
                to,
                obstacle,
            } => {
                json!({"event_type":"BOARD_ENTITY_PLACED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*entity),"from_hex":from,"to_hex":to,"metadata":{"entity_kind":"turret","is_obstacle":obstacle}})
            }
            NativeEvent::BoardEntityRemoved {
                actor,
                entity,
                from,
            } => {
                json!({"event_type":"BOARD_ENTITY_REMOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*entity),"from_hex":from,"to_hex":null,"metadata":{"entity_kind":"turret"}})
            }
            NativeEvent::TokenPlaced {
                actor,
                token,
                to,
                kind,
            } => {
                json!({"event_type":"TOKEN_PLACED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":null,"to_hex":to,"metadata":{"token_type":self.symbols.name(*kind)}})
            }
            NativeEvent::TokenReplaced {
                actor,
                token,
                from,
                to,
                kind,
            } => {
                json!({"event_type":"TOKEN_MOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":to,"metadata":{"token_type":self.symbols.name(*kind),"replaced":true}})
            }
            NativeEvent::TokenWalked {
                actor,
                token,
                from,
                to,
                range,
            } => {
                json!({"event_type":"TOKEN_MOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":to,"metadata":{"range":range}})
            }
            NativeEvent::TokenMoved {
                actor,
                token,
                from,
                to,
            } => {
                json!({"event_type":"TOKEN_MOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":to,"metadata":{}})
            }
            NativeEvent::Swapped { a, b, from, to } => {
                json!({"event_type":"UNITS_SWAPPED","actor_id":self.symbols.name(*a),"target_id":self.symbols.name(*b),"from_hex":from,"to_hex":to,"metadata":{}})
            }
            NativeEvent::Pushed {
                actor,
                target,
                token,
                from,
                to,
                distance,
                collision,
            } => {
                json!({"event_type":if *token{"TOKEN_PUSHED"}else{"UNIT_PUSHED"},"actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*target),"from_hex":from,"to_hex":to,"metadata":{"distance":distance,"collision":collision}})
            }
            NativeEvent::ItemGranted {
                owner,
                stat,
                amount,
            } => {
                json!({"event_type":"ITEM_GAINED","actor_id":self.symbols.name(*owner),"target_id":null,
                    "from_hex":null,"to_hex":null,"metadata":{"stat_type":stat.name(),"amount":amount}})
            }
            NativeEvent::GoldEffect {
                actor,
                target,
                owner,
                amount,
                lost,
            } => {
                let mut metadata =
                    json!({"amount":amount.to_json(&self.symbols),"reason":"effect"});
                if *lost {
                    metadata["owner_id"] = json!(self.symbols.name(*owner));
                }
                json!({"event_type":if *lost {"GOLD_LOST"} else {"GOLD_GAINED"},"actor_id":actor.map(|id|self.symbols.name(id)),"target_id":target.map(|id|self.symbols.name(id)),"from_hex":null,"to_hex":null,"metadata":metadata})
            }
            NativeEvent::Spell(event) => event.to_json(&self.symbols),
            NativeEvent::CoinsStolen {
                actor,
                victim,
                amount,
            } => {
                json!({"event_type":"GOLD_GAINED","actor_id":self.symbols.name(*actor),"target_id":self.symbols.name(*victim),"from_hex":null,"to_hex":null,"metadata":{"amount":amount.to_json(&self.symbols),"reason":"steal"}})
            }
            NativeEvent::AdditionalLifeSpent {
                team,
                target,
                total,
                change,
            } => {
                json!({"event_type":"LIFE_COUNTER_CHANGED","actor_id":null,"target_id":self.symbols.name(*target),
                    "from_hex":null,"to_hex":null,"metadata":{"team":self.symbols.name(*team),"change":change,
                    "remaining":total,"reason":"additional_life_counter"}})
            }
            NativeEvent::LifePenaltyAnnihilation { winner } => {
                json!({"event_type":"GAME_OVER","actor_id":null,"target_id":null,"from_hex":null,"to_hex":null,
                    "metadata":{"reason":"annihilation","winning_team":self.symbols.name(*winner)}})
            }
            NativeEvent::LifeRestored {
                hero,
                team,
                change,
                remaining,
            } => {
                json!({"event_type":"LIFE_COUNTER_CHANGED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"team":self.symbols.name(*team),"change":change,"remaining":remaining,"reason":"life_counter_restored"}})
            }
            NativeEvent::Marker(event) => event.to_json(&self.symbols),
            NativeEvent::Combat(event) => event.to_json(&self.symbols),
            NativeEvent::UnitPlaced { unit, from, to } => {
                json!({"event_type":"UNIT_PLACED","actor_id":self.symbols.name(*unit),"target_id":null,"from_hex":from,"to_hex":to,"metadata":{}})
            }
            NativeEvent::CardRetrieved {
                hero,
                card,
                name,
                kept,
            } => {
                let mut metadata = json!({"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name)});
                if let Some(kept) = kept {
                    metadata["kept_card_id"] = json!(self.symbols.name(*kept));
                    metadata["source"] = json!("alternative_timelines");
                }
                json!({"event_type":"CARD_RETRIEVED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":metadata})
            }
            NativeEvent::GuessSelected {
                actor,
                victim,
                attempt,
            } => {
                json!({"event_type":"CARD_SELECTED_FOR_GUESS","actor_id":actor.map(|id|self.symbols.name(id)),
                    "target_id":self.symbols.name(*victim),"from_hex":null,"to_hex":null,"metadata":{"attempt":attempt}})
            }
            NativeEvent::ResolvedCardsSwapped {
                actor,
                target,
                a,
                b,
            } => json!({
                "event_type":"RESOLVED_CARDS_SWAPPED",
                "actor_id":actor.map(|id|self.symbols.name(id)),
                "target_id":self.symbols.name(*target), "from_hex":null, "to_hex":null,
                "metadata":{"card_a_id":self.symbols.name(*a),"card_b_id":self.symbols.name(*b)}
            }),
            NativeEvent::HeroPieceRemoved { owner, piece, from } => json!({
                "event_type":"UNIT_REMOVED", "actor_id":self.symbols.name(*owner),
                "target_id":self.symbols.name(*piece), "from_hex":from, "to_hex":null,
                "metadata":{"owner_hero_id":self.symbols.name(*owner),"voluntary":true}
            }),
            NativeEvent::HeroPieceSpawned { owner, piece, to } => json!({
                "event_type":"UNIT_PLACED", "actor_id":self.symbols.name(*owner),
                "target_id":self.symbols.name(*piece), "from_hex":null, "to_hex":to,
                "metadata":{"owner_hero_id":self.symbols.name(*owner)}
            }),
            NativeEvent::HeroLaughed(actor) => json!({
                "event_type":"HERO_LAUGHED", "actor_id":self.symbols.name(*actor),
                "target_id":null, "from_hex":null, "to_hex":null, "metadata":{}
            }),
            NativeEvent::HandCardRevealed {
                actor,
                target,
                owner,
                card,
                name,
                color,
                tier,
                tier_value,
            } => json!({"event_type":"CARD_REVEALED",
                    "actor_id":actor.map(|id|self.symbols.name(id)),
                    "target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,
                    "metadata":{"owner_id":self.symbols.name(*owner),
                        "card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name),
                        "card_color":color.map(|id|self.symbols.name(id)),
                        "card_tier":self.symbols.name(*tier),"tier_value":tier_value}}),
            NativeEvent::GuessRevealed {
                actor,
                victim,
                attempt,
                card,
                name,
                actual,
                guessed,
                correct,
            } => {
                json!({"event_type":"GUESSED_CARD_REVEALED","actor_id":actor.map(|id|self.symbols.name(id)),
                    "target_id":self.symbols.name(*victim),"from_hex":null,"to_hex":null,
                    "metadata":{"attempt":attempt,"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name),
                        "card_color":actual.map(|id|self.symbols.name(id)),"guessed_color":self.symbols.name(*guessed),"guess_correct":correct}})
            }
            NativeEvent::DeckCardSwapped {
                actor,
                outgoing,
                incoming,
                state,
                facedown,
            } => {
                json!({"event_type":"DECK_CARD_SWAPPED","actor_id":self.symbols.name(*actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"outgoing_card_id":self.symbols.name(*outgoing),"incoming_card_id":self.symbols.name(*incoming),"incoming_card_state":state,"incoming_is_facedown":facedown}})
            }
            NativeEvent::ItemGained { actor, stat, card } => {
                json!({"event_type":"ITEM_GAINED","actor_id":self.symbols.name(*actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"stat_type":stat.name(),"amount":1,"source_card_id":self.symbols.name(*card)}})
            }
            NativeEvent::TieBreakerFlipped { blue } => {
                json!({"event_type":"TIE_BREAKER_FLIPPED","actor_id":null,"target_id":null,"from_hex":null,"to_hex":null,
                "metadata":{"tie_breaker_team":if *blue {"BLUE"}else{"RED"},"coin_face":if *blue {"BLUE"}else{"ORANGE"}}})
            }
            NativeEvent::DirectionalMove {
                unit,
                from,
                to,
                direction,
            } => {
                json!({"event_type":"UNIT_MOVED","actor_id":self.symbols.name(*unit),"target_id":null,
                    "from_hex":from,"to_hex":to,"metadata":{"direction":direction,"simultaneous":true}})
            }
            NativeEvent::UnitMoved {
                unit,
                from,
                to,
                range,
            } => {
                json!({"event_type":"UNIT_MOVED","actor_id":self.symbols.name(*unit),"target_id":null,"from_hex":from,"to_hex":to,"metadata":{"range":range}})
            }
        }
    }
}
