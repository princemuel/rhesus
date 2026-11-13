use std::env;
use std::process::Command;

pub(crate) fn username() -> Option<String> {
    let key = if cfg!(windows) { "USERNAME" } else { "USER" };

    if let Ok(name) = env::var(key)
        && !name.is_empty()
    {
        return Some(name);
    }

    let out = Command::new("whoami").output().ok()?;
    if !out.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let name = strip_domain(&text);
    (!name.is_empty()).then(|| name.to_owned())
}

/// Keep only the user part since `whoami` prints `DOMAIN\user` on Windows (and
/// on Unix with winbind).
fn strip_domain(domain: &str) -> &str {
    let s = domain.trim();
    s.rsplit_once('\\').map_or(s, |(_, user)| user)
}

#[cfg(test)]
mod tests {
    use super::strip_domain;

    #[test]
    fn strips_windows_domain() {
        assert_eq!(strip_domain("CORP\\sam\r\n"), "sam");
    }

    #[test]
    fn leaves_plain_names_alone() {
        assert_eq!(strip_domain("sam\n"), "sam");
        assert_eq!(strip_domain(""), "");
    }
}
