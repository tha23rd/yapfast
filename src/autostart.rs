//! Starting Yapfast when the person signs in to their computer: a value in the
//! registry's Run key on Windows, an XDG autostart entry on Linux, a launchd
//! agent on macOS. The entry is the setting, so one turned off in Task Manager
//! or System Settings, or a deleted file, shows as off in Settings too. It
//! starts Yapfast with `--background`, in the tray.

use std::path::PathBuf;

#[cfg(target_os = "macos")]
pub use macos::{open_login_items, reopen_at_sign_in};

/// Whether this platform can start Yapfast at sign-in.
pub const SUPPORTED: bool = cfg!(any(target_os = "linux", windows, target_os = "macos"));

/// The name the entry goes by: the Run key's value, and the Inno Setup
/// installer's, which writes the same one.
#[cfg(windows)]
const NAME: &str = "Yapfast";

/// Whether Yapfast starts when the person signs in. On macOS this asks
/// Background Task Management, so it's for a thread other than the UI's.
pub fn enabled() -> bool {
    platform::enabled()
}

/// Starts Yapfast at sign-in, or stops it. On macOS 13 and later System
/// Settings can still have it off after, as `enabled` then says.
pub fn set(on: bool) -> std::io::Result<()> {
    match on {
        true => platform::write(&command()?),
        false => platform::remove(),
    }
}

/// Points an entry that's on at this Yapfast, as after an update to a new
/// folder, as the Windows ZIP, an AppImage and Nix each give a version, or a
/// move, or when it lacks this one's `--data-dir`. A copy run from a disk
/// image or a temporary folder, or on the Mac a build outside an app, leaves
/// it be, so as not to leave it naming nothing once that's gone. It only
/// writes the entry, and never asks the person anything.
pub fn refresh() {
    if !enabled() {
        return;
    }
    let command = match command() {
        Ok(command) => command,
        Err(error) => {
            tracing::info!(%error, "starting at sign-in starts the Yapfast it named");
            return;
        }
    };
    if !takes_over(platform::recorded().as_deref(), &command) {
        return;
    }
    match platform::write(&command) {
        Ok(()) => tracing::info!("starting at sign-in now starts this Yapfast"),
        Err(error) => tracing::warn!(%error, "could not update starting at sign-in"),
    }
}

/// Whether this Yapfast's `command` replaces the one the entry `recorded`:
/// it's another, and this copy is an installed one.
fn takes_over(recorded: Option<&[String]>, command: &[String]) -> bool {
    recorded != Some(command) && installed(std::path::Path::new(&command[0]))
}

/// Whether `program` is an installed Yapfast, to start at sign-in in place of
/// another: everywhere but the Mac, any that isn't somewhere temporary, which
/// `command` refuses; on the Mac, the program inside an app, not a build run
/// from its folder.
fn installed(program: &std::path::Path) -> bool {
    !cfg!(target_os = "macos") || in_app(program)
}

/// Whether `program` is an app's own, in `Something.app/Contents/MacOS`.
fn in_app(program: &std::path::Path) -> bool {
    program.parent().is_some_and(|folder| {
        folder.ends_with("Contents/MacOS")
            && folder
                .parent()
                .and_then(std::path::Path::parent)
                .and_then(std::path::Path::extension)
                .is_some_and(|extension| extension == "app")
    })
}

/// Why `program` can't be started at sign-in, if it runs from a place gone
/// by then: Gatekeeper's temporary copy of a downloaded app that wasn't
/// moved, a disk image on the Mac, or the temporary folder, where Windows
/// runs a program opened inside a ZIP.
fn temporary_place(program: &std::path::Path) -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        if program
            .components()
            .any(|part| part.as_os_str() == "AppTranslocation")
        {
            return Some("Yapfast runs from a temporary copy. Move it to Applications first.");
        }
        if program.starts_with("/Volumes") {
            return Some("Yapfast runs from a disk image. Move it to Applications first.");
        }
    }
    program
        .starts_with(std::env::temp_dir())
        .then_some("Yapfast runs from a temporary folder. Move it somewhere it stays first.")
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
    if let Some(place) = temporary_place(&program) {
        return Err(std::io::Error::other(place));
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
    let text = path
        .into_os_string()
        .into_string()
        .map_err(|path| std::io::Error::other(format!("{} isn't valid Unicode", path.display())))?;
    // Each entry holds its arguments on a line, or in XML, which a control
    // character would break.
    if text.contains(char::is_control) {
        return Err(std::io::Error::other(format!(
            "{text:?} holds a control character"
        )));
    }
    Ok(text)
}

