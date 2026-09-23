//! Каждая команда CLI обязана соответствовать операции `spec/api.yaml`, и наоборот каждая
//! запись в `resources.rs` — быть доступной из CLI. Обновили спеку — CI покажет, что разъехалось.

use aplaut_cli::cli::Cli;
use aplaut_cli::resources::{self, Verb};
use aplaut_cli::spec;
use clap::CommandFactory;

fn leaves(cmd: &clap::Command, prefix: &[String], out: &mut Vec<Vec<String>>) {
    for sub in cmd.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        let mut path = prefix.to_vec();
        path.push(sub.get_name().to_string());
        if sub.has_subcommands() {
            leaves(sub, &path, out);
        } else {
            out.push(path);
        }
    }
}

fn all_commands() -> Vec<Vec<String>> {
    let mut out = Vec::new();
    leaves(&Cli::command(), &[], &mut out);
    out
}

#[test]
fn every_command_maps_to_a_spec_operation() {
    let commands = all_commands();
    assert!(commands.len() >= 5, "{commands:?}");
    for path in &commands {
        let joined = path.join(" ");
        if resources::LOCAL_COMMANDS.contains(&joined.as_str()) {
            continue;
        }
        let resource = resources::find(&path[0])
            .unwrap_or_else(|| panic!("«{joined}» не описана в resources.rs"));
        let verb = resource
            .verbs
            .iter()
            .find(|v| v.name() == path[1])
            .unwrap_or_else(|| panic!("«{joined}»: глагол не описан у ресурса в resources.rs"));
        let (method, operation) = verb.operation();
        assert!(
            spec::has_operation(method, operation),
            "«{joined}» → {method} {operation}: нет в spec/api.yaml"
        );
        if *verb == Verb::Scroll {
            assert!(
                spec::scroll_spec(resource.records_type).is_some(),
                "«{joined}»: {} не поддерживается scroll",
                resource.records_type
            );
        }
    }
}

#[test]
fn every_resource_verb_and_local_command_is_exposed() {
    let commands = all_commands();
    for resource in resources::ALL {
        for verb in resource.verbs {
            let path = vec![resource.name.to_string(), verb.name().to_string()];
            assert!(
                commands.contains(&path),
                "{path:?} есть в resources.rs, но не в CLI"
            );
        }
    }
    for local in resources::LOCAL_COMMANDS {
        let path: Vec<String> = local.split(' ').map(str::to_string).collect();
        assert!(
            commands.contains(&path),
            "{local} объявлена локальной, но не существует"
        );
    }
}
