//! Typed numeric bindings used by generated primary recipes.

use serde::Deserialize;
use serde_json::Value;

use crate::stats::CardStats;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Operand {
    Primary,
    PrimaryAdd { amount: i64 },
    Range,
    RangeOrDefault { default: i64 },
    Radius,
    RadiusOrZero,
    RadiusOrDefault { default: i64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireOperand {
    #[serde(rename = "$operand")]
    operand: Operand,
}

impl Operand {
    pub(crate) fn parse(value: &Value) -> Result<Option<Self>, String> {
        if value.get("$operand").is_none() {
            return Ok(None);
        }
        WireOperand::deserialize(value)
            .map(|wire| Some(wire.operand))
            .map_err(|error| format!("invalid primary operand: {error}"))
    }

    pub(crate) fn resolve(self, stats: &CardStats) -> Result<Option<i64>, String> {
        let resolved = match self {
            Self::Primary => stats.primary_value,
            Self::PrimaryAdd { amount } => stats
                .primary_value
                .checked_add(amount)
                .ok_or("primary operand addition exceeds i64")?,
            Self::Range => stats.range,
            Self::RangeOrDefault { default } => {
                if stats.range == 0 {
                    default
                } else {
                    stats.range
                }
            }
            Self::Radius => return Ok(stats.radius),
            Self::RadiusOrZero => stats.radius.unwrap_or(0),
            Self::RadiusOrDefault { default } => stats
                .radius
                .filter(|radius| *radius != 0)
                .unwrap_or(default),
        };
        Ok(Some(resolved))
    }

    pub(crate) fn required(self, stats: &CardStats) -> Result<i64, String> {
        self.resolve(stats)?
            .ok_or_else(|| "required primary radius is absent".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_operands_preserve_absent_zero_negative_and_overflow_cases() {
        let mut stats = CardStats {
            primary_value: -3,
            range: -1,
            radius: None,
        };
        assert_eq!(Operand::PrimaryAdd { amount: 2 }.required(&stats), Ok(-1));
        assert_eq!(Operand::Radius.resolve(&stats), Ok(None));
        assert!(Operand::Radius.required(&stats).is_err());
        assert_eq!(Operand::RadiusOrZero.required(&stats), Ok(0));
        assert_eq!(
            Operand::RadiusOrDefault { default: 2 }.required(&stats),
            Ok(2)
        );
        assert_eq!(
            Operand::RangeOrDefault { default: 3 }.required(&stats),
            Ok(-1)
        );

        stats.range = 0;
        stats.radius = Some(0);
        assert_eq!(Operand::Radius.resolve(&stats), Ok(Some(0)));
        assert_eq!(
            Operand::RadiusOrDefault { default: 2 }.required(&stats),
            Ok(2)
        );
        assert_eq!(
            Operand::RangeOrDefault { default: 3 }.required(&stats),
            Ok(3)
        );

        stats.radius = Some(-1);
        stats.primary_value = i64::MAX;
        assert_eq!(
            Operand::RadiusOrDefault { default: 2 }.required(&stats),
            Ok(-1)
        );
        assert!(Operand::PrimaryAdd { amount: 3 }.required(&stats).is_err());
    }

    #[test]
    fn typed_operand_parser_distinguishes_tags_from_ordinary_data() {
        assert_eq!(Operand::parse(&json!(1234568103)), Ok(None));
        assert_eq!(
            Operand::parse(&json!({"$operand": {"kind": "radius_or_zero"}})),
            Ok(Some(Operand::RadiusOrZero))
        );
        assert!(Operand::parse(&json!({"$operand": {"kind": "primary_add"}})).is_err());
        assert!(Operand::parse(&json!({"$operand": {"kind": "range"}, "extra": 1})).is_err());
    }
}
