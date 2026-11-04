//! The GUI and non-interactive bootstrap use the same setup operations.
use crate::server::{Navmesh, SetupTask};
use crate::setup::Config;
use std::path::PathBuf;

pub(crate) struct Options {
    pub config: Config,
    pub setup: bool,
    pub validate_only: bool,
    pub help: bool,
    pub doctor: bool,
    pub use_existing: bool,
    /// A first-install server task to run without the GUI.
    pub server_task: Option<(SetupTask, Navmesh)>,
}

pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let args: Vec<String> = args.into_iter().collect();
    let mut base = Config::defaults();
    for pair in args.windows(2) {
        if pair[0] == "--repo" {
            base.repo = PathBuf::from(&pair[1]);
        }
    }
    if std::env::var_os("CARGO_TARGET_DIR").is_none() {
        base.target = base.repo.join("target");
    }
    let base = crate::setup::load(&base).unwrap_or(base);
    let mut options = Options {
        config: base,
        setup: false,
        validate_only: false,
        help: false,
        doctor: false,
        use_existing: false,
        server_task: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--use-existing" => options.use_existing = true,
            "--get-server-source" => {
                options.server_task = Some((SetupTask::Source, Navmesh::All));
            }
            "--prepare-server-data" => {
                options.server_task = Some((SetupTask::Data, Navmesh::All));
            }
            "--make-navmesh" => {
                // An optional MAP or MAP,X,Y narrows the run (the whole world by default).
                let scope = match args.as_slice().first() {
                    Some(next) if !next.starts_with("--") => {
                        Navmesh::parse(&args.next().unwrap_or_default())?
                    }
                    _ => Navmesh::All,
                };
                options.server_task = Some((SetupTask::Navmesh, scope));
            }
            "--doctor" => options.doctor = true,
            "--setup" => options.setup = true,
            "--validate" => options.validate_only = true,
            "--help" | "-h" => options.help = true,
            "--repo" | "--data" | "--target-dir" | "--locale" | "--server" => {
                let value = args
                    .next()
                    .filter(|v| !v.starts_with("--"))
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                match arg.as_str() {
                    "--repo" => options.config.repo = PathBuf::from(value),
                    "--data" => options.config.data = PathBuf::from(value),
                    "--target-dir" => options.config.target = PathBuf::from(value),
                    "--locale" => options.config.locale = value,
                    "--server" => options.config.server = value,
                    _ => unreachable!(),
                }
            }
            _ => return Err(format!("Unknown argument: {arg}. Use --help.")),
        }
    }
    if [
        options.setup,
        options.validate_only,
        options.doctor,
        options.use_existing,
        options.server_task.is_some(),
    ]
    .into_iter()
    .filter(|v| *v)
    .count()
        > 1
    {
        return Err("Choose only one of --setup, --validate, --doctor, --use-existing or a server setup task".into());
    }
    Ok(options)
}

pub(crate) const HELP: &str = "WrathPlus Manager\n\nWithout --setup, opens the graphical client setup and launcher.\n\n  --setup             Validate assets and build the game, then exit; never launches\n  --use-existing      Validate assets and save settings for a detected build; no launch\n  --doctor            Check build prerequisites without compiling or launching\n  --validate          Read-only asset validation, then exit; no build or launch\n  --repo PATH         WrathPlus checkout (defaults to current directory)\n  --data PATH         Your legally obtained 3.3.5a/build 12340 Data folder\n  --locale LOCALE     Asset locale (default enUS)\n  --server HOST:PORT  Authentication server (default 127.0.0.1:3724)\n  --target-dir PATH   Cargo and managed binary directory (default CARGO_TARGET_DIR or repo/target)\n\nLocal server setup (the Server page's steps, without the window):\n  --get-server-source    Download the pinned AzerothCore + Playerbots source and select it\n  --prepare-server-data  Extract dbc, maps and vmaps from the --data client and select them\n  --make-navmesh [MAP[,X,Y]]  Make navigation meshes (whole world, one map or one tile)\n\nNo game files are supplied or downloaded. Managed local server setup is available in the GUI Server page.";

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_paths_with_spaces_and_setup_flag() {
        let args = [
            "--setup",
            "--data",
            "/my games/Data",
            "--server",
            "realm.example:3724",
        ];
        let options = parse(args.map(str::to_string)).unwrap();
        assert!(options.setup);
        assert_eq!(options.config.data, PathBuf::from("/my games/Data"));
        assert_eq!(options.config.server, "realm.example:3724");
    }
    #[test]
    fn read_only_validation_and_explicit_repo_target() {
        let options = parse(
            [
                "--validate",
                "--repo",
                "/checkout",
                "--target-dir",
                "/build cache",
            ]
            .map(str::to_string),
        )
        .unwrap();
        assert!(options.validate_only);
        assert!(!options.setup);
        assert_eq!(options.config.repo, PathBuf::from("/checkout"));
        assert_eq!(options.config.target, PathBuf::from("/build cache"));
        assert!(parse(["--setup".into(), "--validate".into()]).is_err());
    }

    #[test]
    fn server_setup_tasks_take_an_optional_navmesh_scope() {
        let task = |args: &[&str]| parse(args.iter().map(|a| a.to_string())).map(|o| o.server_task);
        assert_eq!(
            task(&["--get-server-source"]).unwrap(),
            Some((SetupTask::Source, Navmesh::All))
        );
        assert_eq!(
            task(&["--make-navmesh", "0,32,48", "--data", "/d"]).unwrap(),
            Some((SetupTask::Navmesh, Navmesh::Tile(0, 32, 48)))
        );
        assert_eq!(
            task(&["--make-navmesh", "--data", "/d"]).unwrap(),
            Some((SetupTask::Navmesh, Navmesh::All))
        );
        assert!(task(&["--make-navmesh", "x"]).is_err());
        assert!(task(&["--prepare-server-data", "--setup"]).is_err());
    }

    #[test]
    fn invalid_arguments_fail_closed() {
        assert!(parse(["--doctor".into()]).unwrap().doctor);
        assert!(parse(["--doctor".into(), "--setup".into()]).is_err());
        assert!(parse(["--doctor".into(), "--validate".into()]).is_err());
        assert!(parse(["--launch".into()]).is_err());
        assert!(parse(["--data".into()]).is_err());
        assert!(parse(["--data".into(), "--setup".into()]).is_err());
    }
}
