//! Planning replacement shares card placement's live authority and revision token.
use super::{ProjectError, ProjectStore, Result, authority, board, visible_issue};
use rbit_collaboration::projects::PlanningInput;

impl ProjectStore {
    pub async fn plan_card(&self, org: i64, actor: i64, id: i64, issue: i64, input: &PlanningInput) -> Result<()> {
        input.validate().map_err(ProjectError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, false).await?;
        board(&mut tx, org, id).await?;
        visible_issue(&mut tx, org, actor, issue).await?;
        if let Some(iteration) = input.iteration_id {
            // Existing archived assignments can be retained while editing other fields.
            let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_iterations it WHERE it.board_id = ?1 AND it.id = ?2 AND (it.archived = 0 OR EXISTS(SELECT 1 FROM project_card_planning p WHERE p.board_id = ?1 AND p.issue_id = ?3 AND p.iteration_id = it.id)))")
                .bind(id).bind(iteration).bind(issue).fetch_one(&mut *tx).await?;
            if !valid {
                return Err(ProjectError::NotFound);
            }
        }
        let changed = sqlx::query("UPDATE project_cards SET revision = (SELECT revision + 1 FROM project_boards WHERE id = project_cards.board_id) WHERE board_id = ? AND issue_id = ? AND revision = ?")
            .bind(id).bind(issue).bind(input.revision).execute(&mut *tx).await?.rows_affected();
        if changed == 0 {
            return Err(ProjectError::Conflict);
        }
        sqlx::query("INSERT INTO project_card_planning(board_id, issue_id, priority, iteration_id, target_date) VALUES(?, ?, ?, ?, ?) ON CONFLICT(board_id, issue_id) DO UPDATE SET priority = excluded.priority, iteration_id = excluded.iteration_id, target_date = excluded.target_date")
            .bind(id).bind(issue).bind(input.priority.code()).bind(input.iteration_id).bind(&input.target_date).execute(&mut *tx).await?;
        sqlx::query("UPDATE project_boards SET revision = revision + 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
