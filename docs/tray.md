# The tray

Closing the window leaves Yapfast running in the system tray: the session, a call, notifications, ringing and sounds go on. The
tray icon is Yapfast's logo in Yap's marigold, with a red dot cut into its
corner while a mention or a DM waits unread, as Discord counts them. Clicking
it opens the window. Its menu has Open Yapfast, Disconnect during a call, and
Quit.

Settings → Advanced has "Close to tray", on unless turned off
(`close_to_tray` in `layout.json`). Off, or with no tray to show the icon,
closing the window quits Yapfast, as it did before; the setting's description
says when there's no tray. A tray that starts after Yapfast, or comes back,
still gets the icon.

## Starting at sign-in

Settings → Advanced → "Start when you sign in" starts Yapfast with
`--background` when the person signs in to their computer (`src/autostart.rs`).
The entry is the setting, not `layout.json`: on Windows a value named
`Yapfast` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, on Linux
`io.github.yapfast.Yapfast.desktop` in `$XDG_CONFIG_HOME/autostart`, on macOS
the launchd agent `~/Library/LaunchAgents/io.github.yapfast.Yapfast.plist`.
Settings read it as they open, and again when the window comes to the front
with them open, so turning Yapfast off in Task Manager's Startup apps (an odd
first byte in `Explorer\StartupApproved\Run`), in System Settings' Login Items
on macOS 13 and later, or a desktop setting `Hidden=true` or deleting the
file, shows as off. Only the switch changes it, through a callback of its own;
a change to any other setting leaves it be. Turning it on here clears Task
Manager's record too; on the Mac only the person can turn it on again there,
so Yapfast opens Login Items for them, and says so under the switch, as it
says why it couldn't write the entry.

One thread reads and writes the entry, as on the Mac each read asks
Background Task Management and turning it on waits for its answer
(`src/ui/sign_in.rs`). It takes requests from a channel, in order; the ones
waiting come down to the latest, so clicks faster than macOS answers end where
the switch was left. Each request carries how many times the switch had
turned, and an answer a later turn overtook changes nothing.

Each start, outside the preview, points an entry that's on at this Yapfast
when the command it holds is another: after an update to a new folder, as the
Windows ZIP, an AppImage and Nix each give a version, after a move, or when
it lacks this Yapfast's `--data-dir`. A copy that runs from somewhere gone by
the next sign-in can't take it over, nor be turned on: a disk image or
Gatekeeper's temporary copy of an app not moved out of Downloads on the Mac,
or the temporary folder, as a program Windows opens inside a ZIP. On the Mac
a build outside an app, as from `cargo run`, leaves it be too. That only
writes the entry, and never opens anything. An AppImage's entry names the
AppImage, not its temporary mount. A path with a control character, which an
entry's line or XML couldn't hold, writes none. Linux's and the Mac's entries
reach the disk whole before they replace the old, and a copy a write cut short
left beside them goes with the next. The preview, and platforms without any of
these, don't show the switch. The Windows installer's "Start Yapfast
automatically when Windows starts" writes the same value when there's none,
so an update leaves the one there, with its `--data-dir` and Task Manager's
"off", and uninstalling removes it.

`--background` restores the saved session and goes to the tray without
showing the window, or shows it when there's no session to restore, since
signing in needs it. With `--preview` it goes to the tray with the sample
conversations, which shows a start at sign-in without an account. Windows
knows at once whether the icon went into the notification area; without it
the window shows. On Linux the bus says a moment later, and a tray that's
missing, or goes while the window is closed, opens the window. The Mac's menu bar always takes the icon. A window made
late, from the tray, fails over to Skia Software as the first does when the
GPU fails.

## The window

