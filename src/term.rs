//! Всё, что видит человек, идёт в stderr: stdout принадлежит данным (clig).
//!
//! Предупреждения печатаются и с `--quiet`: без них можно не заметить, например, что сервер
//! молча ограничил выгрузку 30 днями.

use std::cell::{Cell, RefCell};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TermEnv {
    pub no_color: bool,
    pub term_dumb: bool,
    pub aplaut_no_color: bool,
}

impl TermEnv {
    pub fn capture() -> Self {
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        TermEnv {
            no_color: set("NO_COLOR"),
            term_dumb: std::env::var("TERM").is_ok_and(|t| t == "dumb"),
            aplaut_no_color: set("APLAUT_NO_COLOR"),
        }
    }
}

pub fn color_enabled(stderr_tty: bool, flag_no_color: bool, env: &TermEnv) -> bool {
    stderr_tty && !flag_no_color && !env.no_color && !env.term_dumb && !env.aplaut_no_color
}

pub struct Reporter {
    quiet: bool,
    verbose: bool,
    stderr_tty: bool,
    color: bool,
    progress_shown: Cell<bool>,
    out: RefCell<Box<dyn Write>>,
}

impl Reporter {
    pub fn new(quiet: bool, verbose: bool, stderr_tty: bool, color: bool) -> Self {
        Self::with_writer(quiet, verbose, stderr_tty, color, Box::new(io::stderr()))
    }

    pub fn with_writer(
        quiet: bool,
        verbose: bool,
        stderr_tty: bool,
        color: bool,
        out: Box<dyn Write>,
    ) -> Self {
        Reporter {
            quiet,
            verbose,
            stderr_tty,
            color,
            progress_shown: Cell::new(false),
            out: RefCell::new(out),
        }
    }

    pub fn is_verbose(&self) -> bool {
        self.verbose
    }

    pub fn warn(&self, msg: &str) {
        self.clear_progress();
        let text = if self.color {
            format!("\x1b[33m{msg}\x1b[0m")
        } else {
            msg.to_string()
        };
        self.write(&format!("{text}\n"));
    }

    pub fn info(&self, msg: &str) {
        if self.quiet {
            return;
        }
        self.clear_progress();
        self.write(&format!("{msg}\n"));
    }

    pub fn debug(&self, msg: &str) {
        if !self.verbose {
            return;
        }
        self.clear_progress();
        self.write(&format!("debug: {msg}\n"));
    }

    /// Строка прогресса перерисовывается на месте; без TTY её нет совсем (clig: no animations).
    pub fn progress(&self, msg: &str) {
        if self.quiet || !self.stderr_tty {
            return;
        }
        self.write(&format!("\r{msg}\x1b[K"));
        self.progress_shown.set(true);
    }

    pub fn clear_progress(&self) {
        if self.progress_shown.replace(false) {
            self.write("\r\x1b[K");
        }
    }

    fn write(&self, text: &str) {
        let mut out = self.out.borrow_mut();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

/// Буфер-писатель для тестов: клоны пишут в одно место.
#[derive(Clone, Default)]
pub struct SharedBuf(Arc<Mutex<Vec<u8>>>);

impl SharedBuf {
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captured(quiet: bool, verbose: bool, tty: bool) -> (Reporter, SharedBuf) {
        let buf = SharedBuf::default();
        (Reporter::with_writer(quiet, verbose, tty, false, Box::new(buf.clone())), buf)
    }

    #[test]
    fn quiet_hides_info_but_not_warnings() {
        let (r, buf) = captured(true, false, false);
        r.info("итог");
        r.warn("фильтр не задан");
        assert_eq!(buf.contents(), "фильтр не задан\n");
    }

    #[test]
    fn debug_only_with_verbose() {
        let (r, buf) = captured(false, false, false);
        r.debug("скрыто");
        let (rv, bufv) = captured(false, true, false);
        rv.debug("видно");
        assert_eq!(buf.contents(), "");
        assert_eq!(bufv.contents(), "debug: видно\n");
    }

    #[test]
    fn progress_only_on_tty() {
        let (r, buf) = captured(false, false, false);
        r.progress("reviews: 100");
        assert_eq!(buf.contents(), "");
        let (rt, buft) = captured(false, false, true);
        rt.progress("reviews: 100");
        rt.info("готово");
        assert_eq!(buft.contents(), "\rreviews: 100\x1b[K\r\x1b[Kготово\n");
    }

    #[test]
    fn color_rules_follow_clig() {
        let env = TermEnv::default();
        assert!(color_enabled(true, false, &env));
        assert!(!color_enabled(false, false, &env));
        assert!(!color_enabled(true, true, &env));
        assert!(!color_enabled(true, false, &TermEnv { no_color: true, ..TermEnv::default() }));
        assert!(!color_enabled(true, false, &TermEnv { term_dumb: true, ..TermEnv::default() }));
        assert!(!color_enabled(true, false, &TermEnv { aplaut_no_color: true, ..TermEnv::default() }));
    }
}
