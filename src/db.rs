use rusqlite::{Connection, params};
use serde_json::{Value, json};
use std::path::Path;

use crate::error::{PitError, Result};
use crate::types::*;

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let db = Db { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS issues (
                id            INTEGER PRIMARY KEY AUTOINCREMENT,
                title         TEXT NOT NULL,
                body          TEXT,
                status        TEXT NOT NULL DEFAULT 'open',
                closed_reason TEXT,
                created_at    TEXT NOT NULL,
                updated_at    TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS labels (
                id   INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE
            );

            CREATE TABLE IF NOT EXISTS issue_labels (
                issue_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
                label_id INTEGER NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
                PRIMARY KEY (issue_id, label_id)
            );

            CREATE TABLE IF NOT EXISTS comments (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                issue_id   INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
                body       TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS issues_fts USING fts5(
                title, body, content='issues', content_rowid='id'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS comments_fts USING fts5(
                body, content='comments', content_rowid='id'
            );

            CREATE TRIGGER IF NOT EXISTS issues_ai AFTER INSERT ON issues BEGIN
                INSERT INTO issues_fts(rowid, title, body) VALUES (new.id, new.title, COALESCE(new.body, ''));
            END;
            CREATE TRIGGER IF NOT EXISTS issues_ad AFTER DELETE ON issues BEGIN
                INSERT INTO issues_fts(issues_fts, rowid, title, body) VALUES ('delete', old.id, old.title, COALESCE(old.body, ''));
            END;
            CREATE TRIGGER IF NOT EXISTS issues_au AFTER UPDATE ON issues BEGIN
                INSERT INTO issues_fts(issues_fts, rowid, title, body) VALUES ('delete', old.id, old.title, COALESCE(old.body, ''));
                INSERT INTO issues_fts(rowid, title, body) VALUES (new.id, new.title, COALESCE(new.body, ''));
            END;

            CREATE TRIGGER IF NOT EXISTS comments_ai AFTER INSERT ON comments BEGIN
                INSERT INTO comments_fts(rowid, body) VALUES (new.id, new.body);
            END;
            CREATE TRIGGER IF NOT EXISTS comments_ad AFTER DELETE ON comments BEGIN
                INSERT INTO comments_fts(comments_fts, rowid, body) VALUES ('delete', old.id, old.body);
            END;
            CREATE TRIGGER IF NOT EXISTS comments_au AFTER UPDATE ON comments BEGIN
                INSERT INTO comments_fts(comments_fts, rowid, body) VALUES ('delete', old.id, old.body);
                INSERT INTO comments_fts(rowid, body) VALUES (new.id, new.body);
            END;

            CREATE TABLE IF NOT EXISTS issue_links (
                source_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
                target_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
                link_type TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (source_id, target_id, link_type)
            );"
        )?;

        // Migration: add priority column (idempotent — ignore error if column exists)
        let _ = self.conn.execute("ALTER TABLE issues ADD COLUMN priority TEXT", []);

        Ok(())
    }

    fn now() -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    fn get_or_create_label(&self, name: &str) -> Result<i64> {
        self.conn.execute("INSERT OR IGNORE INTO labels (name) VALUES (?1)", params![name])?;
        Ok(self.conn.query_row("SELECT id FROM labels WHERE name = ?1", params![name], |row| row.get(0))?)
    }

    fn attach_labels(&self, issue_id: i64, labels: &[String]) -> Result<()> {
        for name in labels {
            let label_id = self.get_or_create_label(name)?;
            self.conn.execute(
                "INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?1, ?2)",
                params![issue_id, label_id],
            )?;
        }
        Ok(())
    }

    fn issue_labels(&self, issue_id: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.name FROM labels l
             JOIN issue_labels il ON il.label_id = l.id
             WHERE il.issue_id = ?1
             ORDER BY l.name"
        )?;
        let rows = stmt.query_map(params![issue_id], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn issue_comments(&self, issue_id: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, body, created_at FROM comments WHERE issue_id = ?1 ORDER BY created_at"
        )?;
        let rows = stmt.query_map(params![issue_id], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "issue_id": issue_id,
                "body": row.get::<_, String>(1)?,
                "created_at": row.get::<_, String>(2)?
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn issue_link_list(&self, id: i64) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_id, target_id, link_type, created_at FROM issue_links
             WHERE source_id = ?1 OR target_id = ?1
             ORDER BY created_at"
        )?;
        let rows = stmt.query_map(params![id], |row| {
            Ok(json!({
                "source_id": row.get::<_, i64>(0)?,
                "target_id": row.get::<_, i64>(1)?,
                "link_type": row.get::<_, String>(2)?,
                "created_at": row.get::<_, String>(3)?
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn issue_to_json(&self, id: i64, title: &str, body: Option<&str>, status: &str, closed_reason: Option<&str>, priority: Option<&str>, created_at: &str, updated_at: &str, include_comments: bool) -> Result<Value> {
        let labels = self.issue_labels(id)?;
        let comments = if include_comments { self.issue_comments(id)? } else { vec![] };
        let links = self.issue_link_list(id)?;
        Ok(json!({
            "id": id,
            "title": title,
            "body": body,
            "status": status,
            "priority": priority,
            "labels": labels,
            "closed_reason": closed_reason,
            "created_at": created_at,
            "updated_at": updated_at,
            "comments": comments,
            "links": links
        }))
    }

    fn full_issue(&self, id: i64, include_comments: bool) -> Result<Option<Value>> {
        let mut stmt = self.conn.prepare(
            "SELECT title, body, status, closed_reason, priority, created_at, updated_at FROM issues WHERE id = ?1"
        )?;
        let mut rows = stmt.query(params![id])?;
        match rows.next()? {
            Some(row) => {
                let title: String = row.get(0)?;
                let body: Option<String> = row.get(1)?;
                let status: String = row.get(2)?;
                let closed_reason: Option<String> = row.get(3)?;
                let priority: Option<String> = row.get(4)?;
                let created_at: String = row.get(5)?;
                let updated_at: String = row.get(6)?;
                Ok(Some(self.issue_to_json(
                    id, &title, body.as_deref(), &status, closed_reason.as_deref(),
                    priority.as_deref(), &created_at, &updated_at, include_comments,
                )?))
            }
            None => Ok(None),
        }
    }

    fn require_issue(&self, id: i64, include_comments: bool) -> Result<Value> {
        self.full_issue(id, include_comments)?.ok_or(PitError::NotFound)
    }

    fn assert_exists(&self, id: i64) -> Result<()> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE id = ?1", params![id], |row| row.get(0),
        )?;
        if count == 0 { Err(PitError::NotFound) } else { Ok(()) }
    }

    pub fn create_issue(&self, args: &Value) -> Result<Value> {
        let req = CreateIssueRequest::parse(args)?;
        let now = Self::now();

        self.conn.execute(
            "INSERT INTO issues (title, body, status, priority, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![req.title, req.body, req.status, req.priority, &now, &now],
        )?;

        let id = self.conn.last_insert_rowid();
        self.attach_labels(id, &req.labels)?;
        self.require_issue(id, true)
    }

    pub fn list_issues(&self, args: &Value) -> Result<Value> {
        let req = ListIssuesRequest::parse(args)?;
        let sort_col = req.sort_column();
        let order_dir = req.order_dir();

        let mut where_clauses = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref s) = req.status {
            param_values.push(Box::new(s.clone()));
            where_clauses.push(format!("i.status = ?{}", param_values.len()));
        }

        if let Some(ref p) = req.priority {
            param_values.push(Box::new(p.clone()));
            where_clauses.push(format!("i.priority = ?{}", param_values.len()));
        }

        if !req.labels.is_empty() {
            let placeholders: Vec<String> = req.labels.iter().enumerate()
                .map(|(idx, _)| format!("?{}", param_values.len() + idx + 1))
                .collect();
            for name in &req.labels {
                param_values.push(Box::new(name.clone()));
            }
            where_clauses.push(format!(
                "i.id IN (SELECT il.issue_id FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE l.name IN ({}) GROUP BY il.issue_id HAVING COUNT(DISTINCT l.name) = {})",
                placeholders.join(", "),
                req.labels.len()
            ));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let count_sql = format!("SELECT COUNT(*) FROM issues i {where_sql}");
        let total: i64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(param_values.iter().map(|p| p.as_ref())),
            |row| row.get(0),
        )?;

        let query_sql = format!(
            "SELECT i.id, i.title, i.body, i.status, i.closed_reason, i.priority, i.created_at, i.updated_at
             FROM issues i {where_sql}
             ORDER BY i.{sort_col} {order_dir}
             LIMIT ?{} OFFSET ?{}",
            param_values.len() + 1,
            param_values.len() + 2
        );
        param_values.push(Box::new(req.limit));
        param_values.push(Box::new(req.offset));

        let mut stmt = self.conn.prepare(&query_sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(param_values.iter().map(|p| p.as_ref())),
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            }
        )?;

        let mut issues = Vec::new();
        for row in rows {
            let (id, title, body, status, closed_reason, priority, created_at, updated_at) = row?;
            issues.push(self.issue_to_json(
                id, &title, body.as_deref(), &status, closed_reason.as_deref(),
                priority.as_deref(), &created_at, &updated_at, false,
            )?);
        }

        Ok(json!({
            "issues": issues,
            "total": total,
            "offset": req.offset,
            "limit": req.limit
        }))
    }

    pub fn get_issue(&self, args: &Value) -> Result<Value> {
        let req = IssueIdRequest::parse(args)?;
        self.require_issue(req.id, true)
    }

    pub fn update_issue(&self, args: &Value) -> Result<Value> {
        let req = UpdateIssueRequest::parse(args)?;
        self.assert_exists(req.id)?;

        let now = Self::now();
        let mut sets = vec!["updated_at = ?1".to_string()];
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];

        if let Some(ref title) = req.title {
            param_values.push(Box::new(title.clone()));
            sets.push(format!("title = ?{}", param_values.len()));
        }
        if let Some(ref body) = req.body {
            param_values.push(Box::new(body.clone()));
            sets.push(format!("body = ?{}", param_values.len()));
        }
        if let Some(ref status) = req.status {
            param_values.push(Box::new(status.clone()));
            sets.push(format!("status = ?{}", param_values.len()));
        }
        if let Some(ref reason) = req.closed_reason {
            param_values.push(Box::new(reason.clone()));
            sets.push(format!("closed_reason = ?{}", param_values.len()));
        }
        if let Some(ref priority) = req.priority {
            param_values.push(Box::new(priority.clone()));
            sets.push(format!("priority = ?{}", param_values.len()));
        }

        param_values.push(Box::new(req.id));
        let id_param = param_values.len();
        let sql = format!("UPDATE issues SET {} WHERE id = ?{}", sets.join(", "), id_param);
        self.conn.execute(&sql, rusqlite::params_from_iter(param_values.iter().map(|p| p.as_ref())))?;

        if let Some(ref labels) = req.labels_set {
            self.conn.execute("DELETE FROM issue_labels WHERE issue_id = ?1", params![req.id])?;
            self.attach_labels(req.id, labels)?;
        }
        if let Some(ref labels) = req.labels_add {
            self.attach_labels(req.id, labels)?;
        }
        if let Some(ref labels) = req.labels_remove {
            for name in labels {
                self.conn.execute(
                    "DELETE FROM issue_labels WHERE issue_id = ?1 AND label_id = (SELECT id FROM labels WHERE name = ?2)",
                    params![req.id, name],
                )?;
            }
        }

        self.require_issue(req.id, true)
    }

    pub fn add_comment(&self, args: &Value) -> Result<Value> {
        let req = AddCommentRequest::parse(args)?;
        self.assert_exists(req.id)?;

        let now = Self::now();
        self.conn.execute(
            "INSERT INTO comments (issue_id, body, created_at) VALUES (?1, ?2, ?3)",
            params![req.id, req.body, &now],
        )?;

        let comment_id = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE issues SET updated_at = ?1 WHERE id = ?2", params![&now, req.id])?;

        Ok(json!({
            "id": comment_id,
            "issue_id": req.id,
            "body": req.body,
            "created_at": now
        }))
    }

    pub fn search_issues(&self, args: &Value) -> Result<Value> {
        let req = SearchIssuesRequest::parse(args)?;
        let fts_query = req.fts_query();

        let mut issue_matches: std::collections::HashMap<i64, (bool, bool)> = std::collections::HashMap::new();

        // Search issues FTS
        let mut stmt = self.conn.prepare(
            "SELECT rowid FROM issues_fts WHERE issues_fts MATCH ?1 ORDER BY rank LIMIT 500"
        )?;
        let rows = stmt.query_map(params![fts_query], |row| row.get::<_, i64>(0))?;
        for row in rows {
            issue_matches.insert(row?, (true, false));
        }

        // Search comments FTS
        let mut stmt = self.conn.prepare(
            "SELECT c.issue_id FROM comments_fts cf JOIN comments c ON c.id = cf.rowid WHERE comments_fts MATCH ?1 LIMIT 500"
        )?;
        let rows = stmt.query_map(params![fts_query], |row| row.get::<_, i64>(0))?;
        for row in rows {
            let issue_id = row?;
            issue_matches.entry(issue_id).or_insert((false, false)).1 = true;
        }

        let mut all_ids: Vec<i64> = issue_matches.keys().copied().collect();
        if all_ids.is_empty() {
            return Ok(json!({ "results": [], "total": 0 }));
        }
        all_ids.sort();

        let mut results = Vec::new();
        for &issue_id in &all_ids {
            if let Some(issue) = self.full_issue(issue_id, false)? {
                if let Some(ref s) = req.status {
                    if issue.get("status").and_then(|v| v.as_str()) != Some(s) {
                        continue;
                    }
                }
                if !req.labels.is_empty() {
                    let issue_labels: Vec<&str> = issue.get("labels")
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();
                    if !req.labels.iter().all(|l| issue_labels.contains(&l.as_str())) {
                        continue;
                    }
                }
                let (title_match, comment_match) = issue_matches[&issue_id];
                results.push(json!({
                    "issue": issue,
                    "matches": {
                        "title": title_match,
                        "body": title_match,
                        "comments": comment_match
                    }
                }));
                if results.len() as i64 >= req.limit {
                    break;
                }
            }
        }

        let total = results.len();
        Ok(json!({ "results": results, "total": total }))
    }

    pub fn list_labels(&self) -> Result<Value> {
        let mut stmt = self.conn.prepare(
            "SELECT l.name, COUNT(il.issue_id) as issue_count
             FROM labels l
             LEFT JOIN issue_labels il ON il.label_id = l.id
             GROUP BY l.id, l.name
             ORDER BY l.name"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(json!({
                "name": row.get::<_, String>(0)?,
                "issue_count": row.get::<_, i64>(1)?
            }))
        })?;
        let labels: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
        Ok(json!({ "labels": labels }))
    }

    pub fn link_issues(&self, args: &Value) -> Result<Value> {
        let req = LinkIssuesRequest::parse(args)?;
        self.assert_exists(req.source_id)?;
        self.assert_exists(req.target_id)?;

        let now = Self::now();
        self.conn.execute(
            "INSERT OR IGNORE INTO issue_links (source_id, target_id, link_type, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![req.source_id, req.target_id, req.link_type, &now],
        )?;

        Ok(json!({
            "source_id": req.source_id,
            "target_id": req.target_id,
            "link_type": req.link_type,
            "created_at": now
        }))
    }

    pub fn unlink_issues(&self, args: &Value) -> Result<Value> {
        let req = UnlinkIssuesRequest::parse(args)?;
        let changes = self.conn.execute(
            "DELETE FROM issue_links WHERE source_id = ?1 AND target_id = ?2 AND link_type = ?3",
            params![req.source_id, req.target_id, req.link_type],
        )?;
        if changes == 0 {
            return Err(PitError::NotFound);
        }
        Ok(json!({
            "deleted": true,
            "source_id": req.source_id,
            "target_id": req.target_id,
            "link_type": req.link_type
        }))
    }

    pub fn delete_issue(&self, args: &Value) -> Result<Value> {
        let req = IssueIdRequest::parse(args)?;
        let changes = self.conn.execute("DELETE FROM issues WHERE id = ?1", params![req.id])?;
        if changes == 0 {
            return Err(PitError::NotFound);
        }
        Ok(json!({ "deleted": true, "id": req.id }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_db() -> Db {
        Db::open(Path::new(":memory:")).unwrap()
    }

    fn create(db: &Db, title: &str) -> Value {
        db.create_issue(&json!({"title": title})).unwrap()
    }

    fn create_with(db: &Db, args: Value) -> Value {
        db.create_issue(&args).unwrap()
    }

    // --- create_issue ---

    #[test]
    fn create_issue_basic() {
        let db = test_db();
        let issue = create(&db, "first bug");
        assert_eq!(issue["title"], "first bug");
        assert_eq!(issue["status"], "open");
        assert_eq!(issue["body"], Value::Null);
        assert_eq!(issue["labels"].as_array().unwrap().len(), 0);
        assert_eq!(issue["comments"].as_array().unwrap().len(), 0);
        assert!(issue["id"].as_i64().unwrap() > 0);
        assert!(issue["created_at"].as_str().is_some());
    }

    #[test]
    fn create_issue_with_body_and_labels() {
        let db = test_db();
        let issue = create_with(&db, json!({
            "title": "labeled",
            "body": "details here",
            "labels": ["bug", "p0"]
        }));
        assert_eq!(issue["body"], "details here");
        let labels: Vec<&str> = issue["labels"].as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(labels, vec!["bug", "p0"]);
    }

    #[test]
    fn create_issue_in_progress() {
        let db = test_db();
        let issue = create_with(&db, json!({"title": "wip", "status": "in-progress"}));
        assert_eq!(issue["status"], "in-progress");
    }

    #[test]
    fn create_issue_invalid_status() {
        let db = test_db();
        let err = db.create_issue(&json!({"title": "x", "status": "closed"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid status")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn create_issue_missing_title() {
        let db = test_db();
        assert!(db.create_issue(&json!({})).is_err());
    }

    // --- get_issue ---

    #[test]
    fn get_issue_exists() {
        let db = test_db();
        let created = create(&db, "to get");
        let id = created["id"].as_i64().unwrap();
        let fetched = db.get_issue(&json!({"id": id})).unwrap();
        assert_eq!(fetched["title"], "to get");
        assert_eq!(fetched["id"], id);
    }

    #[test]
    fn get_issue_not_found() {
        let db = test_db();
        let err = db.get_issue(&json!({"id": 999})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn get_issue_includes_comments() {
        let db = test_db();
        let issue = create(&db, "commentable");
        let id = issue["id"].as_i64().unwrap();
        db.add_comment(&json!({"id": id, "body": "first"})).unwrap();
        db.add_comment(&json!({"id": id, "body": "second"})).unwrap();

        let fetched = db.get_issue(&json!({"id": id})).unwrap();
        let comments = fetched["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0]["body"], "first");
        assert_eq!(comments[1]["body"], "second");
    }

    // --- list_issues ---

    #[test]
    fn list_issues_empty() {
        let db = test_db();
        let result = db.list_issues(&json!({})).unwrap();
        assert_eq!(result["total"], 0);
        assert_eq!(result["issues"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn list_issues_returns_all() {
        let db = test_db();
        create(&db, "a");
        create(&db, "b");
        create(&db, "c");
        let result = db.list_issues(&json!({})).unwrap();
        assert_eq!(result["total"], 3);
        assert_eq!(result["issues"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn list_issues_filter_by_status() {
        let db = test_db();
        create(&db, "open one");
        create_with(&db, json!({"title": "wip", "status": "in-progress"}));

        let result = db.list_issues(&json!({"status": "open"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["issues"][0]["title"], "open one");

        let result = db.list_issues(&json!({"status": "in-progress"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["issues"][0]["title"], "wip");
    }

    #[test]
    fn list_issues_filter_by_labels() {
        let db = test_db();
        create_with(&db, json!({"title": "bug1", "labels": ["bug"]}));
        create_with(&db, json!({"title": "feat1", "labels": ["feature"]}));
        create_with(&db, json!({"title": "both", "labels": ["bug", "feature"]}));

        let result = db.list_issues(&json!({"labels": ["bug"]})).unwrap();
        assert_eq!(result["total"], 2);

        let result = db.list_issues(&json!({"labels": ["bug", "feature"]})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["issues"][0]["title"], "both");
    }

    #[test]
    fn list_issues_pagination() {
        let db = test_db();
        for i in 0..5 {
            create(&db, &format!("issue {i}"));
        }

        let result = db.list_issues(&json!({"limit": 2, "offset": 0, "sort": "id", "order": "asc"})).unwrap();
        assert_eq!(result["total"], 5);
        assert_eq!(result["issues"].as_array().unwrap().len(), 2);

        let result = db.list_issues(&json!({"limit": 2, "offset": 3, "sort": "id", "order": "asc"})).unwrap();
        assert_eq!(result["issues"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn list_issues_sort_by_created_asc() {
        let db = test_db();
        create(&db, "first");
        create(&db, "second");
        let result = db.list_issues(&json!({"sort": "id", "order": "asc"})).unwrap();
        let issues = result["issues"].as_array().unwrap();
        assert_eq!(issues[0]["title"], "first");
        assert_eq!(issues[1]["title"], "second");
    }

    #[test]
    fn list_issues_does_not_include_comments() {
        let db = test_db();
        let issue = create(&db, "with comment");
        let id = issue["id"].as_i64().unwrap();
        db.add_comment(&json!({"id": id, "body": "hello"})).unwrap();

        let result = db.list_issues(&json!({})).unwrap();
        let comments = result["issues"][0]["comments"].as_array().unwrap();
        assert!(comments.is_empty());
    }

    // --- update_issue ---

    #[test]
    fn update_issue_title() {
        let db = test_db();
        let issue = create(&db, "old title");
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({"id": id, "title": "new title"})).unwrap();
        assert_eq!(updated["title"], "new title");
    }

    #[test]
    fn update_issue_body() {
        let db = test_db();
        let issue = create(&db, "no body");
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({"id": id, "body": "now has body"})).unwrap();
        assert_eq!(updated["body"], "now has body");
    }

    #[test]
    fn update_issue_status_and_reason() {
        let db = test_db();
        let issue = create(&db, "to close");
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({
            "id": id,
            "status": "closed",
            "closed_reason": "completed"
        })).unwrap();
        assert_eq!(updated["status"], "closed");
        assert_eq!(updated["closed_reason"], "completed");
    }

    #[test]
    fn update_issue_not_found() {
        let db = test_db();
        let err = db.update_issue(&json!({"id": 999})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn update_issue_labels_add() {
        let db = test_db();
        let issue = create_with(&db, json!({"title": "labeled", "labels": ["bug"]}));
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({"id": id, "labels_add": ["p0"]})).unwrap();
        let labels: Vec<&str> = updated["labels"].as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert!(labels.contains(&"bug"));
        assert!(labels.contains(&"p0"));
    }

    #[test]
    fn update_issue_labels_remove() {
        let db = test_db();
        let issue = create_with(&db, json!({"title": "labeled", "labels": ["bug", "p0"]}));
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({"id": id, "labels_remove": ["bug"]})).unwrap();
        let labels: Vec<&str> = updated["labels"].as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(labels, vec!["p0"]);
    }

    #[test]
    fn update_issue_labels_set() {
        let db = test_db();
        let issue = create_with(&db, json!({"title": "labeled", "labels": ["bug", "p0"]}));
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({"id": id, "labels_set": ["feature", "p1"]})).unwrap();
        let labels: Vec<&str> = updated["labels"].as_array().unwrap()
            .iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(labels, vec!["feature", "p1"]);
    }

    #[test]
    fn update_issue_updates_timestamp() {
        let db = test_db();
        let issue = create(&db, "ts test");
        let id = issue["id"].as_i64().unwrap();
        let original_updated = issue["updated_at"].as_str().unwrap().to_string();

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let updated = db.update_issue(&json!({"id": id, "title": "changed"})).unwrap();
        assert_ne!(updated["updated_at"].as_str().unwrap(), original_updated);
    }

    // --- add_comment ---

    #[test]
    fn add_comment_basic() {
        let db = test_db();
        let issue = create(&db, "commentable");
        let id = issue["id"].as_i64().unwrap();
        let comment = db.add_comment(&json!({"id": id, "body": "hello"})).unwrap();
        assert_eq!(comment["issue_id"], id);
        assert_eq!(comment["body"], "hello");
        assert!(comment["id"].as_i64().unwrap() > 0);
    }

    #[test]
    fn add_comment_not_found() {
        let db = test_db();
        let err = db.add_comment(&json!({"id": 999, "body": "nope"})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn add_comment_updates_issue_timestamp() {
        let db = test_db();
        let issue = create(&db, "ts comment");
        let id = issue["id"].as_i64().unwrap();
        let original = issue["updated_at"].as_str().unwrap().to_string();

        std::thread::sleep(std::time::Duration::from_millis(1100));
        db.add_comment(&json!({"id": id, "body": "bump"})).unwrap();

        let fetched = db.get_issue(&json!({"id": id})).unwrap();
        assert_ne!(fetched["updated_at"].as_str().unwrap(), original);
    }

    // --- search_issues ---

    #[test]
    fn search_by_title() {
        let db = test_db();
        create(&db, "fix the parser bug");
        create(&db, "add new feature");

        let result = db.search_issues(&json!({"query": "parser"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["results"][0]["issue"]["title"], "fix the parser bug");
        assert_eq!(result["results"][0]["matches"]["title"], true);
    }

    #[test]
    fn search_by_body() {
        let db = test_db();
        create_with(&db, json!({"title": "issue", "body": "the frobnicator is broken"}));
        create(&db, "unrelated");

        let result = db.search_issues(&json!({"query": "frobnicator"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["results"][0]["issue"]["title"], "issue");
    }

    #[test]
    fn search_by_comment() {
        let db = test_db();
        let issue = create(&db, "vague title");
        let id = issue["id"].as_i64().unwrap();
        db.add_comment(&json!({"id": id, "body": "the xylophone module crashes"})).unwrap();

        let result = db.search_issues(&json!({"query": "xylophone"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["results"][0]["matches"]["comments"], true);
    }

    #[test]
    fn search_with_status_filter() {
        let db = test_db();
        create(&db, "open parser issue");
        let closed = create(&db, "closed parser issue");
        let cid = closed["id"].as_i64().unwrap();
        db.update_issue(&json!({"id": cid, "status": "closed", "closed_reason": "completed"})).unwrap();

        let result = db.search_issues(&json!({"query": "parser", "status": "open"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["results"][0]["issue"]["status"], "open");
    }

    #[test]
    fn search_no_results() {
        let db = test_db();
        create(&db, "nothing relevant");
        let result = db.search_issues(&json!({"query": "zzzznonexistent"})).unwrap();
        assert_eq!(result["total"], 0);
        assert!(result["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn search_respects_limit() {
        let db = test_db();
        for i in 0..10 {
            create(&db, &format!("searchable item {i}"));
        }
        let result = db.search_issues(&json!({"query": "searchable", "limit": 3})).unwrap();
        assert_eq!(result["results"].as_array().unwrap().len(), 3);
    }

    // --- list_labels ---

    #[test]
    fn list_labels_empty() {
        let db = test_db();
        let result = db.list_labels().unwrap();
        assert!(result["labels"].as_array().unwrap().is_empty());
    }

    #[test]
    fn list_labels_with_counts() {
        let db = test_db();
        create_with(&db, json!({"title": "a", "labels": ["bug"]}));
        create_with(&db, json!({"title": "b", "labels": ["bug", "feature"]}));
        create_with(&db, json!({"title": "c", "labels": ["feature"]}));

        let result = db.list_labels().unwrap();
        let labels = result["labels"].as_array().unwrap();
        assert_eq!(labels.len(), 2);

        let bug = labels.iter().find(|l| l["name"] == "bug").unwrap();
        assert_eq!(bug["issue_count"], 2);
        let feature = labels.iter().find(|l| l["name"] == "feature").unwrap();
        assert_eq!(feature["issue_count"], 2);
    }

    // --- delete_issue ---

    #[test]
    fn delete_issue_exists() {
        let db = test_db();
        let issue = create(&db, "to delete");
        let id = issue["id"].as_i64().unwrap();
        let result = db.delete_issue(&json!({"id": id})).unwrap();
        assert_eq!(result["deleted"], true);
        assert_eq!(result["id"], id);

        let err = db.get_issue(&json!({"id": id})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn delete_issue_not_found() {
        let db = test_db();
        let err = db.delete_issue(&json!({"id": 999})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn delete_cascades_comments_and_labels() {
        let db = test_db();
        let issue = create_with(&db, json!({"title": "cascade", "labels": ["bug"]}));
        let id = issue["id"].as_i64().unwrap();
        db.add_comment(&json!({"id": id, "body": "a comment"})).unwrap();

        db.delete_issue(&json!({"id": id})).unwrap();

        // Verify comments are gone
        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM comments WHERE issue_id = ?1", params![id], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0);

        // Verify issue_labels are gone
        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM issue_labels WHERE issue_id = ?1", params![id], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 0);
    }

    // --- label auto-creation and reuse ---

    #[test]
    fn labels_are_reused_across_issues() {
        let db = test_db();
        create_with(&db, json!({"title": "a", "labels": ["bug"]}));
        create_with(&db, json!({"title": "b", "labels": ["bug"]}));

        let count: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM labels WHERE name = 'bug'", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    // --- priority ---

    #[test]
    fn create_issue_with_priority() {
        let db = test_db();
        let issue = create_with(&db, json!({"title": "urgent", "priority": "p0"}));
        assert_eq!(issue["priority"], "p0");
    }

    #[test]
    fn create_issue_without_priority() {
        let db = test_db();
        let issue = create(&db, "no priority");
        assert_eq!(issue["priority"], Value::Null);
    }

    #[test]
    fn create_issue_invalid_priority() {
        let db = test_db();
        let err = db.create_issue(&json!({"title": "x", "priority": "high"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid priority")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn update_issue_priority() {
        let db = test_db();
        let issue = create(&db, "to prioritize");
        let id = issue["id"].as_i64().unwrap();
        let updated = db.update_issue(&json!({"id": id, "priority": "p1"})).unwrap();
        assert_eq!(updated["priority"], "p1");
    }

    #[test]
    fn list_issues_filter_by_priority() {
        let db = test_db();
        create_with(&db, json!({"title": "critical", "priority": "p0"}));
        create_with(&db, json!({"title": "low", "priority": "p3"}));
        create(&db, "no priority");

        let result = db.list_issues(&json!({"priority": "p0"})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["issues"][0]["title"], "critical");
    }

    // --- linking ---

    #[test]
    fn link_and_get_issue_shows_links() {
        let db = test_db();
        let a = create(&db, "blocker");
        let b = create(&db, "blocked");
        let aid = a["id"].as_i64().unwrap();
        let bid = b["id"].as_i64().unwrap();

        db.link_issues(&json!({"source_id": aid, "target_id": bid, "link_type": "blocks"})).unwrap();

        let fetched = db.get_issue(&json!({"id": aid})).unwrap();
        let links = fetched["links"].as_array().unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0]["link_type"], "blocks");
        assert_eq!(links[0]["target_id"], bid);

        // target also sees the link
        let fetched_b = db.get_issue(&json!({"id": bid})).unwrap();
        assert_eq!(fetched_b["links"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn link_self_rejected() {
        let db = test_db();
        let a = create(&db, "self");
        let aid = a["id"].as_i64().unwrap();
        let err = db.link_issues(&json!({"source_id": aid, "target_id": aid, "link_type": "blocks"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("itself")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn link_not_found() {
        let db = test_db();
        let a = create(&db, "exists");
        let aid = a["id"].as_i64().unwrap();
        let err = db.link_issues(&json!({"source_id": aid, "target_id": 999, "link_type": "blocks"})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn unlink_issues_basic() {
        let db = test_db();
        let a = create(&db, "a");
        let b = create(&db, "b");
        let aid = a["id"].as_i64().unwrap();
        let bid = b["id"].as_i64().unwrap();

        db.link_issues(&json!({"source_id": aid, "target_id": bid, "link_type": "relates_to"})).unwrap();
        let result = db.unlink_issues(&json!({"source_id": aid, "target_id": bid, "link_type": "relates_to"})).unwrap();
        assert_eq!(result["deleted"], true);

        let fetched = db.get_issue(&json!({"id": aid})).unwrap();
        assert!(fetched["links"].as_array().unwrap().is_empty());
    }

    #[test]
    fn unlink_not_found() {
        let db = test_db();
        let err = db.unlink_issues(&json!({"source_id": 1, "target_id": 2, "link_type": "blocks"})).unwrap_err();
        match err {
            PitError::NotFound => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn delete_issue_cascades_links() {
        let db = test_db();
        let a = create(&db, "a");
        let b = create(&db, "b");
        let aid = a["id"].as_i64().unwrap();
        let bid = b["id"].as_i64().unwrap();

        db.link_issues(&json!({"source_id": aid, "target_id": bid, "link_type": "blocks"})).unwrap();
        db.delete_issue(&json!({"id": aid})).unwrap();

        let fetched = db.get_issue(&json!({"id": bid})).unwrap();
        assert!(fetched["links"].as_array().unwrap().is_empty());
    }

    // --- search with labels filter ---

    #[test]
    fn search_with_label_filter() {
        let db = test_db();
        create_with(&db, json!({"title": "searchable bug", "labels": ["bug"]}));
        create_with(&db, json!({"title": "searchable feature", "labels": ["feature"]}));

        let result = db.search_issues(&json!({"query": "searchable", "labels": ["bug"]})).unwrap();
        assert_eq!(result["total"], 1);
        assert_eq!(result["results"][0]["issue"]["title"], "searchable bug");
    }

    // --- idempotent migration ---

    #[test]
    fn migrate_is_idempotent() {
        let db = test_db();
        db.migrate().unwrap();
        db.migrate().unwrap();
        create(&db, "still works");
    }
}
