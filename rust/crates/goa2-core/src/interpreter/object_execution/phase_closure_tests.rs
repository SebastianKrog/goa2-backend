//! Reviewed source phase templates must reach their authoritative live adapters.
use super::*;

#[test]
fn supported_source_templates_never_leave_a_parked_or_dense_card_instruction() {
    let data = crate::templates::instructions().unwrap();
    let templates = data["templates"].as_object().unwrap();
    let mut symbols = Symbols::default();
    let mut steps = Vec::new();
    let mut card = false;
    let mut respawn = false;
    for (name, wire) in templates {
        if !BoundObjects::supported(wire) {
            continue;
        }
        let id = BoundObjects::compile_wire(wire, &mut steps, &mut symbols)
            .unwrap_or_else(|error| panic!("source template {name}: {error}"));
        assert!(
            !matches!(
                steps[id].instruction,
                Instruction::ParkedPhase(_) | Instruction::ResolveCard { .. }
            ),
            "source template {name} has no authoritative adapter"
        );
        if wire["type"] == "resolve_card" {
            assert!(matches!(steps[id].instruction, Instruction::LiveCardAction));
            card = true;
        }
        if wire["type"] == "respawn_hero" {
            assert!(matches!(
                steps[id].instruction,
                Instruction::Flow(crate::turn::FlowStep::Respawn(_))
            ));
            respawn = true;
        }
    }
    assert!(card && respawn, "source phase prototypes missing");
}
