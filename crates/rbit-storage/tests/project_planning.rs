//! Live planning authority, private saved views and bounded keyset pagination.
use rbit_collaboration::projects::{BoardInput, Filter, IterationInput, Layout, PlanningInput, Priority, Sort, ViewInput};
use rbit_core::{IssueKind, IssueText, IssueTitle, OrgRole, RepoId, Timestamp, User, Visibility};
use rbit_storage::{
    Database, NewRepository, NewUser, count_work,
    sqlite::{issues::NewIssue, orgs::NewOrganization, projects::ProjectError},
};

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    sql: sqlx::SqlitePool,
    owner: User,
    reader: User,
    org: i64,
    board: i64,
    public: RepoId,
    private: RepoId,
}
async fn user(db: &Database, name: &str) -> User {
    db.accounts()
        .create_user(NewUser {
            name: name.parse().unwrap(),
            email: format!("{name}@example.test").parse().unwrap(),
            display_name: None,
            password_hash: "unused".into(),
            is_admin: false,
        })
        .await
        .unwrap()
}
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rbit.db");
    let db = Database::open(&path).await.unwrap();
    let sql = super::writer_connection::pool(&path).await;
    let owner = user(&db, "owner").await;
    let reader = user(&db, "reader").await;
    let org = db
        .orgs()
        .create(NewOrganization { name: "planning".parse().unwrap(), display_name: None, description: None, creator: owner.id() })
        .await
        .unwrap();
    db.orgs().add_member(org.account.id, reader.id(), OrgRole::Member).await.unwrap();
    sqlx::query("UPDATE org_profiles SET member_permission = 'none' WHERE account_id = ?")
        .bind(org.account.id.0)
        .execute(&sql)
        .await
        .unwrap();
    let mut repos = Vec::new();
    for (name, visibility) in [("public", Visibility::Public), ("private", Visibility::Private)] {
        repos.push(
            db.repositories()
                .create(NewRepository {
                    owner_id: org.account.id,
                    name: name.parse().unwrap(),
                    description: None,
                    visibility,
                    default_branch: "main".into(),
                })
                .await
                .unwrap()
                .id,
        );
    }
    let board = db
        .projects()
        .create(org.account.id.0, owner.id().0, &BoardInput { title: "Planning".into(), columns: vec!["Todo".into(), "Done".into()] })
        .await
        .unwrap()
        .id;
    Fixture { _dir: dir, db, sql, owner, reader, org: org.account.id.0, board, public: repos[0], private: repos[1] }
}
impl Fixture {
    async fn issue(&self, repo: RepoId, title: &str) -> i64 {
        let issue = self
            .db
            .issues()
            .create(&NewIssue {
                repo,
                kind: IssueKind::Issue,
                title: IssueTitle::parse(title).unwrap(),
                body: IssueText::body("").unwrap(),
                author: self.owner.id(),
                at: Timestamp(1),
            })
            .await
            .unwrap();
        self.db.projects().add(self.org, self.owner.id().0, self.board, issue.id.0).await.unwrap();
        issue.id.0
    }
    async fn revision(&self, issue: i64) -> i64 {
        sqlx::query_scalar("SELECT revision FROM project_cards WHERE board_id = ? AND issue_id = ?")
            .bind(self.board)
            .bind(issue)
            .fetch_one(&self.sql)
            .await
            .unwrap()
    }
}
fn iteration() -> IterationInput {
    IterationInput { title: "Iteration one".into(), starts_on: "2026-01-01".into(), ends_on: "2026-01-14".into(), archived: false }
}
fn plan(revision: i64, iteration_id: Option<i64>) -> PlanningInput {
    PlanningInput { revision, priority: Priority::Urgent, target_date: Some("2020-01-01".into()), iteration_id }
}

