//! Starting Yapfast when the person signs in to their computer: a value in the
//! registry's Run key on Windows, an XDG autostart entry on Linux, a launchd
//! agent on macOS. The entry is the setting, so one turned off in Task Manager
//! or System Settings, or a deleted file, shows as off in Settings too. It
//! starts Yapfast with `--background`, in the tray.

use std::path::PathBuf;

/// Whether this platform can start Yapfast at sign-in.
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", windows, target_os = "macos"));

/// The name the entry goes by: the Run key's value, and the Inno Setup
/// installer's, which writes the same one.
#[cfg(windows)]
const NAME: &str = "Yapfast";

/// Whether Yapfast starts when the person signs in.
pub fn enabled() -> bool {
    platform::enabled()
}

/// Starts Yapfast at sign-in, or stops it.
pub fn set(on: bool) -> std::io::Result<()> {
    match on {
        true => platform::write(&command()?),
        false => platform::remove(),
    }
}

/// Points an entry at this Yapfast, after its folder moved or an update.
pub fn refresh() {
    if enabled()
        && let Err(error) = set(true)
    {
        tracing::warn!(%error, "could not update starting at sign-in");
    }
}

/// What starts Yapfast in the tray, with the data folder this one uses.
fn command() -> std::io::Result<Vec<String>> {
    // An AppImage runs from a temporary mount, APPDIR, and names its file in
    // APPIMAGE. Programs it starts inherit both, so they're Yapfast's only
    // when Yapfast runs from inside that mount.
    let exe = std::env::current_exe()?;
    let program = match (std::env::var_os("APPIMAGE"), std::env::var_os("APPDIR")) {
        (Some(image), Some(mount)) if cfg!(target_os = "linux") && exe.starts_with(&mount) => {
            PathBuf::from(image)
        }
        _ => exe,
    };
    // Gatekeeper runs a downloaded app that wasn't moved from a read-only
    // copy in a random folder, which is gone by the next sign-in.
    if cfg!(target_os = "macos")
        && program
            .components()
            .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Err(std::io::Error::other(
            "Yapfast runs from a temporary copy; move it to Applications first",
        ));
    }
    let mut command = vec![path_text(program)?, "--background".to_owned()];
    let arguments: Vec<String> = std::env::args().collect();
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--data-dir")
        && let Some(folder) = arguments.get(index + 1)
    {
        let folder = std::path::absolute(folder)?;
        command.extend(["--data-dir".to_owned(), path_text(folder)?]);
    }
    Ok(command)
}

fn path_text(path: PathBuf) -> std::io::Result<String> {
    path.into_os_string()
        .into_string()
        .map_err(|path| std::io::Error::other(format!("{} isn't valid Unicode", path.display())))
}

/// One argument of the command line, quoted when it holds a space, as
/// Windows splits a command line into arguments.
#[cfg(any(windows, test))]
fn windows_argument(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '"']) {
        return argument.to_owned();
    }
    // Backslashes count only before a quote, where each is doubled.
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

