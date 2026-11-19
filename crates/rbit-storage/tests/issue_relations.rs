//! Real graph races, permission-filtered projections and repository lifecycle behavior.
use rbit_collaboration::issue_relations::{RelationInput, RelationKind};
use rbit_core::{Issue, IssueKind, IssueText, IssueTitle, OrgRole, Permission, Timestamp, User, Visibility};
use rbit_storage::{
    Database, NewRepository, NewUser,
    sqlite::{issue_relations::RelationError, issues::NewIssue, orgs::NewOrganization},
};

struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    sql: sqlx::SqlitePool,
    owner: User,
    reader: User,
    issues: Vec<Issue>,
}
async fn person(db: &Database, name: &str) -> User {
    db.accounts()
        .create_user(NewUser {
            name: name.parse().unwrap(),
            email: format!("{name}@example.com").parse().unwrap(),
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
    let owner = person(&db, "owner").await;
    let reader = person(&db, "reader").await;
    let org = db
        .orgs()
        .create(NewOrganization { name: "acme".parse().unwrap(), display_name: None, description: None, creator: owner.id() })
        .await
        .unwrap();
    db.orgs().add_member(org.account.id, reader.id(), OrgRole::Member).await.unwrap();
    sqlx::query("UPDATE org_profiles SET member_permission = 'none' WHERE account_id = ?")
        .bind(org.account.id.0)
        .execute(&sql)
        .await
        .unwrap();
    let mut issues = Vec::new();
    for (name, visibility) in [("public", Visibility::Public), ("private", Visibility::Private)] {
        let repo = db
            .repositories()
            .create(NewRepository {
                owner_id: org.account.id,
                name: name.parse().unwrap(),
                description: None,
                visibility,
                default_branch: "main".into(),
            })
            .await
            .unwrap();
        for index in 0..3 {
            issues.push(
                db.issues()
                    .create(&NewIssue {
                        repo: repo.id,
                        kind: IssueKind::Issue,
                        title: IssueTitle::parse(&format!("{name} {index}")).unwrap(),
                        body: IssueText::body("").unwrap(),
                        author: owner.id(),
                        at: Timestamp(1),
                    })
                    .await
                    .unwrap(),
            );
        }
    }
    Fixture { _dir: dir, db, sql, owner, reader, issues }
}
fn input(kind: RelationKind, repository: &str, number: i64) -> RelationInput {
    RelationInput { kind, repository: repository.into(), number }
}
async fn state(f: &Fixture, index: usize, closed: bool) {
    sqlx::query("UPDATE issues SET state = ?, state_reason = ?, closed_at = ? WHERE id = ?")
        .bind(if closed { "closed" } else { "open" })
        .bind(closed.then_some("completed"))
        .bind(closed.then_some(2_i64))
        .bind(f.issues[index].id.0)
        .execute(&f.sql)
        .await
        .unwrap();
}

#[tokio::test]
async fn relations_filter_titles_progress_and_blockers_before_counting() {
    let f = fixture().await;
    let store = f.db.issue_relations();
    let source = f.issues[0].id.0;
    store.add(f.owner.id().0, source, &input(RelationKind::Child, "public", 2)).await.unwrap();
    store.add(f.owner.id().0, source, &input(RelationKind::Child, "private", 1)).await.unwrap();
    store.add(f.owner.id().0, source, &input(RelationKind::BlockedBy, "private", 1)).await.unwrap();
    state(&f, 1, true).await;
    let visible = store.get(Some(f.reader.id().0), source).await.unwrap();
    assert_eq!((visible.items.len(), visible.progress.completed, visible.progress.total, visible.progress.blocked), (1, 1, 1, false));
    assert!(!visible.can_edit && !visible.items[0].can_remove);
    assert_eq!(store.get(None, source).await.unwrap().progress, visible.progress);
    let full = store.get(Some(f.owner.id().0), source).await.unwrap();
    assert_eq!((full.progress.completed, full.progress.total, full.progress.blocked), (1, 2, true));
    state(&f, 3, true).await;
    let completed = store.get(Some(f.owner.id().0), source).await.unwrap();
    assert_eq!((completed.progress.completed, completed.progress.blocked), (2, false));
    state(&f, 3, false).await;
    assert!(store.get(Some(f.owner.id().0), source).await.unwrap().progress.blocked);
    assert!(matches!(store.get(Some(f.reader.id().0), f.issues[3].id.0).await, Err(RelationError::NotFound)));
    assert!(matches!(store.add(f.reader.id().0, source, &input(RelationKind::Child, "private", 1)).await, Err(RelationError::NotFound)));
    assert!(matches!(store.add(f.reader.id().0, source, &input(RelationKind::Child, "public", 3)).await, Err(RelationError::Forbidden)));
    f.sql.close().await;
}

#[tokio::test]
async fn relations_atomic_cycles_parents_and_stale_edge_ids() {
    let f = fixture().await;
    let s = f.db.issue_relations();
    let a = f.issues[0].id.0;
    let b = f.issues[1].id.0;
    let child = input(RelationKind::Child, "", 2);
    let reverse = input(RelationKind::BlockedBy, "", 1);
    let (left, right) = tokio::join!(s.add(f.owner.id().0, a, &child), s.add(f.owner.id().0, b, &reverse));
    assert_ne!(left.is_ok(), right.is_ok(), "combined cycle admitted by concurrent writes");
    let edge = s.get(Some(f.owner.id().0), a).await.unwrap().items[0].id;
    s.remove(f.owner.id().0, a, edge).await.unwrap();
    let replacement = s.add(f.owner.id().0, a, &child).await.unwrap();
    assert_ne!(replacement, edge);
    assert!(matches!(s.remove(f.owner.id().0, a, edge).await, Err(RelationError::NotFound)));
    assert_eq!(s.add(f.owner.id().0, a, &child).await.unwrap(), replacement);
    assert!(matches!(s.add(f.owner.id().0, f.issues[2].id.0, &child).await, Err(RelationError::Conflict)));
    assert!(matches!(s.add(f.owner.id().0, a, &input(RelationKind::Child, "", 1)).await, Err(RelationError::Conflict)));
    // Redundant same-direction dependency is valid; reverse parent dependency is not.
    s.add(f.owner.id().0, a, &input(RelationKind::BlockedBy, "", 2)).await.unwrap();
    assert!(matches!(s.add(f.owner.id().0, b, &reverse).await, Err(RelationError::Conflict)));
    let reciprocal = s.get(Some(f.owner.id().0), b).await.unwrap();
    assert_eq!(reciprocal.items.iter().map(|r| r.relationship.as_str()).collect::<Vec<_>>(), ["parent", "blocking"]);
    f.sql.close().await;
}

#[tokio::test]
async fn relations_recheck_grants_archiving_suspension_and_transfer() {
    let f = fixture().await;
    let s = f.db.issue_relations();
    let public = f.issues[0].repo_id;
    f.db.repositories().grant_collaborator(public, f.reader.id(), Permission::Write).await.unwrap();
    let edge = s.add(f.reader.id().0, f.issues[0].id.0, &input(RelationKind::Child, "", 2)).await.unwrap();
    sqlx::query("DELETE FROM collaborators WHERE repo_id = ? AND user_id = ?")
        .bind(public.0)
        .bind(f.reader.id().0)
        .execute(&f.sql)
        .await
        .unwrap();
    assert!(matches!(s.remove(f.reader.id().0, f.issues[0].id.0, edge).await, Err(RelationError::Forbidden)));
    s.add(f.owner.id().0, f.issues[0].id.0, &input(RelationKind::Child, "private", 1)).await.unwrap();
    sqlx::query("UPDATE repositories SET owner_id = ? WHERE id = ?").bind(f.owner.id().0).bind(public.0).execute(&f.sql).await.unwrap();
    let result = s.get(Some(f.owner.id().0), f.issues[0].id.0).await.unwrap();
    assert_eq!(result.items.len(), 1, "transfer removes cross-repository edges, keeps local hierarchy");
    sqlx::query("DELETE FROM issues WHERE id = ?").bind(f.issues[1].id.0).execute(&f.sql).await.unwrap();
    assert!(s.get(Some(f.owner.id().0), f.issues[0].id.0).await.unwrap().items.is_empty());
    sqlx::query("UPDATE user_logins SET suspended_at = 3 WHERE account_id = ?").bind(f.owner.id().0).execute(&f.sql).await.unwrap();
    assert!(matches!(s.get(Some(f.owner.id().0), f.issues[0].id.0).await, Err(RelationError::NotFound)));
    f.sql.close().await;
}

#[tokio::test]
async fn relations_bound_fanout_walk_and_keep_duplicate_add_idempotent() {
    let f = fixture().await;
    let s = f.db.issue_relations();
    let repo = f.issues[0].repo_id.0;
    sqlx::query("WITH RECURSIVE n(x) AS (VALUES(10) UNION ALL SELECT x+1 FROM n WHERE x < 1110) INSERT INTO issues(repo_id, number, kind, title, created_at, updated_at) SELECT ?1, x, 'issue', 'bounded graph', 1, 1 FROM n")
        .bind(repo).execute(&f.sql).await.unwrap();
    let source = f.issues[0].id.0;
    sqlx::query("INSERT INTO issue_relations(source_id, target_id, kind, created_at) SELECT ?1, id, 'child', 1 FROM issues WHERE repo_id = ?2 AND number BETWEEN 10 AND 109")
        .bind(source).bind(repo).execute(&f.sql).await.unwrap();
    assert_eq!(s.get(Some(f.owner.id().0), source).await.unwrap().progress.total, 100);
    assert!(s.add(f.owner.id().0, source, &input(RelationKind::Child, "", 10)).await.is_ok());
    assert!(matches!(s.add(f.owner.id().0, source, &input(RelationKind::Child, "", 110)).await, Err(RelationError::Conflict)));
    assert!(matches!(s.batch(Some(f.owner.id().0), &[source; 101]).await, Err(RelationError::Invalid(_))));
    sqlx::query("INSERT INTO issue_relations(source_id, target_id, kind, created_at) SELECT a.id, b.id, 'blocked_by', 1 FROM issues a JOIN issues b ON b.repo_id = a.repo_id AND b.number = a.number + 1 WHERE a.repo_id = ? AND a.number BETWEEN 110 AND 1109")
        .bind(repo).execute(&f.sql).await.unwrap();
    assert!(matches!(
        s.add(f.owner.id().0, f.issues[1].id.0, &input(RelationKind::BlockedBy, "", 110)).await,
        Err(RelationError::Conflict)
    ));
    f.sql.close().await;
}

#[tokio::test]
async fn relations_board_rollups_share_visibility_and_archive_authority() {
    use rbit_collaboration::projects::{BoardInput, Filter};
    let f = fixture().await;
    let source = f.issues[0].id.0;
    let repo = f.issues[0].repo_id.0;
    let org: i64 = sqlx::query_scalar("SELECT owner_id FROM repositories WHERE id = ?").bind(repo).fetch_one(&f.sql).await.unwrap();
    let boards = f.db.projects();
    let board = boards
        .create(org, f.owner.id().0, &BoardInput { title: "Plan".into(), columns: vec!["Todo".into(), "Done".into()] })
        .await
        .unwrap();
    boards.add(org, f.owner.id().0, board.id, source).await.unwrap();
    let relations = f.db.issue_relations();
    let edge = relations.add(f.owner.id().0, source, &input(RelationKind::BlockedBy, "private", 1)).await.unwrap();
    assert!(boards.page(org, f.owner.id().0, board.id, &Filter::default()).await.unwrap().cards[0].progress.blocked);
    assert!(!boards.page(org, f.reader.id().0, board.id, &Filter::default()).await.unwrap().cards[0].progress.blocked);
    sqlx::query("INSERT INTO repo_archives(repo_id, archived_at) VALUES(?, 1)").bind(repo).execute(&f.sql).await.unwrap();
    let visible = relations.get(Some(f.owner.id().0), source).await.unwrap();
    assert!(!visible.can_edit && !visible.items[0].can_remove);
    assert!(matches!(relations.remove(f.owner.id().0, source, edge).await, Err(RelationError::Forbidden)));
    assert!(matches!(relations.add(f.owner.id().0, source, &input(RelationKind::Child, "", 2)).await, Err(RelationError::Forbidden)));
    f.sql.close().await;
}

#[tokio::test]
async fn relations_concurrent_parents_and_direct_child_progress() {
    let f = fixture().await;
    let s = f.db.issue_relations();
    let target = input(RelationKind::Child, "", 3);
    let (first, second) = tokio::join!(s.add(f.owner.id().0, f.issues[0].id.0, &target), s.add(f.owner.id().0, f.issues[1].id.0, &target));
    assert_ne!(first.is_ok(), second.is_ok(), "a child cannot acquire two parents");
    let edge = first.or(second).unwrap();
    s.remove(f.owner.id().0, f.issues[2].id.0, edge).await.unwrap();
    s.add(f.owner.id().0, f.issues[0].id.0, &input(RelationKind::Child, "", 2)).await.unwrap();
    s.add(f.owner.id().0, f.issues[1].id.0, &target).await.unwrap();
    state(&f, 2, true).await;
    let parent = s.get(Some(f.owner.id().0), f.issues[0].id.0).await.unwrap();
    assert_eq!((parent.progress.completed, parent.progress.total), (0, 1), "rollup counts direct subissues");
    assert_eq!(s.get(Some(f.owner.id().0), f.issues[1].id.0).await.unwrap().progress.completed, 1);
    state(&f, 1, true).await;
    assert_eq!(s.get(Some(f.owner.id().0), f.issues[0].id.0).await.unwrap().progress.completed, 1);
    state(&f, 1, false).await;
    assert_eq!(s.get(Some(f.owner.id().0), f.issues[0].id.0).await.unwrap().progress.completed, 0);
    f.sql.close().await;
}
