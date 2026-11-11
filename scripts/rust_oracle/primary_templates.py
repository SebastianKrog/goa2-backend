"""Freeze reviewed, data-only primary effect recipes using the real effect API.

Only the named pure recipe builders are supported. The native interpreter binds
reserved hero/card/stat operands; arbitrary Python effects are never inferred.
"""

from __future__ import annotations

import json

from fixture_cache import write_text
from oracle import ROOT, digest, verify_sources

HERO = "native-primary-hero"
CARD = "native-primary-card"
PRIMARY = 1234567891
RANGE = 1234567892
RADIUS = 1234567893
EFFECTS = {
    "dead_or_alive": "Bain",
    "hand_crossbow": "Bain",
    "hunter_seeker": "Bain",
    "light_crossbow": "Bain",
    "heavy_crossbow": "Bain",
    "arbalest": "Bain",
    "get_over_here": "Bain",
    "grappling_bolt": "Cutter",
    "walk_the_plank": "Cutter",
    "bombardment": "Cutter",
    "barrage": "Cutter",
    "broadside": "Cutter",
    "outmaneuver": "Cutter",
    "outsmart": "Cutter",
    "x_marks_the_spot": "Cutter",
    "a_fistful_of_coins": "Cutter",
    "brace_for_impact": "Cutter",
    "ramming_speed": "Cutter",
    "crashland": "Cutter",
    "evasive_shot": "Cutter",
    "tumble_shot": "Cutter",
    "daring_strike": "Cutter",
    "bold_thrust": "Cutter",
    "fearless_lunge": "Cutter",
    "bullrush": "Brogan",
    "furious_charge": "Brogan",
    "throwing_axe": "Brogan",
    "throwing_spear": "Brogan",
    "shield": "Brogan",
    "bolster": "Brogan",
    "fortify": "Brogan",
    "war_drummer": "Brogan",
    "master_skald": "Brogan",
    "bulwark": "Brogan",
    "one_man_army": "Brogan",
    "backstab": "Tigerclaw",
    "backstab_with_a_ballista": "Tigerclaw",
    "combat_reflexes": "Tigerclaw",
    "light_fingered": "Tigerclaw",
    "pick_pocket": "Tigerclaw",
    "master_thief": "Tigerclaw",
    "blend_into_shadows": "Tigerclaw",
    "mind_grip": "NebKher",
    "spell_break": "Arien",
    "ebb_and_flow": "Arien",
    "dangerous_current": "Arien",
    "raging_stream": "Arien",
    "violent_torrent": "Arien",
    "slippery_ground": "Arien",
    "deluge": "Arien",
    "noble_blade": "Arien",
    "arcane_whirlpool": "Arien",
    "liquid_leap": "Arien",
    "magical_current": "Arien",
    "stranger_tide": "Arien",
    "onslaught": "Brogan",
    "hit_and_run": "Tigerclaw",
    "leaping_strike": "Tigerclaw",
    "poisoned_dagger": "Tigerclaw",
    "poisoned_dart": "Tigerclaw",
    "rogue_wave": "Arien",
    "tidal_blast": "Arien",
    "blink_strike": "Tigerclaw",
    "mad_dash": "Brogan",
    "brutal_jab": "Brogan",
    "mighty_punch": "Brogan",
    "savage_kick": "Brogan",
    "point_blank_shot": "Sabina",
    "back_to_back": "Sabina",
    "listen_up": "Sabina",
    "roger_roger": "Sabina",
    "ready_and_waiting": "Sabina",
    "troop_movement": "Sabina",
    "marching_orders": "Sabina",
    "path_to_victory": "Sabina",
    "close_support": "Sabina",
    "covering_fire": "Sabina",
    "shootout": "Sabina",
    "bullet_hell": "Sabina",
    "quickdraw": "Sabina",
    "gunslinger": "Sabina",
    "dead_shot": "Sabina",
    "shield_bash": "Brogan",
    "counterattack": "Brogan",
    "steady_advance": "Sabina",
    "unwavering_resolve": "Sabina",
    "prepare_spells": "Gydion",
    "cantrip": "Gydion",
    "elementary_evocation": "Gydion",
    "lesser_evocation": "Gydion",
    "greater_evocation": "Gydion",
    "elementary_abjuration": "Gydion",
    "lesser_abjuration": "Gydion",
    "greater_abjuration": "Gydion",
    "elementary_enchantment": "Gydion",
    "lesser_enchantment": "Gydion",
    "greater_enchantment": "Gydion",
    "lesser_necromancy": "Gydion",
    "greater_necromancy": "Gydion",
    "lesser_conjuration": "Gydion",
    "greater_conjuration": "Gydion",
    "lesser_transmutation": "Gydion",
    "greater_transmutation": "Gydion",
    "challenge_accepted": "Misa",
    "matter_of_honor": "Misa",
    "worthy_opponent": "Misa",
    "power_shot": "Misa",
    "thunder_shot": "Misa",
    "dash_and_slash": "Misa",
    "death_from_above": "Misa",
    "sudden_breeze": "Misa",
    "gust_of_wind": "Misa",
    "crushing_squall": "Misa",
    "living_tornado": "Misa",
    "storm_spirit": "Misa",
    "focus": "Misa",
    "discipline": "Misa",
    "mastery": "Misa",
    "power_overwhelming": "Misa",
    "magnetic_dagger": "Wasp",
    "charged_boomerang": "Wasp",
    "shock": "Wasp",
    "electrocute": "Wasp",
    "telekinesis": "Wasp",
    "mass_telekinesis": "Wasp",
    "electroblast": "Wasp",
    "thunder_boomerang": "Wasp",
    "lift_up": "Wasp",
    "control_gravity": "Wasp",
    "center_of_mass": "Wasp",
    "kinetic_repulse": "Wasp",
    "kinetic_blast": "Wasp",
    "static_barrier": "Wasp",
    "trip_mine": "Min",
    "cluster_mine": "Min",
    "minefield": "Min",
    "smoke_bomb": "Min",
    "crane_stance": "Min",
    "tiger_stance": "Min",
    "dragon_stance": "Min",
    "viper_stance": "Min",
    "cobra_stance": "Min",
    "fast_as_lightning": "Min",
    "death_grenade": "Min",
    "holy_death_grenade": "Min",
    "inner_strength": "Min",
    "monkey_trick": "Hanu",
    "monkey_twist": "Hanu",
    "monkey_business": "Hanu",
    "hear_nothing": "Hanu",
    "see_nothing": "Hanu",
    "helping_hand": "Hanu",
    "even_the_odds": "Hanu",
    "trusted_sidekick": "Hanu",
    "outnumber": "Hanu",
    "pile_on": "Hanu",
    "this_way": "Hanu",
    "that_way": "Hanu",
    "unexpected_journey": "Hanu",
    "there_and_back_again": "Hanu",
    "safe_travels": "Hanu",
    "fight_and_flight": "Hanu",
    "hurry_up": "Hanu",
    "the_ultimate_trick": "Hanu",
    "cleave": "Xargatha",
    "threatening_slash": "Xargatha",
    "deadly_swipe": "Xargatha",
    "lethal_spin": "Xargatha",
    "long_thrust": "Xargatha",
    "rapid_thrusts": "Xargatha",
    "sirens_call": "Xargatha",
    "charm": "Xargatha",
    "control": "Xargatha",
    "dominate": "Xargatha",
    "final_embrace": "Xargatha",
    "constrict": "Xargatha",
    "turn_into_statues": "Xargatha",
    "petrifying_stare": "Xargatha",
    "stone_gaze": "Xargatha",
    "metamorphosis": "Xargatha",
    "devoted_followers": "Xargatha",
    "fresh_converts": "Xargatha",
}


