//! The entry that starts Yapfast at sign-in, as Settings show and change it.
//! On macOS each read asks Background Task Management, and turning it on
//! waits for its answer, so a thread of its own reads and writes the entry,
//! in the order asked, and only its answers come back to the UI thread.

use std::sync::mpsc;

use super::*;

/// What Settings, or a start, ask of the entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SignIn {
    /// Read it: Settings opened, or came to the front again, and the person
    /// may have changed it in Task Manager or System Settings meanwhile.
    Read,
    /// Point it at this Yapfast if it starts another, at start.
    Refresh,
    /// Turn it on or off, as the person flipped the switch.
    Turn(bool),
}

/// A piece of work for the entry's thread: what to do, how many times the
/// switch had turned when it was asked, so an answer a later turn overtook
/// changes nothing, and the window to show the answer in.
struct Work {
    what: SignIn,
    turns: u64,
    window: slint::Weak<YapfastWindow>,
}

thread_local! {
    /// Feeds the entry's thread, started on first use.
    static WORK: RefCell<Option<mpsc::Sender<Work>>> = const { RefCell::new(None) };
    /// How many times the switch has turned.
    static TURNS: Cell<u64> = const { Cell::new(0) };
    /// Whether Yapfast starts at sign-in, as last read, for a window made
    /// after.
    pub(super) static STARTS_AT_SIGN_IN: Cell<bool> = const { Cell::new(false) };
}

/// Asks the entry's thread to do `what`, then to show whether Yapfast starts
/// at sign-in.
pub(super) fn sign_in_entry(window: &YapfastWindow, what: SignIn) {
    if !AUTOSTART.get() {
        return;
    }
    if let SignIn::Turn(_) = what {
        TURNS.set(TURNS.get() + 1);
    }
    let work = Work {
        what,
        turns: TURNS.get(),
        window: window.as_weak(),
    };
    WORK.with_borrow_mut(|sender| {
        if sender.is_none() {
            *sender = start();
        }
        if !sender
            .as_ref()
            .is_some_and(|sender| sender.send(work).is_ok())
        {
            tracing::warn!("could not reach starting at sign-in");
        }
    });
}

/// Starts the entry's thread, which waits for work as long as Yapfast runs.
fn start() -> Option<mpsc::Sender<Work>> {
    let (sender, receiver) = mpsc::channel();
    let started = std::thread::Builder::new()
        .name("yapfast-autostart".into())
        .spawn(move || run(&receiver));
    match started {
        Ok(_) => Some(sender),
        Err(error) => {
            tracing::warn!(%error, "could not start reading starting at sign-in");
            None
        }
    }
}

fn run(receiver: &mpsc::Receiver<Work>) {
    while let Ok(mut work) = receiver.recv() {
        // Clicks can come faster than macOS answers, and only where the
        // switch was left counts.
        while let Ok(later) = receiver.try_recv() {
            work = latest(work, later);
        }
        let turned = match work.what {
            SignIn::Read => None,
            SignIn::Refresh => {
                autostart::refresh();
                None
            }
            SignIn::Turn(on) => Some(
                autostart::set(on)
                    .map(|()| on)
                    .map_err(|error| error.to_string()),
            ),
        };
        let on = autostart::enabled();
        let Work { turns, window, .. } = work;
        let _ = slint::invoke_from_event_loop(move || show(turns, turned, on, &window));
    }
}

/// Of two pieces of work waiting, the one that stands for both: a turn
/// reads after, as pointing it at this Yapfast does, and a later turn
/// replaces an earlier one.
fn latest(earlier: Work, later: Work) -> Work {
    let what = match (earlier.what, later.what) {
        (what @ (SignIn::Turn(_) | SignIn::Refresh), SignIn::Read) => what,
        (what @ SignIn::Turn(_), SignIn::Refresh) => what,
        (_, what) => what,
    };
    Work { what, ..later }
}

/// Shows what the entry's thread found, unless a later turn of the switch
/// overtook it: a turn's result, with what went wrong under the switch, and
/// whether Yapfast starts at sign-in now.
fn show(
    turns: u64,
    turned: Option<Result<bool, String>>,
    on: bool,
    window: &slint::Weak<YapfastWindow>,
) {
    if turns != TURNS.get() {
        return;
    }
    let problem = match turned {
        // Off in System Settings, where only the person can turn it on.
        Some(Ok(true)) if !on => {
            tracing::info!("starting at sign-in is off in System Settings");
            #[cfg(target_os = "macos")]
            autostart::open_login_items();
            Some(
                "Turned off in System Settings, under Login Items, where you can turn it on."
                    .to_owned(),
            )
        }
        Some(Ok(start)) => {
            tracing::info!(start, "starting at sign-in changed");
            Some(String::new())
        }
        Some(Err(error)) => {
            tracing::warn!(%error, "could not change starting at sign-in");
            Some(error)
        }
        None => None,
    };
    STARTS_AT_SIGN_IN.set(on);
    #[cfg(target_os = "macos")]
    autostart::reopen_at_sign_in(!on);
    if let Some(window) = window.upgrade() {
        window.set_start_on_sign_in(on);
        if let Some(problem) = problem {
            window.set_start_on_sign_in_problem(problem.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work(what: SignIn, turns: u64) -> Work {
        Work {
            what,
            turns,
            window: slint::Weak::default(),
        }
    }

    #[test]
    fn work_waiting_comes_down_to_where_the_switch_was_left() {
        let merged = |pieces: &[(SignIn, u64)]| {
            let mut pieces = pieces.iter().map(|&(what, turns)| work(what, turns));
            let first = pieces.next().unwrap();
            let last = pieces.fold(first, latest);
            (last.what, last.turns)
        };
        let (on, off) = (SignIn::Turn(true), SignIn::Turn(false));
        assert_eq!(merged(&[(on, 1), (off, 2), (on, 3)]), (on, 3));
        assert_eq!(merged(&[(on, 1), (off, 2)]), (off, 2));
        // A turn reads after, so a read waiting behind it adds nothing.
        assert_eq!(merged(&[(on, 1), (SignIn::Read, 1)]), (on, 1));
        assert_eq!(merged(&[(SignIn::Read, 0), (off, 1)]), (off, 1));
        assert_eq!(
            merged(&[(SignIn::Refresh, 0), (SignIn::Read, 0)]),
            (SignIn::Refresh, 0)
        );
    }

    #[test]
    fn an_answer_a_later_turn_overtook_changes_nothing() {
        TURNS.set(2);
        STARTS_AT_SIGN_IN.set(false);
        let window = slint::Weak::default();
        show(1, Some(Ok(true)), true, &window);
        assert!(
            !STARTS_AT_SIGN_IN.get(),
            "the first turn's answer came late"
        );
        show(2, Some(Ok(true)), true, &window);
        assert!(STARTS_AT_SIGN_IN.get());
    }
}