#[tokio::test]
async fn project_planning_is_local_revised_and_tracks_live_issue_state() {
    let f = fixture().await;
    let store = f.db.projects();
    let issue = f.issue(f.public, "Visible work").await;
    let hidden = f.issue(f.private, "Private work").await;
    f.db.issue_relations()
        .add(
            f.owner.id().0,
            issue,
            &rbit_collaboration::issue_relations::RelationInput {
                kind: rbit_collaboration::issue_relations::RelationKind::BlockedBy,
                repository: "private".into(),
                number: 1,
            },
        )
        .await
        .unwrap();
    assert!(matches!(store.save_iteration(f.org, f.reader.id().0, f.board, None, &iteration()).await, Err(ProjectError::Forbidden)));
    let it = store.save_iteration(f.org, f.owner.id().0, f.board, None, &iteration()).await.unwrap();
    let revision = f.revision(issue).await;
    store.plan_card(f.org, f.reader.id().0, f.board, issue, &plan(revision, Some(it))).await.unwrap();
    assert!(matches!(store.plan_card(f.org, f.owner.id().0, f.board, issue, &plan(revision, None)).await, Err(ProjectError::Conflict)));
    assert!(matches!(
        store.plan_card(f.org, f.reader.id().0, f.board, hidden, &plan(f.revision(hidden).await, None)).await,
        Err(ProjectError::NotFound)
    ));
    let page = store.page(f.org, f.reader.id().0, f.board, &Filter::default()).await.unwrap();
    assert_eq!(page.cards.len(), 1);
    assert_eq!(page.cards[0].priority, Priority::Urgent);
    assert!(!page.cards[0].progress.blocked);
    assert!(
        store
            .page(f.org, f.owner.id().0, f.board, &Filter { layout: Layout::Table, ..Filter::default() })
            .await
            .unwrap()
            .cards
            .iter()
            .find(|c| c.issue_id == issue)
            .unwrap()
            .progress
            .blocked
    );
    assert!(page.cards[0].overdue);
    assert_eq!(page.cards[0].iteration_id, Some(it));
    sqlx::query("UPDATE issues SET state = 'closed', state_reason = 'completed', closed_at = unixepoch() WHERE id = ?")
        .bind(issue)
        .execute(&f.sql)
        .await
        .unwrap();
    let page = store.page(f.org, f.reader.id().0, f.board, &Filter::default()).await.unwrap();
    assert_eq!(page.cards[0].column, 1);
    assert!(!page.cards[0].overdue);
    sqlx::query("UPDATE issues SET state = 'open', state_reason = NULL, closed_at = NULL WHERE id = ?")
        .bind(issue)
        .execute(&f.sql)
        .await
        .unwrap();
    let other = store
        .create(f.org, f.owner.id().0, &BoardInput { title: "Other".into(), columns: vec!["Todo".into(), "Done".into()] })
        .await
        .unwrap();
    store.add(f.org, f.owner.id().0, other.id, issue).await.unwrap();
    let other_revision = store.page(f.org, f.owner.id().0, other.id, &Filter::default()).await.unwrap().cards[0].revision;
    assert!(matches!(
        store.plan_card(f.org, f.owner.id().0, other.id, issue, &plan(other_revision, Some(it))).await,
        Err(ProjectError::NotFound)
    ));
    assert_eq!(store.page(f.org, f.owner.id().0, other.id, &Filter::default()).await.unwrap().cards[0].priority, Priority::None);
    let revision = f.revision(issue).await;
    let input = plan(revision, Some(it));
    let (a, b) = tokio::join!(
        store.plan_card(f.org, f.owner.id().0, f.board, issue, &input),
        store.move_card(f.org, f.owner.id().0, f.board, issue, revision, Some(0))
    );
    assert_ne!(a.is_ok(), b.is_ok(), "one concurrent edit must conflict");
    let revision = store.page(f.org, f.owner.id().0, f.board, &Filter::default()).await.unwrap().board.revision;
    store.delete(f.org, f.owner.id().0, f.board, revision).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM project_card_planning").fetch_one(&f.sql).await.unwrap(), 0);
}

