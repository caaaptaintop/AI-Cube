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
