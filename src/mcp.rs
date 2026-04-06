use serde::Serialize;
use serde_json::{Value, json};
use crate::db::Db;

const INSTRUCTIONS: &str = include_str!("prompts/mcp.md");

// --- Type-safe tool schema definitions ---

#[derive(Serialize)]
struct ToolDef {
    name: &'static str,
    description: &'static str,
    #[serde(rename = "inputSchema")]
    input_schema: SchemaObject,
}

#[derive(Serialize)]
struct SchemaObject {
    r#type: &'static str,
    properties: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    required: Vec<&'static str>,
}

impl SchemaObject {
    fn new(properties: Value, required: Vec<&'static str>) -> Self {
        Self { r#type: "object", properties, required }
    }
}

fn tool_definitions() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "create_issue",
            description: "Create a new issue.",
            input_schema: SchemaObject::new(json!({
                "title":  { "type": "string", "description": "Required." },
                "body":   { "type": "string" },
                "labels": { "type": "array", "items": { "type": "string" }, "description": "Created if they don't exist." },
                "status": { "type": "string", "enum": ["open", "in-progress"], "default": "open" },
                "priority": { "type": "string", "enum": ["p0", "p1", "p2", "p3"], "description": "Priority level. p0 = critical, p3 = low." }
            }), vec!["title"]),
        },
        ToolDef {
            name: "list_issues",
            description: "List issues with optional filters, sorting, and pagination.",
            input_schema: SchemaObject::new(json!({
                "status": { "type": "string", "enum": ["open", "in-progress", "closed"] },
                "priority": { "type": "string", "enum": ["p0", "p1", "p2", "p3"], "description": "Filter by priority." },
                "labels": { "type": "array", "items": { "type": "string" }, "description": "Match ALL given labels." },
                "sort":   { "type": "string", "enum": ["created", "updated", "id"], "default": "updated" },
                "order":  { "type": "string", "enum": ["asc", "desc"], "default": "desc" },
                "limit":  { "type": "integer", "default": 50, "maximum": 200 },
                "offset": { "type": "integer", "default": 0 }
            }), vec![]),
        },
        ToolDef {
            name: "get_issue",
            description: "Get a single issue by ID, including all comments.",
            input_schema: SchemaObject::new(json!({
                "id": { "type": "integer", "description": "Required." }
            }), vec!["id"]),
        },
        ToolDef {
            name: "update_issue",
            description: "Update an existing issue. Only supplied fields are changed.",
            input_schema: SchemaObject::new(json!({
                "id":            { "type": "integer", "description": "Required." },
                "title":         { "type": "string" },
                "body":          { "type": "string" },
                "status":        { "type": "string", "enum": ["open", "in-progress", "closed"] },
                "closed_reason": { "type": "string", "enum": ["completed", "wontfix", "duplicate"] },
                "priority":      { "type": "string", "enum": ["p0", "p1", "p2", "p3"] },
                "labels_add":    { "type": "array", "items": { "type": "string" } },
                "labels_remove": { "type": "array", "items": { "type": "string" } },
                "labels_set":    { "type": "array", "items": { "type": "string" } }
            }), vec!["id"]),
        },
        ToolDef {
            name: "add_comment",
            description: "Add a comment to an issue.",
            input_schema: SchemaObject::new(json!({
                "id":   { "type": "integer", "description": "Issue ID. Required." },
                "body": { "type": "string", "description": "Required." }
            }), vec!["id", "body"]),
        },
        ToolDef {
            name: "search_issues",
            description: "Full-text search across issue titles, bodies, and comments.",
            input_schema: SchemaObject::new(json!({
                "query":  { "type": "string", "description": "Required." },
                "status": { "type": "string", "enum": ["open", "in-progress", "closed"] },
                "labels": { "type": "array", "items": { "type": "string" }, "description": "Match ALL given labels." },
                "limit":  { "type": "integer", "default": 20 }
            }), vec!["query"]),
        },
        ToolDef {
            name: "list_labels",
            description: "List all labels with issue counts.",
            input_schema: SchemaObject::new(json!({}), vec![]),
        },
        ToolDef {
            name: "delete_issue",
            description: "Delete an issue and all associated comments and labels.",
            input_schema: SchemaObject::new(json!({
                "id": { "type": "integer", "description": "Required." }
            }), vec!["id"]),
        },
        ToolDef {
            name: "link_issues",
            description: "Create a directional link between two issues.",
            input_schema: SchemaObject::new(json!({
                "source_id": { "type": "integer", "description": "Required." },
                "target_id": { "type": "integer", "description": "Required." },
                "link_type": { "type": "string", "enum": ["blocks", "relates_to", "duplicates"], "description": "Required." }
            }), vec!["source_id", "target_id", "link_type"]),
        },
        ToolDef {
            name: "unlink_issues",
            description: "Remove a link between two issues.",
            input_schema: SchemaObject::new(json!({
                "source_id": { "type": "integer", "description": "Required." },
                "target_id": { "type": "integer", "description": "Required." },
                "link_type": { "type": "string", "enum": ["blocks", "relates_to", "duplicates"], "description": "Required." }
            }), vec!["source_id", "target_id", "link_type"]),
        },
    ]
}