#[tokio::test]
async fn project_iterations_archive_without_reassigning_and_delete_only_when_unused() {
    let f = fixture().await;
    let store = f.db.projects();
    let issue = f.issue(f.public, "Assigned").await;
    let other = f.issue(f.public, "Unassigned").await;
    let it = store.save_iteration(f.org, f.owner.id().0, f.board, None, &iteration()).await.unwrap();
    store.plan_card(f.org, f.owner.id().0, f.board, issue, &plan(f.revision(issue).await, Some(it))).await.unwrap();
    assert!(store.delete_iteration(f.org, f.owner.id().0, f.board, it, 1).await.is_err());
    let mut input = iteration();
    input.archived = true;
    store.save_iteration(f.org, f.owner.id().0, f.board, Some((it, 1)), &input).await.unwrap();
    assert!(matches!(store.save_iteration(f.org, f.owner.id().0, f.board, Some((it, 1)), &input).await, Err(ProjectError::Conflict)));
    assert!(matches!(
        store.plan_card(f.org, f.owner.id().0, f.board, other, &plan(f.revision(other).await, Some(it))).await,
        Err(ProjectError::NotFound)
    ));
    store.plan_card(f.org, f.owner.id().0, f.board, issue, &plan(f.revision(issue).await, Some(it))).await.unwrap();
    let view = store
        .save_view(
            f.org,
            f.reader.id().0,
            f.board,
            None,
            &ViewInput { title: "Old iteration".into(), shared: false, filter: Filter { iteration: Some(it), ..Filter::default() } },
        )
        .await
        .unwrap();
    store.plan_card(f.org, f.owner.id().0, f.board, issue, &plan(f.revision(issue).await, None)).await.unwrap();
    store.delete_iteration(f.org, f.owner.id().0, f.board, it, 2).await.unwrap();
    assert!(matches!(
        store.page(f.org, f.reader.id().0, f.board, &Filter { view: Some(view), ..Filter::default() }).await,
        Err(ProjectError::NotFound)
    ));
    let before = f.revision(issue).await;
    store.move_card(f.org, f.owner.id().0, f.board, issue, before, None).await.unwrap();
    store.add(f.org, f.owner.id().0, f.board, issue).await.unwrap();
    assert!(matches!(store.plan_card(f.org, f.owner.id().0, f.board, issue, &plan(before, None)).await, Err(ProjectError::Conflict)));
    let page = store.page(f.org, f.owner.id().0, f.board, &Filter::default()).await.unwrap();
    assert_eq!(page.cards.iter().find(|c| c.issue_id == issue).unwrap().priority, Priority::None);
}

#[tokio::test]
async fn project_views_hide_private_criteria_follow_renames_and_fence_membership() {
    let f = fixture().await;
    let store = f.db.projects();
    let mut input = ViewInput {
        title: "Restricted shared view".into(),
        shared: true,
        filter: Filter { repository: "private".into(), layout: Layout::Table, ..Filter::default() },
    };
    let shared = store.save_view(f.org, f.owner.id().0, f.board, None, &input).await.unwrap();
    assert!(store.views(f.org, f.reader.id().0, f.board).await.unwrap().is_empty());
    assert!(matches!(
        store.page(f.org, f.reader.id().0, f.board, &Filter { view: Some(shared), ..Filter::default() }).await,
        Err(ProjectError::NotFound)
    ));
    f.db.repositories().grant_collaborator(f.private, f.reader.id(), rbit_core::Permission::Read).await.unwrap();
    assert_eq!(store.views(f.org, f.reader.id().0, f.board).await.unwrap()[0].id, shared);
    sqlx::query("DELETE FROM collaborators WHERE repo_id = ? AND user_id = ?")
        .bind(f.private.0)
        .bind(f.reader.id().0)
        .execute(&f.sql)
        .await
        .unwrap();
    assert!(store.views(f.org, f.reader.id().0, f.board).await.unwrap().is_empty());
    input.filter.repository = "public".into();
    let public =
        store.save_view(f.org, f.owner.id().0, f.board, None, &ViewInput { title: "Shared public".into(), ..input.clone() }).await.unwrap();
    assert!(!store.views(f.org, f.reader.id().0, f.board).await.unwrap()[0].can_edit);
    assert!(matches!(store.delete_view(f.org, f.reader.id().0, f.board, public, 1).await, Err(ProjectError::Forbidden)));
    input.shared = false;
    input.title = "Personal".into();
    let personal = store.save_view(f.org, f.reader.id().0, f.board, None, &input).await.unwrap();
    assert!(!store.views(f.org, f.owner.id().0, f.board).await.unwrap().iter().any(|v| v.id == personal));
    assert!(matches!(store.delete_view(f.org, f.owner.id().0, f.board, personal, 1).await, Err(ProjectError::NotFound)));
    sqlx::query("UPDATE repositories SET name = 'renamed' WHERE id = ?").bind(f.private.0).execute(&f.sql).await.unwrap();
    assert_eq!(
        store.views(f.org, f.owner.id().0, f.board).await.unwrap().iter().find(|v| v.id == shared).unwrap().filter.repository,
        "renamed"
    );
    sqlx::query("UPDATE repositories SET owner_id = ? WHERE id = ?").bind(f.owner.id().0).bind(f.private.0).execute(&f.sql).await.unwrap();
    assert!(!store.views(f.org, f.owner.id().0, f.board).await.unwrap().iter().any(|v| v.id == shared));
    sqlx::query("DELETE FROM org_members WHERE org_id = ? AND user_id = ?")
        .bind(f.org)
        .bind(f.reader.id().0)
        .execute(&f.sql)
        .await
        .unwrap();
    assert!(matches!(store.views(f.org, f.reader.id().0, f.board).await, Err(ProjectError::NotFound)));
}

