//! Mirrored destinations retain the source's vacating-mover obstacle exceptions.
use super::*;

pub(super) fn accepts(
    context: &SpatialContext<'_>,
    destination: Hex,
    anchor_key: Symbol,
    partner_key: Symbol,
    through: bool,
) -> Result<bool, String> {
    let Some((anchor, partner)) = context
        .scalar_identity(anchor_key)
        .zip(context.scalar_identity(partner_key))
    else {
        return Ok(false);
    };
    let world = context.world;
    let Some((start, partner_start)) = world
        .position(anchor)
        .zip(world.position(partner))
        .filter(|(a, b)| a != b)
    else {
        return Ok(false);
    };
    if start == destination || !start.straight(destination) {
        return Ok(false);
    }
    let (aq, ar, az) = start.coordinates();
    let (q, r, z) = destination.coordinates();
    let partner_destination = crate::displacement::offset(
        partner_start,
        (
            i64::from(q) - i64::from(aq),
            i64::from(r) - i64::from(ar),
            i64::from(z) - i64::from(az),
        ),
        1,
    )?;
    let actor = context
        .actor
        .filter(|id| !context.symbols.name(*id).is_empty());
    let landing = |hex| -> Result<bool, String> {
        if !world.on_map(hex) {
            return Ok(false);
        }
        let occupant = world.occupant(hex);
        if occupant.is_some_and(|id| id != anchor && id != partner) || world.terrain_hex(hex)? {
            return Ok(false);
        }
        // Source skips dynamic obstacle checks for a vacating co-mover.
        Ok(occupant.is_some() || !world.obstacle_for_actor(hex, actor)?)
    };
    if !landing(destination)? || !landing(partner_destination)? {
        return Ok(false);
    }
    if through {
        return Ok(true);
    }
    let path = |from: Hex, to: Hex, moved: Symbol, other: Symbol| -> Result<bool, String> {
        let Some(line) = from.line_to(to) else {
            return Ok(false);
        };
        for hex in line.iter().take(line.len().saturating_sub(1)) {
            if !world.on_map(*hex) {
                return Ok(false);
            }
            if world.occupant(*hex) == Some(other) {
                continue;
            }
            if world.obstacle_for_actor(*hex, actor)? && !world.passable_token(*hex, &[moved]) {
                return Ok(false);
            }
        }
        Ok(true)
    };
    Ok(path(start, destination, anchor, partner)?
        && path(partner_start, partner_destination, partner, anchor)?)
}