`src/ui/shell.rs` holds the window. Closing it drops the whole component, its
renderer and its window, rather than hiding it: a hidden Slint window keeps
all of that, and saves nothing ([measurements](measurements/README.md)).
What only the window used stops with it: the microphone meters and the camera
test that Voice settings run, the image viewer's picture and an open file
dialog. Half a second later Yapfast gives freed memory back: `malloc_trim` on
Linux; on Windows `HeapCompact` on the process heap, which Rust allocates
from, and `SetProcessWorkingSetSize(-1, -1)`, which empties the working set.
Task Manager's Memory column is the private working set, which the window's
pages would otherwise stay in; they come back from the standby list if
touched.

With Skia Software, closing saves 2.3 MiB of anonymous memory in a channel
(18.6 to 16.3 MiB) and 0.9 MiB in a call; with the GPU, 24 MiB on the first
close ([measurements](measurements/README.md#2026-10-14-closing-the-window-to-the-tray)).
The models stay with the adapter, so a new window shows them without building
them again. Releasing the timeline's rows, the window's copies of pictures and
Skia's caches as well saved 0.25 MiB of heap and no anonymous memory, so they
stay.

Opening makes the window again with the size, place and maximized state of
the last one (Wayland ignores the place), the same conversation scrolled to
the message that was at its top, or to the newest if that showed, and
everything else from the application's state. The adapter remembers what it
last set for a few properties, such as the sidebar's title and the topic, to
set them only on a change; attaching a window forgets that, so the first
refresh fills them in. Yapfast's event loop runs until quit, so Quit, the
renderer restart and closing without a tray end it.

Without a window, backend events still apply. After each burst Yapfast rings for
an incoming DM call, or stops, and updates the tray icon. A notification
clicked while the window is closed opens it on that conversation.

## Linux

The icon is a StatusNotifierItem through [`ksni`](https://crates.io/crates/ksni),
pure Rust over the zbus Yapfast already has for notifications, with ksni's
async-io and blocking features, so it adds only `ksni` and `task-local` to
`Cargo.lock`. It connects to the session bus Yapfast is given. A thread
(`yapfast-tray`) registers it and sends it the unread dot and the call as they
change, since each update waits on the bus; ksni's own thread answers the
tray. ksni follows the StatusNotifierWatcher: Yapfast counts the tray as there
while the watcher is.

KDE Plasma, and most bars such as Waybar, run a watcher. GNOME needs the
AppIndicator extension; without it closing quits.

## Windows

The icon is `Shell_NotifyIconW` through `windows-sys`, already in the tree for
winit, on a hidden window of the UI thread, whose messages winit's loop
dispatches. A left click opens Yapfast; a right click shows the menu at the
pointer with `TrackPopupMenu`. When Explorer starts again it sends
`TaskbarCreated`, and Yapfast adds the icon again. If the notification area
won't take the icon, closing quits.

The bar is a cross-build and Wine (`scripts/build-windows`, see
[the Windows beta](windows-beta.md)). On real Windows, check:

- the icon is crisp at 100%, 125%, 150% and 200% scaling, and its dot shows
  for an unread DM or a mention;
- a left click opens the window, at its size and place, on the same
  conversation; a right click shows Open Yapfast, Disconnect in a call, and
  Quit, which closes when clicking elsewhere;
- a call keeps going with the window closed, and Disconnect ends it;
- Task Manager's Memory column drops when the window closes, and stays down
  for a minute;
- after restarting Explorer, the icon comes back;
- the icon goes when Yapfast quits, and doesn't linger until hovered.
- with "Start when you sign in" on, signing out and in starts Yapfast in the
  tray with no window, and Task Manager's Startup apps lists it; turning it
  off there shows as off in Settings;
- the installer's "Start Yapfast automatically when Windows starts" does the same, an update
  installs over a running Yapfast, and uninstalling leaves nothing in Startup
  apps;
- an update over an entry with `--data-dir`, or one Task Manager has off,
  leaves it so.

The registry side is checked under Wine: `autostart::windows::tests`, ignored
since it writes the current user's Run key, passes with a throwaway
`WINEPREFIX`, and the installer installs, updates over Task Manager's "off",
and uninstalls under `xvfb-run` in one, leaving the data folder. That was
before the installer left a value already there alone, which isn't checked
under Wine yet.

## macOS

The icon is an `NSStatusItem` in the menu bar, through the objc2 AppKit
bindings winit already builds, made on the UI thread, whose run loop winit
runs. As Mac menu bar icons do, a click shows the menu; its items call a small
Objective-C class, `YapfastTrayTarget`, which hands the choice to Slint's event
loop. The menu bar always takes the icon, so closing goes to it.

A Mac app with its window closed keeps its Dock icon, and clicking that should
bring the window back. AppKit asks the application delegate,
`applicationShouldHandleReopen:hasVisibleWindows:`, and winit's delegate
doesn't answer, so Yapfast adds that method to winit's class when the tray
starts: it opens the window, as Open Yapfast does, whatever AppKit counts as
showing. Opening makes Yapfast the active app too, as a click in the menu bar
or in Notification Center doesn't, so the window comes in front.

Command-Q, or Quit in the Dock, asks the delegate
`applicationShouldTerminate:`, which winit doesn't answer either, so AppKit
would end the process where it stands, in a call or with what Yapfast saves
unsaved. Yapfast's answer cancels that and quits as the menu bar's Quit does,
then ends; when the Mac is logging out, restarting or shutting down, which
the quit event's reason says, it lets AppKit go on, as cancelling would stop
that. Command-W closes the window into the menu bar, and Command-M minimizes
it (`src/ui.rs`), in any keyboard layout, which winit's menu leaves out.

The app menu, with Quit on Command-Q, is Slint's, which it makes as a window
first becomes active; winit's own is off. Started at sign-in, with no window,
the menu bar would show an empty app menu and Command-Q would do nothing, so
when there's none yet the tray puts in one with the same items: About,
Services, Hide, Hide Others, Show All and Quit. Slint's takes its place once a
window shows, and stays after it closes.

Starting at sign-in is a launchd agent, as Linux's is a desktop entry:
launchd loads the person's agents as they sign in, and runs this one, which
is `RunAtLoad`, once. It runs Yapfast's own program inside the app, with
`--background`, rather than `open -a` through LaunchServices, which would pass
the arguments only to an app not yet running, and would make `open` the
program System Settings shows, rather than Yapfast's signature;
`AssociatedBundleIdentifiers` names the app too. A program launchd starts is
responsible for itself, as one the Finder opens is, so macOS asks for the
microphone, and keeps notifications, for Yapfast itself, as it would not for
one started from a terminal. `LimitLoadToSessionType` keeps it to sessions at
the screen, not SSH ones, and `ProcessType` `Interactive` gives it what an
app opened from the Finder has, for calls. Each start leaves an entry that's
already right alone.

winit makes the app active as it finishes launching, which, started at
sign-in with no window, would take the keyboard from the app in front. So
`ui::run` decides whether to start hidden before it picks the backend, and
for a hidden start gives winit an event loop with
`with_activate_ignoring_other_apps(false)`. Yapfast stays a regular app, with
its Dock icon from the start, which opens the window, as it does after the
window closes. An accessory app until a window shows would have no Dock icon
to open it from, an icon that came and went, and winit would still activate
it.

With "Reopen windows when logging back in", macOS opens the apps left running
at log-out again, through LaunchServices, as the person signs back in, while
launchd starts the agent: two would run, one with its window. While the entry
is on, Yapfast calls `-[NSApplication disableRelaunchOnLogin]`, as Apple
suggests for an app launchd starts, and `enableRelaunchOnLogin` once it's
off; AppKit counts the calls, so each goes once.

From macOS 13, System Settings → General → Login Items lists the agent under
Allow in the Background, with a switch, and macOS says so in a notification,
Background Items Added, when it's first written.
`+[SMAppService statusForLegacyURL:]` reads the switch: "requires approval"
when the person turned it off. A new entry reads "not found" for about a
second, until macOS has looked it over, then "enabled", as does one whose
program is gone, which macOS doesn't check; so only "requires approval"
counts as off. Turning it on waits up to two seconds for macOS's answer, which
may be that the person turned it off before. SMAppService is new in macOS 13,
so Yapfast looks the class up by name, and before 13 the file alone is the
setting.

The other ways were SMAppService's own. `SMAppService.mainApp`, the app itself
as a login item, passes no arguments, so Yapfast would have to tell a start at
sign-in from its launch Apple event before `ui::run` decides whether to show
the window. `SMAppService.agent(plistName:)`, an agent inside the app at
`Contents/Library/LaunchAgents` with `BundleProgram`, which
`scripts/package-macos.py` would put there, was tried with the ad hoc signed
app: it registers, reads "enabled", and unregistering ends its job. But
registering runs a `RunAtLoad` agent at once, a second Yapfast beside the one
that turned it on; its arguments are fixed in the bundle, so without
`--data-dir`; and deleting the app left it registered and enabled, its job
loaded, until an app at the same path unregistered it. A build outside a
bundle, and macOS 11 and 12, would need the file anyway, so the file stays.

Deleting Yapfast leaves the file: launchd can't start it at each sign-in, and
Login Items keeps listing it. Turning "Start when you sign in" off before
deleting Yapfast removes it; after, delete
`~/Library/LaunchAgents/io.github.yapfast.Yapfast.plist`, and macOS forgets
it.

Checked on a MacBook Air (M4, macOS 15.3), with the disk image's app carrying
this build, outside Applications:

- `autostart::macos::tests::writes_an_agent_launchd_reads` writes an agent
  that `plutil` reads back; `turns_on_and_off_as_system_settings_sees_it`,
  ignored since it writes `~/Library/LaunchAgents`, turns it on, reads it
  back, sees the test, a build outside an app, leave it be, and turns it off.
- Started against a test server on 127.0.0.1, with an entry naming another
  copy that's still there, without `--data-dir`, Yapfast rewrote it with its
  own program and `--data-dir`, and `statusForLegacyURL:` said enabled. The
  same app run from a mounted disk image left it be, and its switch said
  "Yapfast runs from a disk image. Move it to Applications first."
- Through a probe build's control commands, doing what the switch, another
  setting and opening Settings do: turning it on wrote the entry, waited
  0.7 s for "enabled", and disabled relaunching at sign-in; an entry deleted,
  or written, outside Yapfast stayed so as another setting changed; opening
  Settings read it again; turning it off removed it and enabled relaunching.
  On, off and on at once ended on, with one answer shown.
- `launchctl bootstrap` of that entry, with `--preview` and a data folder of
  its own, as signing in loads it: launchd started Yapfast, which had only its
  menu bar icon on screen, no window, was responsible for itself, and logged
  to its data folder. The app menu held all six items, and Command-Q, sent
  through `-[NSApplication sendEvent:]`, quit it through
  `applicationShouldTerminate:` with exit code 0; without the app menu it did
  nothing. NSRunningApplication's `terminate`, as Quit in the Dock does, quit
  it the same way, and launchd didn't start it again. The screen was locked,
  so loginwindow stayed in front, with this build and the one before.
- The app in front keeping the keyboard at an unlocked screen, turning it off
  in System Settings, how Login Items names an ad hoc signed app's entry, and
  a real log-out and sign-in need someone at the Mac.

## Testing

`scripts/ui-session start --tray` runs `testing/tray_host.rs`, a minimal
StatusNotifierWatcher, on the session's private bus. It refuses to run
outside the isolated display, where it would stand in for the desktop's tray.
`do 'tray menu'`, `'tray click Disconnect'`, `'tray activate'` and
`'tray icon NAME'` act as a tray would, `'tray off'` and `'tray on'` take the
tray away and bring it back, and `'window close'` closes the window as its
close button does. `testing/measure_tray.py` measures memory through these.