#[tokio::test]
async fn project_sorted_pages_are_visible_bounded_and_bind_cursor_criteria() {
    let f = fixture().await;
    let store = f.db.projects();
    let hidden = f.issue(f.private, "Invisible low priority").await;
    let hidden_plan = PlanningInput { priority: Priority::Low, ..plan(f.revision(hidden).await, None) };
    store.plan_card(f.org, f.owner.id().0, f.board, hidden, &hidden_plan).await.unwrap();
    let mut visible = Vec::new();
    for index in 0..101 {
        visible.push(f.issue(f.public, &format!("Visible {index}")).await);
    }
    let first = PlanningInput { priority: Priority::High, ..plan(f.revision(visible[0]).await, None) };
    store.plan_card(f.org, f.owner.id().0, f.board, visible[0], &first).await.unwrap();
    let last = PlanningInput { target_date: Some("2030-01-01".into()), ..plan(f.revision(visible[100]).await, None) };
    store.plan_card(f.org, f.owner.id().0, f.board, visible[100], &last).await.unwrap();
    for sort in [Sort::Id, Sort::Priority, Sort::TargetDate] {
        let filter = Filter { sort, layout: Layout::Table, ..Filter::default() };
        let (page, work) = count_work(store.page(f.org, f.reader.id().0, f.board, &filter)).await;
        let page = page.unwrap();
        assert_eq!(page.cards.len(), 100);
        assert_eq!(page.cards[0].issue_id, if sort == Sort::Priority { visible[100] } else { visible[0] });
        assert!(page.cards.iter().all(|c| c.repository == "public"));
        assert_eq!((work.acquisitions, work.statements), (1, 6));
        let cursor = page.next_cursor.unwrap();
        let next = store.page(f.org, f.reader.id().0, f.board, &Filter { cursor: cursor.clone(), ..filter.clone() }).await.unwrap();
        assert_eq!(next.cards.len(), 1);
        assert!(next.next_cursor.is_none());
        assert!(next.cards[0].issue_id > page.cards.last().unwrap().issue_id);
        assert!(matches!(
            store.page(f.org, f.reader.id().0, f.board, &Filter { q: "different".into(), cursor, ..filter }).await,
            Err(ProjectError::Invalid(_))
        ));
    }
    let private = Filter { priority: Some(Priority::Low), ..Filter::default() };
    let page = store.page(f.org, f.reader.id().0, f.board, &private).await.unwrap();
    assert!(page.cards.is_empty());
    assert!(page.next_cursor.is_none());
}

#[tokio::test]
async fn project_catalog_limits_are_scoped_and_checked_before_insertion() {
    let f = fixture().await;
    let store = f.db.projects();
    sqlx::query("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i + 1 FROM n WHERE i < 200) INSERT INTO project_iterations(board_id, title, starts_on, ends_on, created_at) SELECT ?, 'Iteration ' || i, '2026-01-01', '2026-01-14', 1 FROM n").bind(f.board).execute(&f.sql).await.unwrap();
    assert!(matches!(store.save_iteration(f.org, f.owner.id().0, f.board, None, &iteration()).await, Err(ProjectError::Invalid(_))));
    for index in 0..50 {
        store
            .save_view(
                f.org,
                f.reader.id().0,
                f.board,
                None,
                &ViewInput { title: format!("View {index}"), shared: false, filter: Filter::default() },
            )
            .await
            .unwrap();
    }
    let input = ViewInput { title: "Extra".into(), shared: false, filter: Filter::default() };
    assert!(matches!(store.save_view(f.org, f.reader.id().0, f.board, None, &input).await, Err(ProjectError::Invalid(_))));
    store.save_view(f.org, f.owner.id().0, f.board, None, &input).await.unwrap();
    assert_eq!(store.views(f.org, f.reader.id().0, f.board).await.unwrap().len(), 50);
    assert_eq!(store.views(f.org, f.owner.id().0, f.board).await.unwrap().len(), 1);
    f.sql.close().await;
}
