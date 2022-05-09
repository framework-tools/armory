use dialoguer::{Select, theme::ColorfulTheme, console::{Term, style}};

fn main() -> Result<(), std::io::Error> {
    let term = Term::stdout();
    let cwd = std::env::current_dir()?;
    let mut armory_toml = armory_lib::load_armory_toml(&cwd).unwrap();
    let theme = ColorfulTheme::default();

    let version = &armory_toml.version;

    let items = vec![
        ("Patch", {
            let mut version = version.clone();
            version.patch += 1;
            version
        }),
        ("Minor", {
            let mut version = version.clone();
            version.minor += 1;
            version.patch = 0;
            version
        }),
        ("Major", {
            let mut version = version.clone();
            version.major += 1;
            version.minor = 0;
            version.patch = 0;
            version
        })
    ]
        .into_iter()
        .map(|(s, v)| (format!("{} ({})", s, v), v))
        .collect::<Vec<_>>();

    let selected = Select::with_theme(&theme)
        .with_prompt(format!("Select a release type. Current version: {}", version))
        .items(&items.iter().map(|t| &t.0).collect::<Vec<_>>())
        .default(0)
        .interact()?;

    let selected = &items[selected].1;

    println!("You selected: {}", selected);

    armory_toml.version = selected.clone();
    armory_lib::save_armory_toml(&cwd, &armory_toml);

    armory_lib::publish_workspace(&cwd, selected);

    term.write_line(&format!("{} Done!", style("✔").green()))?;

    Ok(())
}

