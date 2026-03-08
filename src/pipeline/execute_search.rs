//! Execute search step — runs the search query via backend.

use serde::Serialize;
use std::time::Instant;

use crate::index::backend::{Backend, SearchHit};
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};

/// Output record for the execute search step.
#[derive(Debug, Serialize)]
pub struct ExecuteSearchOutput {
    /// Number of search hits returned.
    pub hits: usize,
}

impl StepOutput for ExecuteSearchOutput {
    fn format_line(&self) -> String {
        let word = if self.hits == 1 { "hit" } else { "hits" };
        format!("Found {} {word}", self.hits)
    }
}

/// Execute a search query against the index.
///
/// Validates the `--where` clause (quote parity) before running the SQL.
/// Returns the step result and the search hits.
pub(crate) async fn run_execute_search(
    backend: &Backend,
    query_embedding: Vec<f32>,
    where_clause: Option<&str>,
    limit: usize,
) -> (
    ProcessingStepResult<ExecuteSearchOutput>,
    Option<Vec<SearchHit>>,
) {
    // Validate --where clause: unmatched quotes indicate unescaped special characters
    if let Some(w) = where_clause {
        if w.chars().filter(|&c| c == '\'').count() % 2 != 0 {
            let err = ProcessingStepError {
                kind: ErrorKind::User,
                message:
                    "unmatched single quote in --where clause — escape with '' (e.g. O''Brien)"
                        .to_string(),
            };
            return (ProcessingStepResult::Failed(err), None);
        }
        if w.chars().filter(|&c| c == '"').count() % 2 != 0 {
            let err = ProcessingStepError {
                kind: ErrorKind::User,
                message:
                    "unmatched double quote in --where clause — escape with \"\" (e.g. \"\"field\"\")"
                        .to_string(),
            };
            return (ProcessingStepResult::Failed(err), None);
        }
    }

    let start = Instant::now();
    match backend.search(query_embedding, where_clause, limit).await {
        Ok(hits) => {
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: ExecuteSearchOutput { hits: hits.len() },
            };
            (ProcessingStepResult::Completed(step), Some(hits))
        }
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            (ProcessingStepResult::Failed(err), None)
        }
    }
}
