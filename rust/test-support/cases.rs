// Stream one case from a monolithic frozen JSON fixture at a time.
use serde::Deserialize;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::{collections::HashMap, fmt, fs::File, io::BufReader, path::Path};

struct Case<'a> {
    names: Option<&'a [&'a str]>,
}
impl<'de> DeserializeSeed<'de> for Case<'_> {
    type Value = Option<Value>;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        if self.names.is_none() {
            Value::deserialize(deserializer).map(Some)
        } else {
            deserializer.deserialize_map(self)
        }
    }
}
impl<'de> Visitor<'de> for Case<'_> {
    type Value = Option<Value>;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a named fixture input case")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let names = self.names.unwrap();
        let mut selected = None;
        let mut fields = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if key == "name" {
                if selected.is_some() {
                    return Err(de::Error::custom("duplicate fixture name"));
                }
                let name = map.next_value::<String>()?;
                selected = Some(names.contains(&name.as_str()));
                if selected == Some(true) {
                    fields.insert(key, Value::String(name));
                } else {
                    fields.clear();
                }
            } else if key == "expected" || selected == Some(false) {
                // Named rule/fork tests need inputs only. Full oracle scans
                // use the unfiltered branch above and compare every snapshot.
                map.next_value::<IgnoredAny>()?;
            } else {
                fields.insert(key, map.next_value::<Value>()?);
            }
        }
        match selected {
            Some(true) => Ok(Some(Value::Object(fields))),
            Some(false) => Ok(None),
            None => Err(de::Error::custom("fixture case name missing")),
        }
    }
}
struct Array<'a, F> {
    verify: &'a mut F,
    names: Option<&'a [&'a str]>,
}
impl<'de, F: FnMut(Value)> DeserializeSeed<'de> for Array<'_, F> {
    type Value = usize;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<usize, D::Error> {
        deserializer.deserialize_seq(self)
    }
}
impl<'de, F: FnMut(Value)> Visitor<'de> for Array<'_, F> {
    type Value = usize;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a fixture case array")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<usize, A::Error> {
        let mut checked = 0;
        while let Some(case) = sequence.next_element_seed(Case { names: self.names })? {
            if let Some(case) = case {
                (self.verify)(case);
            }
            checked += 1;
        }
        Ok(checked)
    }
}
struct Corpus<'a, F> {
    field: &'a str,
    verify: &'a mut F,
    names: Option<&'a [&'a str]>,
}
impl<'de, F: FnMut(Value)> DeserializeSeed<'de> for Corpus<'_, F> {
    type Value = usize;
    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<usize, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de, F: FnMut(Value)> Visitor<'de> for Corpus<'_, F> {
    type Value = usize;
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a frozen fixture corpus")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<usize, A::Error> {
        let mut checked = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == self.field {
                if checked.is_some() {
                    return Err(de::Error::custom("duplicate fixture array"));
                }
                checked = Some(map.next_value_seed(Array {
                    verify: &mut *self.verify,
                    names: self.names,
                })?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        checked.ok_or_else(|| de::Error::custom(format!("missing fixture array {}", self.field)))
    }
}
pub fn visit_array(path: &Path, field: &str, verify: impl FnMut(Value)) -> usize {
    visit(path, field, None, verify)
}
fn visit(path: &Path, field: &str, names: Option<&[&str]>, mut verify: impl FnMut(Value)) -> usize {
    let mut deserializer =
        serde_json::Deserializer::from_reader(BufReader::new(File::open(path).unwrap()));
    let checked = Corpus {
        field,
        verify: &mut verify,
        names,
    }
    .deserialize(&mut deserializer)
    .unwrap_or_else(|error| panic!("{} {field}: {error}", path.display()));
    deserializer.end().unwrap();
    assert!(checked > 0, "fixture array must not be empty");
    checked
}
// Cache only explicitly reused cases, never the entire expanded corpus.
pub fn selected_inputs(path: &Path, field: &str, names: &[&str]) -> HashMap<String, Value> {
    let mut selected = HashMap::new();
    visit(path, field, Some(names), |case| {
        if let Some(name) = case["name"].as_str().filter(|name| names.contains(name)) {
            let name = name.to_owned();
            assert!(
                selected.insert(name.clone(), case).is_none(),
                "duplicate case {name}"
            );
        }
    });
    assert_eq!(
        selected.len(),
        names.len(),
        "every named fixture must exist"
    );
    selected
}
