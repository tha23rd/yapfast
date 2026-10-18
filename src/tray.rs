//! Yapfast's icon in the system tray, which keeps Yapfast running with its window
//! closed: a StatusNotifierItem on Linux, on the session bus Yapfast is given,
//! a notification-area icon on Windows, and a menu bar icon on macOS. Clicking
//! it opens the window, or on macOS its menu; the menu opens Yapfast, leaves a
//! call and quits.

use std::cell::Cell;

/// What the tray asks for, or says. Each runs on Slint's event loop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Open,
    Disconnect,
    Quit,
    /// A tray came or went, which decides what closing the window does. The
    /// menu bar is always there.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Available,
}

/// What the icon and its menu show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// A mention or a DM waits unread: the icon has a dot.
    pub unread: bool,
    /// In a call: the menu offers Disconnect.
    pub call: bool,
}

pub struct Tray {
    platform: platform::Tray,
    shown: Cell<Shown>,
}

impl Tray {
    /// Puts Yapfast's icon in the tray. `chosen` runs on Slint's event loop with
    /// what the person picked. `None` where there's no tray to use.
    pub fn start(chosen: fn(Choice)) -> Option<Self> {
        let platform = platform::Tray::start(chosen)?;
        Some(Self {
            platform,
            shown: Cell::new(Shown::default()),
        })
    }

    /// Whether a tray shows the icon now, so the window can close into it.
    pub fn available(&self) -> bool {
        self.platform.available()
    }

    /// Shows the unread dot and the call's Disconnect, telling the tray only
    /// when they change.
    pub fn show(&self, shown: Shown) {
        if self.shown.replace(shown) != shown {
            self.platform.show(shown);
        }
    }
}

/// Yapfast's logo in Yap's marigold, `size` pixels square, with a red dot for
/// unread messages cut into its corner.
fn logo(size: u32, unread: bool) -> Option<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    let mut svg = include_str!("../ui/icons/logo.svg")
        .replacen(
            r#"width="24" height="24""#,
            &format!(r#"width="{size}" height="{size}""#),
            1,
        )
        .replacen(r##"fill="#fff" mask"##, r##"fill="#ffbf47" mask"##, 1);
    if unread {
        svg = svg
            .replacen("</mask>", r#"<circle cx="74" cy="74" r="21" /></mask>"#, 1)
            .replacen(
                "</svg>",
                r##"<circle cx="74" cy="74" r="15" fill="#f0545e" /></svg>"##,
                1,
            );
    }
    slint::Image::load_from_svg_data(svg.as_bytes())
        .ok()?
        .to_rgba8()
}

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "macos")]
pub use macos::{activate, step_aside};
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
use none as platform;
#[cfg(windows)]
use windows as platform;

#[cfg(target_os = "linux")]
mod linux {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    use ksni::{MenuItem, menu::StandardItem};

    use super::{Choice, Shown};

    /// The tray's sizes of the icon; the tray scales the nearest.
    const SIZES: [u32; 2] = [32, 64];

    pub struct Tray {
        updates: mpsc::Sender<Shown>,
        /// Registered with a running StatusNotifierWatcher.
        online: Arc<AtomicBool>,
    }

    impl Tray {
        pub fn start(chosen: fn(Choice)) -> Option<Self> {
            let icons = |unread| {
                SIZES
                    .iter()
                    .filter_map(|&size| super::logo(size, unread))
                    .map(|pixels| ksni::Icon {
                        width: pixels.width() as i32,
                        height: pixels.height() as i32,
                        // ARGB, in network byte order.
                        data: pixels
                            .as_slice()
                            .iter()
                            .flat_map(|pixel| [pixel.a, pixel.r, pixel.g, pixel.b])
                            .collect(),
                    })
                    .collect()
            };
            let online = Arc::new(AtomicBool::new(false));
            let item = Item {
                shown: Shown::default(),
                icons: [icons(false), icons(true)],
                chosen,
                online: online.clone(),
            };
            let (updates, shown) = mpsc::channel();
            // Registering waits on the bus, and so does each update.
            let started = std::thread::Builder::new()
                .name("yapfast-tray".into())
                .spawn(move || run(item, shown));
            if let Err(error) = started {
                tracing::warn!(%error, "could not start the tray icon");
                return None;
            }
            Some(Self { updates, online })
        }

