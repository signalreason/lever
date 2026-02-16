use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunPaths {
    pub run_dir_rel: PathBuf,
    pub run_dir_abs: PathBuf,
    pub pack_dir_rel: PathBuf,
    pub pack_dir_abs: PathBuf,
    pub prompt_path: PathBuf,
    pub result_path_rel: PathBuf,
    pub result_path_abs: PathBuf,
    pub codex_log_rel: PathBuf,
    pub codex_log_abs: PathBuf,
    pub task_snapshot_path: PathBuf,
}

pub fn run_paths(workspace: &Path, task_id: &str, run_id: &str) -> RunPaths {
    let run_dir_rel = PathBuf::from(".ralph")
        .join("runs")
        .join(task_id)
        .join(run_id);
    let run_dir_abs = workspace.join(&run_dir_rel);
    let pack_dir_rel = run_dir_rel.join("pack");
    let pack_dir_abs = run_dir_abs.join("pack");
    let prompt_path = run_dir_abs.join("prompt.md");
    let result_path_rel = run_dir_rel.join("result.json");
    let result_path_abs = workspace.join(&result_path_rel);
    let codex_log_rel = run_dir_rel.join("codex.jsonl");
    let codex_log_abs = workspace.join(&codex_log_rel);
    let task_snapshot_path = run_dir_abs.join("task.json");

    RunPaths {
        run_dir_rel,
        run_dir_abs,
        pack_dir_rel,
        pack_dir_abs,
        prompt_path,
        result_path_rel,
        result_path_abs,
        codex_log_rel,
        codex_log_abs,
        task_snapshot_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_paths_pack_location_is_deterministic() {
        let workspace = PathBuf::from("workspace");
        let paths = run_paths(&workspace, "TASK-1", "run-123");

        assert_eq!(
            paths.run_dir_rel,
            PathBuf::from(".ralph/runs/TASK-1/run-123")
        );
        assert_eq!(
            paths.pack_dir_rel,
            PathBuf::from(".ralph/runs/TASK-1/run-123/pack")
        );
        assert_eq!(
            paths.pack_dir_abs,
            PathBuf::from("workspace/.ralph/runs/TASK-1/run-123/pack")
        );
        assert_eq!(
            paths.prompt_path,
            PathBuf::from("workspace/.ralph/runs/TASK-1/run-123/prompt.md")
        );
        assert_eq!(
            paths.result_path_rel,
            PathBuf::from(".ralph/runs/TASK-1/run-123/result.json")
        );
        assert_eq!(
            paths.codex_log_rel,
            PathBuf::from(".ralph/runs/TASK-1/run-123/codex.jsonl")
        );
        assert_eq!(
            paths.task_snapshot_path,
            PathBuf::from("workspace/.ralph/runs/TASK-1/run-123/task.json")
        );
    }
}
