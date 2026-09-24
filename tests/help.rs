//! Справка — справочник для агента (спека agent mode §6): у каждой команды примеры, поля JSON
//! и коды выхода, поэтому скиллу не нужно дублировать документацию.

use aplaut_cli::cli::Cli;
use clap::CommandFactory;

/// Листовые подкоманды (без встроенных `help`) и их длинная справка.
fn leaves(cmd: &mut clap::Command, path: &str, out: &mut Vec<(String, String)>) {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_string())
        .filter(|n| n != "help")
        .collect();
    if names.is_empty() {
        out.push((path.to_string(), cmd.render_long_help().to_string()));
        return;
    }
    for name in names {
        let sub = cmd.find_subcommand_mut(&name).unwrap();
        leaves(sub, &format!("{path} {name}"), out);
    }
}

#[test]
fn every_leaf_command_documents_examples_json_and_exit_codes() {
    let mut root = Cli::command();
    root.build();
    let mut found = Vec::new();
    leaves(&mut root, "aplaut", &mut found);
    assert!(found.len() >= 10, "{found:?}");
    for (path, help) in &found {
        for section in ["Примеры:", "JSON (--json):", "Ошибки:", "Коды выхода:"]
        {
            assert!(help.contains(section), "{path}: нет «{section}»");
        }
    }
}

#[test]
fn root_help_explains_the_agent_contract() {
    let help = Cli::command().render_long_help().to_string();
    for needle in [
        "Для агентов и скриптов:",
        "--json",
        "--no-input",
        "--yes",
        "--dry-run",
        "retry_after",
    ] {
        assert!(help.contains(needle), "нет «{needle}»");
    }
}

#[test]
fn get_help_lists_includes_from_the_spec() {
    let mut root = Cli::command();
    root.build();
    for resource in aplaut_cli::resources::ALL {
        let help = root
            .find_subcommand_mut(resource.name)
            .and_then(|c| c.find_subcommand_mut("get"))
            .unwrap_or_else(|| panic!("{} get", resource.name))
            .render_long_help()
            .to_string();
        for include in aplaut_cli::spec::get_includes(resource.records_type).unwrap() {
            assert!(
                help.contains(include),
                "{} get: нет «{include}»",
                resource.name
            );
        }
    }
}
