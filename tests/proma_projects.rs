#![cfg(unix)]

use serde_json::json;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kitter"))
        .env("KITTER_HOME", root.join("config"))
        .env("KITTER_PROMA_HOME", root.join("proma"))
        .env("CODEX_HOME", root.join("codex"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn merged_project_routes_installation_and_rejects_ambiguous_removal() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let business = root.join("business");
    let a = root.join("proma/agent-workspaces/a");
    let b = root.join("proma/agent-workspaces/b");
    let source = root.join("source/route-check");
    let sentinel = business.join("skills/route-check/SKILL.md");
    for directory in [&a, &b, &source, sentinel.parent().unwrap()] {
        fs::create_dir_all(directory).unwrap();
    }
    fs::write(&sentinel, "business data").unwrap();
    fs::write(
        source.join("SKILL.md"),
        "---\nname: route-check\ndescription: Fixture\n---\n",
    )
    .unwrap();
    let binding =
        |slug: &str| json!({"id": slug, "slug": slug, "name": slug, "projectRootPath": business});
    let index = root.join("proma/agent-workspaces.json");
    fs::write(
        &index,
        serde_json::to_vec(&json!({"version": 2, "workspaces": [binding("a")]})).unwrap(),
    )
    .unwrap();
    assert!(
        cli(&root, &["add", "local", source.to_str().unwrap()])
            .status
            .success()
    );
    let finder = business.to_str().unwrap();
    assert!(
        cli(
            &root,
            &[
                "install",
                "route-check",
                "--project",
                finder,
                "--target",
                "codex",
                "--target",
                "proma"
            ]
        )
        .status
        .success()
    );
    let a_entry = a.join("skills/route-check");
    let b_entry = b.join("skills/route-check");
    let codex = business.join(".codex/skills/route-check");
    assert!(codex.symlink_metadata().unwrap().file_type().is_symlink());
    assert!(a_entry.symlink_metadata().unwrap().is_dir());
    assert!(a_entry.join(".kitter-proma.json").is_file());
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "business data");

    fs::write(
        &index,
        serde_json::to_vec(&json!({"version": 2, "workspaces": [binding("a"), binding("b")]}))
            .unwrap(),
    )
    .unwrap();
    assert!(
        cli(
            &root,
            &[
                "install",
                "route-check",
                "--project",
                b.to_str().unwrap(),
                "--target",
                "proma"
            ]
        )
        .status
        .success()
    );
    for args in [
        vec![
            "uninstall",
            "route-check",
            "--project",
            finder,
            "--target",
            "proma",
        ],
        vec!["uninstall", "route-check", "--project", finder],
    ] {
        assert!(!cli(&root, &args).status.success());
        assert!(a_entry.join("SKILL.md").is_file());
        assert!(b_entry.join("SKILL.md").is_file());
        assert!(codex.exists());
    }
    let config_path = root.join("config/config.json");
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
    config["recent_projects"] = json!([b, business]);
    fs::write(config_path, serde_json::to_vec(&config).unwrap()).unwrap();
    let removal = cli(&root, &["remove", "route-check"]);
    assert!(String::from_utf8_lossy(&removal.stderr).contains("多个 Proma 工作区"));
    assert!(a_entry.join("SKILL.md").is_file());
    assert!(b_entry.join("SKILL.md").is_file());
    assert!(codex.exists());
    assert!(cli(&root, &["show", "route-check"]).status.success());
    assert!(
        cli(
            &root,
            &[
                "uninstall",
                "--project",
                finder,
                "--path",
                a_entry.to_str().unwrap(),
                "--target",
                "proma"
            ]
        )
        .status
        .success()
    );
    assert!(!a_entry.exists());
    assert!(b_entry.join("SKILL.md").is_file());
    assert!(
        cli(
            &root,
            &[
                "uninstall",
                "route-check",
                "--project",
                b.to_str().unwrap(),
                "--target",
                "proma"
            ]
        )
        .status
        .success()
    );
    assert!(!b_entry.exists());
    assert!(codex.exists());
    assert_eq!(fs::read_to_string(&sentinel).unwrap(), "business data");
}