/// A launchd agent that runs `command` when the person signs in, as
/// `~/Library/LaunchAgents` holds them: only in sessions at the screen, not
/// over SSH, without the limits launchd puts on other jobs, as an app opened
/// from the Finder has none, for calls, and under the app's name in System
/// Settings.
#[cfg(any(target_os = "macos", test))]
fn launch_agent(command: &[String]) -> String {
    let arguments: String = command
        .iter()
        .map(|argument| format!("\t\t<string>{}</string>\n", xml_text(argument)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{id}</string>
	<key>ProgramArguments</key>
	<array>
{arguments}	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>AssociatedBundleIdentifiers</key>
	<array>
		<string>{id}</string>
	</array>
</dict>
</plist>
"#,
        id = crate::icon::APP_ID,
    )
}

/// Text as a property list's XML holds it.
#[cfg(any(target_os = "macos", test))]
fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
use none as platform;
#[cfg(windows)]
use windows as platform;

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};

    pub fn enabled() -> bool {
        config().is_some_and(|config| entry_enabled(&config))
    }

    pub fn write(command: &[String]) -> std::io::Result<()> {
        let config = config().ok_or_else(|| std::io::Error::other("no home folder"))?;
        write_entry(&config, command)
    }

    pub fn remove() -> std::io::Result<()> {
        match config() {
            Some(config) => match std::fs::remove_file(entry(&config)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
            None => Ok(()),
        }
    }

    /// `$XDG_CONFIG_HOME`, or `~/.config`.
    fn config() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| std::env::home_dir().map(|home| home.join(".config")))
    }

    fn entry(config: &Path) -> PathBuf {
        config
            .join("autostart")
            .join(format!("{}.desktop", crate::icon::APP_ID))
    }

    /// The entry is there and not turned off, as desktops turn one off
    /// without deleting it.
    pub(super) fn entry_enabled(config: &Path) -> bool {
        std::fs::read_to_string(entry(config)).is_ok_and(|entry| {
            !entry.lines().any(|line| {
                matches!(
                    line.replace(' ', "").as_str(),
                    "Hidden=true" | "X-GNOME-Autostart-enabled=false"
                )
            })
        })
    }

    pub(super) fn write_entry(config: &Path, command: &[String]) -> std::io::Result<()> {
        let path = entry(config);
        let folder = path.parent().expect("the entry is in a folder");
        std::fs::create_dir_all(folder)?;
        let exec: Vec<String> = command.iter().map(|argument| quoted(argument)).collect();
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Yapfast\n\
             Comment=Starts Yapfast in the system tray when you sign in\n\
             Exec={}\nIcon={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exec.join(" "),
            crate::icon::APP_ID,
        );
        // Whole or not at all: a desktop reading it never sees half.
        let file = tempfile::NamedTempFile::new_in(folder)?;
        std::fs::write(file.path(), text)?;
        file.persist(&path).map(drop).map_err(|error| error.error)
    }

    /// One argument of an entry's Exec line, quoted as the Desktop Entry
    /// specification asks when it holds anything but plain characters.
    fn quoted(argument: &str) -> String {
        let argument = argument.replace('%', "%%");
        let plain = !argument.is_empty()
            && argument
                .chars()
                .all(|character| character.is_alphanumeric() || "-_./=:+,@%".contains(character));
        if plain {
            return argument;
        }
        let mut quoted = String::from('"');
        for character in argument.chars() {
            if matches!(character, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(character);
        }
        quoted.push('"');
        quoted
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn writes_an_entry_that_starts_in_the_tray() {
            let config = tempfile::tempdir().unwrap();
            assert!(!entry_enabled(config.path()));
            let command = [
                "/opt/Yapfast Builds/yapfast".to_owned(),
                "--background".to_owned(),
                "--data-dir".to_owned(),
                "/home/a/100% $data".to_owned(),
            ];
            write_entry(config.path(), &command).unwrap();
            assert!(entry_enabled(config.path()));
            let text = std::fs::read_to_string(entry(config.path())).unwrap();
            assert!(text.contains(
                r#"Exec="/opt/Yapfast Builds/yapfast" --background --data-dir "/home/a/100%% \$data""#
            ));
            assert!(text.contains("Name=Yapfast\n"));
        }

        #[test]
        fn an_entry_a_desktop_turned_off_is_off() {
            let config = tempfile::tempdir().unwrap();
            write_entry(config.path(), &["yapfast".to_owned()]).unwrap();
            let path = entry(config.path());
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, text.replace("enabled=true", "enabled=false")).unwrap();
            assert!(!entry_enabled(config.path()));
            std::fs::write(&path, "[Desktop Entry]\nHidden=true\n").unwrap();
            assert!(!entry_enabled(config.path()));
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::io;

    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR},
        System::Registry::{
            HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW,
            RegGetValueW, RegSetKeyValueW,
        },
    };

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// Where Task Manager's Startup apps page keeps an entry turned off.
    const APPROVED: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    pub fn enabled() -> bool {
        let run = query(RUN, RRF_RT_REG_SZ);
        // Its first byte is odd while Task Manager has the entry turned off.
        let off = query(APPROVED, RRF_RT_REG_BINARY)
            .is_some_and(|approved| approved.first().is_some_and(|byte| byte & 1 == 1));
        run.is_some() && !off
    }

    pub fn write(command: &[String]) -> io::Result<()> {
        let line: Vec<String> = command
            .iter()
            .map(|argument| super::windows_argument(argument))
            .collect();
        let value = wide(&line.join(" "));
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                wide(RUN).as_ptr(),
                wide(super::NAME).as_ptr(),
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        checked(status)?;
        // Turning it on here turns it on in Task Manager too.
        delete(APPROVED)
    }

    pub fn remove() -> io::Result<()> {
        delete(RUN)?;
        delete(APPROVED)
    }

    fn delete(key: &str) -> io::Result<()> {
        let status = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                wide(key).as_ptr(),
                wide(super::NAME).as_ptr(),
            )
        };
        match status {
            ERROR_FILE_NOT_FOUND => Ok(()),
            status => checked(status),
        }
    }

    /// The entry's value under `key`, if it has one of the `kind` asked for.
    fn query(key: &str, kind: u32) -> Option<Vec<u8>> {
        let (key, name) = (wide(key), wide(super::NAME));
        let mut size = 0u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                kind,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut data = vec![0u8; size as usize];
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                kind,
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut size,
            )
        };
        (status == ERROR_SUCCESS).then(|| {
            data.truncate(size as usize);
            data
        })
    }

    fn checked(status: WIN32_ERROR) -> io::Result<()> {
        match status {
            ERROR_SUCCESS => Ok(()),
            status => Err(io::Error::from_raw_os_error(status as i32)),
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain([0]).collect()
    }

    /// Writes the current user's registry: run it under Wine with a
    /// throwaway WINEPREFIX, not on a Windows account.
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        #[ignore = "writes the current user's Run key"]
        fn turns_on_and_off_as_task_manager_sees_it() {
            remove().unwrap();
            assert!(!enabled());
            write(&[
                r"C:\Program Files\Yapfast\yapfast.exe".into(),
                "--background".into(),
            ])
            .unwrap();
            assert!(enabled());
            let value = query(RUN, RRF_RT_REG_SZ).unwrap();
            let text: Vec<u16> = value
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .take_while(|&unit| unit != 0)
                .collect();
            assert_eq!(
                String::from_utf16(&text).unwrap(),
                r#""C:\Program Files\Yapfast\yapfast.exe" --background"#
            );
            // Task Manager turns it off with an odd first byte.
            let disabled = [3u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            let status = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    wide(APPROVED).as_ptr(),
                    wide(super::super::NAME).as_ptr(),
                    windows_sys::Win32::System::Registry::REG_BINARY,
                    disabled.as_ptr().cast(),
                    disabled.len() as u32,
                )
            };
            checked(status).unwrap();
            assert!(!enabled());
            write(&["yapfast.exe".into()]).unwrap();
            assert!(enabled(), "turning it on here turns it on in Task Manager");
            remove().unwrap();
            assert!(!enabled());
            assert!(query(APPROVED, RRF_RT_REG_BINARY).is_none());
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        io,
        path::{Path, PathBuf},
    };

    use objc2::{msg_send, runtime::AnyClass};
    use objc2_foundation::{NSString, NSURL};

    // ServiceManagement has been in macOS since 10.6. Only SMAppService, which
    // reads System Settings' switch, is new, in macOS 13, so it's looked up
    // when used, and before 13 isn't there.
    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    /// `SMAppServiceStatusRequiresApproval`: the person turned the entry off
    /// in System Settings.
    const REQUIRES_APPROVAL: isize = 2;

    pub fn enabled() -> bool {
        agents().is_some_and(|agents| {
            let path = entry(&agents);
            path.exists() && allowed(&path)
        })
    }

    pub fn write(command: &[String]) -> io::Result<()> {
        let agents = agents().ok_or_else(|| io::Error::other("no home folder"))?;
        write_entry(&agents, command)?;
        // Only the person can turn it on again there, so show them where.
        if !allowed(&entry(&agents)) {
            open_login_items();
            return Err(io::Error::other(
                "Yapfast is turned off in System Settings, under Login Items",
            ));
        }
        Ok(())
    }

    pub fn remove() -> io::Result<()> {
        match agents() {
            Some(agents) => match std::fs::remove_file(entry(&agents)) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            },
            None => Ok(()),
        }
    }

    /// `~/Library/LaunchAgents`, whose agents launchd loads as the person
    /// signs in.
    fn agents() -> Option<PathBuf> {
        std::env::home_dir().map(|home| home.join("Library").join("LaunchAgents"))
    }

    fn entry(agents: &Path) -> PathBuf {
        agents.join(format!("{}.plist", crate::icon::APP_ID))
    }

    pub(super) fn write_entry(agents: &Path, command: &[String]) -> io::Result<()> {
        let path = entry(agents);
        let text = super::launch_agent(command);
        // Each start leaves an entry that's already right alone, rather than
        // have macOS look it over again.
        if std::fs::read(&path).is_ok_and(|old| old == text.as_bytes()) {
            return Ok(());
        }
        std::fs::create_dir_all(agents)?;
        // Whole or not at all: launchd reading it never sees half.
        let file = tempfile::NamedTempFile::new_in(agents)?;
        std::fs::write(file.path(), text)?;
        file.persist(&path).map(drop).map_err(|error| error.error)
    }

    /// Whether System Settings lets the entry run. From macOS 13 its Login
    /// Items list it under Allow in the Background, with a switch; before,
    /// an entry that's there runs.
    fn allowed(path: &Path) -> bool {
        let Some(service) = AnyClass::get(c"SMAppService") else {
            return true;
        };
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // SAFETY: the class method takes a file URL and returns an
        // SMAppServiceStatus, an NSInteger.
        let status: isize = unsafe { msg_send![service, statusForLegacyURL: &*url] };
        // A new entry reads as not found for a moment, until macOS has
        // looked it over, then as enabled.
        status != REQUIRES_APPROVAL
    }

    /// Opens System Settings at Login Items, from macOS 13.
    fn open_login_items() {
        if let Some(service) = AnyClass::get(c"SMAppService") {
            // SAFETY: a class method without arguments that returns nothing.
            let () = unsafe { msg_send![service, openSystemSettingsLoginItems] };
        }
    }

    #[cfg(test)]
    mod tests {
        use serde_json::{Value, json};

        use super::*;

        /// The entry as macOS's own property list parser reads it.
        fn read(agents: &Path) -> Value {
            let output = std::process::Command::new("plutil")
                .args(["-convert", "json", "-o", "-"])
                .arg(entry(agents))
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            serde_json::from_slice(&output.stdout).unwrap()
        }

        #[test]
        fn writes_an_agent_launchd_reads() {
            let agents = tempfile::tempdir().unwrap();
            let command = [
                "/Applications/A & B/Yapfast.app/Contents/MacOS/yapfast".to_owned(),
                "--background".to_owned(),
                "--data-dir".to_owned(),
                "/Users/a/<data>".to_owned(),
            ];
            write_entry(agents.path(), &command).unwrap();
            let agent = read(agents.path());
            assert_eq!(agent["Label"], json!(crate::icon::APP_ID));
            assert_eq!(agent["ProgramArguments"], json!(command));
            assert_eq!(agent["RunAtLoad"], json!(true));
            assert_eq!(agent["LimitLoadToSessionType"], json!("Aqua"));
        }

        /// Writes the person's own LaunchAgents, which System Settings shows,
        /// and macOS may say so: run it on a Mac you test on.
        #[test]
        #[ignore = "writes ~/Library/LaunchAgents"]
        fn turns_on_and_off_as_system_settings_sees_it() {
            remove().unwrap();
            assert!(!enabled());
            write(&["/Applications/Yapfast.app/Contents/MacOS/yapfast".into()]).unwrap();
            assert!(enabled());
            // Each start points it at the Yapfast that's running.
            super::super::refresh();
            let program = std::env::current_exe().unwrap();
            let agents = agents().unwrap();
            assert_eq!(
                read(&agents)["ProgramArguments"],
                json!([program.to_string_lossy(), "--background"])
            );
            assert!(enabled());
            remove().unwrap();
            assert!(!enabled());
            assert!(!entry(&agents).exists());
        }
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod none {
    pub fn enabled() -> bool {
        false
    }

    pub fn write(_: &[String]) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "starting at sign-in isn't supported here",
        ))
    }

    pub fn remove() -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{launch_agent, windows_argument};

    #[test]
    fn writes_a_launch_agent_that_starts_in_the_tray() {
        let agent = launch_agent(&[
            "/Applications/A & B/Yapfast.app/Contents/MacOS/yapfast".to_owned(),
            "--background".to_owned(),
            "--data-dir".to_owned(),
            "/Users/a/<data>".to_owned(),
        ]);
        assert!(agent.contains(
            "\t<key>ProgramArguments</key>\n\t<array>\n\
             \t\t<string>/Applications/A &amp; B/Yapfast.app/Contents/MacOS/yapfast</string>\n\
             \t\t<string>--background</string>\n\
             \t\t<string>--data-dir</string>\n\
             \t\t<string>/Users/a/&lt;data&gt;</string>\n\
             \t</array>\n"
        ));
        assert!(agent.contains("<key>Label</key>\n\t<string>io.github.yapfast.Yapfast</string>"));
        assert!(agent.contains("<key>RunAtLoad</key>\n\t<true/>"));
    }

    #[test]
    fn quotes_windows_arguments_as_windows_splits_them() {
        assert_eq!(windows_argument("--background"), "--background");
        assert_eq!(
            windows_argument(r"C:\Users\A B\Yapfast\yapfast.exe"),
            r#""C:\Users\A B\Yapfast\yapfast.exe""#
        );
        assert_eq!(windows_argument(r"C:\A B\"), r#""C:\A B\\""#);
        assert_eq!(windows_argument(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(windows_argument(""), r#""""#);
    }
}
