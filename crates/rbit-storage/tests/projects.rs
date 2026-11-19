//! Project visibility, atomic authority, revision races and lifecycle projection.
use rbit_collaboration::projects::{BoardInput, Filter};
use rbit_core::{IssueKind, IssueText, IssueTitle, OrgRole, Timestamp, User, Visibility};
use rbit_storage::{
    Database, NewRepository, NewUser,
    sqlite::{issues::NewIssue, orgs::NewOrganization, projects::ProjectError},
};

async fn user(db: &Database, name: &str) -> User {
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

async fn hidden_cards(db: &Database, owner: &User, org: i64, board: i64, repo: rbit_core::RepoId) {
    for index in 0..101 {
        let issue = db
            .issues()
            .create(&NewIssue {
                repo,
                kind: IssueKind::Issue,
                title: IssueTitle::parse(&format!("Secret {index}")).unwrap(),
                body: IssueText::body("").unwrap(),
                author: owner.id(),
                at: Timestamp(2),
            })
            .await
            .unwrap();
        db.projects().add(org, owner.id().0, board, issue.id.0).await.unwrap();
    }
}

async fn repository(db: &Database, owner_id: rbit_core::AccountId, name: &str, visibility: Visibility) -> rbit_core::Repository {
    db.repositories()
        .create(NewRepository { owner_id, name: name.parse().unwrap(), description: None, visibility, default_branch: "main".into() })
        .await
        .unwrap()
}

#[tokio::test]
async fn membership_visibility_revisions_and_completion() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("rbit.db")).await.unwrap();
    let owner = user(&db, "owner").await;
    let member = user(&db, "member").await;
    let outsider = user(&db, "outsider").await;
    let org = db
        .orgs()
        .create(NewOrganization { name: "acme".parse().unwrap(), display_name: None, description: None, creator: owner.id() })
        .await
        .unwrap();
    let org_id = org.account.id;
    db.orgs().add_member(org_id, member.id(), OrgRole::Member).await.unwrap();
    let pool = super::writer_connection::pool(&dir.path().join("rbit.db")).await;
    sqlx::query("UPDATE org_profiles SET member_permission = 'none' WHERE account_id = ?").bind(org_id.0).execute(&pool).await.unwrap();
    let input = BoardInput { title: "Ship".into(), columns: vec!["Todo".into(), "Doing".into(), "Done".into()] };
    let store = db.projects();
    let board = store.create(org_id.0, owner.id().0, &input).await.unwrap();
    assert!(matches!(store.list(org_id.0, outsider.id().0).await, Err(ProjectError::NotFound)));
    assert!(matches!(store.create(org_id.0, member.id().0, &input).await, Err(ProjectError::Forbidden)));
    let mut issues = Vec::new();
    for (name, visibility) in [("public", Visibility::Public), ("private", Visibility::Private)] {
        let repo = repository(&db, org_id, name, visibility).await;
        let issue = db
            .issues()
            .create(&NewIssue {
                repo: repo.id,
                kind: IssueKind::Issue,
                title: IssueTitle::parse(name).unwrap(),
                body: IssueText::body("").unwrap(),
                author: owner.id(),
                at: Timestamp(1),
            })
            .await
            .unwrap();
        store.add(org_id.0, owner.id().0, board.id, issue.id.0).await.unwrap();
        issues.push(issue);
    }
    let before = store.page(org_id.0, owner.id().0, board.id, &Filter::default()).await.unwrap();
    store.add(org_id.0, owner.id().0, board.id, issues[0].id.0).await.unwrap();
    assert_eq!(store.page(org_id.0, owner.id().0, board.id, &Filter::default()).await.unwrap().board.revision, before.board.revision);
    let page = store.page(org_id.0, member.id().0, board.id, &Filter::default()).await.unwrap();
    assert_eq!(page.cards.len(), 1);
    assert_eq!(page.cards[0].repository, "public");
    assert!(page.next.is_none());
    let public = issues[0].id.0;
    let private = issues[1].id.0;
    assert!(matches!(store.move_card(org_id.0, member.id().0, board.id, private, 1, Some(1)).await, Err(ProjectError::NotFound)));
    store.move_card(org_id.0, member.id().0, board.id, public, 2, Some(1)).await.unwrap();
    assert!(matches!(store.move_card(org_id.0, owner.id().0, board.id, public, 2, Some(2)).await, Err(ProjectError::Conflict)));
    sqlx::query("UPDATE issues SET state = 'closed', closed_at = 2, state_reason = 'completed' WHERE id = ?")
        .bind(public)
        .execute(&pool)
        .await
        .unwrap();
    let page = store.page(org_id.0, member.id().0, board.id, &Filter::default()).await.unwrap();
    assert_eq!(page.cards[0].column, 2);
    sqlx::query("UPDATE issues SET state = 'open', closed_at = NULL, state_reason = NULL WHERE id = ?")
        .bind(public)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(store.page(org_id.0, member.id().0, board.id, &Filter::default()).await.unwrap().cards[0].column, 1);
    let hidden = Filter { repository: "private".into(), ..Filter::default() };
    assert!(store.page(org_id.0, member.id().0, board.id, &hidden).await.unwrap().cards.is_empty());
    // Removal and re-addition cannot make an old revision valid again.
    let revision = store.page(org_id.0, owner.id().0, board.id, &Filter::default()).await.unwrap().cards[0].revision;
    store.move_card(org_id.0, owner.id().0, board.id, public, revision, None).await.unwrap();
    store.add(org_id.0, owner.id().0, board.id, public).await.unwrap();
    assert!(matches!(store.move_card(org_id.0, owner.id().0, board.id, public, revision, Some(1)).await, Err(ProjectError::Conflict)));
    // Hidden rows cannot consume the page or reveal a next cursor.
    hidden_cards(&db, &owner, org_id.0, board.id, issues[1].repo_id).await;
    let member_page = store.page(org_id.0, member.id().0, board.id, &Filter::default()).await.unwrap();
    assert_eq!(member_page.cards.len(), 1);
    assert!(member_page.next.is_none());
    let owner_page = store.page(org_id.0, owner.id().0, board.id, &Filter::default()).await.unwrap();
    assert_eq!(owner_page.cards.len(), 100);
    let next_page =
        store.page(org_id.0, owner.id().0, board.id, &Filter { after: owner_page.next.unwrap(), ..Filter::default() }).await.unwrap();
    assert_eq!(next_page.cards.len(), 3);
    assert!(next_page.next.is_none());
    assert!(next_page.cards.iter().all(|card| owner_page.cards.iter().all(|previous| previous.issue_id != card.issue_id)));
    sqlx::query("DELETE FROM org_members WHERE org_id = ? AND user_id = ?")
        .bind(org_id.0)
        .bind(member.id().0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(store.move_card(org_id.0, member.id().0, board.id, public, 2, Some(0)).await, Err(ProjectError::NotFound)));
    store.delete(org_id.0, owner.id().0, board.id, owner_page.board.revision).await.unwrap();
    let cards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_cards").fetch_one(&pool).await.unwrap();
    assert_eq!(cards, 0);
    let issue_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issues").fetch_one(&pool).await.unwrap();
    assert_eq!(issue_count, 103);
    pool.close().await;
}
