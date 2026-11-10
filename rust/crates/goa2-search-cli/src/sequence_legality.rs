use goa2_core::{
    api::experimental_search::{DecisionMove, ExecutionBudget, NativeSession, SequenceVerdict},
    cards::CardId,
    codec::StateEnvelope,
    data::{Datum, Symbol},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireMove {
    Card {
        player_id: String,
        card: u32,
    },
    Pass {
        player_id: String,
    },
    FinishPlanning {
        player_id: String,
    },
    Input {
        player_id: String,
        request_id: Option<String>,
        selection: Value,
    },
}

pub(super) fn player(session: &mut NativeSession, id: String) -> Result<Symbol, String> {
    match session
        .import_selection(&Value::String(id))
        .map_err(|e| e.to_string())?
    {
        Datum::Text(id) => Ok(id),
        _ => Err("expected player identity".into()),
    }
}

impl WireMove {
    fn decode(self, session: &mut NativeSession) -> Result<DecisionMove, String> {
        Ok(match self {
            Self::Card { player_id, card } => DecisionMove::Card {
                hero: player(session, player_id)?,
                card: CardId(card),
            },
            Self::Pass { player_id } => DecisionMove::Pass {
                hero: player(session, player_id)?,
            },
            Self::FinishPlanning { player_id } => DecisionMove::FinishPlanning {
                hero: player(session, player_id)?,
            },
            Self::Input {
                player_id,
                request_id,
                selection,
            } => DecisionMove::Input {
                player: player(session, player_id)?,
                request: request_id.map(|id| session.import_request_token(&id)),
                selection: session
                    .import_selection(&selection)
                    .map_err(|e| e.to_string())?,
            },
        })
    }
}

pub(super) fn check(
    envelope: StateEnvelope,
    sequences: Vec<Vec<WireMove>>,
) -> Result<Value, String> {
    let mut session = NativeSession::compile(envelope).map_err(|e| e.to_string())?;
    let sequences = sequences
        .into_iter()
        .map(|sequence| {
            sequence
                .into_iter()
                .map(|decision| decision.decode(&mut session))
                .collect()
        })
        .collect::<Result<Vec<Vec<_>>, String>>()?;
    let batch = session
        .check_sequences(&sequences, &ExecutionBudget::default())
        .map_err(|e| e.to_string())?;
    let results = batch
        .verdicts
        .into_iter()
        .map(|verdict| match verdict {
            SequenceVerdict::Legal { checked } => {
                json!({"legal":true,"checked":checked,"failed_at":null,"reason":null})
            }
            SequenceVerdict::Illegal { failed_at, reason } => {
                json!({"legal":false,"checked":failed_at,"failed_at":failed_at,"reason":reason})
            }
            SequenceVerdict::Unknown { failed_at, reason } => {
                json!({"legal":null,"checked":failed_at,"failed_at":failed_at,"reason":reason})
            }
        })
        .collect::<Vec<_>>();
    Ok(json!({"results":results,"applied_decisions":batch.applied_decisions}))
}