        pub fn available(&self) -> bool {
            self.online.load(Ordering::Relaxed)
        }

        pub fn show(&self, shown: Shown) {
            let _ = self.updates.send(shown);
        }
    }

    fn run(item: Item, shown: mpsc::Receiver<Shown>) {
        use ksni::blocking::TrayMethods;
        let online = item.online.clone();
        let chosen = item.chosen;
        // Until registering says otherwise; it reports a missing watcher.
        online.store(true, Ordering::Relaxed);
        // A tray that starts after Yapfast, or comes back, still gets the icon.
        let handle = match item.assume_sni_available(true).spawn() {
            Ok(handle) => handle,
            Err(error) => {
                online.store(false, Ordering::Relaxed);
                tracing::info!(%error, "no tray icon; closing the window quits Yapfast");
                return;
            }
        };
        if online.load(Ordering::Relaxed) {
            tracing::info!("tray icon shown");
            choose(chosen, Choice::Available);
        }
        for shown in shown {
            handle.update(|item| item.shown = shown);
        }
        handle.shutdown().wait();
    }

    struct Item {
        shown: Shown,
        /// The icon without the unread dot, and with it.
        icons: [Vec<ksni::Icon>; 2],
        chosen: fn(Choice),
        online: Arc<AtomicBool>,
    }

    fn choose(chosen: fn(Choice), choice: Choice) {
        let _ = slint::invoke_from_event_loop(move || chosen(choice));
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            "yapfast".into()
        }

        fn title(&self) -> String {
            "Yapfast".into()
        }