# These reviewed builders require the authoritative spellbook bindings. The
# canonical dense-path corpus keeps its own reviewed closure until that bridge
# is implemented; the object corpus covers every recipe in EFFECTS.
GYDION_EFFECTS = (
    "prepare_spells",
    "cantrip",
    "elementary_evocation",
    "lesser_evocation",
    "greater_evocation",
    "elementary_abjuration",
    "lesser_abjuration",
    "greater_abjuration",
    "elementary_enchantment",
    "lesser_enchantment",
    "greater_enchantment",
    "lesser_necromancy",
    "greater_necromancy",
    "lesser_conjuration",
    "greater_conjuration",
    "lesser_transmutation",
    "greater_transmutation",
)
MISA_STATEFUL_EFFECTS = {"swoop_in": "Misa", "watch_how_i_soar": "Misa"}
MISA_EFFECTS = (
    "challenge_accepted",
    "matter_of_honor",
    "worthy_opponent",
    "power_shot",
    "thunder_shot",
    "dash_and_slash",
    "death_from_above",
    "sudden_breeze",
    "gust_of_wind",
    "crushing_squall",
    "living_tornado",
    "storm_spirit",
    "focus",
    "discipline",
    "mastery",
    "power_overwhelming",
    "swoop_in",
    "watch_how_i_soar",
)
WASP_EFFECTS = (
    "magnetic_dagger",
    "charged_boomerang",
    "shock",
    "electrocute",
    "telekinesis",
    "mass_telekinesis",
    "electroblast",
    "thunder_boomerang",
    "lift_up",
    "control_gravity",
    "center_of_mass",
    "kinetic_repulse",
    "kinetic_blast",
    "static_barrier",
)
MIN_STATEFUL_EFFECTS = {"perfect_self": "Min"}
MIN_EFFECTS = (
    "trip_mine",
    "cluster_mine",
    "minefield",
    "smoke_bomb",
    "crane_stance",
    "tiger_stance",
    "dragon_stance",
    "viper_stance",
    "cobra_stance",
    "fast_as_lightning",
    "death_grenade",
    "holy_death_grenade",
    "inner_strength",
    "perfect_self",
)
HANU_EFFECTS = (
    "monkey_trick",
    "monkey_twist",
    "monkey_business",
    "hear_nothing",
    "see_nothing",
    "helping_hand",
    "even_the_odds",
    "trusted_sidekick",
    "outnumber",
    "pile_on",
    "this_way",
    "that_way",
    "unexpected_journey",
    "there_and_back_again",
    "safe_travels",
    "fight_and_flight",
    "hurry_up",
    "the_ultimate_trick",
)
XARGATHA_EFFECTS = (
    "cleave",
    "threatening_slash",
    "deadly_swipe",
    "lethal_spin",
    "long_thrust",
    "rapid_thrusts",
    "sirens_call",
    "charm",
    "control",
    "dominate",
    "final_embrace",
    "constrict",
    "turn_into_statues",
    "petrifying_stare",
    "stone_gaze",
    "metamorphosis",
    "devoted_followers",
    "fresh_converts",
)
IGNATIA_COIN_RECIPES = dict.fromkeys(
    ("playing_with_fire", "erratic_fireblast", "loosely_aimed_firebolts",
     "crack_of_doom", "imminent_eruption", "chaos_bolt",
     "unstable_portal", "chaos_gate", "abrupt_combustion",
     "spontaneous_immolation", "violent_conflagration",
     "searing_heat", "scorching_blaze", "path_of_ashes",
     "path_of_cinders", "path_of_flames"), "Ignatia",
)

