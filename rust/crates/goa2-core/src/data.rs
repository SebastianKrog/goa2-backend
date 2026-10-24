//! Search-local, typed values and interned symbols. JSON conversion is boundary-only.
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol(pub(crate) usize);
#[derive(Clone, Debug, Default)]
pub struct Symbols {
    names: Vec<String>,
    ids: HashMap<String, Symbol>,
}
impl Symbols {
    pub fn intern(&mut self, name: &str) -> Symbol {
        if let Some(&id) = self.ids.get(name) {
            return id;
        }
        let id = Symbol(self.names.len());
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), id);
        id
    }
    pub fn name(&self, id: Symbol) -> &str {
        &self.names[id.0]
    }
    pub fn lookup(&self, name: &str) -> Option<Symbol> {
        self.ids.get(name).copied()
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Datum {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(Symbol),
    List(Vec<Datum>),
    Object(Vec<(Symbol, Datum)>),
    Instance(u32),
}
impl Datum {
    pub fn from_json(value: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        Ok(match value {
            Value::Null => Self::Null,
            Value::Bool(v) => Self::Bool(*v),
            Value::Number(v) => {
                if v.is_f64() {
                    Self::Float(v.as_f64().ok_or("invalid float")?)
                } else {
                    Self::Int(v.as_i64().ok_or("unsupported integer outside i64")?)
                }
            }
            Value::String(v) => Self::Text(symbols.intern(v)),
            Value::Array(items) => Self::List(
                items
                    .iter()
                    .map(|v| Self::from_json(v, symbols))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(fields) if fields.contains_key("$goa2_instance") => {
                if fields.len() != 1 {
                    return Err("malformed physical instance reference".into());
                }
                Self::Instance(
                    fields["$goa2_instance"]
                        .as_u64()
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or("invalid instance index")?,
                )
            }
            Value::Object(fields) => Self::Object(
                fields
                    .iter()
                    .map(|(key, v)| Ok((symbols.intern(key), Self::from_json(v, symbols)?)))
                    .collect::<Result<_, String>>()?,
            ),
        })
    }
    pub fn to_json(&self, symbols: &Symbols) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(v) => Value::Bool(*v),
            Self::Int(v) => Value::from(*v),
            Self::Float(v) => Value::from(*v),
            Self::Text(id) => Value::String(symbols.name(*id).into()),
            Self::List(items) => Value::Array(items.iter().map(|v| v.to_json(symbols)).collect()),
            Self::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, v)| (symbols.name(*key).into(), v.to_json(symbols)))
                    .collect::<Map<_, _>>(),
            ),
            Self::Instance(id) => serde_json::json!({"$goa2_instance":id}),
        }
    }
    /// Pydantic's cube coordinate conversion is stricter than Select NUMBER's
    /// int() conversion: fractional floats and exponent strings are invalid.
    pub fn hex(
        &self,
        keys: [Symbol; 3],
        symbols: &Symbols,
    ) -> Result<crate::geometry::Hex, String> {
        let Self::Object(fields) = self else {
            return Err("hex requires an object".into());
        };
        let mut result = [0; 3];
        for (i, key) in keys.into_iter().enumerate() {
            let value = fields
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, v)| v)
                .ok_or("hex coordinate missing")?;
            let number = match value {
                Self::Float(v) if v.fract() != 0.0 => None,
                Self::Text(id) => {
                    let text = symbols.name(*id).trim();
                    let integer = if let Some((integer, fraction)) = text.split_once('.') {
                        if fraction.is_empty() || fraction.bytes().any(|b| b != b'0') {
                            return Err("invalid fractional coordinate".into());
                        }
                        integer
                    } else {
                        text
                    };
                    let digits = integer.strip_prefix(['+', '-']).unwrap_or(integer);
                    if digits.is_empty()
                        || !digits.as_bytes().first().unwrap().is_ascii_digit()
                        || !digits.as_bytes().last().unwrap().is_ascii_digit()
                        || digits.contains("__")
                        || digits.bytes().any(|b| !b.is_ascii_digit() && b != b'_')
                    {
                        None
                    } else {
                        integer.replace('_', "").parse::<i64>().ok()
                    }
                }
                _ => value.to_integer(symbols)?,
            }
            .ok_or("invalid hex coordinate")?;
            result[i] = i32::try_from(number).map_err(|_| "hex coordinate outside i32")?;
        }
        crate::geometry::Hex::new(result[0], result[1], result[2])
    }
    pub fn truthy(&self, symbols: &Symbols) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(v) => *v,
            Self::Int(v) => *v != 0,
            Self::Float(v) => *v != 0.0,
            Self::Text(id) => !symbols.name(*id).is_empty(),
            Self::List(v) => !v.is_empty(),
            Self::Object(v) => !v.is_empty(),
            Self::Instance(_) => true,
        }
    }
    /// Python's int() coercion for ordinary JSON numeric inputs. Non-ASCII
    /// numeric text is an explicitly unsupported boundary, never a false verdict.
    pub fn to_integer(&self, symbols: &Symbols) -> Result<Option<i64>, String> {
        match self {
            Self::Int(v) => Ok(Some(*v)),
            Self::Bool(v) => Ok(Some(i64::from(*v))),
            Self::Float(v) => {
                if *v >= i64::MIN as f64 && *v < 9_223_372_036_854_775_808.0 {
                    Ok(Some(v.trunc() as i64))
                } else {
                    Err("unsupported integer coercion outside i64".into())
                }
            }
            Self::Text(id) => {
                let value = symbols.name(*id);
                if !value.is_ascii() {
                    return Err("unsupported non-ASCII integer coercion".into());
                }
                let text = value.trim_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c']);
                let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
                if digits.is_empty()
                    || !digits.as_bytes().first().unwrap().is_ascii_digit()
                    || !digits.as_bytes().last().unwrap().is_ascii_digit()
                    || digits.contains("__")
                    || digits.bytes().any(|b| !b.is_ascii_digit() && b != b'_')
                {
                    return Ok(None);
                }
                let clean = text.replace('_', "");
                clean
                    .parse::<i64>()
                    .map(Some)
                    .map_err(|_| "unsupported integer coercion outside i64".into())
            }
            _ => Ok(None),
        }
    }
}
/// Missing and present-null values are different. Keys are resolved once at import.
#[derive(Clone, Debug)]
pub struct Registers {
    slots: Vec<Option<Datum>>,
    order: Vec<Symbol>,
}
impl Registers {
    pub fn new(symbols: &Symbols) -> Self {
        Self {
            slots: vec![None; symbols.len()],
            order: Vec::new(),
        }
    }
    pub fn clear(&mut self) {
        self.slots.fill(None);
        self.order.clear();
    }
    pub fn get(&self, key: Symbol) -> Option<&Datum> {
        self.slots.get(key.0).and_then(Option::as_ref)
    }
    pub fn set(&mut self, key: Symbol, value: Datum) {
        if key.0 >= self.slots.len() {
            self.slots.resize(key.0 + 1, None);
        }
        if self.slots[key.0].is_none() {
            self.order.push(key);
        }
        self.slots[key.0] = Some(value);
    }
    pub fn remove(&mut self, key: Symbol) {
        if let Some(slot) = self.slots.get_mut(key.0) {
            *slot = None;
            self.order.retain(|item| *item != key);
        }
    }
    pub fn to_json(&self, symbols: &Symbols) -> Value {
        Value::Object(
            self.order
                .iter()
                .map(|key| {
                    (
                        symbols.name(*key).to_owned(),
                        self.get(*key).unwrap().to_json(symbols),
                    )
                })
                .collect(),
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absence_null_and_python_truth_are_distinct() {
        let mut symbols = Symbols::default();
        let key = symbols.intern("key");
        let mut context = Registers::new(&symbols);
        assert!(context.get(key).is_none());
        context.set(key, Datum::Null);
        assert_eq!(context.get(key), Some(&Datum::Null));
        assert!(!Datum::Null.truthy(&symbols));
        assert!(!Datum::List(vec![]).truthy(&symbols));
        assert!(!Datum::Float(0.0).truthy(&symbols));
        context.remove(key);
        assert!(context.get(key).is_none());
    }
    #[test]
    fn context_insertion_order_survives_updates_deletion_and_reinsertion() {
        let mut symbols = Symbols::default();
        let first = symbols.intern("first");
        let second = symbols.intern("second");
        let mut context = Registers::new(&symbols);
        context.set(second, Datum::Int(1));
        context.set(first, Datum::Int(2));
        context.set(second, Datum::Int(3));
        assert_eq!(
            context
                .to_json(&symbols)
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        context.remove(second);
        context.set(second, Datum::Int(4));
        assert_eq!(
            context
                .to_json(&symbols)
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }
    #[test]
    fn hex_coordinates_reject_fractional_values_while_number_choices_truncate() {
        let mut symbols = Symbols::default();
        let keys = ["q", "r", "s"].map(|name| symbols.intern(name));
        let invalid =
            Datum::from_json(&serde_json::json!({"q":1.5,"r":-1.5,"s":0}), &mut symbols).unwrap();
        assert!(invalid.hex(keys, &symbols).is_err());
        assert_eq!(Datum::Float(1.5).to_integer(&symbols).unwrap(), Some(1));
        let valid = Datum::from_json(
            &serde_json::json!({"q":"1_0.0","r":"-10.00","s":0}),
            &mut symbols,
        )
        .unwrap();
        assert_eq!(
            valid.hex(keys, &symbols).unwrap().coordinates(),
            (10, -10, 0)
        );
        for text in [".0", "1.", "1e0", "1.0_0"] {
            let value =
                Datum::from_json(&serde_json::json!({"q":text,"r":0,"s":0}), &mut symbols).unwrap();
            assert!(value.hex(keys, &symbols).is_err());
        }
    }
    #[test]
    fn integer_coercion_matches_json_decision_inputs() {
        let mut symbols = Symbols::default();
        for (value, expected) in [
            (serde_json::json!(" +1_024 "), Some(1024)),
            (serde_json::json!("1__2"), None),
            (serde_json::json!(-2.9), Some(-2)),
            (serde_json::json!(true), Some(1)),
            (Value::Null, None),
        ] {
            let datum = Datum::from_json(&value, &mut symbols).unwrap();
            assert_eq!(datum.to_integer(&symbols).unwrap(), expected);
        }
    }
}
