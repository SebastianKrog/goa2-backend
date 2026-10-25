//! Noninteractive entity queries and scalar context conditions.
//! Candidate filtering is shared with selection, without its targeting validation.
use crate::{
    data::{Datum, Symbols},
    primary::{PrimaryOperands, PrimaryValues},
    spatial::{HexFilter, SpatialContext, compile_filter},
    targeting::{UnitCandidates, UnitFilter, compile_effective_unit_filters},
};
use serde_json::Value;
use std::cmp::Ordering;
#[derive(Clone, Debug)]
pub enum Query {
    Units {
        tokens: bool,
        filters: Vec<UnitFilter>,
    },
    Hexes(Vec<HexFilter>),
    Empty,
}
impl Query {
    pub fn compile(raw: &Value, symbols: &mut Symbols, collect: bool) -> Result<Self, String> {
        Ok(match raw["target_type"].as_str().unwrap_or("UNIT") {
            "UNIT" | "UNIT_OR_TOKEN" => Self::Units {
                tokens: raw["target_type"] == "UNIT_OR_TOKEN",
                filters: compile_effective_unit_filters(raw, symbols, !collect)?,
            },
            "HEX" if !collect => Self::Hexes(
                raw["filters"]
                    .as_array()
                    .ok_or("count filters missing")?
                    .iter()
                    .map(|f| compile_filter(f, symbols))
                    .collect::<Result<_, _>>()?,
            ),
            "HEX" | "CARD" | "NUMBER" | "TOKEN" => Self::Empty,
            _ => return Err("query target type not compiled".into()),
        })
    }
    pub fn empty(collect: bool) -> Datum {
        if collect {
            Datum::List(Vec::new())
        } else {
            Datum::Int(0)
        }
    }
    pub fn evaluate(&self, context: &SpatialContext<'_>, collect: bool) -> Result<Datum, String> {
        match self {
            Self::Empty => Ok(Self::empty(collect)),
            Self::Units { tokens, filters } => {
                // Count/Collect do not call can_be_targeted, unlike SelectStep.
                let candidates = context.world.select_units(
                    if *tokens {
                        UnitCandidates::UnitsAndTokens
                    } else {
                        UnitCandidates::Units
                    },
                    None,
                    filters,
                    context,
                )?;
                if collect {
                    Ok(Datum::List(
                        candidates.into_iter().map(Datum::Text).collect(),
                    ))
                } else {
                    i64::try_from(candidates.len())
                        .map(Datum::Int)
                        .map_err(|_| "count outside i64".into())
                }
            }
            Self::Hexes(filters) => {
                let candidates = context.world.tile_hexes();
                let mut accepted = vec![true; candidates.len()];
                for filter in filters {
                    if !accepted.iter().any(|v| *v) {
                        break;
                    }
                    filter.apply_scan(context, candidates, &mut accepted)?;
                }
                i64::try_from(accepted.into_iter().filter(|v| *v).count())
                    .map(Datum::Int)
                    .map_err(|_| "count outside i64".into())
            }
        }
    }
    pub fn bind(&mut self, slots: &PrimaryOperands, values: &PrimaryValues) -> Result<(), String> {
        match self {
            Self::Units { filters, .. } => {
                for f in filters {
                    slots.unit_filter(f, values)?;
                }
            }
            Self::Hexes(filters) => {
                for f in filters {
                    slots.hex_filter(f, values)?;
                }
            }
            Self::Empty => {}
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Comparison {
    Ge,
    Gt,
    Eq,
    Le,
    Lt,
    Ne,
    Never,
}
impl Comparison {
    pub fn parse(s: &str) -> Self {
        match s {
            ">=" => Self::Ge,
            ">" => Self::Gt,
            "==" => Self::Eq,
            "<=" => Self::Le,
            "<" => Self::Lt,
            "!=" => Self::Ne,
            _ => Self::Never,
        }
    }
    pub fn evaluate(self, value: Option<&Datum>, threshold: i64) -> Result<bool, String> {
        let order = match value {
            Some(Datum::Int(v)) => v.cmp(&threshold),
            Some(Datum::Bool(v)) => i64::from(*v).cmp(&threshold),
            Some(Datum::Float(v)) => {
                if !v.is_finite() {
                    return Err("nonfinite context comparison not compiled".into());
                }
                // Avoid converting an arbitrary i64 threshold to f64: Python
                // compares float/int exactly even above the 53-bit mantissa.
                if *v < i64::MIN as f64 {
                    Ordering::Less
                } else if *v >= 9_223_372_036_854_775_808.0 {
                    Ordering::Greater
                } else {
                    let integer = v.trunc() as i64;
                    let order = integer.cmp(&threshold);
                    if order == Ordering::Equal {
                        v.fract().partial_cmp(&0.0).unwrap()
                    } else {
                        order
                    }
                }
            }
            _ => 0.cmp(&threshold),
        };
        Ok(match self {
            Self::Ge => order != Ordering::Less,
            Self::Gt => order == Ordering::Greater,
            Self::Eq => order == Ordering::Equal,
            Self::Le => order != Ordering::Greater,
            Self::Lt => order == Ordering::Less,
            Self::Ne => order != Ordering::Equal,
            Self::Never => false,
        })
    }
}
