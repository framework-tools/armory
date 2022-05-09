use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::Path,
};

use cargo::{
    core::{resolver::CliFeatures, Dependency, Workspace, PackageId},
    ops::{Packages, PublishOpts},
    Config,
};
use retry::{delay, retry_with_index};
use semver::Version;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmoryTOML {
    pub version: Version,
}

pub fn load_armory_toml(workspace_dir: &Path) -> Result<ArmoryTOML, String> {
    toml::from_slice(
        &fs::read_to_string(workspace_dir.join("armory.toml"))
            .unwrap()
            .as_bytes(),
    )
    .map_err(|_| "Failed to parse armory.toml".to_string())
}

pub fn save_armory_toml(workspace_dir: &Path, armory_toml: &ArmoryTOML) {
    let mut file = fs::File::create(workspace_dir.join("armory.toml")).unwrap();
    file.write_all(toml::to_string(armory_toml).unwrap().as_bytes())
        .unwrap();
}

pub fn publish_workspace(dir: &Path, version: &Version) {
    let mut cfg = Config::default().unwrap();
    cfg.set_values(cfg.load_values().unwrap()).unwrap();
    cfg.load_credentials().unwrap();

    // let dir = Path::new("/Users/albert/repos/armory/");
    let workspace_toml = dir.clone().join("Cargo.toml");
    let mut workspace = Workspace::new(&workspace_toml, &cfg).unwrap();

    // directed acyclic graph to figure out which dependencies
    // to publish first.
    let mut graph: HashMap<String, HashSet<String>> = HashMap::new();

    for pkg in workspace.members_mut() {
        let mut local_deps = HashSet::new();
        let manifest = pkg.manifest_mut();
        let package_id = manifest.package_id();
        let new_id = PackageId::new(
            &package_id.name().to_string(),
            version.clone(),
            package_id.source_id()
        ).unwrap();

        *manifest.summary_mut() = manifest.summary_mut()
            .clone()
            .override_id(new_id)
            .map_dependencies(|dep| {
            if dep.source_id().is_path() {
                local_deps.insert(dep.package_name().to_string());

                // modify the dependency version to the one in the armory.toml
                // and set any values to the old ones
                let mut new_dep = Dependency::parse(
                    &dep.package_name().to_string(),
                    Some(&version.to_string()),
                    dep.source_id().clone(),
                )
                .unwrap();
                new_dep.set_default_features(dep.uses_default_features());
                new_dep.set_features(
                    dep.features()
                        .iter()
                        .map(|feature| feature.to_string())
                        .collect::<Vec<String>>(),
                );
                new_dep.set_kind(dep.kind());
                new_dep.set_optional(dep.is_optional());
                new_dep.set_platform(dep.platform().cloned());
                new_dep.set_public(dep.is_public());
                if let Some(reg_id) = dep.registry_id() {
                    new_dep.set_registry_id(reg_id.clone());
                }
                new_dep
            } else {
                dep
            }
        });

        graph.insert(pkg.name().to_string(), local_deps);
    }

    let mut already_published: HashSet<String> = HashSet::new();

    for current_package in graph.keys() {
        publish_crate(
            &workspace,
            &cfg,
            current_package,
            &graph,
            &mut already_published,
        )
    }
}

fn publish_crate(
    workspace: &Workspace,
    cfg: &Config,
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
            publish_crate(workspace, cfg, local_dep, all_packages, already_published);
        }
    }

    retry_with_index(delay::Fibonacci::from_millis(2500), |current_try| {
        match cargo::ops::publish(
            workspace,
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
            },
        ) {
            Ok(_) => Ok(()),
            Err(e) => {
                if current_try > 6 {
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
