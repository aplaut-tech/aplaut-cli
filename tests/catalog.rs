//! Каталог кодов в README — контракт для агентов (спека agent mode §4): каждый код ошибки и
//! предупреждения из src/ должен быть там описан.

use std::fs;
use std::path::Path;

fn sources(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = fs::read_to_string(&path).unwrap();
            // Коды из тестов не публичны.
            out.push(text.split("#[cfg(test)]").next().unwrap().to_string());
        }
    }
}

/// Первая строковая константа после `marker`, если до неё только пробелы и `Exit::…,`.
fn codes_after(text: &str, marker: &str) -> Vec<String> {
    let mut codes = Vec::new();
    for (at, _) in text.match_indices(marker) {
        let mut rest = text[at + marker.len()..].trim_start();
        if let Some(after_exit) = rest.strip_prefix("Exit::") {
            let comma = after_exit.find(',').unwrap_or(0);
            rest = after_exit[comma + 1..].trim_start();
        }
        if let Some(body) = rest.strip_prefix('"') {
            let code = &body[..body.find('"').unwrap()];
            if !code.is_empty() && code.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                codes.push(code.to_string());
            }
        }
    }
    codes
}

#[test]
fn every_error_and_warning_code_is_documented_in_readme() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut texts = Vec::new();
    sources(&root.join("src"), &mut texts);
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let mut codes: Vec<String> = texts
        .iter()
        .flat_map(|t| {
            [
                "CliError::usage(",
                "CliError::general(",
                "CliError::new(",
                ".warn(",
            ]
            .iter()
            .flat_map(move |m| codes_after(t, m))
        })
        .collect();
    // Собираются не литералом в конструкторе: `CliError::io`, транспорт (`http.rs`), 401 (`api_error.rs`).
    codes.extend(
        [
            "io_error",
            "timeout",
            "network_error",
            "response_too_large",
            "unauthorized",
            "invalid_token",
        ]
        .map(String::from),
    );
    // Прочие статусы HTTP — `http_<status>` через format!: в README это одна строка каталога.
    if texts.iter().any(|t| t.contains("format!(\"http_{")) {
        codes.push("http_<status>".into());
    }
    codes.sort();
    codes.dedup();
    assert!(codes.len() > 40, "сканер сломался: {codes:?}");
    let missing: Vec<&String> = codes
        .iter()
        .filter(|c| !readme.contains(&format!("`{c}`")))
        .collect();
    assert!(missing.is_empty(), "не описаны в README: {missing:?}");
}
