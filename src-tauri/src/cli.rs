//! The command line: `pswm [<file.kdbx>] [--data-dir <path>]`, `--help`,
//! `--version`, and `--autostart` (added by "Start with Windows").

use std::path::{Path, PathBuf};

/// Passed by the "Start with Windows" entry: start in the tray, locked.
pub const AUTOSTART: &str = "--autostart";

pub const USAGE: &str = "\
Usage:
  pswm [<file.kdbx>] [--data-dir <path>]
  pswm --help | --version

  <file.kdbx>        Open this database (as a local file, not synced)
  --data-dir <path>  Keep the settings and caches in <path>
  --help             Show this help
  --version          Show the version";

#[derive(Debug, Default, PartialEq)]
pub struct Options {
    pub database: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
    pub autostart: bool,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Run(Options),
    Help,
    Version,
}

/// Reads the arguments (without the program name). Relative paths are taken
/// from `cwd`: a second launch passes on the folder it was started in.
pub fn parse(args: impl IntoIterator<Item = String>, cwd: &Path) -> Result<Command, String> {
    let mut options = Options::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" | "/?" => return Ok(Command::Help),
            "--version" | "-V" => return Ok(Command::Version),
            AUTOSTART => options.autostart = true,
            "--data-dir" => {
                let dir = args.next().ok_or("--data-dir needs a folder")?;
                options.data_dir = Some(cwd.join(dir));
            }
            other if other.starts_with('-') => return Err(format!("Unknown option {other}")),
            file if options.database.is_none() => options.database = Some(cwd.join(file)),
            file => return Err(format!("Only one database can be opened, not also {file}")),
        }
    }
    Ok(Command::Run(options))
}

/// Writes to the console the app was started from, if any: the release build
/// is a window app and has no console of its own.
pub fn print(text: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        // SAFETY: no pointers; fails harmlessly when there is no parent console
        // (or the debug build already has one).
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
    println!("{text}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|a| a.to_string()), Path::new(r"C:\work"))
    }

    #[test]
    fn a_database_and_a_data_folder() {
        let Ok(Command::Run(options)) = parse_args(&["base.kdbx", "--data-dir", r"D:\pswm"]) else { panic!() };
        assert_eq!(options.database, Some(PathBuf::from(r"C:\work\base.kdbx")));
        assert_eq!(options.data_dir, Some(PathBuf::from(r"D:\pswm")));
        assert!(!options.autostart);
    }

    #[test]
    fn help_version_and_autostart() {
        assert_eq!(parse_args(&["base.kdbx", "--help"]), Ok(Command::Help));
        assert_eq!(parse_args(&["--version"]), Ok(Command::Version));
        assert_eq!(parse_args(&[AUTOSTART]), Ok(Command::Run(Options { autostart: true, ..Options::default() })));
        assert_eq!(parse_args(&[]), Ok(Command::Run(Options::default())));
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse_args(&["--nope"]).is_err());
        assert!(parse_args(&["--data-dir"]).is_err());
        assert!(parse_args(&["a.kdbx", "b.kdbx"]).is_err());
    }
}