// --- JSON-RPC handling ---

pub fn handle_message(db: &Db, msg: &Value) -> Option<Value> {
    let id = msg.get("id");
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

    match method {
        "initialize" => {
            Some(json_rpc_result(id, json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "pit", "version": "0.1.0" },
                "instructions": INSTRUCTIONS
            })))
        }
        "notifications/initialized" => None,
        "tools/list" => {
            let tools = serde_json::to_value(tool_definitions()).unwrap();
            Some(json_rpc_result(id, json!({ "tools": tools })))
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or(json!({}));
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            let result = match tool_name {
                "create_issue" => db.create_issue(&arguments),
                "list_issues" => db.list_issues(&arguments),
                "get_issue" => db.get_issue(&arguments),
                "update_issue" => db.update_issue(&arguments),
                "add_comment" => db.add_comment(&arguments),
                "search_issues" => db.search_issues(&arguments),
                "list_labels" => db.list_labels(),
                "delete_issue" => db.delete_issue(&arguments),
                "link_issues" => db.link_issues(&arguments),
                "unlink_issues" => db.unlink_issues(&arguments),
                _ => Err(crate::error::PitError::InvalidParams(format!("unknown tool: {tool_name}"))),
            };

            match result {
                Ok(value) => Some(json_rpc_result(id, json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string(&value).unwrap()
                    }]
                }))),
                Err(e) => {
                    let (code, message) = e.to_json_rpc();
                    Some(json_rpc_error(id, code, &message))
                }
            }
        }
        "ping" => Some(json_rpc_result(id, json!({}))),
        _ => Some(json_rpc_error(id, -32600, &format!("unknown method: {method}"))),
    }
}

fn json_rpc_result(id: Option<&Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "result": result
    })
}

