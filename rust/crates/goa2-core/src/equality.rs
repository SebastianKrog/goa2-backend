//! Python-compatible equality for card models and their JSON metadata.
//! This is rule state: private revisions can change list-removal precedence.
use crate::{cards::CardId, data::Datum, world::NativeWorld};

fn integer_float(integer: i64, float: f64) -> bool {
    float.is_finite()
        && float.fract() == 0.0
        && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&float)
        && float as i64 == integer
}
pub fn python_json_equal(a: &Datum, b: &Datum) -> bool {
    match (a, b) {
        (Datum::Null, Datum::Null) => true,
        (Datum::Bool(a), Datum::Bool(b)) => a == b,
        (Datum::Int(a), Datum::Int(b)) => a == b,
        (Datum::Float(a), Datum::Float(b)) => a == b,
        (Datum::Bool(a), Datum::Int(b)) | (Datum::Int(b), Datum::Bool(a)) => i64::from(*a) == *b,
        (Datum::Int(a), Datum::Float(b)) | (Datum::Float(b), Datum::Int(a)) => {
            integer_float(*a, *b)
        }
        (Datum::Bool(a), Datum::Float(b)) | (Datum::Float(b), Datum::Bool(a)) => {
            integer_float(i64::from(*a), *b)
        }
        (Datum::Text(a), Datum::Text(b)) => a == b,
        (Datum::List(a), Datum::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| python_json_equal(a, b))
        }
        (Datum::Object(a), Datum::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    b.iter()
                        .find(|(other, _)| other == key)
                        .is_some_and(|(_, other)| python_json_equal(value, other))
                })
        }
        (Datum::Instance(a), Datum::Instance(b)) => a == b,
        _ => false,
    }
}
impl NativeWorld {
    pub fn card_model_equal(&self, a: CardId, b: CardId) -> Result<bool, String> {
        let left = self.card(a).ok_or("equality card missing")?;
        let right = self.card(b).ok_or("equality card missing")?;
        for card in [left, right] {
            if let Some(equality) = card.equality {
                let expected = if self.definitions.cards[card.definition].spell {
                    self.definitions.spell_model
                } else {
                    self.definitions.card_model
                };
                if equality.model != expected {
                    return Err("card equality model class not compiled".into());
                }
            }
        }
        if a == b {
            return Ok(true);
        }
        if self.definitions.cards[left.definition].spell
            != self.definitions.cards[right.definition].spell
        {
            return Ok(false);
        }
        if let (Some(a), Some(b)) = (left.equality, right.equality)
            && (a.model != b.model || a.revision != b.revision)
        {
            return Ok(false);
        }
        if left.lifecycle != right.lifecycle
            || left.facedown != right.facedown
            || left.played_this_round != right.played_this_round
            || left.active != right.active
            || left.enraged != right.enraged
            || left.passive_uses != right.passive_uses
            || left.initiative_override != right.initiative_override
        {
            return Ok(false);
        }
        if left.definition != right.definition
            && !python_json_equal(
                &self.definitions.card_equality_fields[left.definition],
                &self.definitions.card_equality_fields[right.definition],
            )
        {
            return Ok(false);
        }
        let (Some(a), Some(b)) = (left.equality, right.equality) else {
            return Err("distinct equal-looking cards require private equality metadata".into());
        };
        if !a.json_metadata || !b.json_metadata {
            return Err("non-JSON physical card metadata equality is not bound".into());
        }
        Ok(python_json_equal(&left.metadata, &right.metadata))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Symbols;
    #[test]
    fn python_numbers_do_not_round_large_integers_before_comparing() {
        assert!(python_json_equal(&Datum::Bool(true), &Datum::Float(1.0)));
        assert!(python_json_equal(&Datum::Bool(false), &Datum::Float(-0.0)));
        assert!(!python_json_equal(
            &Datum::Int(9_007_199_254_740_993),
            &Datum::Float(9_007_199_254_740_992.0)
        ));
        assert!(!python_json_equal(
            &Datum::Int(i64::MAX),
            &Datum::Float(9_223_372_036_854_775_808.0)
        ));
        assert!(python_json_equal(
            &Datum::Int(i64::MIN),
            &Datum::Float(-9_223_372_036_854_775_808.0)
        ));
        assert!(!python_json_equal(&Datum::Int(1), &Datum::Float(1.5)));
    }
    #[test]
    fn metadata_dictionary_order_is_ignored_but_list_order_is_preserved() {
        let mut symbols = Symbols::default();
        let a = symbols.intern("a");
        let b = symbols.intern("b");
        assert!(python_json_equal(
            &Datum::Object(vec![(a, Datum::Bool(true)), (b, Datum::Int(2))]),
            &Datum::Object(vec![(b, Datum::Float(2.0)), (a, Datum::Int(1))])
        ));
        assert!(!python_json_equal(
            &Datum::List(vec![Datum::Int(1), Datum::Int(2)]),
            &Datum::List(vec![Datum::Int(2), Datum::Int(1)])
        ));
    }
}
