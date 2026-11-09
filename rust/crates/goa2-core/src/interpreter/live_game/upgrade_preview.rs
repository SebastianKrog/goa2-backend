//! Read-only preview of an ordered prefix of a hero's upgrades.
use super::{LiveGame, Result, invalid};
use crate::{interpreter::RequestChoices, round::UpgradePlayer};
use serde_json::{Value, json};

impl LiveGame {
    pub fn preview_upgrades(
        &self,
        hero_id: &str,
        request_id: &str,
        cards: &[String],
    ) -> Result<Value> {
        self.preview_upgrades_at(hero_id, request_id, cards, self.clock_timestamp())
    }

    pub fn preview_upgrades_at(
        &self,
        hero_id: &str,
        request_id: &str,
        cards: &[String],
        now_ms: u64,
    ) -> Result<Value> {
        let hero = self.hero(hero_id)?;
        let request = self
            .session
            .last_request
            .as_ref()
            .filter(|request| self.session.request_token(request.id) == request_id)
            .ok_or_else(|| invalid("Refresh the current upgrade offer"))?;
        let RequestChoices::Upgrades(players) = &request.choices else {
            return Err(invalid("Refresh the current upgrade offer"));
        };
        if cards.len() > 6
            || !players.iter().any(|row| {
                row.hero == hero && row.remaining > 0 && cards.len() as i64 <= row.remaining
            })
        {
            return Err(invalid("No matching pending upgrades"));
        }

        let mut preview = Self {
            session: self.session.clone(),
            revision: self.revision,
            upgrade_history: self.upgrade_history.clone(),
            rollback: None,
            lane_progress: self.lane_progress.clone(),
            clock: self.clock.clone(),
        };
        for card in cards {
            preview.session.apply_preview_upgrade(hero, card)?;
        }
        let remaining = preview
            .session
            .round
            .pending
            .iter()
            .find_map(|(id, n)| (*id == hero).then_some(*n))
            .unwrap_or(0);
        let mut options = json!([]);
        if remaining > 0 {
            let rules = preview
                .session
                .round_rules
                .as_ref()
                .ok_or_else(|| invalid("Missing upgrade rules"))?;
            let mut request = request.clone();
            request.choices = RequestChoices::Upgrades(vec![UpgradePlayer {
                hero,
                remaining,
                options: rules.options(&preview.session.world, hero),
            }]);
            let offer = preview.session.request_json(&request);
            options = offer["context"]["players"][hero_id]["options"].clone();
        }
        Ok(json!({
            "request_id": request_id,
            "card_ids": cards,
            "remaining": remaining,
            "options": options,
            "view": preview.client_view_at(Some(hero_id), now_ms)?["view"],
        }))
    }
}