        fn category(&self) -> ksni::Category {
            ksni::Category::Communications
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icons[usize::from(self.shown.unread)].clone()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "Yapfast".into(),
                description: if self.shown.unread {
                    "Unread messages".into()
                } else {
                    String::new()
                },
                ..Default::default()
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            choose(self.chosen, Choice::Open);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let item = |label: &str, choice| {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |item: &mut Self| choose(item.chosen, choice)),
                    ..Default::default()
                }
                .into()
            };
            let mut menu = vec![item("Open Yapfast", Choice::Open)];
            if self.shown.call {
                menu.push(item("Disconnect", Choice::Disconnect));
            }
            menu.push(MenuItem::Separator);
            menu.push(item("Quit", Choice::Quit));
            menu
        }

        fn watcher_online(&self) {
            tracing::info!("tray icon shown");
            self.online.store(true, Ordering::Relaxed);
            choose(self.chosen, Choice::Available);
        }

        fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
            tracing::info!(?reason, "no tray to show the icon");
            self.online.store(false, Ordering::Relaxed);
            choose(self.chosen, Choice::Available);
            // Wait for it to come back.
            true
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::cell::{Cell, RefCell};

    use windows_sys::{
        Win32::{
            Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Shell::{
                    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
                    NOTIFY_ICON_MESSAGE, NOTIFYICONDATAW, Shell_NotifyIconW,
                },
                WindowsAndMessaging::{
                    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
                    DestroyIcon, DestroyMenu, DestroyWindow, GetCursorPos, GetSystemMetrics, HICON,
                    MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW, RegisterWindowMessageW,
                    SM_CXSMICON, SetForegroundWindow, SetMenuDefaultItem, TPM_NONOTIFY,
                    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU,
                    WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
                },
            },
        },
        w,
    };

    use super::{Choice, Shown};

    /// The message the icon sends the hidden window with what happened to it.
    const ICON_MESSAGE: u32 = WM_APP + 1;
    /// Keyboard selection, which the shell sends as `NIN_SELECT` and `NIN_KEYSELECT`.
    const NIN_SELECT: u32 = WM_USER;
    const NIN_KEYSELECT: u32 = WM_USER + 1;
    const OPEN: usize = 1;
    const DISCONNECT: usize = 2;
    const QUIT: usize = 3;

    thread_local! {
        /// What the hidden window's procedure needs: the tray runs on the UI
        /// thread, whose message loop winit runs.
        static ICON: RefCell<Option<Icon>> = const { RefCell::new(None) };
        static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    }

    struct Icon {
        data: NOTIFYICONDATAW,
        /// The notification area shows it.
        added: bool,
        /// Without the unread dot, and with it.
        icons: [HICON; 2],
        shown: Shown,
        chosen: fn(Choice),
    }

    pub struct Tray {
        window: HWND,
    }

    impl Tray {
        pub fn start(chosen: fn(Choice)) -> Option<Self> {
            // SAFETY: plain Win32 calls on the UI thread; every pointer passed
            // lives across the call, and the window and icons are released in
            // `drop`.
            unsafe {
                let instance = GetModuleHandleW(std::ptr::null());
                let class = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    hInstance: instance,
                    lpszClassName: w!("YapfastTray"),
                    ..std::mem::zeroed()
                };
                RegisterClassW(&class);
                // A hidden top-level window rather than a message-only one,
                // which wouldn't hear that Explorer restarted.
                let window = CreateWindowExW(
                    0,
                    w!("YapfastTray"),
                    w!("Yapfast"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    instance,
                    std::ptr::null(),
                );
                if window.is_null() {
                    tracing::warn!("could not make the tray icon's window");
                    return None;
                }
                TASKBAR_CREATED.set(RegisterWindowMessageW(w!("TaskbarCreated")));
                let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 256) as u32;
                let icons = [icon(size, false), icon(size, true)];
                let mut data = NOTIFYICONDATAW {
                    cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                    hWnd: window,
                    uID: 1,
                    uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
                    uCallbackMessage: ICON_MESSAGE,
                    hIcon: icons[0],
                    ..std::mem::zeroed()
                };
                for (place, unit) in data.szTip.iter_mut().zip("Yapfast".encode_utf16()) {
                    *place = unit;
                }
                let added = notify(NIM_ADD, &data);
                if !added {
                    tracing::info!("no notification area; closing the window quits Yapfast");
                }
                ICON.set(Some(Icon {
                    data,
                    added,
                    icons,
                    shown: Shown::default(),
                    chosen,
                }));
                Some(Self { window })
            }
        }

        pub fn available(&self) -> bool {
            ICON.with_borrow(|icon| icon.as_ref().is_some_and(|icon| icon.added))
        }

        pub fn show(&self, shown: Shown) {
            let data = ICON.with_borrow_mut(|icon| {
                let icon = icon.as_mut()?;
                icon.shown = shown;
                icon.data.hIcon = icon.icons[usize::from(shown.unread)];
                Some(icon.data)
            });
            if let Some(data) = data {
                notify(NIM_MODIFY, &data);
            }
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            if let Some(icon) = ICON.take() {
                notify(NIM_DELETE, &icon.data);
                for picture in icon.icons {
                    // SAFETY: the pictures are this tray's, and no longer shown.
                    unsafe { DestroyIcon(picture) };
                }
            }
            // SAFETY: the window is this tray's, on this thread.
            unsafe { DestroyWindow(self.window) };
        }
    }

    /// Tells the notification area about the icon. Not while `ICON` is
    /// borrowed: waiting on Explorer, the thread can run the window procedure.
    fn notify(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> bool {
        // SAFETY: the data names this tray's live window and icon.
        unsafe { Shell_NotifyIconW(message, data) != 0 }
    }

    /// The logo as an icon with an alpha channel: colour as BGRA, and an empty
    /// mask, which the alpha replaces.
    fn icon(size: u32, unread: bool) -> HICON {
        let Some(pixels) = super::logo(size, unread) else {
            return std::ptr::null_mut();
        };
        let colour: Vec<u8> = pixels
            .as_slice()
            .iter()
            .flat_map(|pixel| [pixel.b, pixel.g, pixel.r, pixel.a])
            .collect();
        // Rows of the 1-bit mask are padded to 16 bits.
        let mask = vec![0u8; size.div_ceil(16) as usize * 2 * size as usize];
        // SAFETY: both buffers hold `size` rows of `size` pixels.
        unsafe {
            CreateIcon(
                GetModuleHandleW(std::ptr::null()),
                size as i32,
                size as i32,
                1,
                32,
                mask.as_ptr(),
                colour.as_ptr(),
            )
        }
    }

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == ICON_MESSAGE {
            match lparam as u32 {
                WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => choose(Choice::Open),
                WM_RBUTTONUP | WM_CONTEXTMENU => menu(window),
                _ => {}
            }
            return 0;
        }
        // Explorer started again, with an empty notification area.
        if message == TASKBAR_CREATED.get() && message != 0 {
            if let Some(data) = ICON.with_borrow(|icon| icon.as_ref().map(|icon| icon.data)) {
                let added = notify(NIM_ADD, &data);
                ICON.with_borrow_mut(|icon| {
                    if let Some(icon) = icon {
                        icon.added = added;
                    }
                });
            }
            choose(Choice::Available);
            return 0;
        }
        // SAFETY: the default for every other message.
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    fn choose(choice: Choice) {
        if let Some(chosen) = ICON.with_borrow(|icon| icon.as_ref().map(|icon| icon.chosen)) {
            // Not from inside the window procedure, which the menu's own loop runs.
            let _ = slint::invoke_from_event_loop(move || chosen(choice));
        }
    }

    /// The icon's menu, at the pointer.
    fn menu(window: HWND) {
        let call = ICON.with_borrow(|icon| icon.as_ref().is_some_and(|icon| icon.shown.call));
        // SAFETY: the menu is made, shown and destroyed here, for a window of
        // this thread.
        let picked = unsafe {
            let menu = CreatePopupMenu();
            AppendMenuW(menu, MF_STRING, OPEN, w!("Open Yapfast"));
            if call {
                AppendMenuW(menu, MF_STRING, DISCONNECT, w!("Disconnect"));
            }
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, QUIT, w!("Quit"));
            SetMenuDefaultItem(menu, OPEN as u32, 0);
            let mut at = POINT { x: 0, y: 0 };
            GetCursorPos(&mut at);
            // Without this the menu stays open when clicking elsewhere.
            SetForegroundWindow(window);
            let picked = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                at.x,
                at.y,
                0,
                window,
                std::ptr::null(),
            );
            PostMessageW(window, WM_NULL, 0, 0);
            DestroyMenu(menu);
            picked as usize
        };
        match picked {
            OPEN => choose(Choice::Open),
            DISCONNECT => choose(Choice::Disconnect),
            QUIT => choose(Choice::Quit),
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::Cell;

    use image::ImageEncoder as _;
    use objc2::{
        AllocAnyThread, DefinedClass, Encode, MainThreadMarker, MainThreadOnly, define_class, ffi,
        msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationTerminateReply, NSEventModifierFlags, NSImage, NSMenu,
        NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
    };
    use objc2_foundation::{
        NSAppleEventDescriptor, NSAppleEventManager, NSData, NSObject, NSSize, NSString,
    };

    use super::{Choice, Shown};

    /// The icon's side in the menu bar, in points.
    const SIDE: f64 = 18.;

    thread_local! {
        /// What a click on the Dock icon picks; see `reopen_from_dock`.
        static CHOSEN: Cell<Option<fn(Choice)>> = const { Cell::new(None) };
    }

    struct Ivars {
        chosen: fn(Choice),
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Target has
        // no Drop of its own.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "YapfastTrayTarget"]
        #[ivars = Ivars]
        /// What the menu's items call.
        struct Target;

        impl Target {
            #[unsafe(method(open:))]
            fn open(&self, _: Option<&AnyObject>) {
                choose(self.ivars().chosen, Choice::Open);
            }

            #[unsafe(method(disconnect:))]
            fn disconnect(&self, _: Option<&AnyObject>) {
                choose(self.ivars().chosen, Choice::Disconnect);
            }

            #[unsafe(method(quit:))]
            fn quit(&self, _: Option<&AnyObject>) {
                choose(self.ivars().chosen, Choice::Quit);
            }
        }
    );

    /// Runs `chosen` once AppKit is done with the click; false once Slint's
    /// event loop is gone.
    fn choose(chosen: fn(Choice), choice: Choice) -> bool {
        slint::invoke_from_event_loop(move || chosen(choice)).is_ok()
    }

    pub struct Tray {
        item: Retained<NSStatusItem>,
        disconnect: Retained<NSMenuItem>,
        /// Without the unread dot, and with it.
        icons: [Option<Retained<NSImage>>; 2],
        /// Menu items hold their target weakly.
        _target: Retained<Target>,
    }

    impl Tray {
        pub fn start(chosen: fn(Choice)) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let target = Target::alloc(mtm).set_ivars(Ivars { chosen });
            // SAFETY: NSObject's init, on a Target with its ivars set.
            let target: Retained<Target> = unsafe { msg_send![super(target), init] };
            let menu = NSMenu::new(mtm);
            let entry = |title: &str, action: Sel| {
                // SAFETY: Target answers each action, and lives as long as
                // the menu, in the Tray.
                let item = unsafe {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(title),
                        Some(action),
                        &NSString::new(),
                    );
                    item.setTarget(Some(&target));
                    item
                };
                menu.addItem(&item);
                item
            };
            entry("Open Yapfast", sel!(open:));
            let disconnect = entry("Disconnect", sel!(disconnect:));
            disconnect.setHidden(true);
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            entry("Quit Yapfast", sel!(quit:));
            let item =
                NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
            item.setMenu(Some(&menu));
            let icons = [false, true].map(icon);
            if let Some(button) = item.button(mtm) {
                button.setImage(icons[0].as_deref());
                button.setToolTip(Some(&NSString::from_str("Yapfast")));
            }
            teach_delegate(mtm, chosen);
            app_menu(mtm);
            Some(Self {
                item,
                disconnect,
                icons,
                _target: target,
            })
        }

        /// The menu bar always has room for the icon, if only off screen.
        pub fn available(&self) -> bool {
            true
        }

        pub fn show(&self, shown: Shown) {
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            if let Some(button) = self.item.button(mtm) {
                button.setImage(self.icons[usize::from(shown.unread)].as_deref());
            }
            self.disconnect.setHidden(!shown.call);
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }

    /// The logo at twice the menu bar's size, for Retina displays.
    fn icon(unread: bool) -> Option<Retained<NSImage>> {
        let pixels = super::logo((SIDE * 2.) as u32, unread)?;
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ExtendedColorType::Rgba8,
            )
            .ok()?;
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png))?;
        image.setSize(NSSize::new(SIDE, SIDE));
        Some(image)
    }

    /// Makes Yapfast the active app, showing it if hidden, so a window it
    /// opens comes in front of the rest.
    pub fn activate() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        app.unhide(None);
        // `activate`, which replaces it, needs macOS 14.
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }

    /// Hides Yapfast if it's the active app, which started in the menu bar it
    /// can be: winit activates it when no other app is active, as early at
    /// sign-in. The next app then gets the keyboard, rather than a Yapfast
    /// with no window. The menu bar icon stays, and opening the window shows
    /// Yapfast again.
    pub fn step_aside() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        if app.isActive() {
            app.hide(None);
        }
    }

    /// Teaches winit's application delegate two things Mac apps do, which it
    /// doesn't: clicking the Dock icon opens the window, and quitting from
    /// the app menu or the Dock quits as the menu bar icon's Quit does,
    /// leaving a call and keeping what Yapfast saves, rather than ending the
    /// process where it stands.
    fn teach_delegate(mtm: MainThreadMarker, chosen: fn(Choice)) {
        /// Opens the window, whether or not one shows: AppKit counts some
        /// windows that aren't Yapfast's, and opening an open one brings it
        /// forward. Answering no keeps AppKit from restoring a minimized
        /// window again after.
        unsafe extern "C-unwind" fn reopen(_: &AnyObject, _: Sel, _: &AnyObject, _: Bool) -> Bool {
            if let Some(chosen) = CHOSEN.get() {
                choose(chosen, Choice::Open);
            }
            Bool::NO
        }
        /// Cancels AppKit's quitting for Yapfast's own, which ends the
        /// process once done, unless the Mac is logging out, restarting or
        /// shutting down, which cancelling would stop.
        unsafe extern "C-unwind" fn terminate(
            _: &AnyObject,
            _: Sel,
            _: &AnyObject,
        ) -> NSApplicationTerminateReply {
            match CHOSEN.get() {
                Some(chosen) if !powering_off() && choose(chosen, Choice::Quit) => {
                    NSApplicationTerminateReply::TerminateCancel
                }
                _ => NSApplicationTerminateReply::TerminateNow,
            }
        }
        CHOSEN.set(Some(chosen));
        let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
            return;
        };
        let delegate: &AnyObject = (*delegate).as_ref();
        let class = std::ptr::from_ref::<AnyClass>(delegate.class()).cast_mut();
        type Reopen = unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, Bool) -> Bool;
        type Terminate =
            unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> NSApplicationTerminateReply;
        let object = <&AnyObject>::ENCODING;
        let methods = [
            (
                sel!(applicationShouldHandleReopen:hasVisibleWindows:),
                // SAFETY: the same function, with its arguments as the
                // encoding below says.
                unsafe { std::mem::transmute::<Reopen, Imp>(reopen) },
                format!(
                    "{}{object}{}{object}{}\0",
                    Bool::ENCODING,
                    Sel::ENCODING,
                    Bool::ENCODING
                ),
            ),
            (
                sel!(applicationShouldTerminate:),
                // SAFETY: as above.
                unsafe { std::mem::transmute::<Terminate, Imp>(terminate) },
                format!(
                    "{}{object}{}{object}\0",
                    NSApplicationTerminateReply::ENCODING,
                    Sel::ENCODING
                ),
            ),
        ];
        for (selector, method, types) in methods {
            // SAFETY: `types` describes `method`, which AppKit calls with the
            // delegate, the selector and the application, and for reopening
            // whether a window shows. Adding a method winit's class already
            // has does nothing.
            unsafe { ffi::class_addMethod(class, selector, method, types.as_ptr().cast()) };
        }
    }

    /// Gives the menu bar Yapfast's app menu, with Quit, until Slint puts in
    /// its own with the same items. Slint makes that as a window first becomes
    /// active, so after starting at sign-in, with no window, the app menu was
    /// empty and Command-Q did nothing.
    fn app_menu(mtm: MainThreadMarker) {
        let app = NSApplication::sharedApplication(mtm);
        if app.mainMenu().is_some() {
            return;
        }
        let menu = NSMenu::new(mtm);
        let entry = |title: &str, action: Option<Sel>, key: &str| {
            // SAFETY: each action is NSApplication's own, which the menu
            // reaches without a target.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    action,
                    &NSString::from_str(key),
                )
            };
            menu.addItem(&item);
            item
        };
        entry(
            "About Yapfast",
            Some(sel!(orderFrontStandardAboutPanel:)),
            "",
        );
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let services = NSMenu::new(mtm);
        entry("Services", None, "").setSubmenu(Some(&services));
        app.setServicesMenu(Some(&services));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        entry("Hide Yapfast", Some(sel!(hide:)), "h");
        entry("Hide Others", Some(sel!(hideOtherApplications:)), "h").setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command | NSEventModifierFlags::Option,
        );
        entry("Show All", Some(sel!(unhideAllApplications:)), "");
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        entry("Quit Yapfast", Some(sel!(terminate:)), "q");
        let bar = NSMenu::new(mtm);
        let item = NSMenuItem::new(mtm);
        item.setSubmenu(Some(&menu));
        bar.addItem(&item);
        app.setMainMenu(Some(&bar));
    }

    /// Whether the quit AppKit asks about is the Mac logging out, restarting
    /// or shutting down, whose quit event says why.
    fn powering_off() -> bool {
        const REASON: u32 = u32::from_be_bytes(*b"why?");
        NSAppleEventManager::sharedAppleEventManager()
            .currentAppleEvent()
            .is_some_and(|event| {
                // SAFETY: the method takes an AEKeyword, a four-character
                // code, and returns the attribute or nil. objc2's binding
                // needs all of Core Services for its type.
                let reason: Option<Retained<NSAppleEventDescriptor>> =
                    unsafe { msg_send![&*event, attributeDescriptorForKeyword: REASON] };
                reason.is_some()
            })
    }
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod none {
    use super::{Choice, Shown};

    pub struct Tray;

    impl Tray {
        pub fn start(_: fn(Choice)) -> Option<Self> {
            None
        }

        pub fn available(&self) -> bool {
            false
        }

        pub fn show(&self, _: Shown) {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_logo_is_marigold_and_unread_adds_a_red_dot() {
        let plain = super::logo(32, false).unwrap();
        let unread = super::logo(32, true).unwrap();
        assert_eq!((plain.width(), plain.height()), (32, 32));
        let marigold = |pixel: &slint::Rgba8Pixel| {
            pixel.a == 255 && (pixel.r, pixel.g, pixel.b) == (0xff, 0xbf, 0x47)
        };
        let red = |pixel: &slint::Rgba8Pixel| {
            pixel.a == 255 && (pixel.r, pixel.g, pixel.b) == (0xf0, 0x54, 0x5e)
        };
        assert!(plain.as_slice().iter().any(marigold));
        assert!(!plain.as_slice().iter().any(red));
        assert!(unread.as_slice().iter().any(marigold));
        // The dot is in the bottom-right corner.
        let corner = (24 * 32 + 24) as usize;
        assert!(red(&unread.as_slice()[corner]));
    }
}