LIVE_ONLY_EFFECTS = {
    "magic_trick": "Razzle",
    "aaaand_its_gone": "Razzle",
    "phantom_strike": "Razzle",
    "hit_and_gone": "Razzle",
    "into_thin_air": "Razzle",
    "crowd_control": "Razzle",
    "rummage": "Razzle",
    "ransack": "Razzle",
    "tightrope": "Razzle",
    "high_wire": "Razzle",
    "wire_dancers": "Razzle",
    "diabolical_laughter": "NebKher",
    "an_illusion_of_choice": "NebKher",
    "phantasmal_sentry": "NebKher",
    "phantasmal_warrior": "NebKher",
    "phantasmal_champion": "NebKher",
    "fleeting_image": "NebKher",
    "multiple_projections": "NebKher",
    "master_of_illusions": "NebKher",
    "twist_fate": "NebKher",
    "devious_scheme": "NebKher",
    "illusionary_force": "NebKher",
    "illusionary_army": "NebKher",
    "imbue_doubt": "NebKher",
    "time_to_reconsider": "NebKher",
    "makeshift_minigun": "Trinkets",
    "gatling_gun": "Trinkets",
    "supercharged_cannon": "Trinkets",
    "steam_discharge": "Trinkets",
    "flame_belcher": "Trinkets",
    "salvage_parts": "Trinkets",
    "rapid_redeployment": "Trinkets",
    "early_prototype": "Trinkets",
    "updated_design": "Trinkets",
    "perfected_design": "Trinkets",
    "self_destruct": "Trinkets",
    "emergency_protocol": "Trinkets",
    "disruptor_jolt": "Trinkets",
    "disruptor_pulse": "Trinkets",
    "disruptor_grid": "Trinkets",
    "deployable_barrier": "Trinkets",
    "deployable_bastion": "Trinkets",
    "drag_off": "Widget",
    "carry_away": "Widget",
    "airborne_attack": "Widget",
    "airborne_assault": "Widget",
    "diversionary_strike": "Widget",
    "diversionary_attack": "Widget",
    "diversionary_assault": "Widget",
    "fiery_breath": "Widget",
    "flaming_breath": "Widget",
    "scorching_breath": "Widget",
    "nibble": "Widget",
    "gnaw": "Widget",
    "safe_landing": "Widget",
    "dragon_bond": "Widget",
    "take_off": "Widget",
    "all_aboard": "Widget",
    "jinx": "Cordelia",
    "bewitch": "Cordelia",
    "trouble_brewing": "Cordelia",
    "recipe_for_disaster": "Cordelia",
    "charmed_step": "Cordelia",
    "candy_trail": "Cordelia",
    "enchanted_path": "Cordelia",
    "healing_spores": "Cordelia",
    "fungal_favor": "Cordelia",
    "toxic_tranquility": "Cordelia",
    "vile_vial": "Cordelia",
    "potion_explosion": "Cordelia",
    "collateral_misfortune": "Cordelia",
    "fatal_bonds": "Cordelia",
    "broom_for_improvement": "Cordelia",
    "broomstick_beatdown": "Cordelia",
    "this_is_my_broomstick": "Cordelia",
    "equilibrium": "Ignatia",
    "unbound_fury": "Ursafar",
    "inscribe_the_runes": "Snorri",
    "shadow_step": "Whisper",
    "shadow_walk": "Whisper",
    "creeping_shadow": "Whisper",
    "seeds_of_fear": "Whisper",
    "sprouts_of_panic": "Whisper",
    "blooming_nightmare": "Whisper",
    "cruel_twist": "Whisper",
    "sealed_fate": "Whisper",
    "blood_fury": "Whisper",
    "blood_frenzy": "Whisper",
    "lesser_evil": "Whisper",
    "greater_good": "Whisper",
    "grim_reaper": "Whisper",
    "tread_lightly": "Brynn",
    "cover_tracks": "Brynn",
    "hide_traces": "Brynn",
    "brynn_high_ground": "Brynn",
    "elevated_ambush": "Brynn",
    "peak_precision": "Brynn",
    "familiar_ground": "Brynn",
    "bear_trap": "Brynn",
    "log_trap": "Brynn",
    "deadfall_trap": "Brynn",
    "true_grit": "Brynn",
    "die_hard": "Brynn",
    "split_attack": "Brynn",
    "split_throw": "Brynn",
    "mountain_guide": "Brynn",
    "expedition_leader": "Brynn",
    "decoy": "Brynn",
    "trace": "Garrus",
    "chase": "Garrus",
    "hunt_down": "Garrus",
    "menace": "Garrus",
    "threaten": "Garrus",
    "terrify": "Garrus",
    "hold_ground": "Garrus",
    "make_a_stand": "Garrus",
    "battle_ready": "Garrus",
    "light_pilum": "Garrus",
    "heavy_pilum": "Garrus",
    "angry_strike": "Garrus",
    "chilling_howl": "Garrus",
    "form_up": "Garrus",
    "testudo": "Garrus",
    "blunt_force": "Garrus",
    "send_flying": "Garrus",
    "clear_shot": "Silverarrow",
    "opportunity_shot": "Silverarrow",
    "snap_shot": "Silverarrow",
    "long_shot": "Silverarrow",
    "rain_of_arrows": "Silverarrow",
    "grappling_branches": "Silverarrow",
    "entangling_vines": "Silverarrow",
    "grasping_roots": "Silverarrow",
    "natures_blessing": "Silverarrow",
    "fae_healing": "Silverarrow",
    "disorient": "Silverarrow",
    "lead_astray": "Silverarrow",
    "divert_attention": "Silverarrow",
    "shoot_and_scoot": "Silverarrow",
    "trailblazer": "Silverarrow",
    "warning_shot": "Silverarrow",
    "treetop_sentinel": "Silverarrow",
    "come_to_aid": "Takahide",
    "bring_the_relief": "Takahide",
    "commit_reserves": "Takahide",
    "pledge_of_allegiance": "Takahide",
    "loyal_retainer": "Takahide",
    "calculated_risk": "Takahide",
    "tactical_gambit": "Takahide",
    "proven_warrior": "Takahide",
    "chosen_champion": "Takahide",
    "the_right_hand": "Takahide",
    "set_an_example": "Takahide",
    "lead_from_the_front": "Takahide",
    "hold_my_sake": "Takahide",
    "spinning_blade": "Takahide",
    "blade_helix": "Takahide",
    "temporal_punch": "Emmitt",
    "temporal_slam": "Emmitt",
    "temporal_judgment": "Emmitt",
    "reverse_time": "Emmitt",
    "time_loop": "Emmitt",
    "time_warp": "Emmitt",
    "time_walk": "Emmitt",
    "fast_forward": "Emmitt",
    "back_to_the_future": "Emmitt",
    "time_snare": "Emmitt",
    "time_trap": "Emmitt",
    "time_bomb": "Emmitt",
    "flashback": "Emmitt",
    "deja_vu": "Emmitt",
    "unstable_timeline": "Emmitt",
    "awaken": "Mortimer",
    "stage_dive": "Mortimer",
    "crowd_drift": "Mortimer",
    "gathering_horde": "Mortimer",
    "crawling_dead": "Mortimer",
    "walking_dead": "Mortimer",
    "braains": "Mortimer",
    "corpse_slam": "Mortimer",
    "morbid_mosh": "Mortimer",
    "robbing_zombies": "Mortimer",
    "master_of_puppets": "Mortimer",
    "ready_for_war": "Takahide",
    "drinking_buddies": "Bain",
    "another_one": "Bain",
    "a_game_of_chance": "Bain",
    "dead_mans_hand": "Bain",
    "were_not_done_yet": "Bain",
    "snipe": "Swift",
    "prepared_shot": "Swift",
    "killshot": "Swift",
    "shotgun": "Swift",
    "super_shotgun": "Swift",
    "steam_jump": "Swift",
    "assault_jump": "Swift",
    "suppress": "Swift",
    "pin_down": "Swift",
    "killing_ground": "Swift",
    "mark_for_death": "Swift",
    "hunting_season": "Swift",
    "delayed_jump": "Swift",
    "mobile_scout": "Swift",
    "drop_trooper": "Swift",
    "accept_surrender": "Rowenna",
    "glorious_triumph": "Rowenna",
    "opening_shots": "Rowenna",
    "opening_volley": "Rowenna",
    "stand_guard": "Rowenna",
    "protect_the_weak": "Rowenna",
    "defend_the_innocent": "Rowenna",
    "close_quarters": "Rowenna",
    "melee": "Rowenna",
    "grand_melee": "Rowenna",
    "throw_the_gauntlet": "Rowenna",
    "seismic_slam": "Mrak",
    "seismic_assault": "Mrak",
    "epicenter": "Mrak",
    "treacherous_ground": "Mrak",
    "rockslide": "Mrak",
    "avalanche": "Mrak",
    "stomping_step": "Mrak",
    "ground_shaker": "Mrak",
    "stone_grip": "Mrak",
    "fissure": "Mrak",
    "stone_carapace": "Mrak",
    "rock_solid": "Mrak",
    "boulder_rush": "Mrak",
    "boulder_blitz": "Mrak",
    "boulderdozer": "Mrak",
    "rolling_stone": "Mrak",
    "strolling_stone": "Mrak",
    "weakness": "Dodger",
    "enfeeblement": "Dodger",
    "burning_skull": "Dodger",
    "blazing_skull": "Dodger",
    "littlefinger_of_death": "Dodger",
    "finger_of_death": "Dodger",
    "middlefinger_of_death": "Dodger",
    "toss_away": "Wuk",
    "mighty_throw": "Wuk",
    "monstrous_throw": "Wuk",
    "into_the_canopy": "Wuk",
    "treetop_ride": "Wuk",
    "claim_dominance": "Wuk",
    "assert_dominance": "Wuk",
    "gifts_of_nature": "Wuk",
    "tree_of_plenty": "Wuk",
    "abundance": "Wuk",
    "natures_protector": "Wuk",
    "natures_guardian": "Wuk",
    "natures_champion": "Wuk",
    "mystic_saplings": "Wuk",
    "tree_slam": "Wuk",
    "trample": "Wuk",
    "angry_stampede": "Wuk",
    "glacial_barrier": "Tali",
    "wall_of_frost": "Tali",
    "pack_ice": "Tali",
    "ancestral_totem": "Tali",
    "venerated_totem": "Tali",
    "ice_blast": "Tali",
    "commune_with_spirits": "Tali",
    "cold_snap": "Tali",
    "snowstorm": "Tali",
}
SNORRI_STATEFUL_EFFECTS = {effect: "Snorri" for effect in (
    "safe_passage", "hidden_passage", "deep_passage",
    "runic_dagger", "runic_hammer", "runic_battleaxe",
    "runecaster", "runeblaster", "runetrap", "runebomb", "ancestral_boon", "ancestral_grace", "rune_sigils",
)}
WHISPER_ULTIMATE_EFFECTS = dict.fromkeys(("death_seeker", "swift_justice"), "Whisper")
WHISPER_TRAIL_EFFECTS = {
    "crimson_trail": 1,
    "sanguine_path": 2,
    "blood_pilgrimage": 3,
}
TAKAHIDE_GOLD_EFFECTS = dict.fromkeys(
    ("float_like_a_butterfly", "sting_like_a_bee", "strike_like_a_tiger", "bushido"), "Takahide"
)
EMMITT_RETRIEVAL_EFFECTS = dict.fromkeys(("time_capsule", "future_proof"), "Emmitt")
MORTIMER_ULTIMATE_EFFECTS = dict.fromkeys(("crowd_surf", "army_of_darkness", "racing_dead", "knife_of_the_living_dead", "braaaaaaaains", "macabre_mayhem", "stalking_scalpers"), "Mortimer")
BAIN_BOUNTY_MOVEMENT = dict.fromkeys(("vantage_point", "high_ground"), "Bain")
ROWENNA_LANCE_ATTACKS = dict.fromkeys(
    ("token_of_gratitude", "fair_share", "paragon_of_grace", "feat_of_bravery", "paragon_of_valor", "code_of_chivalry"),
    "Rowenna",
)
TALI_WINTER_WEAPONS = dict.fromkeys(("winter_dagger", "winter_spear", "winter_scepter"), "Tali")
URSAFAR_RAGE_RECIPES = dict.fromkeys(
    ("sniff_out", "eyes_on_the_prey", "apex_predator", "rip", "cold_ire", "eyes_of_flame",
     "prey_drive", "prey_abundance", "feeding_frenzy",
     "prowling_brute", "rampaging_beast", "unstoppable_force", "claws_that_catch", "tear",
     "instinctive_reaction", "evolutionary_response", "angry_roar"),
    "Ursafar",
)
ALL_EFFECTS = {
    **URSAFAR_RAGE_RECIPES,
    **IGNATIA_COIN_RECIPES,
    "fight_as_one": "Widget",
    "crack_in_reality": "NebKher",
    "shift_reality": "NebKher",
    "theatrics": "Razzle",
    "spectacle": "Razzle",
    "stunt_doubles": "Razzle",
    "alleyoop": "Razzle",
    "group_performance": "Razzle",
    "team_spirit": "Razzle",
    "guardian_spirit": "Tali",
    "warrior_spirit": "Tali",
    "spirit_wolf": "Tali",
    "spirit_bear": "Tali",
    "blizzard": "Tali",
    **TALI_WINTER_WEAPONS,
    "death_trap": "Dodger",
    "dread_razor": "Dodger",
    "dark_ritual": "Dodger",
    "darker_ritual": "Dodger",
    "darkest_ritual": "Dodger",
    "necromancy": "Dodger",
    "necromastery": "Dodger",
    **ROWENNA_LANCE_ATTACKS,
    "bounce": "Swift",
    "reload": "Swift",
    **BAIN_BOUNTY_MOVEMENT,
    **MORTIMER_ULTIMATE_EFFECTS,
    **EMMITT_RETRIEVAL_EFFECTS,
    **TAKAHIDE_GOLD_EFFECTS,
    **EFFECTS, **MISA_STATEFUL_EFFECTS, **MIN_STATEFUL_EFFECTS,
    **LIVE_ONLY_EFFECTS, **SNORRI_STATEFUL_EFFECTS,
    **WHISPER_ULTIMATE_EFFECTS, **dict.fromkeys(WHISPER_TRAIL_EFFECTS, "Whisper"),
}
DENSE_EFFECT_IDS = (
    "bullrush",
    "furious_charge",
    "throwing_axe",
    "throwing_spear",
    "shield",
    "bolster",
    "fortify",
    "war_drummer",
    "master_skald",
    "bulwark",
    "one_man_army",
    "backstab",
    "backstab_with_a_ballista",
    "combat_reflexes",
    "light_fingered",
    "pick_pocket",
    "master_thief",
    "blend_into_shadows",
    "mind_grip",
    "spell_break",
    "ebb_and_flow",
    "dangerous_current",
    "raging_stream",
    "violent_torrent",
    "slippery_ground",
    "deluge",
    "noble_blade",
    "arcane_whirlpool",
    "liquid_leap",
    "magical_current",
    "stranger_tide",
    "onslaught",
    "hit_and_run",
    "leaping_strike",
    "poisoned_dagger",
    "poisoned_dart",
    "rogue_wave",
    "tidal_blast",
    "blink_strike",
    "mad_dash",
    "brutal_jab",
    "mighty_punch",
    "savage_kick",
    "point_blank_shot",
    "back_to_back",
    "listen_up",
    "roger_roger",
    "ready_and_waiting",
    "troop_movement",
    "marching_orders",
    "path_to_victory",
    "close_support",
    "covering_fire",
    "shootout",
    "bullet_hell",
    "quickdraw",
    "gunslinger",
    "dead_shot",
    "shield_bash",
    "counterattack",
    "steady_advance",
    "unwavering_resolve",
    "devoted_followers",
    "fresh_converts",
)
DENSE_EFFECTS = {effect: EFFECTS[effect] for effect in DENSE_EFFECT_IDS}