#[test]
fn isolated_proma_scope_excludes_unrelated_agent_globals() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let workspace = root.join("proma/agent-workspaces/default");
    let proma_skill = workspace.join("skills/local-proma-only");
    let foreign = root.join("codex/skills/foreign-codex-global");
    let bin = root.join("bin");
    let local_codex_root = workspace.join(".codex/skills");
    for directory in [&proma_skill, &foreign, &bin, &local_codex_root] { fs::create_dir_all(directory).unwrap(); }
    for (folder, name) in [(&proma_skill, "local-proma-only"), (&foreign, "foreign-codex-global")] {
        fs::write(folder.join("SKILL.md"), format!("---\nname: {name}\ndescription: Scope regression fixture\n---\n")).unwrap();
    }
    for name in ["codex", "proma"] {
        let file = bin.join(name); fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(file, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(root.join("proma/agent-workspaces.json"), serde_json::to_vec(&json!({
        "version":2,"workspaces":[{"id":"default","slug":"default","name":"Default"}]
    })).unwrap()).unwrap();
    let inspect = |path: &Path| {
        let result = Command::new(env!("CARGO_BIN_EXE_kitter"))
            .env("KITTER_HOME", root.join("config")).env("KITTER_PROMA_HOME", root.join("proma"))
            .env("CODEX_HOME", root.join("codex")).env("KITTER_CODEX_PROJECTS_HOME", root.join("codex"))
            .env("PATH", std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()))).unwrap())
            .args(["project",path.to_str().unwrap(),"--view","skills","--json"]).output().unwrap();
        assert!(result.status.success(), "CLI inspection failed");
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap()
    };
    let isolated = inspect(&workspace);
    let agents = isolated["agents"].as_array().unwrap();
    assert_eq!(agents.iter().map(|e| e["agent"].as_str().unwrap()).collect::<Vec<_>>(), ["proma"]);
    assert!(agents[0]["skills"].as_array().unwrap().iter().any(|s| s["name"]=="local-proma-only"));
    assert!(!agents.iter().flat_map(|e| e["skills"].as_array().unwrap()).any(|s| s["name"]=="foreign-codex-global"));
    let local_codex = local_codex_root.join("local-codex");
    fs::create_dir(&local_codex).unwrap();
    fs::write(local_codex.join("SKILL.md"), "---\nname: local-codex\ndescription: Local Agent installation\n---\n").unwrap();
    let local = inspect(&workspace);
    let agents = local["agents"].as_array().unwrap();
    assert_eq!(agents.iter().map(|e| e["agent"].as_str().unwrap()).collect::<Vec<_>>(), ["proma"]);
    assert!(!agents.iter().flat_map(|e| e["skills"].as_array().unwrap()).any(|s| s["name"]=="foreign-codex-global"));
    assert!(local_codex.join("SKILL.md").is_file());
    fs::remove_file(local_codex.join("SKILL.md")).unwrap();
    fs::remove_dir(&local_codex).unwrap();
    let isolated_again = inspect(&workspace);
    let agents = isolated_again["agents"].as_array().unwrap();
    assert_eq!(agents.iter().map(|e| e["agent"].as_str().unwrap()).collect::<Vec<_>>(), ["proma"]);
    assert!(!agents.iter().flat_map(|e| e["skills"].as_array().unwrap()).any(|s| s["name"]=="foreign-codex-global"));
    fs::create_dir(&local_codex).unwrap();
    fs::write(local_codex.join("SKILL.md"), "---\nname: local-codex\ndescription: Local Agent installation\n---\n").unwrap();
    fs::write(root.join("codex/.codex-global-state.json"), serde_json::to_vec(&json!({
        "local-projects":{"shared":{"name":"Shared","rootPaths":[workspace]}}
    })).unwrap()).unwrap();
    let shared = inspect(&workspace);
    let agents = shared["agents"].as_array().unwrap();
    assert_eq!(agents.iter().map(|e| e["agent"].as_str().unwrap()).collect::<Vec<_>>(), ["codex","proma"]);
    assert!(agents.iter().flat_map(|e| e["skills"].as_array().unwrap()).any(|s| s["name"]=="foreign-codex-global"));
    assert!(agents.iter().flat_map(|e| e["skills"].as_array().unwrap()).any(|s| s["name"]=="local-codex"));
}
