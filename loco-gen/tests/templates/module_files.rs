//! Templates name a module with children as `<dir>/mod.rs`. An app in the
//! Rust 2018 layout keeps that module in `<dir>.rs` beside `<dir>/` instead
//! (`clippy::mod_module_files`), and generators write there: into the file
//! that exists, and, when a generator creates the module, into the file the
//! app's own `src/` layout calls for.
use super::utils::APP_TASK;
use loco_gen::{generate, AppInfo, Component, RRgen};
use std::fs;

const LIB_RS: &str = "pub mod app;
pub mod controllers;
";

const CARGO_TOML: &str = "[package]
name = \"tester\"

[dependencies]
loco-rs = { version = \"1.2\" }
";

fn run(tree: &tree_fs::Tree, component: Component) -> loco_gen::Result<()> {
    let rrgen = RRgen::with_working_dir(&tree.root);
    generate(
        &rrgen,
        component,
        &AppInfo {
            app_name: "tester".to_string(),
            working_dir: tree.root.clone(),
        },
    )
    .map(|_| ())
}

fn task(name: &str) -> Component {
    Component::Task {
        name: name.to_string(),
    }
}

fn notifier(name: &str) -> Component {
    Component::Notifier {
        name: name.to_string(),
    }
}

fn data(name: &str) -> Component {
    Component::Data {
        name: name.to_string(),
    }
}

fn read(tree: &tree_fs::Tree, path: &str) -> String {
    fs::read_to_string(tree.root.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The `pub mod` lines of a module file.
fn mods(tree: &tree_fs::Tree, path: &str) -> Vec<String> {
    read(tree, path)
        .lines()
        .filter(|l| l.starts_with("pub mod"))
        .map(ToString::to_string)
        .collect()
}

fn exists(tree: &tree_fs::Tree, path: &str) -> bool {
    tree.root.join(path).exists()
}

/// An app in the 2018 layout: `src/controllers.rs` beside `src/controllers/`.
fn module_files_app() -> tree_fs::TreeBuilder {
    tree_fs::TreeBuilder::default()
        .drop(true)
        .add("src/lib.rs", LIB_RS)
        .add("src/controllers.rs", "pub mod home;\n")
        .add_empty("src/controllers/home.rs")
        .add("Cargo.toml", CARGO_TOML)
}

/// An app in the `mod.rs` layout: `src/controllers/mod.rs`.
fn mod_rs_app() -> tree_fs::TreeBuilder {
    tree_fs::TreeBuilder::default()
        .drop(true)
        .add("src/lib.rs", LIB_RS)
        .add("src/controllers/mod.rs", "pub mod home;\n")
        .add_empty("src/controllers/home.rs")
        .add("Cargo.toml", CARGO_TOML)
}

#[test]
fn injects_into_the_module_file_beside_the_directory() {
    let tree = module_files_app()
        .add("src/tasks.rs", "pub mod existing;\n")
        .add("tests/tasks.rs", "pub mod existing;\n")
        .add("src/app.rs", APP_TASK)
        .create()
        .unwrap();

    run(&tree, task("cleanup")).expect("generate task");

    assert_eq!(
        mods(&tree, "src/tasks.rs"),
        ["pub mod existing;", "pub mod cleanup;"]
    );
    assert_eq!(
        mods(&tree, "tests/tasks.rs"),
        ["pub mod existing;", "pub mod cleanup;"]
    );
    assert!(exists(&tree, "src/tasks/cleanup.rs"));
    assert!(exists(&tree, "tests/tasks/cleanup.rs"));
    assert!(!exists(&tree, "src/tasks/mod.rs"));
    assert!(!exists(&tree, "tests/tasks/mod.rs"));
}

#[test]
fn an_existing_mod_rs_is_still_injected_into() {
    // The app's `src/` is otherwise in the 2018 layout, but this module keeps
    // its `mod.rs`: the file that exists wins.
    let tree = module_files_app()
        .add("src/tasks/mod.rs", "pub mod existing;\n")
        .add_empty("tests/tasks/mod.rs")
        .add("src/app.rs", APP_TASK)
        .create()
        .unwrap();

    run(&tree, task("cleanup")).expect("generate task");

    assert_eq!(
        mods(&tree, "src/tasks/mod.rs"),
        ["pub mod existing;", "pub mod cleanup;"]
    );
    assert!(!exists(&tree, "src/tasks.rs"));
}

#[test]
fn a_missing_module_file_is_named_as_the_app_lays_it_out() {
    let tree = module_files_app()
        .add("src/app.rs", APP_TASK)
        .create()
        .unwrap();

    let err = run(&tree, task("cleanup")).expect_err("no tasks module to inject into");

    assert_eq!(
        err.to_string(),
        "cannot inject into `src/tasks.rs`: file does not exist"
    );
}

#[test]
fn notifier_creates_its_module_file_beside_the_directory() {
    let tree = module_files_app().create().unwrap();

    run(&tree, notifier("queue_paused")).expect("generate notifier");
    run(&tree, notifier("hotfix_landed")).expect("generate a second notifier");

    assert_eq!(
        mods(&tree, "src/notifiers.rs"),
        ["pub mod queue_paused;", "pub mod hotfix_landed;"]
    );
    assert!(exists(&tree, "src/notifiers/queue_paused.rs"));
    assert!(!exists(&tree, "src/notifiers/mod.rs"));
}

#[test]
fn notifier_creates_mod_rs_in_a_mod_rs_app() {
    let tree = mod_rs_app().create().unwrap();

    run(&tree, notifier("queue_paused")).expect("generate notifier");

    assert!(read(&tree, "src/notifiers/mod.rs").contains("pub mod queue_paused;"));
    assert!(!exists(&tree, "src/notifiers.rs"));
}

#[test]
fn notifier_creates_mod_rs_when_the_layout_is_mixed() {
    // One `mod.rs` under `src/` keeps the layout generators always wrote.
    let tree = module_files_app()
        .add("src/workers/mod.rs", "pub mod report;\n")
        .add_empty("src/workers/report.rs")
        .create()
        .unwrap();

    run(&tree, notifier("queue_paused")).expect("generate notifier");

    assert!(read(&tree, "src/notifiers/mod.rs").contains("pub mod queue_paused;"));
    assert!(!exists(&tree, "src/notifiers.rs"));
}

#[test]
fn data_creates_its_module_file_beside_the_directory() {
    let tree = module_files_app().create().unwrap();

    run(&tree, data("stocks")).expect("generate data");

    assert_eq!(mods(&tree, "src/data.rs"), ["pub mod stocks;"]);
    assert!(exists(&tree, "src/data/stocks.rs"));
    assert!(!exists(&tree, "src/data/mod.rs"));
    assert!(read(&tree, "src/lib.rs").contains("pub mod data;"));
}

#[test]
fn data_creates_mod_rs_in_a_mod_rs_app() {
    let tree = mod_rs_app().create().unwrap();

    run(&tree, data("stocks")).expect("generate data");

    assert_eq!(mods(&tree, "src/data/mod.rs"), ["pub mod stocks;"]);
    assert!(!exists(&tree, "src/data.rs"));
}

fn mailer(name: &str) -> Component {
    Component::Mailer {
        name: name.to_string(),
    }
}

#[test]
fn mailer_injects_into_the_module_file_beside_the_directory() {
    let tree = module_files_app()
        .add("src/mailers.rs", "pub mod auth;\n")
        .add_empty("src/mailers/auth.rs")
        .create()
        .unwrap();

    run(&tree, mailer("digest")).expect("generate mailer");

    assert_eq!(
        mods(&tree, "src/mailers.rs"),
        ["pub mod auth;", "pub mod digest;"]
    );
    assert!(exists(&tree, "src/mailers/digest.rs"));
    assert!(!exists(&tree, "src/mailers/mod.rs"));
}

#[test]
fn oauth_injects_into_the_module_file_beside_the_directory() {
    let tree = module_files_app()
        .add("src/app.rs", super::utils::APP_ROUTS)
        .add("config/development.yaml", "server:\n  port: 5150\n")
        .add(
            "Cargo.toml",
            "[package]\nname = \"tester\"\n\n[dependencies]\nloco-rs = { version = \"1.2\", features = [\"auth_oauth\"] }\n",
        )
        .create()
        .unwrap();

    let rrgen = RRgen::with_working_dir(&tree.root);
    generate(
        &rrgen,
        Component::OAuth {
            provider: "github".to_string(),
        },
        &AppInfo {
            app_name: "tester".to_string(),
            working_dir: tree.root.clone(),
        },
    )
    .expect("generate oauth");

    assert_eq!(
        mods(&tree, "src/controllers.rs"),
        ["pub mod home;", "pub mod oauth;"]
    );
    assert!(exists(&tree, "src/controllers/oauth.rs"));
    assert!(!exists(&tree, "src/controllers/mod.rs"));
}

/// A custom template injecting into `src/tasks/mod.rs`, placed by `placement`.
fn tasks_injection(placement: &str) -> String {
    format!(
        "to: src/tasks/cleanup.rs\ninjections:\n- into: src/tasks/mod.rs\n  {placement}content: \"pub mod cleanup;\"\n---\n"
    )
}

#[test]
fn an_unmatched_anchor_names_the_module_file_used() {
    let tree = module_files_app()
        .add("src/tasks.rs", "pub mod existing;\n")
        .create()
        .unwrap();
    let rrgen = RRgen::with_working_dir(&tree.root);

    let err = rrgen
        .generate(
            &tasks_injection("before: \"no such line\"\n  "),
            &serde_json::json!({}),
        )
        .expect_err("the anchor matches nothing");

    assert!(
        err.to_string()
            .starts_with("cannot inject into `src/tasks.rs`: no line matches"),
        "{err}"
    );
    assert!(!exists(&tree, "src/tasks/cleanup.rs"));
}

#[test]
fn an_injection_with_no_placement_names_the_module_file_used() {
    let tree = module_files_app()
        .add("src/tasks.rs", "pub mod existing;\n")
        .create()
        .unwrap();
    let rrgen = RRgen::with_working_dir(&tree.root);

    let err = rrgen
        .generate(&tasks_injection(""), &serde_json::json!({}))
        .expect_err("the injection has no placement");

    assert!(
        err.to_string()
            .starts_with("cannot inject into `src/tasks.rs`: the injection says where"),
        "{err}"
    );
}
