//! Desktop notifications. On Linux they go to the freedesktop notification service,
//! which GNOME, KDE and the other desktops run, and on macOS to its Notification
//! Center; clicking one opens its conversation. Elsewhere Yapfast doesn't show
//! them yet.

use yapfast_core::chat::Alert;

/// How a notification shows.
#[derive(Clone, Copy)]
pub struct Style {
    /// Say what the message says, rather than only who wrote.
    pub text: bool,
}

/// Shows notifications from a thread of its own, so a slow notification service never
/// holds up the window.
pub struct Notifier {
    #[cfg(target_os = "linux")]
    requests: std::sync::mpsc::Sender<(Alert, Style)>,
    #[cfg(target_os = "macos")]
    center: Option<macos::Center>,
}

impl Notifier {
    /// Starts showing notifications. `opened` runs on Slint's event loop with the
    /// conversation of a notification someone clicked.
    pub fn start(opened: fn(String)) -> Self {
        #[cfg(target_os = "linux")]
        {
            let (requests, alerts) = std::sync::mpsc::channel();
            let started = std::thread::Builder::new()
                .name("yapfast-notify".into())
                .spawn(move || linux::run(alerts, opened));
            if let Err(error) = started {
                tracing::warn!(%error, "could not start desktop notifications");
            }
            Self { requests }
        }
        #[cfg(target_os = "macos")]
        {
            Self {
                center: macos::Center::start(opened),
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = opened;
            Self {}
        }
    }

    pub fn show(&self, alert: Alert, style: Style) {
        #[cfg(target_os = "linux")]
        let _ = self.requests.send((alert, style));
        #[cfg(target_os = "macos")]
        if let Some(center) = &self.center {
            center.show(&alert, style);
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = (alert, style);
    }
}

/// The body of a notification: the message, or for privacy only that there is one.
/// A call's says only that it's ringing.
fn body(alert: &Alert, style: Style) -> String {
    if style.text || alert.call {
        alert.body.clone()
    } else {
        "New message".into()
    }
}

/// Notification Center hands notifications to the app they come from, so
/// clicking one in it opens the conversation here.
#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use objc2::{
        AllocAnyThread, DefinedClass, define_class, msg_send,
        rc::Retained,
        runtime::{Bool, NSObjectProtocol, ProtocolObject},
    };
    use objc2_foundation::{NSBundle, NSError, NSObject, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use yapfast_core::chat::Alert;

    use super::Style;

    struct Ivars {
        opened: fn(String),
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and Delegate has
        // no Drop of its own.
        #[unsafe(super(NSObject))]
        #[name = "YapfastNotificationDelegate"]
        #[ivars = Ivars]
        /// Shows notifications while Yapfast is in front too, and opens the
        /// conversation of one that's clicked.
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn present(
                &self,
                _: &UNUserNotificationCenter,
                _: &UNNotification,
                show: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                show.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn clicked(
                &self,
                _: &UNUserNotificationCenter,
                response: &UNNotificationResponse,
                done: &block2::DynBlock<dyn Fn()>,
            ) {
                let room = response
                    .notification()
                    .request()
                    .content()
                    .threadIdentifier()
                    .to_string();
                let opened = self.ivars().opened;
                let _ = slint::invoke_from_event_loop(move || opened(room));
                done.call(());
            }
        }
    );

    pub struct Center {
        center: Retained<UNUserNotificationCenter>,
        _delegate: Retained<Delegate>,
    }

    impl Center {
        /// Asks to show notifications, which macOS asks the person once.
        /// `None` outside an app bundle, as from `cargo run`: Notification
        /// Center knows apps by their bundle, and ends a process without one
        /// that asks.
        pub fn start(opened: fn(String)) -> Option<Self> {
            NSBundle::mainBundle().bundleIdentifier()?;
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let delegate = Delegate::alloc().set_ivars(Ivars { opened });
            // SAFETY: NSObject's init, on a Delegate with its ivars set.
            let delegate: Retained<Delegate> = unsafe { msg_send![super(delegate), init] };
            center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert,
                &RcBlock::new(|allowed: Bool, _: *mut NSError| {
                    if !allowed.as_bool() {
                        tracing::info!("notifications are turned off for Yapfast");
                    }
                }),
            );
            Some(Self {
                center,
                _delegate: delegate,
            })
        }

        pub fn show(&self, alert: &Alert, style: Style) {
            let content = UNMutableNotificationContent::new();
            content.setTitle(&NSString::from_str(&alert.title));
            content.setBody(&NSString::from_str(&super::body(alert, style)));
            // Groups a conversation's notifications, and says which to open.
            // Yapfast plays its own sounds, so it asks for none.
            content.setThreadIdentifier(&NSString::from_str(&alert.room));
            // Each its own, so none replaces another.
            let made = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
                &NSString::from_str(&made.to_string()),
                &content,
                None,
            );
            self.center.addNotificationRequest_withCompletionHandler(
                &request,
                Some(&RcBlock::new(|error: *mut NSError| {
                    if !error.is_null() {
                        tracing::warn!("could not show a notification");
                    }
                })),
            );
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex, mpsc::Receiver},
    };

    use yapfast_core::chat::Alert;
    use zbus::{
        blocking::{Connection, MessageIterator},
        zvariant::{StructureBuilder, Value},
    };

    use super::Style;

    const SERVICE: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";

    /// Shows each alert until the window closes, and opens the conversation of one
    /// that's clicked.
    pub fn run(alerts: Receiver<(Alert, Style)>, opened: fn(String)) {
        let connection = match Connection::session() {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "no session bus for desktop notifications");
                return;
            }
        };
        let capabilities: Vec<String> = connection
            .call_method(Some(SERVICE), PATH, Some(SERVICE), "GetCapabilities", &())
            .and_then(|reply| reply.body().deserialize())
            .unwrap_or_default();
        let markup = capabilities
            .iter()
            .any(|capability| capability == "body-markup");
        // Which conversation each notification shown is from, by its id.
        let shown = Arc::new(Mutex::new(HashMap::<u32, String>::new()));
        let clicks = connection.clone();
        let rooms = shown.clone();
        let listening = std::thread::Builder::new()
            .name("yapfast-notify-clicks".into())
            .spawn(move || listen(&clicks, &rooms, opened));
        if let Err(error) = listening {
            tracing::warn!(%error, "notifications can't open their conversations");
        }
        for (alert, style) in alerts {
            let mut body = super::body(&alert, style);
            if markup {
                body = escape(&body);
            }
            let mut hints = HashMap::<&str, Value>::new();
            hints.insert("category", Value::from("im.received"));
            // Yapfast plays its own sounds, and rings for a call itself.
            hints.insert("suppress-sound", Value::from(true));
            // The server finds Yapfast's name and icon through its desktop entry.
            hints.insert("desktop-entry", Value::from(crate::icon::APP_ID));
            if let Some(picture) = &alert.picture
                && let Some(image) = image_data(picture)
            {
                hints.insert("image-data", image);
            }
            let actions = ["default", "Open"];
            let request = (
                "Yapfast",
                0u32,
                "",
                alert.title.as_str(),
                body.as_str(),
                &actions[..],
                hints,
                -1i32,
            );
            let id = connection
                .call_method(Some(SERVICE), PATH, Some(SERVICE), "Notify", &request)
                .and_then(|reply| reply.body().deserialize::<u32>());
            match id {
                Ok(id) => {
                    shown
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .insert(id, alert.room);
                }
                Err(error) => tracing::warn!(%error, "could not show a notification"),
            }
        }
    }

    /// Opens the conversation of a notification that's clicked, and forgets those closed.
    fn listen(connection: &Connection, shown: &Mutex<HashMap<u32, String>>, opened: fn(String)) {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface(SERVICE)
            .and_then(|rule| rule.path(PATH))
            .map(|rule| rule.build());
        let messages =
            rule.and_then(|rule| MessageIterator::for_match_rule(rule, connection, Some(64)));
        let messages = match messages {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(%error, "notifications can't open their conversations");
                return;
            }
        };
        for message in messages.flatten() {
            let header = message.header();
            let shown = || {
                shown
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
            };
            match header.member().map(|member| member.as_str()) {
                Some("ActionInvoked") => {
                    let Ok((id, action)) = message.body().deserialize::<(u32, String)>() else {
                        continue;
                    };
                    let room = shown().get(&id).cloned();
                    if let (Some(room), "default") = (room, action.as_str()) {
                        let _ = slint::invoke_from_event_loop(move || opened(room));
                    }
                }
                Some("NotificationClosed") => {
                    if let Ok((id, _reason)) = message.body().deserialize::<(u32, u32)>() {
                        shown().remove(&id);
                    }
                }
                _ => {}
            }
        }
    }

    /// The author's picture, as the specification's `(iiibiiay)` image.
    fn image_data(picture: &yapfast_core::chat::Picture) -> Option<Value<'static>> {
        let width = i32::try_from(picture.width).ok()?;
        let height = i32::try_from(picture.height).ok()?;
        let structure = StructureBuilder::new()
            .add_field(width)
            .add_field(height)
            .add_field(width.checked_mul(4)?)
            .add_field(true)
            .add_field(8i32)
            .add_field(4i32)
            .add_field(picture.rgba.to_vec())
            .build()
            .ok()?;
        Some(Value::from(structure))
    }

    /// Text for a service that reads the body as markup.
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
}
