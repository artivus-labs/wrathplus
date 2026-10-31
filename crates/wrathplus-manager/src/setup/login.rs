//! Private attended login defaults. Never execute a shell or persist credentials in Manager JSON.
use super::*;

fn parse(contents: &str) -> Result<std::collections::BTreeMap<String, String>> {
    let mut values = std::collections::BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            return Err("Invalid private login defaults: expected KEY=value".into());
        };
        let key = key.trim();
        if !matches!(key, "AZEROTHBENCH_TEST_USER" | "AZEROTHBENCH_TEST_PASSWORD") {
            continue;
        }
        let value = value.trim();
        let value = if value.starts_with(['\'', '"']) {
            let quote = value.chars().next().unwrap();
            value
                .strip_prefix(quote)
                .and_then(|v| v.strip_suffix(quote))
                .ok_or("Invalid quoting in private login defaults")?
        } else {
            value
        };
        values.insert(key.to_owned(), value.to_owned());
    }
    Ok(values)
}

pub(super) fn configure(command: &mut Command, config: &Config) -> Result<()> {
    let path = config.repo.join(".azerothbench-test.env");
    let values = match fs::read_to_string(path) {
        Ok(contents) => parse(&contents)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(_) => return Err("Could not read private login defaults".into()),
    };
    apply(command, &values, |key| env::var(key).ok());
    Ok(())
}

fn apply(
    command: &mut Command,
    values: &std::collections::BTreeMap<String, String>,
    env: impl Fn(&str) -> Option<String>,
) {
    for (target, source) in [
        ("WOW_USER", "AZEROTHBENCH_TEST_USER"),
        ("WOW_PASS", "AZEROTHBENCH_TEST_PASSWORD"),
    ] {
        if let Some(value) = env(target)
            .or_else(|| env(source))
            .or_else(|| values.get(source).cloned())
            .filter(|v| !v.is_empty())
        {
            command.env(target, value);
        }
    }
    // Prefill only: never authenticate or enter the world on the player's behalf.
    command
        .env("WOW_FILL_ONLY", "1")
        .env("WOW_BG", "0")
        .env("WOW_GM", "off")
        .env("WOW_GOD", "off");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_defaults_prefill_without_shell_or_secret_arguments() {
        let values = parse("# private\nexport AZEROTHBENCH_TEST_USER='synthetic'\nAZEROTHBENCH_TEST_PASSWORD=\"$(not executed); secret\"\nWOW_PROBE_EXIT_AT=1\n").unwrap();
        let mut command = Command::new("unused");
        apply(&mut command, &values, |_| None);
        let vars: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(k, v)| (k.to_str().unwrap(), v.unwrap().to_str().unwrap()))
            .collect();
        assert_eq!(vars["WOW_USER"], "synthetic");
        assert_eq!(vars["WOW_PASS"], "$(not executed); secret");
        assert_eq!(vars["WOW_FILL_ONLY"], "1");
        assert_eq!(vars["WOW_BG"], "0");
        assert!(!vars.contains_key("WOW_PROBE_EXIT_AT"));
        assert_eq!(command.get_args().count(), 0);
    }
    #[test]
    fn explicit_environment_wins_and_missing_defaults_are_optional() {
        let values = parse("AZEROTHBENCH_TEST_USER=file-user\n").unwrap();
        let mut command = Command::new("unused");
        apply(&mut command, &values, |key| {
            (key == "WOW_USER").then(|| "env-user".into())
        });
        assert!(command
            .get_envs()
            .any(|(k, v)| k == "WOW_USER" && v == Some(std::ffi::OsStr::new("env-user"))));
        let fixture = super::super::tests::Fixture::new();
        assert!(configure(&mut command, &fixture.config).is_ok());
        fs::write(
            fixture.config.repo.join(".azerothbench-test.env"),
            "AZEROTHBENCH_TEST_USER=synthetic\nAZEROTHBENCH_TEST_PASSWORD=synthetic-password\n",
        )
        .unwrap();
        assert!(configure(&mut command, &fixture.config).is_ok());
        fs::write(
            fixture.config.repo.join(".azerothbench-test.env"),
            "AZEROTHBENCH_TEST_PASSWORD='unclosed",
        )
        .unwrap();
        let error = configure(&mut command, &fixture.config).unwrap_err();
        assert!(!error.contains("unclosed"));
        assert!(parse("AZEROTHBENCH_TEST_PASSWORD='unclosed").is_err());
    }
}
