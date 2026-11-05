//! Search-local, typed values and interned symbols. JSON conversion is boundary-only.
use serde_json::{Map, Value};
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Symbol(pub(crate) usize);
#[derive(Clone, Debug, Default)]
struct SymbolTable {
    names: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, Symbol>,
}

#[derive(Clone, Debug, Default)]
pub struct Symbols {
    shared: Option<Arc<SymbolTable>>,
    compiled: Option<Arc<SymbolTable>>,
    local: SymbolTable,
}
impl Symbols {
    /// Reserve stable IDs for the process-wide rule programs.
    pub(crate) fn freeze_shared(&mut self) {
        debug_assert!(self.shared.is_none() && self.compiled.is_none());
        self.shared = Some(Arc::new(std::mem::take(&mut self.local)));
    }

    /// Share the compiled prefix while keeping subsequent inserts branch-local.
    pub(crate) fn freeze_compiled(&mut self) {
        debug_assert!(self.compiled.is_none());
        self.compiled = Some(Arc::new(std::mem::take(&mut self.local)));
    }
    pub fn intern(&mut self, name: &str) -> Symbol {
        if let Some(id) = self.lookup(name) {
            return id;
        }
        let id = Symbol(self.len());
        let name: Arc<str> = Arc::from(name);
        self.local.names.push(Arc::clone(&name));
        self.local.ids.insert(name, id);
        id
    }
    pub fn name(&self, id: Symbol) -> &str {
        let mut index = id.0;
        for table in [&self.shared, &self.compiled].into_iter().flatten() {
            if index < table.names.len() {
                return &table.names[index];
            }
            index -= table.names.len();
        }
        &self.local.names[index]
    }
    pub fn lookup(&self, name: &str) -> Option<Symbol> {
        self.local
            .ids
            .get(name)
            .copied()
            .or_else(|| self.compiled.as_ref()?.ids.get(name).copied())
            .or_else(|| self.shared.as_ref()?.ids.get(name).copied())
    }
    pub fn len(&self) -> usize {
        self.local.names.len()
            + self.compiled.as_ref().map_or(0, |table| table.names.len())
            + self.shared.as_ref().map_or(0, |table| table.names.len())
    }
    pub(crate) fn hash_local(&self, state: &mut impl std::hash::Hasher) {
        use std::hash::Hash;
        self.local.names.hash(state);
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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
impl std::hash::Hash for Datum {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Null => {}
            Self::Bool(value) => value.hash(state),
            Self::Int(value) => value.hash(state),
            Self::Float(value) => {
                // PartialEq treats both representations of zero as equal.
                (if *value == 0.0 { 0 } else { value.to_bits() }).hash(state);
            }
            Self::Text(value) => value.hash(state),
            Self::List(values) => values.hash(state),
            Self::Object(fields) => fields.hash(state),
            Self::Instance(value) => value.hash(state),
        }
    }
}

impl Datum {
    pub(crate) fn python_scalar_text(&self, symbols: &Symbols) -> Option<String> {
        Some(match self {
            Self::Null => "None".into(),
            Self::Bool(value) => if *value { "True" } else { "False" }.into(),
            Self::Int(value) => value.to_string(),
            Self::Float(value) => python_float_text(*value),
            Self::Text(id) => symbols.name(*id).to_owned(),
            _ => return None,
        })
    }
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
/// Read-only context shared by dense execution and sparse live predicate inputs.
pub trait RegisterRead: Sync {
    fn get(&self, key: Symbol) -> Option<&Datum>;
    /// Python Hex equality needs model provenance, not just coordinate fields.
    fn excludes_hex(&self, _key: Symbol, _hex: crate::geometry::Hex) -> Option<bool> {
        None
    }
}
#[derive(Clone, Debug, Default)]
pub(crate) struct SparseRegisters(
    pub Vec<(Symbol, Datum)>,
    pub Vec<(Symbol, Vec<crate::geometry::Hex>)>,
);
impl RegisterRead for SparseRegisters {
    fn excludes_hex(&self, key: Symbol, hex: crate::geometry::Hex) -> Option<bool> {
        self.1
            .iter()
            .find_map(|(field, values)| (*field == key).then(|| values.contains(&hex)))
    }
    fn get(&self, key: Symbol) -> Option<&Datum> {
        self.0
            .iter()
            .find_map(|(id, value)| (*id == key).then_some(value))
    }
}
/// Mutation interface for typed rules with either dense or sparse context.
pub trait RegisterWrite: RegisterRead {
    fn set(&mut self, key: Symbol, value: Datum);
}
impl RegisterWrite for SparseRegisters {
    fn set(&mut self, key: Symbol, value: Datum) {
        self.1.retain(|(field, _)| *field != key);
        if let Some((_, slot)) = self.0.iter_mut().find(|(id, _)| *id == key) {
            *slot = value;
        } else {
            self.0.push((key, value));
        }
    }
}
impl RegisterWrite for Registers {
    fn set(&mut self, key: Symbol, value: Datum) {
        Registers::set(self, key, value);
    }
}
impl RegisterRead for Registers {
    fn get(&self, key: Symbol) -> Option<&Datum> {
        Registers::get(self, key)
    }
}

/// Missing and present-null values are different. Keys are resolved once at import.
#[derive(Clone, Debug, Default, Hash)]
pub struct Registers {
    entries: Vec<(Symbol, Datum)>,
}
impl Registers {
    pub fn new(_symbols: &Symbols) -> Self {
        Self {
            entries: Vec::new(),
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn get(&self, key: Symbol) -> Option<&Datum> {
        self.entries
            .iter()
            .find(|(id, _)| *id == key)
            .map(|(_, value)| value)
    }
    pub fn set(&mut self, key: Symbol, value: Datum) {
        if let Some((_, existing)) = self.entries.iter_mut().find(|(id, _)| *id == key) {
            *existing = value;
        } else {
            self.entries.push((key, value));
        }
    }
    pub fn remove(&mut self, key: Symbol) {
        if let Some(index) = self.entries.iter().position(|(id, _)| *id == key) {
            self.entries.remove(index);
        }
    }
    pub fn to_json(&self, symbols: &Symbols) -> Value {
        Value::Object(
            self.entries
                .iter()
                .map(|(key, value)| (symbols.name(*key).to_owned(), value.to_json(symbols)))
                .collect(),
        )
    }
}
/// Python repr's fixed/scientific thresholds and explicit signed exponent.
fn python_float_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf"
        } else {
            "inf"
        }
        .into();
    }
    let scientific = format!("{value:e}");
    let (mantissa, exponent) = scientific.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    if !(-4..16).contains(&exponent) {
        format!("{mantissa}e{exponent:+03}")
    } else {
        let mut text = value.to_string();
        if !text.contains('.') {
            text.push_str(".0");
        }
        text
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

/// Per-worker observation of a predicate's temporary context binding. Clones
/// start clean: completed preflights must never leak into another search branch.
#[derive(Debug, Default)]
pub struct FilterObservation(std::sync::atomic::AtomicBool);
impl Clone for FilterObservation {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl FilterObservation {
    pub fn clear(&self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn record(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn observed(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}
