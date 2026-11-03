//! Authoritative arena turn boundaries and staged round-end orchestration.
use super::*;

#[derive(Clone, Copy)]
pub(super) enum Field {
    Phase,
    Turn,
    DiscardLog,
    Locations,
    Snapshot,
    Pending,
    Second,
    Done,
    Unresolved,
    Played,
    Current,
    ResolvedCount,
    CardState,
    ExpiryIds,
    Planning,
    Resolution,
    Cleanup,
    Resolved,
    Hand,
    Extra,
    Facedown,
    Revealed,
    HeroId,
    CardId,
    Owner,
    Tied,
    Coin,
    Red,
    Blue,
    Revelation,
    InitiativeOverride,
    Discard,
    HandState,
    TeamRed,
    TeamBlue,
    Requests,
    From,
    To,
    Deck,
    Spells,
    Ultimate,
    PlayedRound,
    UnresolvedState,
    RollbackFrozen,
    RollbackReanchor,
    Confirm,
    ConfirmPrompt,
    PassiveUses,
    Guess,
    Reveal,
    Guesser,
    Revealer,
    Piece,
    EffectActive,
    ProcessedMinions,
    UnitId,
    TargetHex,
    Range,
    UnitKey,
    DestinationKey,
    Occupant,
    MoveAction,
    Through,
    Straight,
    Crossed,
    Success,
    Prechosen,
    MineIds,
    Output,
    MinePrompt,
    MinesLiteral,
    MinesKey,
    Victim,
    VictimKey,
    Blast,
    MineVictim,
    Basic,
    Immunity,
    Prompt,
    CardKey,
    ContextHero,
    OverridePlayer,
    Containers,
    ShieldsOnly,
    AllowedCards,
    DecisionPurpose,
    PlayedContainer,
    CardToDiscard,
    DiscardPurpose,
    Auto,
    ContextHeroKey,
    OverridePlayerKey,
    Container,
    IncludeFacedown,
    CardActions,
    CardColors,
    CardColorKey,
    CardTierKey,
    CardActive,
    CardItem,
    ExcludeCardKeys,
    CardStates,
    ExcludeCardStates,
    SelectedColor,
    SelectedTier,
    Filters,
    HeroKey,
    Source,
    DiscardState,
    DiscardedId,
    DiscardedOwner,
    DiscardSource,
    CurrentSource,
    Trigger,
    AfterDiscard,
    PassiveOptional,
    PassiveProvider,
    RuneSlots,
    PerformingCard,
    CurrentCard,
    PrepareSpells,
    Wish,
    BasicActionCard,
    CurrentAction,
    DefenseKind,
    AttackCard,
    IceBlast,
    IceVictim,
    DefeatLog,
    DefeatMarks,
    AfterPrimary,
    BeforeSpellbook,
    BeforeAction,
    AfterBasic,
    AfterAttack,
    Value,
    BeforeAttack,
    AfterMarker,
    AfterLaugh,
    AfterPush,
    AfterBasicSkill,
    AfterMovement,
    AfterResolve,
    PushVictim,
    MarkerTarget,
    RuneAxe,
    RuneBird,
    RuneAnvil,
    RuneHorn,
    RuneDefenseOutput,
    RuneActionOutput,
    BeforeDefense,
    Options,
    MaxRange,
    FlurryRepeat,
    AttackEffect,
    BasicActionType,
    BasicActionValue,
    BasicActionEffect,
    BasicActionRange,
    AttackKind,
    MovementKind,
    SkillKind,
    Damage,
    MatchingOptions,
    MatchesOutput,
    ValueMap,
    ValueOutput,
    EmptyOwner,
    Mandatory,
    FilterType,
    OriginId,
    OriginKey,
    OriginHexKey,
    MinRange,
    MinRangeKey,
    MaxRangeKey,
    DistanceMode,
    ExcludeId,
    ExcludeIdKey,
    IsObstacle,
    HasSpawn,
    MinionOnly,
    IsEmpty,
    MustNotHave,
    BattleOnly,
    ZoneChange,
    TargetTags,
    RangeStat,
    FullDistance,
    PerformingOwner,
    SourceCard,
    TargetType,
    SkipSelf,
    SkipImmunity,
    Relation,
    UnitType,
    ExcludeSelf,
    ExcludeKeys,
    TargetKey,
    IdsKey,
    ExcludeActing,
    TokenType,
    MinionTypes,
    ActionType,
    CardColor,
    InputKey,
    Operator,
    Threshold,
    RetrieveAll,
    TokenPool,
    HexKey,
    OwnerIdKey,
    ExistingTokenKey,
    OverflowKey,
    ImmuneActions,
    TokenOverride,
    SubstitutionActor,
    MiscEntities,
    TokenId,
    TokenKey,
    TokenOwner,
    TokenCount,
    DirectionRefKey,
    TrailDestKey,
    ExcludedHexesKey,
    SlotIndex,
    SlotKeys,
    SlotFilters,
    FreeHexes,
    RemovableHexes,
    RemovalBudget,
    RemainingRemovals,
    BatchFreed,
    AnchorKey,
    PartnerKey,
    AnchorDestinationKey,
    PartnerDestinationKey,
    AnchorIsToken,
    IgnorePathObstacles,
    OccupiedHexKeys,
    OptInPrompt,
    PlacedFlagKey,
    KeyPrefix,
    FlagKey,
    ActiveIf,
    SkipIf,
    Level,
    RockKeys,
    RockListKey,
    UsedFlagKey,
    RockAffected,
    TurretId,
    EntityKind,
    EntityId,
    EntityName,
    Selections,
    MinSelections,
    MaxSelections,
    Gold,
    Teams,
    Life,
    Markers,
    TargetId,
    SourceId,
    DefeatedHeroes,
    LastDefeatedMinion,
    KillerId,
    AssistMultiplier,
    MinionId,
    TriedEffects,
    Winner,
    IndividualWinner,
    VictoryCondition,
    GameOverPhase,
    InputStack,
    Condition,
    MaxValue,
    SelectedCardId,
    KillerKey,
    BlockSucceeded,
    LastCombatTarget,
    DefenseValue,
    DefenseInvalid,
    AutoBlock,
    IgnoreMinionDefense,
    MinionDefenseModifier,
    DefenseInitiative,
    DefenseBonus,
    ByActor,
    AttackTargetKey,
    AttackOutputKey,
    TargetFilters,
    DamageBonusKey,
    RangeBonusKey,
    DamageStat,
    Ranged,
    AttackRanged,
    Attacker,
    AttackDamage,
    AttackBasic,
    TargetPlayerKey,
    SaveKey,
    ActorKey,
    DefenderId,
    PrePamActor,
    DiscardPam,
    PrimaryDefense,
    AbortOnDefeat,
    DefenseCardId,
    DefenseCardColor,
    BlockPrimaryDefense,
    SnorriRuneDefense,
    PassChoice,
    PreActionMoveHex,
    PreActionDiscardVictim,
    ProcessedEffects,
    UnitAId,
    UnitBId,
    UnitAKey,
    UnitBKey,
    SystemName,
    MarkerType,
    SkipMarkers,
    Amount,
    AmountKey,
    StartingLife,
    ResourceTeam,
    Scale,
    RepeatsDone,
    MaxRepeats,
    LanePushChecked,
    RepeatZones,
    PushDistance,
    DistanceKey,
    SourceHex,
    PushSourceKey,
    PushIgnore,
    CollisionOutput,
    ExcludeTargetKey,
    BehindLength,
    CardOwnerKey,
    PreviousSlot,
    SuppressAfterResolve,
    IsTerrain,
    AllowedSpellIds,
    CasterId,
    CasterKey,
    SpellOwner,
    SpellCaster,
    CastSpell,
    Started,
    RequiredCasts,
    PerformWish,
    WishCount,
    WishCountedCaster,
    MaxRemovals,
    RemovedSpellIds,
    SpellbookState,
    OutsideSpellbookState,
    WishVictoryCondition,
    FilterContainer,
    MinCards,
    MaxCards,
    ExcludeFacedown,
    SubFilters,
    MinCount,
    MaxCount,
    IncludeTokens,
    HexFilters,
    LaneBound,
    LaneRespawnHex,
    ReferenceKey,
    FromHexKey,
    ToHexKey,
    ExpectedType,
    KeyA,
    KeyB,
    Operation,
    Items,
    ItemState,
}
const NAMES: [&str; 381] = [
    "phase",
    "turn",
    "turn_discard_log",
    "entity_locations",
    "last_turn_positions",
    "pending_inputs",
    "pending_second_cards",
    "planning_done",
    "unresolved_hero_ids",
    "played_cards",
    "current_turn_card",
    "resolved_turn_count",
    "state",
    "expire_effect_ids",
    "PLANNING",
    "RESOLUTION",
    "CLEANUP",
    "RESOLVED",
    "hand",
    "extra_turn_card",
    "is_facedown",
    "public_revealed_cards",
    "hero_id",
    "card_id",
    "resolution_owner_id",
    "tied_hero_ids",
    "tie_breaker_team",
    "RED",
    "BLUE",
    "REVELATION",
    "initiative_override",
    "discard_pile",
    "HAND",
    "team:RED",
    "team:BLUE",
    "starting_position_requests",
    "from",
    "to",
    "deck",
    "spells",
    "ultimate_card",
    "played_this_round",
    "UNRESOLVED",
    "rollback_frozen",
    "rollback_reanchor_pending",
    "CONFIRM",
    "Confirm your action.",
    "passive_uses_this_turn",
    "card_guess",
    "card_reveal",
    "guesser_id",
    "revealer_id",
    "acting_piece_id",
    "is_active",
    "processed_minion_ids",
    "unit_id",
    "target_hex_arg",
    "range_val",
    "unit_key",
    "destination_key",
    "occupant_id",
    "is_movement_action",
    "pass_through_obstacles",
    "force_straight_line",
    "crossed_obstacles_output_key",
    "success_output_key",
    "mine_path_prechosen",
    "triggered_mine_ids",
    "output_key",
    "Choose which mines to move through",
    "mine_ids",
    "mine_ids_key",
    "victim_id",
    "victim_key",
    "mine_blast",
    "mine_victim_id",
    "card_is_basic",
    "immunity_source_id",
    "prompt",
    "card_key",
    "context_hero_id",
    "override_player_id",
    "card_containers",
    "restrict_played_to_shields",
    "allowed_card_ids",
    "decision_purpose",
    "PLAYED",
    "card_to_discard",
    "discard",
    "auto_select_if_one",
    "context_hero_id_key",
    "override_player_id_key",
    "card_container",
    "include_facedown",
    "card_action_types",
    "card_colors",
    "card_color_key",
    "card_tier_key",
    "card_is_active",
    "card_has_item",
    "exclude_card_id_keys",
    "card_states",
    "exclude_card_states",
    "selected_card_color_key",
    "selected_card_tier_key",
    "filters",
    "hero_key",
    "source",
    "DISCARD",
    "discarded_card_id",
    "discarded_card_owner_id",
    "discard_source",
    "current_turn",
    "trigger",
    "after_card_discard",
    "is_optional",
    "active_effect_id",
    "rune_slots",
    "performing_card_id",
    "current_card_id",
    "prepare_spells",
    "wish",
    "basic_action_card_id",
    "current_action_type",
    "DEFENSE",
    "attack_card_id",
    "ice_blast",
    "ice_blast_victim",
    "defeat_log",
    "defeat_marks",
    "after_primary_action",
    "before_prepare_spellbook",
    "before_action",
    "after_basic_action",
    "after_attack",
    "value",
    "before_attack",
    "after_place_marker",
    "after_laugh",
    "after_push",
    "after_basic_skill",
    "after_movement",
    "after_resolve_card",
    "push_victim_id",
    "marker_target_id",
    "axe",
    "bird",
    "anvil",
    "horn",
    "snorri_ult_rune_defense",
    "snorri_ult_rune_action",
    "before_defense",
    "options",
    "max_range",
    "is_flurry_repeat",
    "attack_effect_id",
    "basic_action_type",
    "basic_action_value",
    "basic_action_effect_id",
    "basic_action_range",
    "ATTACK",
    "MOVEMENT",
    "SKILL",
    "damage",
    "matching_options",
    "matches_output_key",
    "value_map",
    "value_output_key",
    "",
    "is_mandatory",
    "type",
    "origin_id",
    "origin_key",
    "origin_hex_key",
    "min_range",
    "min_range_key",
    "max_range_key",
    "distance_mode",
    "exclude_id",
    "exclude_id_key",
    "is_obstacle",
    "has_spawn_point",
    "minion_only",
    "is_empty",
    "must_not_have",
    "battle_zone_only",
    "require_zone_change",
    "target_tags",
    "range_stat_type",
    "force_full_distance",
    "performing_card_owner_id",
    "source_card_id",
    "target_type",
    "skip_self_filter",
    "skip_immunity_filter",
    "relation",
    "unit_type",
    "exclude_self",
    "exclude_keys",
    "target_key",
    "ids_key",
    "exclude_acting",
    "token_type",
    "minion_types",
    "action_type",
    "card_color",
    "input_key",
    "operator",
    "threshold",
    "retrieve_all_discarded",
    "token_pool",
    "hex_key",
    "owner_id_key",
    "existing_token_key",
    "overflow_selection_key",
    "is_immune_to_enemy_actions",
    "token_type_override",
    "substitution_actor_id",
    "misc_entities",
    "token_id",
    "token_key",
    "owner_id",
    "count",
    "direction_ref_key",
    "dest_key",
    "excluded_hexes_key",
    "slot_index",
    "slot_keys",
    "slot_filters",
    "free_hexes",
    "removable_hexes",
    "removal_budget",
    "remaining_removals",
    "_batch_freed_hexes",
    "anchor_key",
    "partner_key",
    "anchor_dest_key",
    "partner_dest_key",
    "anchor_is_token",
    "ignore_path_obstacles",
    "occupied_hex_keys",
    "opt_in_prompt",
    "placed_flag_key",
    "key_prefix",
    "key",
    "active_if_key",
    "skip_if_key",
    "level",
    "rock_hex_keys",
    "rock_hexes_key",
    "used_flag_key",
    "_rock_ult_affected",
    "trinkets_turret",
    "entity_kind",
    "id",
    "name",
    "selections",
    "min_selections",
    "max_selections",
    "gold",
    "teams",
    "life_counters",
    "markers",
    "target_id",
    "source_id",
    "heroes_defeated_this_round",
    "last_defeated_minion_id",
    "killer_id",
    "assist_multiplier",
    "minion_id",
    "tried_effect_ids",
    "winner",
    "individual_winner_id",
    "victory_condition",
    "GAME_OVER",
    "input_stack",
    "condition",
    "max_value",
    "selected_card_id",
    "killer_key",
    "block_succeeded",
    "last_combat_target",
    "defense_value",
    "defense_invalid",
    "auto_block",
    "ignore_minion_defense",
    "minion_defense_modifier",
    "defense_uses_initiative",
    "defense_bonus",
    "by_actor",
    "target_id_key",
    "target_output_key",
    "target_filters",
    "damage_bonus_key",
    "range_bonus_key",
    "damage_stat_type",
    "is_ranged",
    "attack_is_ranged",
    "attacker_id",
    "attack_damage",
    "attack_is_basic",
    "target_player_key",
    "save_key",
    "actor_key",
    "defender_id",
    "_pre_pam_actor",
    "_discard_pam",
    "is_primary_defense",
    "abort_on_defeat",
    "defense_card_id",
    "defense_card_color",
    "block_primary_defense",
    "snorri_ult_rune_defense",
    "PASS",
    "pre_action_move_hex",
    "pre_action_discard_victim",
    "processed_effect_ids",
    "unit_a_id",
    "unit_b_id",
    "unit_a_key",
    "unit_b_key",
    "system",
    "marker_type",
    "skip_markers",
    "amount",
    "amount_key",
    "starting_life_counters",
    "team",
    "scale",
    "repeats_done",
    "max_repeats",
    "lane_push_checked",
    "battle_zones",
    "distance",
    "distance_key",
    "source_hex",
    "source_key",
    "ignore_obstacles",
    "collision_output_key",
    "exclude_target_key",
    "length",
    "card_owner_key",
    "previous_slot",
    "suppress_after_resolve_card",
    "is_terrain",
    "allowed_spell_ids",
    "caster_id",
    "caster_key",
    "spell_owner_id",
    "spell_caster_id",
    "cast_spell_id",
    "started",
    "required_casts",
    "perform_wish_action",
    "wish_cast_count",
    "wish_cast_counted_caster_id",
    "max_removals",
    "removed_spell_ids",
    "SPELLBOOK",
    "OUTSIDE_SPELLBOOK",
    "WISH",
    "container",
    "min_cards",
    "max_cards",
    "exclude_facedown",
    "sub_filters",
    "min_count",
    "max_count",
    "include_tokens",
    "hex_filters",
    "lane_bound",
    "lane_respawn_hex",
    "reference_key",
    "from_hex_key",
    "to_hex_key",
    "expected_type",
    "key_a",
    "key_b",
    "operation",
    "items",
    "ITEM",
];
#[derive(Clone, Debug)]
pub(super) struct PhaseBindings {
    keys: [Symbol; NAMES.len()],
    pub(super) heroes: Vec<(Symbol, ObjectId)>,
    pub(super) tiles: Arc<[(ObjectId, bool)]>,
}
impl PhaseBindings {
    pub fn compile(source: &StateEnvelope, arena: &mut ObjectArena) -> Result<Self> {
        let keys = NAMES.map(|name| Arc::make_mut(&mut arena.symbols).intern(name));
        let mut heroes = Vec::new();
        for (team, value) in source.state["teams"]
            .as_object()
            .ok_or_else(|| unsupported("teams missing"))?
        {
            for (index, hero) in value["heroes"]
                .as_array()
                .ok_or_else(|| unsupported("heroes missing"))?
                .iter()
                .enumerate()
            {
                let id = arena
                    .symbols
                    .lookup(
                        hero["id"]
                            .as_str()
                            .ok_or_else(|| unsupported("hero ID missing"))?,
                    )
                    .ok_or_else(|| unsupported("hero ID symbol missing"))?;
                let ObjectValue::Object(object) = arena
                    .value_at_path(&[
                        ObjectPathPart::Key("teams".into()),
                        ObjectPathPart::Key(team.clone()),
                        ObjectPathPart::Key("heroes".into()),
                        ObjectPathPart::Index(index),
                    ])
                    .map_err(unsupported)?
                else {
                    return Err(unsupported("hero model binding missing"));
                };
                heroes.push((id, object));
            }
        }
        // Walk ordered arena rows once instead of searching a dictionary path
        // for each tile. This table is immutable and shared by worker clones.
        let hex_key = Arc::make_mut(&mut arena.symbols).intern("hex");
        let coordinates = ["q", "r", "s"].map(|key| Arc::make_mut(&mut arena.symbols).intern(key));
        let ObjectValue::Object(rows) = arena
            .value_at_path(&[
                ObjectPathPart::Key("board".into()),
                ObjectPathPart::Key("tiles".into()),
            ])
            .map_err(unsupported)?
        else {
            return Err(unsupported("board tile dictionary missing"));
        };
        let mut tiles = Vec::new();
        for (key, value) in arena
            .fields(rows)
            .ok_or_else(|| unsupported("board tile dictionary missing"))?
        {
            let ObjectValue::Object(tile) = value else {
                return Err(unsupported("tile model missing"));
            };
            let Some(ObjectValue::Object(hex)) = arena.field(*tile, hex_key) else {
                return Err(unsupported("tile Hex binding missing"));
            };
            let mut point = [0; 3];
            for (index, key) in coordinates.iter().enumerate() {
                let Some(ObjectValue::Atom(Datum::Int(value))) = arena.field(hex, *key) else {
                    return Err(unsupported("tile coordinate must be an integer"));
                };
                point[index] =
                    i32::try_from(value).map_err(|_| unsupported("tile coordinate outside i32"))?;
            }
            let hex = Hex::new(point[0], point[1], point[2]).map_err(unsupported)?;
            let (q, r, s) = hex.coordinates();
            let consistent = arena.symbols.name(*key) == format!("({q}, {r}, {s})");
            tiles.push((*tile, consistent));
        }
        let mut aliases = HashMap::new();
        for (tile, _) in &tiles {
            *aliases.entry(*tile).or_insert(0usize) += 1;
        }
        for (tile, consistent) in &mut tiles {
            *consistent &= aliases[tile] == 1;
        }
        Ok(Self {
            keys,
            heroes,
            tiles: tiles.into(),
        })
    }
    pub(super) fn k(&self, field: Field) -> Symbol {
        self.keys[field as usize]
    }
    pub(super) fn root(&self, arena: &ObjectArena, field: Field) -> Result<ObjectId> {
        match arena.root_field(self.k(field)) {
            Some(ObjectValue::Object(id)) => Ok(id),
            _ => Err(unsupported("phase collection binding missing")),
        }
    }
    pub fn effect_ids(&self, arena: &ObjectArena, step: ObjectId) -> Result<Vec<Symbol>> {
        let Some(ObjectValue::Object(list)) = arena.field(step, self.k(Field::ExpiryIds)) else {
            return Err(unsupported("deferred expiry IDs missing"));
        };
        arena
            .sequence(list)
            .ok_or_else(|| unsupported("deferred expiry IDs must be a list"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Text(id)) => Ok(*id),
                _ => Err(unsupported("deferred expiry ID must be text")),
            })
            .collect()
    }
}
impl NativeSession {
    pub(super) fn phase_step(
        &mut self,
        objects: &mut BoundObjects,
        name: &str,
    ) -> Result<ObjectId> {
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipe = &program
            .phase_recipes
            .iter()
            .find(|(key, _)| *key == name)
            .ok_or_else(|| unsupported("phase recipe missing"))?
            .1;
        self.next_step = self
            .next_step
            .checked_add(1)
            .ok_or_else(|| unsupported("step generation counter exhausted"))?;
        objects.generated_step(recipe, &program, self.next_step, None)
    }
    pub(super) fn phase_generation_check(&self, groups: usize, tail: u64) -> Result<()> {
        u64::try_from(groups)
            .ok()
            .and_then(|count| count.checked_mul(2))
            .and_then(|count| count.checked_add(tail))
            .and_then(|count| self.next_step.checked_add(count))
            .filter(|next| *next < u64::MAX)
            .ok_or_else(|| unsupported("step generation counter exhausted"))?;
        Ok(())
    }
    fn finishing_payloads(
        &self,
        objects: &BoundObjects,
        finishing: &[(Symbol, ObjectId)],
    ) -> Result<Vec<(Symbol, Vec<ObjectId>)>> {
        finishing
            .iter()
            .map(|(source, list)| Ok((*source, objects.steps_in_list(*list)?)))
            .collect()
    }
    pub(super) fn wrap_finishings(
        &mut self,
        objects: &mut BoundObjects,
        groups: Vec<(Symbol, Vec<ObjectId>)>,
    ) -> Result<Vec<ObjectId>> {
        self.phase_generation_check(groups.len(), 0)?;
        let program = objects.scheduler.as_ref().unwrap().clone();
        let mut queued = Vec::new();
        for (actor, steps) in groups {
            self.next_step += 1;
            queued.push(objects.generated_step(
                &program.actor,
                &program,
                self.next_step,
                Some(actor),
            )?);
            queued.extend(steps);
            self.next_step += 1;
            queued.push(objects.generated_step(&program.marker, &program, self.next_step, None)?);
        }
        Ok(queued)
    }
    pub(super) fn phase_set(&mut self, objects: &mut BoundObjects, phase: Phase) -> Result<()> {
        let field = match phase {
            Phase::Planning => Field::Planning,
            Phase::Resolution => Field::Resolution,
            Phase::Revelation => Field::Revelation,
            Phase::Cleanup => Field::Cleanup,
            _ => return Err(unsupported("unbound phase header")),
        };
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Phase),
                ObjectValue::Atom(Datum::Text(objects.phases.k(field))),
            )
            .map_err(unsupported)?;
        self.planning.phase = phase;
        Ok(())
    }
    fn clear_discard_log_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let empty = objects.arena.dictionary(Vec::new()).map_err(unsupported)?;
        objects
            .arena
            .set_root(objects.phases.k(Field::DiscardLog), empty)
            .map_err(unsupported)?;
        self.world.history.discarded.clear();
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        Ok(())
    }
    pub(super) fn preflight_bound_advance(&self, objects: &BoundObjects) -> Result<()> {
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("pending inputs must be a dictionary"))?;
        let locations = p.root(&objects.arena, Field::Locations)?;
        if objects.arena.fields(locations).is_none() {
            return Err(unsupported("positions must be a dictionary"));
        }
        for (_, model) in &p.heroes {
            let Some(ObjectValue::Object(hand)) = objects.arena.field(*model, p.k(Field::Hand))
            else {
                return Err(unsupported("hero hand missing"));
            };
            if objects.arena.sequence(hand).is_none() {
                return Err(unsupported("hero hand must be a sequence"));
            }
        }
        self.phase_generation_check(0, 1)
    }
    pub(super) fn check_bound_phase(
        &self,
        objects: &BoundObjects,
        instruction: &Instruction,
        actor_choice: Option<&crate::planning::ActorChoice>,
    ) -> Result<()> {
        use crate::turn::FlowStep;
        match instruction {
            Instruction::Battle(_) => Ok(()),
            Instruction::Flow(FlowStep::Cleanup) => Ok(()),
            Instruction::Flow(FlowStep::Upgrades | FlowStep::Reset) => Ok(()),
            Instruction::ParkedPhase(family) => Err(unsupported(format!(
                "arena {} consumer not yet bound",
                self.symbols.name(*family)
            ))),
            Instruction::Flow(FlowStep::Respawn(_)) => Ok(()),
            Instruction::ResolveCard { .. } => Err(unsupported(
                "dense card-resolution instruction reached the authoritative arena",
            )),
            Instruction::RetrieveUnresolved { .. } => Ok(()),
            Instruction::Flow(FlowStep::Next) => {
                if matches!(actor_choice, Some(crate::planning::ActorChoice::Finished)) {
                    self.preflight_end_turn_bound(objects)
                } else {
                    self.phase_generation_check(0, 5)
                }
            }
            Instruction::Flow(FlowStep::Advance) => self.preflight_bound_advance(objects),
            Instruction::Flow(FlowStep::EndPhase) => {
                let indices = self
                    .world
                    .effects
                    .duration_indices(crate::effects::Duration::ThisRound);
                let finishing = objects.finishing_for_indices(&self.world.effects, &indices)?;
                self.finishing_payloads(objects, &finishing)?;
                self.phase_generation_check(finishing.len(), 3)
            }
            _ => Ok(()),
        }
    }
    fn preflight_end_turn_bound(&self, objects: &BoundObjects) -> Result<()> {
        let expiry = self.collect_turn_expiry_bound(objects)?;
        self.finishing_payloads(objects, &expiry.finishing)?;
        self.phase_generation_check(expiry.finishing.len(), 1)?;
        self.preflight_bound_advance(objects)
    }
    /// The source end_turn helper, including rule-v9 deferred effect removal.
    pub fn end_turn(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| session.end_turn_bound(objects))
    }
    pub(super) fn end_turn_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        self.preflight_end_turn_bound(objects)?;
        let expiry = self.collect_turn_expiry_bound(objects)?;
        let groups = self.finishing_payloads(objects, &expiry.finishing)?;
        self.clear_discard_log_bound(objects)?;
        if groups.is_empty() {
            self.remove_effects_bound(objects, &expiry.ids)?;
        }
        if !groups.is_empty() {
            let mut queued = self.wrap_finishings(objects, groups)?;
            let advance = self.phase_step(objects, "advance_turn")?;
            let ids = objects.arena.list(
                expiry
                    .ids
                    .into_iter()
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
                    .collect(),
            );
            objects
                .arena
                .set_field(advance, objects.phases.k(Field::ExpiryIds), ids)
                .map_err(unsupported)?;
            queued.push(advance);
            return self.push_live_steps_bound(objects, &queued);
        }
        self.advance_turn_bound(objects)
    }
    pub(super) fn advance_turn_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        self.preflight_bound_advance(objects)?;
        if self.world.combat.turn >= 4 {
            self.phase_set(objects, Phase::Cleanup)?;
            let step = self.phase_step(objects, "end_phase")?;
            return self.push_live_steps_bound(objects, &[step]);
        }
        self.world.combat.turn += 1;
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Turn),
                ObjectValue::Atom(Datum::Int(self.world.combat.turn)),
            )
            .map_err(unsupported)?;
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let fields = objects
            .arena
            .fields(locations)
            .ok_or_else(|| unsupported("position fields missing"))?
            .to_vec();
        let snapshot = objects.arena.dictionary(fields).map_err(unsupported)?;
        objects
            .arena
            .set_root(objects.phases.k(Field::Snapshot), snapshot)
            .map_err(unsupported)?;
        self.boundary.snapshot(&self.world);
        self.phase_set(objects, Phase::Planning)?;
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let mut passed = false;
        for (hero, model) in objects.phases.heroes.clone() {
            let Some(ObjectValue::Object(hand)) =
                objects.arena.field(model, objects.phases.k(Field::Hand))
            else {
                return Err(unsupported("hand binding missing"));
            };
            if objects
                .arena
                .sequence(hand)
                .is_some_and(|hand| hand.is_empty())
            {
                objects
                    .arena
                    .set_field(pending, hero, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                passed = true;
            }
        }
        if passed
            && !self.pending_planning_open_bound(objects)?
            && objects
                .arena
                .fields(pending)
                .is_some_and(|entries| entries.len() >= objects.phases.heroes.len())
        {
            return self.reveal_all_passes_bound(objects, pending);
        }
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        Ok(())
    }
    fn reveal_all_passes_bound(
        &mut self,
        objects: &mut BoundObjects,
        _pending: ObjectId,
    ) -> Result<()> {
        self.start_revelation_bound(objects)
    }
    pub(super) fn end_phase_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        self.check_bound_phase(
            objects,
            &Instruction::Flow(crate::turn::FlowStep::EndPhase),
            None,
        )?;
        let finishing = self.expire_effects_bound(objects, crate::effects::Duration::ThisRound)?;
        let groups = self.finishing_payloads(objects, &finishing)?;
        let mut steps = self.wrap_finishings(objects, groups)?;
        for name in ["minion_battle", "check_lane_push", "end_cleanup"] {
            steps.push(self.phase_step(objects, name)?);
        }
        self.push_live_steps_bound(objects, &steps)
    }
}