fn json_rpc_error(id: Option<&Value>, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.cloned().unwrap_or(Value::Null),
        "error": { "code": code, "message": message }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn test_db() -> Db {
        Db::open(Path::new(":memory:")).unwrap()
    }

    fn call(db: &Db, method: &str, params: Value) -> Option<Value> {
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        handle_message(db, &msg)
    }

    fn call_tool(db: &Db, name: &str, arguments: Value) -> Value {
        call(db, "tools/call", json!({"name": name, "arguments": arguments})).unwrap()
    }

    // --- initialize ---

    #[test]
    fn initialize_returns_server_info() {
        let db = test_db();
        let resp = call(&db, "initialize", json!({})).unwrap();
        let result = &resp["result"];
        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert_eq!(result["serverInfo"]["name"], "pit");
        assert_eq!(result["serverInfo"]["version"], "0.1.0");
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["instructions"].as_str().unwrap().len() > 0);
    }

    // --- notifications/initialized ---

    #[test]
    fn notifications_initialized_returns_none() {
        let db = test_db();
        let msg = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert!(handle_message(&db, &msg).is_none());
    }

    // --- tools/list ---

    #[test]
    fn tools_list_returns_all_tools() {
        let db = test_db();
        let resp = call(&db, "tools/list", json!({})).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 10);

        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"create_issue"));
        assert!(names.contains(&"list_issues"));
        assert!(names.contains(&"get_issue"));
        assert!(names.contains(&"update_issue"));
        assert!(names.contains(&"add_comment"));
        assert!(names.contains(&"search_issues"));
        assert!(names.contains(&"list_labels"));
        assert!(names.contains(&"delete_issue"));
        assert!(names.contains(&"link_issues"));
        assert!(names.contains(&"unlink_issues"));
    }

    #[test]
    fn tools_list_schemas_are_valid() {
        let db = test_db();
        let resp = call(&db, "tools/list", json!({})).unwrap();
        let tools = resp["result"]["tools"].as_array().unwrap();
        for tool in tools {
            assert!(tool["name"].as_str().is_some());
            assert!(tool["description"].as_str().is_some());
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert!(tool["inputSchema"]["properties"].is_object());
        }
    }

    // --- tools/call routing ---

    #[test]
    fn tools_call_create_and_get() {
        let db = test_db();
        let resp = call_tool(&db, "create_issue", json!({"title": "test"}));
        assert!(resp["result"]["content"][0]["text"].as_str().is_some());

        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let issue: Value = serde_json::from_str(text).unwrap();
        assert_eq!(issue["title"], "test");
        let id = issue["id"].as_i64().unwrap();

        let resp = call_tool(&db, "get_issue", json!({"id": id}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let fetched: Value = serde_json::from_str(text).unwrap();
        assert_eq!(fetched["title"], "test");
    }

    #[test]
    fn tools_call_list_issues() {
        let db = test_db();
        call_tool(&db, "create_issue", json!({"title": "a"}));
        let resp = call_tool(&db, "list_issues", json!({}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["total"], 1);
    }

    #[test]
    fn tools_call_update_issue() {
        let db = test_db();
        let resp = call_tool(&db, "create_issue", json!({"title": "old"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let issue: Value = serde_json::from_str(text).unwrap();
        let id = issue["id"].as_i64().unwrap();

        let resp = call_tool(&db, "update_issue", json!({"id": id, "title": "new"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let updated: Value = serde_json::from_str(text).unwrap();
        assert_eq!(updated["title"], "new");
    }

    #[test]
    fn tools_call_add_comment() {
        let db = test_db();
        let resp = call_tool(&db, "create_issue", json!({"title": "t"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let id = serde_json::from_str::<Value>(text).unwrap()["id"].as_i64().unwrap();

        let resp = call_tool(&db, "add_comment", json!({"id": id, "body": "noted"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let comment: Value = serde_json::from_str(text).unwrap();
        assert_eq!(comment["body"], "noted");
    }

    #[test]
    fn tools_call_search_issues() {
        let db = test_db();
        call_tool(&db, "create_issue", json!({"title": "searchable widget"}));
        let resp = call_tool(&db, "search_issues", json!({"query": "widget"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["total"], 1);
    }

    #[test]
    fn tools_call_list_labels() {
        let db = test_db();
        call_tool(&db, "create_issue", json!({"title": "t", "labels": ["bug"]}));
        let resp = call_tool(&db, "list_labels", json!({}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["labels"][0]["name"], "bug");
    }

    #[test]
    fn tools_call_delete_issue() {
        let db = test_db();
        let resp = call_tool(&db, "create_issue", json!({"title": "doomed"}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let id = serde_json::from_str::<Value>(text).unwrap()["id"].as_i64().unwrap();

        let resp = call_tool(&db, "delete_issue", json!({"id": id}));
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let data: Value = serde_json::from_str(text).unwrap();
        assert_eq!(data["deleted"], true);
    }

    // --- error cases ---

    #[test]
    fn tools_call_unknown_tool() {
        let db = test_db();
        let resp = call_tool(&db, "nonexistent", json!({}));
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32600);
    }

    #[test]
    fn tools_call_returns_error_for_not_found() {
        let db = test_db();
        let resp = call_tool(&db, "get_issue", json!({"id": 999}));
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32602);
    }

    #[test]
    fn tools_call_returns_error_for_invalid_params() {
        let db = test_db();
        let resp = call_tool(&db, "create_issue", json!({}));
        assert!(resp["error"].is_object());
        assert_eq!(resp["error"]["code"], -32600);
    }

    // --- ping ---

    #[test]
    fn ping_returns_empty_result() {
        let db = test_db();
        let resp = call(&db, "ping", json!({})).unwrap();
        assert!(resp["result"].is_object());
        assert_eq!(resp["id"], 1);
    }

    // --- unknown method ---

    #[test]
    fn unknown_method_returns_error() {
        let db = test_db();
        let resp = call(&db, "foobar", json!({})).unwrap();
        assert_eq!(resp["error"]["code"], -32600);
        assert!(resp["error"]["message"].as_str().unwrap().contains("foobar"));
    }

    // --- json-rpc envelope ---

    #[test]
    fn response_has_jsonrpc_field() {
        let db = test_db();
        let resp = call(&db, "ping", json!({})).unwrap();
        assert_eq!(resp["jsonrpc"], "2.0");
    }

    #[test]
    fn response_preserves_id() {
        let db = test_db();
        let msg = json!({"jsonrpc": "2.0", "id": 42, "method": "ping", "params": {}});
        let resp = handle_message(&db, &msg).unwrap();
        assert_eq!(resp["id"], 42);
    }

    #[test]
    fn response_null_id_when_missing() {
        let db = test_db();
        let msg = json!({"jsonrpc": "2.0", "method": "ping"});
        let resp = handle_message(&db, &msg).unwrap();
        assert!(resp["id"].is_null());
    }
}
