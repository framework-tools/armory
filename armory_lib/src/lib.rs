use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::Path
};

use cargo::{
    core::{resolver::CliFeatures, Workspace},
    ops::{Packages, PublishOpts},
    Config,
};
use retry::{delay, retry_with_index};
use semver::Version;
use serde::{Deserialize, Serialize};
use toml_edit::Document;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmoryTOML {
    pub version: Version,
}

pub fn load_armory_toml(workspace_dir: &Path) -> Result<ArmoryTOML, String> {
    toml::from_str(
        &fs::read_to_string(workspace_dir.join("armory.toml"))
            .expect("Failed to read armory.toml in workspace root"),
    )
    .map_err(|_| "Failed to parse armory.toml".to_string())
}

pub fn save_armory_toml(workspace_dir: &Path, armory_toml: &ArmoryTOML) {
    let mut file = fs::File::create(workspace_dir.join("armory.toml")).unwrap();
    file.write_all(toml::to_string(armory_toml)
        .expect("Failed to serialize armory.toml")
        .as_bytes()
    ).expect("Failed to write armory.toml");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceManifest {
    pub workspace: WorkspaceDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceDefinition {
    pub members: Vec<String>,
}

fn update_member_deps(dir: &Path, version: &Version) -> HashMap<String, HashSet<String>>{
    // directed acyclic graph to figure out which dependencies
    // to publish first.
    let mut graph: HashMap<String, HashSet<String>> = HashMap::new();

    let workspace_toml: WorkspaceManifest = toml::from_str(
        &fs::read_to_string(dir.join("Cargo.toml"))
            .expect("Failed to read Cargo.toml in workspace root"),
    ).expect("Failed to parse Cargo.toml in workspace root");

    for member in workspace_toml.workspace.members {
        let member_dir = dir.join(&member);
        let member_toml = fs::read_to_string(member_dir.join("Cargo.toml")).unwrap();
        let mut member_toml = member_toml.parse::<Document>().unwrap();
        let mut local_deps = HashSet::new();

        member_toml["package"]["version"] = toml_edit::value(version.to_string());
        let deps = member_toml.get_mut("dependencies").map(|deps| deps.as_table_mut());
        match deps {
            Some(Some(table)) => {
                for (name, dep) in table.iter_mut() {
                    if let Some(dep) = dep.as_table_like_mut() {
                        if let Some(Some(_)) = dep.get("path").map(|dep| dep.as_str()) {
                            // this is a local dependency, so we will need to update the version
                            dep.insert("version", toml_edit::value(version.to_string()));
                            local_deps.insert(name.trim().into());
                        }
                    }
                }
            }
            _ => {}
        }

        let mut file = fs::File::create(member_dir.join("Cargo.toml")).unwrap();
        file.write_all(member_toml.to_string().as_bytes()).unwrap();


        graph.insert(member.trim().into(), local_deps);
    }

    // now we have a graph of dependencies, we can figure out which
    // dependencies to publish first, in the next stage
    graph
}

pub fn publish_workspace(dir: &Path, version: &Version) {

    let graph = update_member_deps(dir, version);

    let mut already_published: HashSet<String> = HashSet::new();

    for current_package in graph.keys() {
        publish_crate(
            dir,
            current_package,
            &graph,
            &mut already_published,
        )
    }
}

fn publish_crate(
    dir: &Path,
    current_package: &str,
    all_packages: &HashMap<String, HashSet<String>>,
    already_published: &mut HashSet<String>,
) {

    if already_published.contains(current_package) {
        return;
    }
    // publish all the local dependencies first
    for local_dep in all_packages.get(current_package).unwrap() {
        if !already_published.contains(local_dep) {
            publish_crate(dir, local_dep, all_packages, already_published);
        }
    }

    retry_with_index(delay::Fibonacci::from_millis(4000), |current_try| {
        let cfg = Config::default().unwrap();
        cfg.set_values(cfg.load_values().unwrap()).unwrap();
        cfg.load_credentials().unwrap();

        let workspace = Workspace::new(&dir.clone().join("Cargo.toml"), &cfg).unwrap();

        match cargo::ops::publish(
            &workspace,
            &PublishOpts {
                token: None,
                config: &cfg,
                verify: false,
                allow_dirty: true,
                registry: None,
                dry_run: false,
                targets: vec![],
                to_publish: Packages::Packages(vec![current_package.to_string()]),
                cli_features: CliFeatures::new_all(true),
                index: None,
                jobs: None,
                keep_going: false,
            },
        ) {
            Ok(_) => Ok(()),
            Err(e) => {
                if current_try > 5{
                    panic!("ARMORY: failed to publish {} after {} attempts: {:#?}",
                            current_package, current_try, e);
                } else {
                    println!("ARMORY: failed to publish {} after {} attempts: {:#?}",
                        current_package, current_try, e);
                }
                Err(e)
            }
        }
    })
    .unwrap();

    already_published.insert(current_package.to_string());
}
