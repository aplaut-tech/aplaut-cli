//! `aplaut self update` (спека self-update §1): текст людям; сама работа — в `crate::update`.

use std::time::Duration;

use crate::cli::SelfVerb;
use crate::error::CliError;
use crate::update::{self, Options, SelfUpdate};

use super::{Ctx, Outcome, DRY_RUN_PREFIX};

pub fn run(verb: SelfVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        SelfVerb::Update(dry) => {
            let result = update::self_update(&Options {
                dry_run: dry.dry_run,
                timeout: Duration::from_secs(ctx.global.timeout),
                github_token: ctx.env.github_token.as_deref(),
            })?;
            ctx.reporter.info(&message(&result, dry.dry_run));
            Ok(Outcome::stdout(result).with_dry_run(dry.dry_run))
        }
    }
}

fn message(result: &SelfUpdate, dry_run: bool) -> String {
    // По `update_available`, а не `updated`: под --dry-run `updated` — будущий результат (D2).
    let text = if result.update_available && dry_run {
        format!(
            "доступна версия {} (установлена {})",
            result.latest, result.current
        )
    } else if result.update_available {
        format!("aplaut обновлён: {} → {}", result.current, result.latest)
    } else if result.latest == result.current {
        format!("aplaut {} — последняя версия", result.current)
    } else {
        format!(
            "aplaut {} новее последнего релиза {} — обновлять нечего",
            result.current, result.latest
        )
    };
    if dry_run {
        format!("{DRY_RUN_PREFIX} {text}")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::message;
    use crate::update::SelfUpdate;

    fn result(current: &str, latest: &str, updated: bool, update_available: bool) -> SelfUpdate {
        SelfUpdate {
            current: current.into(),
            latest: latest.into(),
            updated,
            update_available,
        }
    }

    #[test]
    fn messages_follow_the_spec_table() {
        for (r, dry_run, expected) in [
            (
                result("0.2.0", "0.3.0", true, true),
                false,
                "aplaut обновлён: 0.2.0 → 0.3.0",
            ),
            (
                result("0.3.0", "0.3.0", false, false),
                false,
                "aplaut 0.3.0 — последняя версия",
            ),
            (
                result("0.4.0-dev", "0.3.0", false, false),
                false,
                "aplaut 0.4.0-dev новее последнего релиза 0.3.0 — обновлять нечего",
            ),
            (
                result("0.2.0", "0.3.0", true, true),
                true,
                "Пробный запуск, ничего не изменено: доступна версия 0.3.0 (установлена 0.2.0)",
            ),
            (
                result("0.3.0", "0.3.0", false, false),
                true,
                "Пробный запуск, ничего не изменено: aplaut 0.3.0 — последняя версия",
            ),
        ] {
            assert_eq!(message(&r, dry_run), expected);
        }
    }
}
