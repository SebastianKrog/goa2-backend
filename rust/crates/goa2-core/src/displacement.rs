//! Straight displacement and vector arithmetic without Python/JSON per step.
use crate::{geometry::Hex, world::NativeWorld};
#[derive(Clone, Debug)]
pub struct PushResult {
    pub to: Hex,
    pub distance: usize,
    pub collision: bool,
}
pub fn offset(target: Hex, delta: (i64, i64, i64), scale: i64) -> Result<Hex, String> {
    let (q, r, s) = target.coordinates();
    let component = |base: i32, d: i64| {
        i32::try_from(i128::from(base) + i128::from(d) * i128::from(scale))
            .map_err(|_| "displacement coordinate outside i32".to_string())
    };
    Hex::new(
        component(q, delta.0)?,
        component(r, delta.1)?,
        component(s, delta.2)?,
    )
}
pub fn behind(origin: Hex, target: Hex, scale: i64) -> Result<Option<Hex>, String> {
    let d = i64::from(origin.distance(target));
    if d == 0 {
        return Ok(None);
    }
    let (a, b, c) = origin.coordinates();
    let (q, r, s) = target.coordinates();
    let direction = (
        (i64::from(q) - i64::from(a)).div_euclid(d),
        (i64::from(r) - i64::from(b)).div_euclid(d),
        (i64::from(s) - i64::from(c)).div_euclid(d),
    );
    offset(target, direction, scale).map(Some)
}
pub fn push(
    world: &NativeWorld,
    origin: Hex,
    target: Hex,
    distance: i64,
    ignore: bool,
) -> Result<Option<PushResult>, String> {
    if origin == target || !origin.straight(target) {
        return Ok(None);
    }
    let d = i64::from(origin.distance(target));
    let (a, b, c) = origin.coordinates();
    let (q, r, s) = target.coordinates();
    let direction = (
        (i64::from(q) - i64::from(a)) / d,
        (i64::from(r) - i64::from(b)) / d,
        (i64::from(s) - i64::from(c)) / d,
    );
    let mut path = vec![target];
    let mut collision = false;
    for _ in 0..distance {
        let next = offset(*path.last().unwrap(), direction, 1)?;
        if !world.on_map(next) || world.obstacle(next) && !ignore {
            collision = true;
            break;
        }
        path.push(next);
    }
    if ignore {
        while path.len() > 1 && world.obstacle(*path.last().unwrap()) {
            path.pop();
            collision = true;
        }
    }
    Ok(Some(PushResult {
        to: *path.last().unwrap(),
        distance: path.len() - 1,
        collision,
    }))
}

/// Full source push path: topology is anchored at the starting hex and at
/// every edge; passable tokens can be crossed but never used as landing spaces.
pub(crate) fn push_path(
    world: &NativeWorld,
    origin: Hex,
    target: Hex,
    distance: i64,
    ignore: bool,
    actor: crate::data::Symbol,
    victim: crate::data::Symbol,
) -> Result<Option<(PushResult, Vec<Hex>)>, String> {
    if origin == target || !origin.straight(target) {
        return Ok(None);
    }
    let d = i64::from(origin.distance(target));
    let (a, b, c) = origin.coordinates();
    let (q, r, s) = target.coordinates();
    let direction = (
        (i64::from(q) - i64::from(a)) / d,
        (i64::from(r) - i64::from(b)) / d,
        (i64::from(s) - i64::from(c)) / d,
    );
    let mut path = vec![target];
    let mut collision = false;
    for _ in 0..distance {
        let previous = *path.last().unwrap();
        let next = offset(previous, direction, 1)?;
        if !world.on_map(next)
            || !world.effect_connected(target, next, &[victim])?
            || !world.effect_connected(previous, next, &[victim])?
        {
            collision = true;
            break;
        }
        if world.obstacle_for_actor(next, Some(actor))?
            && !world.passable_token(next, &[])
            && !ignore
        {
            collision = true;
            break;
        }
        path.push(next);
    }
    while path.len() > 1 && world.passable_token(*path.last().unwrap(), &[]) {
        path.pop();
        collision = true;
    }
    if ignore {
        while path.len() > 1 && world.obstacle_for_actor(*path.last().unwrap(), Some(actor))? {
            path.pop();
            collision = true;
        }
    }
    Ok(Some((
        PushResult {
            to: *path.last().unwrap(),
            distance: path.len() - 1,
            collision,
        },
        path,
    )))
}

#[derive(Clone, Debug)]
pub struct PushSpec {
    pub target: Option<crate::data::Symbol>,
    pub key: Option<crate::data::Symbol>,
    pub source: Option<Hex>,
    pub source_key: Option<crate::data::Symbol>,
    pub distance: i64,
    pub distance_key: Option<crate::data::Symbol>,
    pub collision: Option<crate::data::Symbol>,
    pub ignore: bool,
    pub victim_output: crate::data::Symbol,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vector_flooring_rejects_non_cube_results_instead_of_rounding() {
        let origin = Hex::new(0, 0, 0).unwrap();
        assert!(behind(origin, Hex::new(2, -1, -1).unwrap(), 1).is_err());
        assert_eq!(
            behind(origin, Hex::new(2, -2, 0).unwrap(), -2).unwrap(),
            Some(origin)
        );
        assert_eq!(behind(origin, origin, 1).unwrap(), None);
    }
}
