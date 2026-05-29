use super::{rules, AppState};
use crate::storage::RuleCurationStatus;
use tracing::{debug, warn};

const FEEDBACK_BATCH_TRIGGER: usize = 10;
const ENCOUNTER_BATCH_TRIGGER: usize = 20;
const FEEDBACK_BATCH_LIMIT: usize = 10;

pub(super) fn schedule_x_rule_curation(state: AppState) {
    tokio::spawn(async move {
        run_x_rule_curation_if_due(state).await;
    });
}

async fn run_x_rule_curation_if_due(state: AppState) {
    let _guard = state.rule_curation.lock().await;

    loop {
        let status = match state.content_store.x_rule_curation_status() {
            Ok(status) => status,
            Err(error) => {
                warn!(%error, "failed to inspect X rule curation queue");
                return;
            }
        };

        if !should_run_x_rule_curation(&status) {
            return;
        }

        match rules::generate_x_rule_set_proposal(&state, 1, FEEDBACK_BATCH_LIMIT).await {
            Ok(proposal) => {
                debug!(
                    proposal_id = proposal.id.as_str(),
                    source = proposal.source.as_str(),
                    feedback_count = proposal.feedback_count,
                    active_rule_count = proposal.active_rule_count,
                    "generated automatic X rule-set proposal"
                );
            }
            Err(error) => {
                warn!(?error, "automatic X rule curation failed");
                return;
            }
        }
    }
}

fn should_run_x_rule_curation(status: &RuleCurationStatus) -> bool {
    status.unprocessed_feedback_count >= FEEDBACK_BATCH_TRIGGER
        || (status.unprocessed_feedback_count > 0
            && status.encounters_since_last_run >= ENCOUNTER_BATCH_TRIGGER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curation_runs_for_ten_unprocessed_feedback_rows() {
        assert!(should_run_x_rule_curation(&status(10, 0)));
    }

    #[test]
    fn curation_runs_for_one_feedback_after_twenty_encounters() {
        assert!(should_run_x_rule_curation(&status(1, 20)));
    }

    #[test]
    fn curation_waits_for_small_feedback_and_low_browsing_activity() {
        assert!(!should_run_x_rule_curation(&status(1, 19)));
        assert!(!should_run_x_rule_curation(&status(0, 100)));
    }

    fn status(
        unprocessed_feedback_count: usize,
        encounters_since_last_run: usize,
    ) -> RuleCurationStatus {
        RuleCurationStatus {
            unprocessed_feedback_count,
            total_encounters: encounters_since_last_run,
            last_run_total_encounters: 0,
            encounters_since_last_run,
        }
    }
}
