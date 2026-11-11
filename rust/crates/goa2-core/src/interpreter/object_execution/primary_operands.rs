//! Extract operand bindings from live recipe fields before compiling the step.

use super::{Datum, Result, Symbols, Value, json, unsupported};
use crate::{objects::ObjectSlot, stats::CardStats};

pub(super) type Binding = (Vec<ObjectSlot>, crate::primary::Operand);

pub(super) fn extract(wire: &mut Value, symbols: &mut Symbols) -> Result<Vec<Binding>> {
    fn visit(
        value: &mut Value,
        symbols: &mut Symbols,
        path: &mut Vec<ObjectSlot>,
        output: &mut Vec<Binding>,
    ) -> Result<()> {
        if let Some(operand) = crate::primary::Operand::parse(value).map_err(unsupported)? {
            output.push((path.clone(), operand));
            // The compiled step requires a numeric field. Its live field is
            // replaced at the recorded path before the recipe is instantiated.
            *value = json!(0);
            return Ok(());
        }
        match value {
            Value::Array(values) => {
                for (index, child) in values.iter_mut().enumerate() {
                    path.push(ObjectSlot::Index(index));
                    visit(child, symbols, path, output)?;
                    path.pop();
                }
            }
            Value::Object(fields) => {
                for (key, child) in fields {
                    path.push(ObjectSlot::Field(symbols.intern(key)));
                    visit(child, symbols, path, output)?;
                    path.pop();
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut bindings = Vec::new();
    visit(wire, symbols, &mut Vec::new(), &mut bindings)?;
    Ok(bindings)
}

pub(super) fn bind(fields: &mut Datum, bindings: &[Binding], stats: &CardStats) -> Result<()> {
    for (path, operand) in bindings {
        let mut field = &mut *fields;
        for slot in path {
            field = match (field, slot) {
                (Datum::Object(fields), ObjectSlot::Field(key)) => fields
                    .iter_mut()
                    .find(|(id, _)| id == key)
                    .map(|(_, value)| value),
                (Datum::List(fields), ObjectSlot::Index(index)) => fields.get_mut(*index),
                _ => None,
            }
            .ok_or_else(|| unsupported("primary operand binding path drift"))?;
        }
        *field = operand
            .resolve(stats)
            .map_err(unsupported)?
            .map_or(Datum::Null, Datum::Int);
    }
    Ok(())
}
