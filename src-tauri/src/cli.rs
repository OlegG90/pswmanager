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
        // SAFETY: no pointers; fails when there is no parent console, or when
        // the debug build already has a console of its own.
        let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0;
        println!("{text}");
        if attached {
            show_prompt_again();
        }
    }
    #[cfg(not(windows))]
    println!("{text}");
}

/// The shell does not wait for a window app: it printed its prompt before
/// our text. Pressing Enter for the user draws the prompt again below it —
/// only when the text went to the console itself, not to a file or a pipe.
#[cfg(windows)]
fn show_prompt_again() {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, WriteConsoleInputW, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD,
        KEY_EVENT_RECORD_0, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    const VK_RETURN: u16 = 0x0D;
    // SAFETY: handles come from the console just attached; the records live
    // for the call.
    unsafe {
        let mut mode = 0;
        if GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut mode) == 0 {
            return; // redirected
        }
        let key = |down| INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            Event: INPUT_RECORD_0 {
                KeyEvent: KEY_EVENT_RECORD {
                    bKeyDown: down,
                    wRepeatCount: 1,
                    wVirtualKeyCode: VK_RETURN,
                    wVirtualScanCode: 0x1C,
                    uChar: KEY_EVENT_RECORD_0 { UnicodeChar: 0x0D },
                    dwControlKeyState: 0,
                },
            },
        };
        let keys = [key(1), key(0)];
        let mut written = 0;
        WriteConsoleInputW(GetStdHandle(STD_INPUT_HANDLE), keys.as_ptr(), keys.len() as u32, &mut written);
    }
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