CHARGE_ATTACKS = ("daring_strike", "bold_thrust", "fearless_lunge")
CHARGE_PRIMARY_PROBE = 1234568100
PRIMARY_PLUS_TWO = 1234568102


def bind_charge_damage(effect_id, fields):
    if effect_id not in CHARGE_ATTACKS:
        return
    attacks = [row for row in fields if row["type"] == "attack_sequence"]
    assert [row["damage"] for row in attacks] == [CHARGE_PRIMARY_PROBE + 2, CHARGE_PRIMARY_PROBE]
    for attack, operand in zip(attacks, (PRIMARY_PLUS_TWO, PRIMARY), strict=True):
        attack["damage"] = operand


def build_catalog():
    from model_traces import setup

    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.effects import CardEffectRegistry
    from goa2.engine.stats import CardStats

    state = setup()
    recipes = {}
    ranged_steps = {}
    radius_fallbacks = {}
    range_fallbacks = {}
    from primary_radius import bind_radius_fallbacks, bind_range_fallbacks, merge_operand_bindings

    for effect_id, name in EFFECTS.items():
        primary = CHARGE_PRIMARY_PROBE if effect_id in CHARGE_ATTACKS else PRIMARY
        hero = HeroRegistry.get(name).model_copy(update={"id": HERO})
        card = next(
            c
            for c in [*hero.deck, *([hero.ultimate_card] if hero.ultimate_card else [])]
            if c.effect_id == effect_id
        ).model_copy(update={"id": CARD, "is_facedown": False})
        stats = CardStats(primary_value=primary, range=RANGE, radius=RADIUS)
        steps = CardEffectRegistry.get(effect_id).get_steps_with_stats(state, hero, card, stats)
        wires = [step.model_dump(mode="json") for step in steps]

        def labels(wire):
            if isinstance(wire, dict):
                for key, value in wire.items():
                    if key == "step_id":
                        wire[key] = "native-step-primary-template"
                    else:
                        labels(value)
            elif isinstance(wire, list):
                for value in wire:
                    labels(value)

        for wire in wires:
            labels(wire)
        variants = []
        for ranged in (False, True):
            changed = card.model_copy(update={"is_ranged": ranged})
            changed_steps = CardEffectRegistry.get(effect_id).get_steps_with_stats(
                state, hero, changed, stats
            )
            variant = [step.model_dump(mode="json") for step in changed_steps]
            for wire in variant:
                labels(wire)
            variants.append(variant)
        assert len(variants[0]) == len(
            variants[1]
        ), "stateful ranged recipe shape needs an explicit native port"
        bindings = []
        for i, (a, b) in enumerate(zip(*variants, strict=True)):
            if a != b:
                assert (
                    a["type"] == b["type"] == "attack_sequence" and a["is_ranged"] != b["is_ranged"]
                )
                assert {**a, "is_ranged": False} == {
                    **b,
                    "is_ranged": False,
                }, "unsupported ranged-dependent recipe"
                bindings.append(i)
        ranged_steps[effect_id] = bindings
        variants = []
        for radius in (None, 0):
            other = CardEffectRegistry.get(effect_id).get_steps_with_stats(
                state, hero, card, CardStats(primary_value=primary, range=RANGE, radius=radius)
            )
            variant = [step.model_dump(mode="json") for step in other]
            for wire in variant:
                labels(wire)
            variants.append(variant)
        radius_fields = bind_radius_fallbacks(
            wires, variants[0], variants[1], RADIUS, radius_fallbacks
        )
        zero_steps = CardEffectRegistry.get(effect_id).get_steps_with_stats(
            state, hero, card, CardStats(primary_value=primary, range=0, radius=RADIUS)
        )
        zero_wires = [step.model_dump(mode="json") for step in zero_steps]
        for wire in zero_wires:
            labels(wire)
        # Radius substitutions occur in identical positions in both range variants.
        range_fields = bind_range_fallbacks(wires, zero_wires, RANGE, range_fallbacks)
        recipes[effect_id] = merge_operand_bindings(wires, radius_fields, range_fields, RANGE)
        bind_charge_damage(effect_id, recipes[effect_id])
    source = {
        "schema_version": 3,
        "source_sha256": verify_sources()["source_sha256"],
        "operands": {
            "hero": HERO,
            "card": CARD,
            "primary": PRIMARY,
            "range": RANGE,
            "radius": RADIUS,
        },
        "recipes": recipes,
        "ranged_steps": ranged_steps,
        "radius_fallbacks": radius_fallbacks,
        "range_fallbacks": range_fallbacks,
    }
    from primary_operand_wire import encode_primary_operands

    source = encode_primary_operands(
        source, primary=PRIMARY, range_=RANGE, radius=RADIUS,
        primary_plus_two=PRIMARY_PLUS_TWO,
    )
    payload = {**source, "catalog_sha256": digest(source)}
    return payload


def export():
    payload = build_catalog()
    recipes = payload["recipes"]
    write_text(
        ROOT / "rust/content/catalogs/primary-effects.json", json.dumps(payload, indent=2) + "\n"
    )
    print(
        {
            "effects": len(recipes),
            "steps": sum(map(len, recipes.values())),
            "catalog_sha256": payload["catalog_sha256"],
        }
    )


if __name__ == "__main__":
    export()
