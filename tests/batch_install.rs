#![cfg(unix)]

use kitter::{InstallTarget, project};
use serde_json::json;
use std::{fs, path::PathBuf, process::Command};

#[test]
fn batch_install_routes_proma_and_other_agents_to_each_selected_project() {
    // Isolate Proma's environment from other parallel tests.
    if let Some(root) = std::env::var_os("KITTER_MULTI_INSTALL_TEST_ROOT") {
        let root = PathBuf::from(root);
        let projects = [root.join("project-a"), root.join("project-b")];
        let source = root.join("library/demo");
        let report = project::install_batch(&projects, &[("demo".into(), source.clone())],
            &[InstallTarget::Codex, InstallTarget::Proma, InstallTarget::ClaudeCode]);
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(report.installed, 6);
        assert_eq!(report.installed_projects.len(), 2);
        for (project, slug) in projects.iter().zip(["a", "b"]) {
            assert_eq!(project.join(".codex/skills/demo").canonicalize().unwrap(), source);
            assert_eq!(project.join(".claude/skills/demo").canonicalize().unwrap(), source);
            let entry = root.join(format!("proma/agent-workspaces/{slug}/skills/demo"));
            assert!(entry.symlink_metadata().unwrap().is_dir());
            assert!(entry.join(".kitter-proma.json").is_file());
            assert_eq!(entry.join("local-data").canonicalize().unwrap(), source.join("local-data"));
            assert_eq!(fs::read_to_string(project.join("skills/demo/SKILL.md")).unwrap(), "Business project data");
        }
        assert!(!root.join("proma/agent-workspaces/unselected/skills/demo").exists());
        assert_eq!(fs::read_to_string(source.join("local-data/state.txt")).unwrap(), "Existing local state");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("library/demo");
    fs::create_dir_all(source.join("local-data")).unwrap();
    fs::write(source.join("SKILL.md"), "---\nname: demo\ndescription: Batch fixture\n---\n").unwrap();
    fs::write(source.join("local-data/state.txt"), "Existing local state").unwrap();
    let mut workspaces = Vec::new();
    for slug in ["a", "b", "unselected"] {
        let project = root.join(format!("project-{slug}"));
        let data = project.join("skills/demo");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("SKILL.md"), "Business project data").unwrap();
        fs::create_dir_all(root.join(format!("proma/agent-workspaces/{slug}"))).unwrap();
        workspaces.push(json!({"id":slug,"slug":slug,"name":slug,"projectRootPath":project}));
    }
    fs::write(root.join("proma/agent-workspaces.json"),
        serde_json::to_vec(&json!({"version":2,"workspaces":workspaces})).unwrap()).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "batch_install_routes_proma_and_other_agents_to_each_selected_project", "--nocapture"])
        .env("KITTER_MULTI_INSTALL_TEST_ROOT", &root)
        .env("KITTER_PROMA_HOME", root.join("proma"))
        .output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}
