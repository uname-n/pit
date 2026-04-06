use std::fmt;

#[derive(Debug)]
pub enum PitError {
    /// Invalid request parameters (JSON-RPC -32600)
    InvalidParams(String),
    /// Resource not found (JSON-RPC -32602)
    NotFound,
    /// Internal database/server error (JSON-RPC -32603)
    Internal(String),
}

impl fmt::Display for PitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PitError::InvalidParams(msg) => write!(f, "{msg}"),
            PitError::NotFound => write!(f, "NOT_FOUND"),
            PitError::Internal(msg) => write!(f, "{msg}"),
        }
    }
}

impl From<rusqlite::Error> for PitError {
    fn from(e: rusqlite::Error) -> Self {
        PitError::Internal(e.to_string())
    }
}

impl PitError {
    pub fn to_json_rpc(&self) -> (i32, String) {
        match self {
            PitError::InvalidParams(msg) => (-32600, msg.clone()),
            PitError::NotFound => (-32602, "NOT_FOUND".into()),
            PitError::Internal(msg) => (-32603, msg.clone()),
        }
    }
}

pub type Result<T> = std::result::Result<T, PitError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_invalid_params() {
        let e = PitError::InvalidParams("bad field".into());
        assert_eq!(e.to_string(), "bad field");
    }

    #[test]
    fn display_not_found() {
        assert_eq!(PitError::NotFound.to_string(), "NOT_FOUND");
    }

    #[test]
    fn display_internal() {
        let e = PitError::Internal("db broke".into());
        assert_eq!(e.to_string(), "db broke");
    }

    #[test]
    fn to_json_rpc_invalid_params() {
        let e = PitError::InvalidParams("missing title".into());
        assert_eq!(e.to_json_rpc(), (-32600, "missing title".into()));
    }

    #[test]
    fn to_json_rpc_not_found() {
        assert_eq!(PitError::NotFound.to_json_rpc(), (-32602, "NOT_FOUND".into()));
    }

    #[test]
    fn to_json_rpc_internal() {
        let e = PitError::Internal("oops".into());
        assert_eq!(e.to_json_rpc(), (-32603, "oops".into()));
    }

    #[test]
    fn from_rusqlite_error() {
        let sqlite_err = rusqlite::Error::QueryReturnedNoRows;
        let pit_err: PitError = sqlite_err.into();
        match pit_err {
            PitError::Internal(msg) => assert!(msg.contains("Query returned no rows")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }
}
