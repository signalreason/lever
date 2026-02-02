use std::{
    error::Error,
    fmt::{self, Display, Formatter},
};

use serde_json::Value;

#[derive(Debug)]
pub struct TaskMetadataError {
    pub task_id: String,
    pub missing: Vec<&'static str>,
}

impl TaskMetadataError {
    pub fn exit_code(&self) -> i32 {
        2
    }
}

impl Display for TaskMetadataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Task {} missing required metadata: {}",
            self.task_id,
            self.missing.join(", ")
        )
    }
}

impl Error for TaskMetadataError {}

pub fn validate_task_metadata(task_id: &str, raw: &Value) -> Result<(), TaskMetadataError> {
    let title_valid = matches!(
        raw.get("title"),
        Some(Value::String(value)) if !value.is_empty()
    );

    let dod_valid = match raw.get("definition_of_done") {
        Some(Value::Array(items)) => {
            !items.is_empty()
                && items.iter().all(|item| match item {
                    Value::String(value) => !value.is_empty(),
                    _ => false,
                })
        }
        _ => false,
    };

    let recommended_valid = match raw.get("recommended") {
        Some(Value::Object(map)) => {
            if map.len() != 1 {
                false
            } else {
                matches!(
                    map.get("approach"),
                    Some(Value::String(value)) if !value.is_empty()
                )
            }
        }
        _ => false,
    };

    let mut missing = Vec::new();
    if !title_valid {
        missing.push("title");
    }
    if !dod_valid {
        missing.push("definition_of_done");
    }
    if !recommended_valid {
        missing.push("recommended.approach");
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(TaskMetadataError {
            task_id: task_id.to_string(),
            missing,
        })
    }
}
