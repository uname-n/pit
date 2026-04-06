use serde::Deserialize;

use crate::error::{PitError, Result};

fn deserialize_args<'de, T: Deserialize<'de>>(args: &'de serde_json::Value) -> Result<T> {
    T::deserialize(args).map_err(|e| PitError::InvalidParams(e.to_string()))
}

// --- create_issue ---

#[derive(Debug, Deserialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default = "default_open")]
    pub status: String,
    pub priority: Option<String>,
}

fn default_open() -> String { "open".into() }

fn validate_priority(p: &str) -> Result<()> {
    match p {
        "p0" | "p1" | "p2" | "p3" => Ok(()),
        _ => Err(PitError::InvalidParams(format!("invalid priority: {p} (must be p0, p1, p2, or p3)"))),
    }
}

impl CreateIssueRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        let req: Self = deserialize_args(args)?;
        match req.status.as_str() {
            "open" | "in-progress" => {}
            _ => return Err(PitError::InvalidParams(format!("invalid status: {}", req.status))),
        }
        if let Some(ref p) = req.priority {
            validate_priority(p)?;
        }
        Ok(req)
    }
}

// --- list_issues ---

#[derive(Debug, Deserialize)]
pub struct ListIssuesRequest {
    pub status: Option<String>,
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_order")]
    pub order: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_sort() -> String { "updated".into() }
fn default_order() -> String { "desc".into() }
fn default_limit() -> i64 { 50 }

impl ListIssuesRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        let mut req: Self = deserialize_args(args)?;
        req.limit = req.limit.min(200);
        if let Some(ref p) = req.priority {
            validate_priority(p)?;
        }
        Ok(req)
    }

    pub fn sort_column(&self) -> &str {
        match self.sort.as_str() {
            "created" => "created_at",
            "id" => "id",
            _ => "updated_at",
        }
    }

    pub fn order_dir(&self) -> &str {
        if self.order == "asc" { "ASC" } else { "DESC" }
    }
}

// --- get_issue / delete_issue ---

#[derive(Debug, Deserialize)]
pub struct IssueIdRequest {
    pub id: i64,
}

impl IssueIdRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        deserialize_args(args)
    }
}

// --- update_issue ---

#[derive(Debug, Deserialize)]
pub struct UpdateIssueRequest {
    pub id: i64,
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<String>,
    pub closed_reason: Option<String>,
    pub priority: Option<String>,
    pub labels_add: Option<Vec<String>>,
    pub labels_remove: Option<Vec<String>>,
    pub labels_set: Option<Vec<String>>,
}

impl UpdateIssueRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        let req: Self = deserialize_args(args)?;
        if let Some(ref s) = req.status {
            match s.as_str() {
                "open" | "in-progress" | "closed" => {}
                _ => return Err(PitError::InvalidParams(format!("invalid status: {s}"))),
            }
        }
        if let Some(ref r) = req.closed_reason {
            match r.as_str() {
                "completed" | "wontfix" | "duplicate" => {}
                _ => return Err(PitError::InvalidParams(format!("invalid closed_reason: {r}"))),
            }
        }
        if let Some(ref p) = req.priority {
            validate_priority(p)?;
        }
        let label_ops = [&req.labels_add, &req.labels_remove, &req.labels_set]
            .iter()
            .filter(|o| o.is_some())
            .count();
        if label_ops > 1 {
            return Err(PitError::InvalidParams(
                "labels_add, labels_remove, and labels_set are mutually exclusive".into(),
            ));
        }
        Ok(req)
    }
}

// --- add_comment ---

#[derive(Debug, Deserialize)]
pub struct AddCommentRequest {
    pub id: i64,
    pub body: String,
}

impl AddCommentRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        deserialize_args(args)
    }
}

// --- search_issues ---

#[derive(Debug, Deserialize)]
pub struct SearchIssuesRequest {
    pub query: String,
    pub status: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default = "default_search_limit")]
    pub limit: i64,
}

fn default_search_limit() -> i64 { 20 }

impl SearchIssuesRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        deserialize_args(args)
    }

    pub fn fts_query(&self) -> String {
        self.query
            .split_whitespace()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

// --- link_issues ---

#[derive(Debug, Deserialize)]
pub struct LinkIssuesRequest {
    pub source_id: i64,
    pub target_id: i64,
    pub link_type: String,
}

fn validate_link_type(t: &str) -> Result<()> {
    match t {
        "blocks" | "relates_to" | "duplicates" => Ok(()),
        _ => Err(PitError::InvalidParams(format!("invalid link_type: {t} (must be blocks, relates_to, or duplicates)"))),
    }
}

impl LinkIssuesRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        let req: Self = deserialize_args(args)?;
        if req.source_id == req.target_id {
            return Err(PitError::InvalidParams("cannot link an issue to itself".into()));
        }
        validate_link_type(&req.link_type)?;
        Ok(req)
    }
}

