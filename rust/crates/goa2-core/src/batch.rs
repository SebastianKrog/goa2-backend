//! Hypothetical slot assignments use register overlays, never mutate the world.
use crate::{
    data::{Datum, RegisterRead, Symbol},
    geometry::Hex,
    spatial::{HexFilter, SpatialContext},
};

struct Overlay<'a> {
    base: &'a dyn RegisterRead,
    values: Vec<(Symbol, Datum)>,
    hexes: Vec<(Symbol, Vec<Hex>)>,
}
impl RegisterRead for Overlay<'_> {
    fn excludes_hex(&self, key: Symbol, hex: Hex) -> Option<bool> {
        self.hexes
            .iter()
            .find_map(|(field, values)| (*field == key).then(|| values.contains(&hex)))
            .or_else(|| {
                if self.values.iter().any(|(field, _)| *field == key) {
                    None
                } else {
                    self.base.excludes_hex(key, hex)
                }
            })
    }
    fn get(&self, key: Symbol) -> Option<&Datum> {
        self.values
            .iter()
            .rev()
            .find_map(|(k, v)| (*k == key).then_some(v))
            .or_else(|| self.base.get(key))
    }
}
fn hex_value(hex: Hex, keys: [Symbol; 3]) -> Datum {
    let (q, r, s) = hex.coordinates();
    Datum::Object(
        keys.into_iter()
            .zip([q, r, s])
            .map(|(k, v)| (k, Datum::Int(i64::from(v))))
            .collect(),
    )
}
fn with_registers<'a>(
    context: &'a SpatialContext<'_>,
    registers: &'a dyn RegisterRead,
) -> SpatialContext<'a> {
    SpatialContext {
        count_observer: None,
        world: context.world,
        registers,
        symbols: context.symbols,
        actor: context.actor,
        frame: context.frame,
        coordinates: context.coordinates,
        range_override: context.range_override,
        through_override: context.through_override,
    }
}
pub fn context_hexes(
    value: Option<&Datum>,
    context: &SpatialContext<'_>,
) -> Result<Vec<Hex>, String> {
    let Some(value) = value.filter(|v| v.truthy(context.symbols)) else {
        return Ok(Vec::new());
    };
    let values = match value {
        Datum::List(v) => v.as_slice(),
        v => std::slice::from_ref(v),
    };
    values
        .iter()
        .filter(|v| matches!(v, Datum::Object(_)))
        .map(|v| v.hex(context.coordinates, context.symbols))
        .collect()
}
#[derive(Clone, Debug)]
pub struct BatchSearch {
    pub slot_index: i64,
    pub keys: Vec<Symbol>,
    pub filters: Vec<Vec<HexFilter>>,
    pub free: Vec<Hex>,
    pub removable: Vec<Hex>,
    pub budget: i64,
    pub freed_key: Symbol,
}
impl BatchSearch {
    fn cost(&self, h: Hex) -> i64 {
        i64::from(!self.free.contains(&h) && self.removable.contains(&h))
    }
    fn extend(&self, freed: &[Hex], hexes: &[Hex]) -> Vec<Hex> {
        let mut result = freed.to_vec();
        // Match the source comprehension: duplicates in one input batch are
        // tested against the old freed list, not deduplicated against each other.
        result.extend(
            hexes
                .iter()
                .filter(|h| self.cost(**h) > 0 && !freed.contains(h)),
        );
        result
    }
    fn definitely_freed(&self) -> Vec<Hex> {
        if self.budget >= self.removable.len() as i64 {
            self.extend(&self.free, &self.removable)
        } else {
            self.free.clone()
        }
    }
    pub fn feasible(&self, context: &SpatialContext<'_>) -> Result<bool, String> {
        if self.keys.len() != self.filters.len() {
            return Ok(false);
        }
        self.complete(
            context,
            0,
            &mut Vec::new(),
            self.budget,
            &self.definitely_freed(),
        )
    }
    pub fn accepts(&self, candidate: Hex, context: &SpatialContext<'_>) -> Result<bool, String> {
        let Ok(index) = usize::try_from(self.slot_index) else {
            return Ok(false);
        };
        if index >= self.keys.len() || self.keys.len() != self.filters.len() {
            return Ok(false);
        }
        let overlay = Overlay {
            base: context.registers,
            values: vec![(self.keys[index], hex_value(candidate, context.coordinates))],
            hexes: vec![(self.keys[index], vec![candidate])],
        };
        let scratch = with_registers(context, &overlay);
        let mut chosen = Vec::new();
        for key in &self.keys[..=index] {
            let Some(value @ Datum::Object(_)) = scratch.registers.get(*key) else {
                return Ok(false);
            };
            let h = value.hex(context.coordinates, context.symbols)?;
            if chosen.contains(&h) {
                return Ok(false);
            }
            chosen.push(h);
        }
        let used: i64 = chosen.iter().map(|h| self.cost(*h)).sum();
        if used > self.budget {
            return Ok(false);
        }
        let freed = self.extend(&self.definitely_freed(), &chosen);
        self.complete(&scratch, index + 1, &mut chosen, self.budget - used, &freed)
    }
    fn complete(
        &self,
        context: &SpatialContext<'_>,
        index: usize,
        chosen: &mut Vec<Hex>,
        budget: i64,
        freed: &[Hex],
    ) -> Result<bool, String> {
        if index >= self.keys.len() {
            return Ok(true);
        }
        for &hex in context.world.tile_hexes() {
            if chosen.contains(&hex) {
                continue;
            }
            let cost = self.cost(hex);
            if cost > budget {
                continue;
            }
            let cand_freed = if cost > 0 {
                self.extend(freed, &[hex])
            } else {
                freed.to_vec()
            };
            let overlay = Overlay {
                base: context.registers,
                hexes: vec![
                    (self.freed_key, cand_freed.clone()),
                    (self.keys[index], vec![hex]),
                ],
                values: vec![
                    (
                        self.freed_key,
                        Datum::List(
                            cand_freed
                                .iter()
                                .map(|h| hex_value(*h, context.coordinates))
                                .collect(),
                        ),
                    ),
                    (self.keys[index], hex_value(hex, context.coordinates)),
                ],
            };
            // Slot filters see earlier choices, not this slot's new assignment.
            let eval_overlay = Overlay {
                base: context.registers,
                values: vec![overlay.values[0].clone()],
                hexes: vec![overlay.hexes[0].clone()],
            };
            let eval = with_registers(context, &eval_overlay);
            let mut accepted = [true];
            for filter in &self.filters[index] {
                if (self.free.contains(&hex) || self.removable.contains(&hex))
                    && matches!(filter, HexFilter::Obstacle { wanted: false, .. })
                {
                    continue;
                }
                filter.apply_scan(&eval, &[hex], &mut accepted)?;
                if !accepted[0] {
                    break;
                }
            }
            if !accepted[0] {
                continue;
            }
            let scratch = with_registers(context, &overlay);
            chosen.push(hex);
            let complete =
                self.complete(&scratch, index + 1, chosen, budget - cost, &cand_freed)?;
            chosen.pop();
            if complete {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
#[derive(Clone, Debug)]
pub struct RemovalSearch {
    pub batch: BatchSearch,
    pub on_board: Vec<(Symbol, Hex)>,
    pub remaining: i64,
}
impl RemovalSearch {
    pub fn accepts(&self, id: Symbol, context: &SpatialContext<'_>) -> Result<bool, String> {
        let Some(hex) = context.world.position(id) else {
            return Ok(false);
        };
        let mut search = self.batch.clone();
        search.free = vec![hex];
        search.removable = self
            .on_board
            .iter()
            .filter_map(|(uid, h)| (*uid != id).then_some(*h))
            .collect();
        search.budget = self.remaining.saturating_sub(1).max(0);
        search.feasible(context)
    }
}
#[derive(Clone, Debug)]
pub struct FarthestAdjacent {
    pub origin: crate::spatial::Origin,
    pub anchor: Symbol,
    pub occupied: Vec<Symbol>,
    pub freed: Symbol,
}
impl FarthestAdjacent {
    pub fn accepts(&self, candidate: Hex, context: &SpatialContext<'_>) -> Result<bool, String> {
        let Some(origin) = context
            .origin(&self.origin, true)?
            .and_then(|id| context.world.resolved_position(id))
        else {
            return Ok(false);
        };
        let anchor = match context.registers.get(self.anchor) {
            Some(Datum::Text(id)) => context.world.resolved_position(*id),
            _ => None,
        };
        let Some(anchor) = anchor else {
            return Ok(false);
        };
        let freed = context_hexes(context.registers.get(self.freed), context)?;
        let mut occupied = Vec::new();
        for key in &self.occupied {
            occupied.extend(context_hexes(context.registers.get(*key), context)?);
        }
        let mut maximum = None;
        let mut candidate_distance = None;
        for hex in anchor.neighbors() {
            if !context.world.on_map(hex)
                || !context.world.effect_connected(anchor, hex, &[])?
                || occupied.contains(&hex)
                || (context.world.obstacle(hex) && !freed.contains(&hex))
            {
                continue;
            }
            let distance = if context.world.effect_connected(origin, hex, &[])? {
                u64::from(origin.distance(hex))
            } else {
                u64::MAX
            };
            maximum = Some(maximum.map_or(distance, |old: u64| old.max(distance)));
            if hex == candidate {
                candidate_distance = Some(distance);
            }
        }
        Ok(candidate_distance.is_some() && candidate_distance == maximum)
    }
}