/// A Run value's arguments, split as Windows splits a command line: quotes
/// group, and backslashes count only before a quote, two for each one kept.
#[cfg(any(windows, test))]
fn windows_arguments(line: &str) -> Vec<String> {
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let (mut started, mut quoted, mut backslashes) = (false, false, 0);
    for character in line.chars() {
        if character == '\\' {
            backslashes += 1;
            started = true;
            continue;
        }
        if character == '"' {
            argument.extend(std::iter::repeat_n('\\', backslashes / 2));
            match backslashes % 2 {
                1 => argument.push('"'),
                _ => quoted = !quoted,
            }
            started = true;
        } else {
            argument.extend(std::iter::repeat_n('\\', backslashes));
            if matches!(character, ' ' | '\t') && !quoted {
                if started {
                    arguments.push(std::mem::take(&mut argument));
                }
                started = false;
            } else {
                argument.push(character);
                started = true;
            }
        }
        backslashes = 0;
    }
    argument.extend(std::iter::repeat_n('\\', backslashes));
    if started {
        arguments.push(argument);
    }
    arguments
}

/// One argument of a desktop entry's Exec line, quoted as the Desktop Entry
/// specification asks when it holds anything but plain characters.
#[cfg(any(target_os = "linux", test))]
fn desktop_argument(argument: &str) -> String {
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

/// An Exec line's arguments, as `desktop_argument` quotes them; `None` for
/// one it couldn't have written.
#[cfg(any(target_os = "linux", test))]
fn desktop_arguments(exec: &str) -> Option<Vec<String>> {
    let mut arguments = Vec::new();
    let mut characters = exec.chars().peekable();
    loop {
        while characters.next_if_eq(&' ').is_some() {}
        let Some(first) = characters.next() else {
            return Some(arguments);
        };
        let mut argument = String::new();
        if first == '"' {
            loop {
                match characters.next()? {
                    '"' => break,
                    '\\' => argument.push(characters.next()?),
                    character => argument.push(character),
                }
            }
        } else {
            argument.push(first);
            while let Some(character) = characters.next_if(|&character| character != ' ') {
                argument.push(character);
            }
        }
        arguments.push(argument.replace("%%", "%"));
    }
}

/// An agent's arguments, as `launch_agent` writes them.
#[cfg(any(target_os = "macos", test))]
fn agent_arguments(agent: &str) -> Option<Vec<String>> {
    let (_, arguments) = agent.split_once("<key>ProgramArguments</key>")?;
    let (arguments, _) = arguments.split_once("</array>")?;
    let arguments = arguments.split("<string>").skip(1).map(|argument| {
        let (text, _) = argument.split_once("</string>")?;
        Some(
            text.replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&"),
        )
    });
    arguments.collect()
}

/// Writes `text` to `path` whole or not at all, so launchd or a desktop
/// reading it never sees half, and on disk before it takes the old one's
/// place. A write cut short before, as by a crash, left its copy beside it,
/// which goes first; the copies' names start with a dot, which launchd and
/// desktops pass over.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn write_whole(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let folder = path.parent().expect("the entry is in a folder");
    std::fs::create_dir_all(folder)?;
    let prefix = format!(".{}.", crate::icon::APP_ID);
    for leftover in std::fs::read_dir(folder)?.flatten() {
        if leftover.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(leftover.path());
        }
    }
    let mut file = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(folder)?;
    file.write_all(text.as_bytes())?;
    file.as_file().sync_all()?;
    file.persist(path).map(drop).map_err(|error| error.error)
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

    pub fn recorded() -> Option<Vec<String>> {
        entry_command(&config()?)
    }

    pub(super) fn entry_command(config: &Path) -> Option<Vec<String>> {
        let entry = std::fs::read_to_string(entry(config)).ok()?;
        let exec = entry.lines().find_map(|line| line.strip_prefix("Exec="))?;
        super::desktop_arguments(exec)
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
        let exec: Vec<String> = command
            .iter()
            .map(|argument| super::desktop_argument(argument))
            .collect();
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Yapfast\n\
             Comment=Starts Yapfast in the system tray when you sign in\n\
             Exec={}\nIcon={}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exec.join(" "),
            crate::icon::APP_ID,
        );
        super::write_whole(&entry(config), &text)
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
            assert_eq!(entry_command(config.path()).as_deref(), Some(&command[..]));
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

    pub fn recorded() -> Option<Vec<String>> {
        let value = query(RUN, RRF_RT_REG_SZ)?;
        let line: Vec<u16> = value
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|&unit| unit != 0)
            .collect();
        Some(super::windows_arguments(&String::from_utf16_lossy(&line)))
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
        cell::Cell,
        io,
        path::{Path, PathBuf},
        time::Duration,
    };

    use objc2::{MainThreadMarker, msg_send, rc::autoreleasepool, runtime::AnyClass};
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSString, NSURL};

    // ServiceManagement has been in macOS since 10.6. Only SMAppService, which
    // reads System Settings' switch, is new, in macOS 13, so it's looked up
    // when used, and before 13 isn't there.
    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    /// `SMAppServiceStatusEnabled`: the entry runs at sign-in.
    const ENABLED: isize = 1;
    /// `SMAppServiceStatusRequiresApproval`: the person turned the entry off
    /// in System Settings.
    const REQUIRES_APPROVAL: isize = 2;

    thread_local! {
        /// Yapfast asked AppKit not to open it again as the person signs in.
        static NOT_REOPENED: Cell<bool> = const { Cell::new(false) };
    }

    /// The entry is there, and System Settings doesn't have it off. Before
    /// macOS has looked a new entry over, for about a second, it says it
    /// isn't found, and it says it's enabled even of one whose program is
    /// gone: only "requires approval" is off.
    pub fn enabled() -> bool {
        agents().is_some_and(|agents| {
            let path = entry(&agents);
            path.exists() && status(&path) != Some(REQUIRES_APPROVAL)
        })
    }

    pub fn write(command: &[String]) -> io::Result<()> {
        let agents = agents().ok_or_else(|| io::Error::other("no home folder"))?;
        let path = entry(&agents);
        if !write_entry(&agents, command)? {
            return Ok(());
        }
        // Wait for macOS to look it over, so what `enabled` says after is its
        // answer, which may be that the person turned Yapfast off before.
        for _ in 0..20 {
            if status(&path).is_none_or(|status| matches!(status, ENABLED | REQUIRES_APPROVAL)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }

    pub fn recorded() -> Option<Vec<String>> {
        let agent = std::fs::read_to_string(entry(&agents()?)).ok()?;
        super::agent_arguments(&agent)
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

    /// Writes the entry, unless it's already right, so macOS doesn't look it
    /// over again; whether it wrote.
    pub(super) fn write_entry(agents: &Path, command: &[String]) -> io::Result<bool> {
        let path = entry(agents);
        let text = super::launch_agent(command);
        if std::fs::read(&path).is_ok_and(|old| old == text.as_bytes()) {
            return Ok(false);
        }
        super::write_whole(&path, &text)?;
        Ok(true)
    }

    /// What System Settings → General → Login Items says of the entry, from
    /// macOS 13, which lists it under Allow in the Background with a switch.
    /// `None` before, where the file alone is the setting.
    fn status(path: &Path) -> Option<isize> {
        let service = AnyClass::get(c"SMAppService")?;
        // Off the UI thread, where nothing else drains what it autoreleases.
        autoreleasepool(|_| {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
            // SAFETY: the class method takes a file URL and returns an
            // SMAppServiceStatus, an NSInteger.
            Some(unsafe { msg_send![service, statusForLegacyURL: &*url] })
        })
    }

    /// Opens System Settings at Login Items, from macOS 13: once the person
    /// turned Yapfast off there, only they can turn it on again.
    pub fn open_login_items() {
        if let Some(service) = AnyClass::get(c"SMAppService") {
            // SAFETY: a class method without arguments that returns nothing.
            let () = unsafe { msg_send![service, openSystemSettingsLoginItems] };
        }
    }

    /// Whether macOS may open Yapfast again as the person signs back in,
    /// which "Reopen windows when logging back in" does for apps left
    /// running. While the entry is on, it starts Yapfast already, and two
    /// would run, one with its window. AppKit counts these calls, so each
    /// way goes once.
    pub fn reopen_at_sign_in(reopen: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        if NOT_REOPENED.replace(!reopen) == !reopen {
            return;
        }
        let app = NSApplication::sharedApplication(mtm);
        match reopen {
            true => app.enableRelaunchOnLogin(),
            false => app.disableRelaunchOnLogin(),
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
            assert!(write_entry(agents.path(), &command).unwrap());
            let agent = read(agents.path());
            assert_eq!(agent["Label"], json!(crate::icon::APP_ID));
            assert_eq!(agent["ProgramArguments"], json!(command));
            assert_eq!(agent["RunAtLoad"], json!(true));
            assert_eq!(agent["LimitLoadToSessionType"], json!("Aqua"));
            let text = std::fs::read_to_string(entry(agents.path())).unwrap();
            assert_eq!(
                super::super::agent_arguments(&text).as_deref(),
                Some(&command[..])
            );
            assert!(!write_entry(agents.path(), &command).unwrap(), "it's right");
        }

        /// Writes the person's own LaunchAgents, which System Settings shows,
        /// and macOS may say so: run it on a Mac you test on.
        #[test]
        #[ignore = "writes ~/Library/LaunchAgents"]
        fn turns_on_and_off_as_system_settings_sees_it() {
            remove().unwrap();
            assert!(!enabled());
            let gone = [
                "/Applications/Gone/Yapfast.app/Contents/MacOS/yapfast",
                "--background",
            ]
            .map(String::from);
            write(&gone).unwrap();
            assert!(enabled(), "macOS has it on, though its program is gone");
            assert_eq!(recorded().as_deref(), Some(&gone[..]));
            // A build outside an app, as this test is, leaves it be.
            super::super::refresh();
            let agents = agents().unwrap();
            assert_eq!(read(&agents)["ProgramArguments"], json!(gone));
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

    pub fn recorded() -> Option<Vec<String>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn reads_back_the_command_each_entry_holds() {
        let command = [
            "/Applications/A & B/Yapfast.app/Contents/MacOS/yapfast",
            "--background",
            "--data-dir",
            "/Users/a/<data>",
        ]
        .map(String::from);
        assert_eq!(
            agent_arguments(&launch_agent(&command)).as_deref(),
            Some(&command[..])
        );
        let command = [
            "/opt/Yapfast Builds/100% \"yap\" $fast",
            "--background",
            "--data-dir",
            r"/home/a/back\slash",
            "",
        ]
        .map(String::from);
        let exec: Vec<String> = command.iter().map(|it| desktop_argument(it)).collect();
        assert_eq!(
            desktop_arguments(&exec.join(" ")).as_deref(),
            Some(&command[..])
        );
        assert_eq!(desktop_arguments(r#""unclosed"#), None);
        let command = [
            r"C:\Program Files\Yapfast\yapfast.exe",
            "--background",
            "--data-dir",
            r"C:\A B\",
            r#"say "hi""#,
            "",
        ];
        let line = command.map(windows_argument).join(" ");
        assert_eq!(windows_arguments(&line), command, "{line}");
        // As the installer writes it.
        assert_eq!(
            windows_arguments(r#""C:\Users\A\Programs\Yapfast\yapfast.exe" --background"#),
            [r"C:\Users\A\Programs\Yapfast\yapfast.exe", "--background"]
        );
    }

    #[test]
    fn an_installed_copy_takes_over_an_entry_that_starts_another() {
        let new = [
            if cfg!(target_os = "macos") {
                "/Applications/Yapfast.app/Contents/MacOS/yapfast"
            } else {
                "/opt/yapfast-1.1/yapfast"
            },
            "--background",
        ]
        .map(String::from);
        let old = ["/opt/yapfast-1.0/yapfast", "--background"].map(String::from);
        assert!(takes_over(Some(&old), &new), "an update to a new folder");
        assert!(!takes_over(Some(&new), &new));
        let mut with_data = new.to_vec();
        with_data.extend(["--data-dir".to_owned(), "/data".to_owned()]);
        assert!(takes_over(Some(&new), &with_data), "this one's data folder");
        assert!(takes_over(None, &new), "an entry it can't read");
        assert!(in_app(Path::new(
            "/Applications/Yapfast.app/Contents/MacOS/yapfast"
        )));
        assert!(!in_app(Path::new("/Users/a/yapfast/target/fast/yapfast")));
    }

    #[test]
    fn a_copy_in_the_temporary_folder_cant_be_started_at_sign_in() {
        let zip = std::env::temp_dir().join("Temp1_Yapfast.zip");
        assert!(temporary_place(&zip.join("yapfast.exe")).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_disk_image_or_a_build_outside_an_app_doesnt_take_it_over() {
        let app = "Yapfast.app/Contents/MacOS/yapfast";
        let image = Path::new("/Volumes/Yapfast").join(app);
        assert!(temporary_place(&image).is_some());
        let translocated = Path::new("/private/var/folders/x/AppTranslocation/1/d").join(app);
        assert!(temporary_place(&translocated).is_some());
        assert!(temporary_place(&Path::new("/Applications").join(app)).is_none());
        let build = ["/Users/a/yapfast/target/fast/yapfast".to_owned()];
        assert!(!takes_over(None, &build));
    }

    #[test]
    fn refuses_a_path_an_entry_couldnt_hold() {
        assert!(path_text(PathBuf::from("/Applications/Yap\nfast.app")).is_err());
        assert!(path_text(PathBuf::from("/Applications/Yapfast.app")).is_ok());
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn writes_entries_whole_and_clears_what_a_cut_write_left() {
        let folder = tempfile::tempdir().unwrap();
        let leftover = folder.path().join(".io.github.yapfast.Yapfast.cut");
        std::fs::write(&leftover, "half").unwrap();
        let path = folder.path().join("entry");
        write_whole(&path, "whole").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "whole");
        let names: Vec<_> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|file| file.unwrap().file_name())
            .collect();
        assert_eq!(names, ["entry"]);
    }

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
