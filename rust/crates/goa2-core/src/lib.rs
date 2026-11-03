//! Native rules primitives. No Python calls or service dependencies.
#![forbid(unsafe_code)]
pub mod batch;
pub mod bitset;
pub mod cards;
pub mod codec;
pub mod data;
pub mod diagnostics;
pub mod geometry;
pub mod ids;
pub mod interpreter;
pub mod lifecycle;
pub mod planning;
pub mod spatial;
pub mod stats;
pub mod world;

pub mod actions;
pub mod zones;

pub mod targeting;

pub mod combat;

pub mod history;
pub mod reactions;

pub mod attack;

pub mod markers;

pub mod card_selection;

pub mod primary;

mod templates;

pub mod displacement;

pub mod minion_return;

pub mod battle;

pub mod turn;

pub mod round;

pub mod auras;

pub mod passives;

pub mod queries;

pub mod retrieval;

pub mod effects;

pub mod effect_steps;

mod effect_record;
mod effect_rules;

pub mod validation;

pub mod terrain;

pub mod mines;

mod protection;

pub mod objects;

pub mod equality;

mod pre_action;

pub mod spells;