// --- unlink_issues ---

#[derive(Debug, Deserialize)]
pub struct UnlinkIssuesRequest {
    pub source_id: i64,
    pub target_id: i64,
    pub link_type: String,
}

impl UnlinkIssuesRequest {
    pub fn parse(args: &serde_json::Value) -> Result<Self> {
        let req: Self = deserialize_args(args)?;
        validate_link_type(&req.link_type)?;
        Ok(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- CreateIssueRequest ---

    #[test]
    fn create_issue_minimal() {
        let req = CreateIssueRequest::parse(&json!({"title": "bug"})).unwrap();
        assert_eq!(req.title, "bug");
        assert_eq!(req.body, None);
        assert!(req.labels.is_empty());
        assert_eq!(req.status, "open");
    }

    #[test]
    fn create_issue_full() {
        let req = CreateIssueRequest::parse(&json!({
            "title": "fix it",
            "body": "details",
            "labels": ["bug", "p0"],
            "status": "in-progress"
        })).unwrap();
        assert_eq!(req.title, "fix it");
        assert_eq!(req.body.as_deref(), Some("details"));
        assert_eq!(req.labels, vec!["bug", "p0"]);
        assert_eq!(req.status, "in-progress");
    }

    #[test]
    fn create_issue_missing_title() {
        let err = CreateIssueRequest::parse(&json!({})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("title")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn create_issue_invalid_status() {
        let err = CreateIssueRequest::parse(&json!({"title": "x", "status": "closed"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid status")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    // --- ListIssuesRequest ---

    #[test]
    fn list_issues_defaults() {
        let req = ListIssuesRequest::parse(&json!({})).unwrap();
        assert_eq!(req.status, None);
        assert!(req.labels.is_empty());
        assert_eq!(req.sort, "updated");
        assert_eq!(req.order, "desc");
        assert_eq!(req.limit, 50);
        assert_eq!(req.offset, 0);
    }

    #[test]
    fn list_issues_limit_capped() {
        let req = ListIssuesRequest::parse(&json!({"limit": 999})).unwrap();
        assert_eq!(req.limit, 200);
    }

    #[test]
    fn list_issues_sort_column() {
        let req = ListIssuesRequest::parse(&json!({"sort": "created"})).unwrap();
        assert_eq!(req.sort_column(), "created_at");

        let req = ListIssuesRequest::parse(&json!({"sort": "id"})).unwrap();
        assert_eq!(req.sort_column(), "id");

        let req = ListIssuesRequest::parse(&json!({"sort": "unknown"})).unwrap();
        assert_eq!(req.sort_column(), "updated_at");
    }

    #[test]
    fn list_issues_order_dir() {
        let req = ListIssuesRequest::parse(&json!({"order": "asc"})).unwrap();
        assert_eq!(req.order_dir(), "ASC");

        let req = ListIssuesRequest::parse(&json!({"order": "desc"})).unwrap();
        assert_eq!(req.order_dir(), "DESC");
    }

    // --- IssueIdRequest ---

    #[test]
    fn issue_id_valid() {
        let req = IssueIdRequest::parse(&json!({"id": 42})).unwrap();
        assert_eq!(req.id, 42);
    }

    #[test]
    fn issue_id_missing() {
        let err = IssueIdRequest::parse(&json!({})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("id")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    // --- UpdateIssueRequest ---

    #[test]
    fn update_issue_minimal() {
        let req = UpdateIssueRequest::parse(&json!({"id": 1})).unwrap();
        assert_eq!(req.id, 1);
        assert!(req.title.is_none());
        assert!(req.status.is_none());
    }

    #[test]
    fn update_issue_invalid_status() {
        let err = UpdateIssueRequest::parse(&json!({"id": 1, "status": "nope"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid status")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn update_issue_invalid_closed_reason() {
        let err = UpdateIssueRequest::parse(&json!({"id": 1, "closed_reason": "nope"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid closed_reason")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn update_issue_mutually_exclusive_labels() {
        let err = UpdateIssueRequest::parse(&json!({
            "id": 1,
            "labels_add": ["a"],
            "labels_remove": ["b"]
        })).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("mutually exclusive")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn update_issue_single_label_op_ok() {
        UpdateIssueRequest::parse(&json!({"id": 1, "labels_add": ["a"]})).unwrap();
        UpdateIssueRequest::parse(&json!({"id": 1, "labels_remove": ["a"]})).unwrap();
        UpdateIssueRequest::parse(&json!({"id": 1, "labels_set": ["a"]})).unwrap();
    }

    // --- AddCommentRequest ---

    #[test]
    fn add_comment_valid() {
        let req = AddCommentRequest::parse(&json!({"id": 5, "body": "noted"})).unwrap();
        assert_eq!(req.id, 5);
        assert_eq!(req.body, "noted");
    }

    #[test]
    fn add_comment_missing_body() {
        let err = AddCommentRequest::parse(&json!({"id": 5})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("body")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    // --- SearchIssuesRequest ---

    #[test]
    fn search_issues_defaults() {
        let req = SearchIssuesRequest::parse(&json!({"query": "bug"})).unwrap();
        assert_eq!(req.query, "bug");
        assert_eq!(req.status, None);
        assert_eq!(req.limit, 20);
    }

    #[test]
    fn search_issues_missing_query() {
        let err = SearchIssuesRequest::parse(&json!({})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("query")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn fts_query_escaping() {
        let req = SearchIssuesRequest::parse(&json!({"query": "hello world"})).unwrap();
        assert_eq!(req.fts_query(), "\"hello\" \"world\"");
    }

    #[test]
    fn fts_query_quotes_escaped() {
        let req = SearchIssuesRequest::parse(&json!({"query": "say \"hi\""})).unwrap();
        // "hi" -> replace quotes -> ""hi"" -> wrap -> """hi"""
        assert_eq!(req.fts_query(), "\"say\" \"\"\"hi\"\"\"");
    }

    #[test]
    fn fts_query_single_term() {
        let req = SearchIssuesRequest::parse(&json!({"query": "bug"})).unwrap();
        assert_eq!(req.fts_query(), "\"bug\"");
    }

    // --- priority validation ---

    #[test]
    fn create_issue_with_priority() {
        let req = CreateIssueRequest::parse(&json!({"title": "x", "priority": "p0"})).unwrap();
        assert_eq!(req.priority, Some("p0".into()));
    }

    #[test]
    fn create_issue_invalid_priority() {
        let err = CreateIssueRequest::parse(&json!({"title": "x", "priority": "high"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid priority")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn update_issue_with_priority() {
        let req = UpdateIssueRequest::parse(&json!({"id": 1, "priority": "p2"})).unwrap();
        assert_eq!(req.priority, Some("p2".into()));
    }

    #[test]
    fn list_issues_with_priority() {
        let req = ListIssuesRequest::parse(&json!({"priority": "p1"})).unwrap();
        assert_eq!(req.priority, Some("p1".into()));
    }

    #[test]
    fn list_issues_invalid_priority() {
        let err = ListIssuesRequest::parse(&json!({"priority": "urgent"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid priority")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    // --- LinkIssuesRequest ---

    #[test]
    fn link_issues_valid() {
        let req = LinkIssuesRequest::parse(&json!({"source_id": 1, "target_id": 2, "link_type": "blocks"})).unwrap();
        assert_eq!(req.source_id, 1);
        assert_eq!(req.target_id, 2);
        assert_eq!(req.link_type, "blocks");
    }

    #[test]
    fn link_issues_self_link_rejected() {
        let err = LinkIssuesRequest::parse(&json!({"source_id": 1, "target_id": 1, "link_type": "blocks"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("itself")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn link_issues_invalid_type() {
        let err = LinkIssuesRequest::parse(&json!({"source_id": 1, "target_id": 2, "link_type": "depends_on"})).unwrap_err();
        match err {
            PitError::InvalidParams(msg) => assert!(msg.contains("invalid link_type")),
            other => panic!("expected InvalidParams, got {other:?}"),
        }
    }

    #[test]
    fn link_issues_all_types_valid() {
        for t in &["blocks", "relates_to", "duplicates"] {
            LinkIssuesRequest::parse(&json!({"source_id": 1, "target_id": 2, "link_type": t})).unwrap();
        }
    }

    // --- UnlinkIssuesRequest ---

    #[test]
    fn unlink_issues_valid() {
        let req = UnlinkIssuesRequest::parse(&json!({"source_id": 1, "target_id": 2, "link_type": "blocks"})).unwrap();
        assert_eq!(req.source_id, 1);
        assert_eq!(req.link_type, "blocks");
    }

    // --- SearchIssuesRequest with labels ---

    #[test]
    fn search_issues_with_labels() {
        let req = SearchIssuesRequest::parse(&json!({"query": "bug", "labels": ["urgent"]})).unwrap();
        assert_eq!(req.labels, vec!["urgent"]);
    }

    #[test]
    fn search_issues_labels_default_empty() {
        let req = SearchIssuesRequest::parse(&json!({"query": "bug"})).unwrap();
        assert!(req.labels.is_empty());
    }
}
