use super::*;

fn message_row_plain(message: &ChatMessage) -> MessageRow {
    message_row(message, &HashSet::new())
}

#[cfg(feature = "discord")]
#[test]
fn statuses_show_their_custom_emoji_as_pictures() {
    let (words, emoji) = activity_line(Some("<:pog:5>  back\nsoon"));
    assert_eq!(words, "back soon");
    assert!(emoji.ends_with("/emojis/5.png?size=64"), "{emoji}");
    let (words, emoji) = activity_line(Some("☕ brb"));
    assert_eq!((words.as_str(), emoji.as_str()), ("☕ brb", ""));

    // The account's own status.
    let mut status = yapfast_core::chat::CustomStatus {
        text: "Shipping it".into(),
        emoji: Some("pog".into()),
        emoji_id: Some("5".into()),
        expires_at_ms: None,
    };
    let (text, picture) = status_line(&status);
    assert_eq!(text, "Shipping it");
    assert!(picture.is_some_and(|url| url.ends_with("/emojis/5.png?size=64")));
    status.emoji = Some("🍅".into());
    status.emoji_id = None;
    assert_eq!(status_line(&status), ("🍅 Shipping it".into(), None));
}

#[test]
fn equal_lists_stay_so_rows_count_as_unchanged() {
    let shown = ModelRc::new(VecModel::from(vec![1, 2]));
    let kept = same_list(&shown, ModelRc::new(VecModel::from(vec![1, 2])));
    assert!(kept == shown, "the list on screen stays");
    let changed = same_list(&shown, ModelRc::new(VecModel::from(vec![1, 3])));
    assert!(changed != shown);
    assert_eq!(changed.row_data(1), Some(3));

    // A profile built again from the same view isn't set again.
    let view = yapfast_core::application::ProfileView {
        profile: yapfast_core::chat::Profile {
            id: "@sam:test".into(),
            name: "Sam".into(),
            servers: vec![yapfast_core::chat::ProfileServer {
                id: "s".into(),
                name: "Server".into(),
                icon: None,
                banner: None,
                members: Some(4),
                channels: 2,
                nick: None,
            }],
            roles: vec![ProfileRole {
                name: "Mod".into(),
                color: Some(0xff0000),
            }],
            ..Default::default()
        },
        ..Default::default()
    };
    let state = yapfast_core::application::AppState::default();
    let mut pictures = HashMap::new();
    let shown = profile_data(&mut pictures, &state, &view);
    let again = profile_data(&mut pictures, &state, &view);
    assert!(changed_profile(&shown, again).is_none());
    let mut renamed = view.clone();
    renamed.profile.name = "Samuel".into();
    let changed = changed_profile(&shown, profile_data(&mut pictures, &state, &renamed))
        .expect("a new name shows");
    assert!(changed.servers == shown.servers, "its lists stay");
}

#[test]
fn blank_lines_between_lines_show() {
    assert_eq!(keep_blank_lines("a\n\nb"), "a\n\u{a0}\nb");
    assert_eq!(keep_blank_lines("a\n\n\nb"), "a\n\u{a0}\n\u{a0}\nb");
    assert_eq!(keep_blank_lines("a\nb"), "a\nb");
    assert_eq!(
        keep_blank_lines("\n\na\n\n"),
        "\n\na\n\n",
        "only between lines"
    );
    assert_eq!(
        keep_blank_lines("a\n\n```\nx\n\ny\n```\n\nb"),
        "a\n\u{a0}\n```\nx\n\ny\n```\n\u{a0}\nb",
        "code keeps its lines"
    );
    // A blank line shows as a line of its own, unlike a line break.
    assert_ne!(markdown("one\n\ntwo"), markdown("one\ntwo"));
}

#[test]
fn the_composer_sends_on_enter_and_breaks_lines_on_shift_enter() {
    use slint::platform::{Key, WindowEvent};
    let window = one_message("Hi");
    let sent = Rc::new(Cell::new(0));
    let count = sent.clone();
    window.on_send(move || count.set(count.get() + 1));
    window.invoke_focus_composer();
    let press = |text: SharedString| {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    };
    press("a".into());
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Shift.into(),
    });
    press(Key::Return.into());
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Shift.into(),
    });
    press("b".into());
    assert_eq!(window.get_draft(), "a\nb");
    assert_eq!(sent.get(), 0);
    press(Key::Return.into());
    assert_eq!(sent.get(), 1);
    assert_eq!(
        window.get_draft(),
        "a\nb",
        "sending is the application's to clear"
    );
}

#[test]
fn the_composers_menu_cuts_copies_and_pastes_through_the_application() {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    let window = one_message("Hi");
    let pastes = Rc::new(Cell::new(0));
    let count = pastes.clone();
    let weak = window.as_weak();
    window.on_paste_into_composer(move || {
        count.set(count.get() + 1);
        weak.unwrap().invoke_paste_composer_text();
    });
    window.invoke_focus_composer();
    for letter in "hello".chars() {
        let text: SharedString = letter.into();
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
    }
    let press = |x, y, button| {
        let position = slint::LogicalPosition::new(x, y);
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        settle(&window);
    };
    // The menu opens above the pointer, its rows 34px apart, the last
    // centred 22px above it: Select All, then Paste, Copy and Cut.
    let (x, y) = (600., 720.);
    let choose = |row: f32| {
        press(x, y, PointerEventButton::Right);
        press(x + 40., y - 22. - 34. * row, PointerEventButton::Left);
    };
    choose(0.);
    choose(3.);
    assert_eq!(window.get_draft(), "", "Select All, then Cut");
    choose(1.);
    assert_eq!(pastes.get(), 1, "Paste goes through the application");
    assert_eq!(window.get_draft(), "hello");
    choose(0.);
    choose(2.);
    // To the end: End, or on the Mac, where End scrolls, Command-Right, which
    // is Slint's Control there.
    let to_end: &[Key] = if cfg!(target_os = "macos") {
        &[Key::Control, Key::RightArrow]
    } else {
        &[Key::End]
    };
    for key in to_end {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: (*key).into(),
        });
    }
    for key in to_end.iter().rev() {
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: (*key).into(),
        });
    }
    choose(1.);
    assert_eq!(window.get_draft(), "hellohello", "Select All, then Copy");
    // Without a selection there is nothing to cut: the rows are Paste and
    // Select All, so the third from the bottom is outside the menu.
    choose(2.);
    assert_eq!(window.get_draft(), "hellohello");
}

#[cfg(feature = "discord")]
#[test]
fn files_dropped_on_the_window_stage_or_say_why_not() {
    let (window, adapter, folder) = discord_server_window();
    adapter
        .borrow_mut()
        .app
        .dispatch(Action::Navigate(Navigation::Server("discord:10".into())));
    adapter
        .borrow_mut()
        .action(&window, Action::SelectRoom("discord:10:11".into()));
    // As once its history has come.
    adapter.borrow_mut().app.state.room_ready = true;
    let file = |name: &str, size: u64| {
        let path = folder.path().join(name);
        std::fs::File::create(&path).unwrap().set_len(size).unwrap();
        path
    };
    let drag = |drag| adapter.borrow_mut().drag(&window, drag);
    drag(Drag::Entered);
    assert!(window.get_dragging());
    assert_eq!(window.get_drop_destination(), "#general");
    drag(Drag::Left);
    assert!(!window.get_dragging());
    drag(Drag::Entered);
    drag(Drag::Dropped(vec![
        file("cat.png", 3),
        file("notes.txt", 5),
    ]));
    assert!(!window.get_dragging());
    assert_eq!(window.get_drop_refusal(), "");
    assert_eq!(adapter.borrow().app.state.staged().len(), 2);
    // Two waiting and nine more is too many, and none of them go.
    let many: Vec<_> = (0..9)
        .map(|index| file(&format!("{index}.txt"), 1))
        .collect();
    drag(Drag::Entered);
    drag(Drag::Dropped(many));
    assert_eq!(window.get_drop_refusal(), "Too many uploads!");
    assert_eq!(
        window.get_drop_refusal_help(),
        "You can only upload 10 files at a time!"
    );
    assert_eq!(adapter.borrow().app.state.staged().len(), 2);
    // The next drag puts the refusal away.
    drag(Drag::Entered);
    assert_eq!(window.get_drop_refusal(), "");
    // Without Nitro, in a server without boosts: 20 MiB.
    drag(Drag::Dropped(vec![file("big.mov", 21 * 1024 * 1024)]));
    assert_eq!(window.get_drop_refusal(), "Your files are too powerful");
    assert_eq!(
        window.get_drop_refusal_help(),
        "Max file size is 20.00 MB please."
    );
    drag(Drag::Entered);
    drag(Drag::Dropped(vec![folder.path().into()]));
    assert_eq!(window.get_drop_refusal(), "Invalid file type");
    assert_eq!(adapter.borrow().app.state.staged().len(), 2);
}

#[test]
fn the_keyboard_steps_through_messages_and_acts_on_them() {
    use slint::platform::{Key, WindowEvent};
    let window = showing(
        ["One", "Two", "Three"]
            .into_iter()
            .map(|body| ChatMessage {
                author: "Sam".into(),
                body: body.into(),
                can_reply: true,
                ..Default::default()
            })
            .collect(),
    );
    let replied = Rc::new(RefCell::new(String::new()));
    let reply = replied.clone();
    window.on_reply(move |item| *reply.borrow_mut() = item.into());
    let edits = Rc::new(Cell::new(0));
    let count = edits.clone();
    window.on_edit_last(move || count.set(count.get() + 1));
    let key = |text: SharedString, pressed: bool| {
        window.window().dispatch_event(if pressed {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    };
    let press = |text: SharedString| {
        key(text.clone(), true);
        key(text, false);
    };
    window.invoke_focus_composer();
    // Up in the empty composer edits; Alt+Up goes to the newest message.
    press(Key::UpArrow.into());
    assert_eq!(edits.get(), 1);
    key(Key::Alt.into(), true);
    press(Key::UpArrow.into());
    key(Key::Alt.into(), false);
    assert_eq!(window.get_active_item(), "i2");
    press(Key::UpArrow.into());
    press(Key::UpArrow.into());
    press(Key::UpArrow.into());
    assert_eq!(window.get_active_item(), "i0", "the first stays");
    press(Key::DownArrow.into());
    assert_eq!(window.get_active_item(), "i1");
    press("r".into());
    assert_eq!(*replied.borrow(), "i1");
    // The reply goes back to the composer, so letters type again.
    press("x".into());
    assert_eq!(window.get_draft(), "x");
}

#[test]
fn preferences_set_the_clock_text_size_previews_and_unmuting() {
    i_slint_backend_testing::init_no_event_loop();
    let message = ChatMessage {
        timestamp: 1_791_329_400,
        time: "23:30".into(),
        embeds: vec![yapfast_core::chat::Embed::default()],
        ..Default::default()
    };
    // Missing preferences take their defaults: 24 hours, default text, previews.
    let defaults: Layout = serde_json::from_str("{}").unwrap();
    assert_eq!(
        defaults.noise_suppression,
        yapfast_core::noise::Suppressor::DeepFilterNet
    );
    let row = defaults.restyle(&message, message_row(&message, &HashSet::new()));
    assert_eq!(
        row.time,
        yapfast_core::clock::time_on(message.timestamp, false)
    );
    assert_eq!(row.embeds.row_count(), 1);
    let layout = Layout {
        twelve_hour_clock: true,
        text_size: Some(2),
        link_previews: Some(false),
        unmute_on_join: Some(true),
        noise_suppression: yapfast_core::noise::Suppressor::RNNoise,
        noise_strength: yapfast_core::noise::Strength::Light,
        input_sensitivity: Some(-45),
        reduce_room_echo: true,
        ..Default::default()
    };
    let row = layout.restyle(&message, message_row(&message, &HashSet::new()));
    let time = yapfast_core::clock::time_on(message.timestamp, true);
    assert!(time.ends_with(" AM") || time.ends_with(" PM"));
    assert_eq!(row.time, time);
    assert_eq!(row.time_styled, slint::StyledText::from_plain_text(&time));
    assert_eq!(row.short_time, time.split(' ').next().unwrap());
    assert_eq!(row.embeds.row_count(), 0);
    // A message that is only a card says it's hidden; one with text doesn't.
    assert_eq!(
        row.hidden_card,
        "A card, hidden while link previews are off."
    );
    let titled = ChatMessage {
        embeds: vec![yapfast_core::chat::Embed {
            title: "Server status".into(),
            ..Default::default()
        }],
        ..message.clone()
    };
    assert!(
        layout
            .restyle(&titled, message_row(&titled, &HashSet::new()))
            .hidden_card
            .contains("\"Server status\"")
    );
    let with_text = ChatMessage {
        body: "Look".into(),
        ..message.clone()
    };
    assert!(
        layout
            .restyle(&with_text, message_row(&with_text, &HashSet::new()))
            .hidden_card
            .is_empty()
    );

    let window = YapfastWindow::new().unwrap();
    layout.apply(&window);
    assert!(window.get_twelve_hour_clock() && !window.get_link_previews());
    // Discord calls suppress noise with the chosen network, as far as chosen.
    assert_eq!(window.get_noise_suppression(), 1);
    assert_eq!(window.get_noise_strength(), 2);
    assert_eq!(
        yapfast_core::noise::chosen(),
        yapfast_core::noise::Suppressor::RNNoise
    );
    let saved = serde_json::to_value(&layout).unwrap();
    assert_eq!(saved["noise_suppression"], "rnnoise");
    assert_eq!(saved["noise_strength"], "light");
    assert_eq!(window.get_sensitivity(), -45);
    assert!(window.get_dereverb() && !defaults.reduce_room_echo);
    assert_eq!(defaults.sensitivity(), -60);
    let out_of_range = Layout {
        input_sensitivity: Some(12),
        ..Default::default()
    };
    assert_eq!(out_of_range.sensitivity(), 0);
    // The third choice is Off; an index out of range is the default.
    assert_eq!(
        chosen(&SUPPRESSORS, 2),
        yapfast_core::noise::Suppressor::Off
    );
    assert_eq!(chosen(&STRENGTHS, -1), yapfast_core::noise::Strength::Full);
    assert_eq!(window.get_message_font_size(), 18.);
    // Joining a call turns the microphone on once it connects.
    let unmuted = Rc::new(Cell::new(false));
    let record = unmuted.clone();
    window.on_join_unmuted(move || record.set(true));
    window.set_muted(true);
    window.set_voice_ready(true);
    slint::platform::update_timers_and_animations();
    assert!(unmuted.get());
    // Deafening on joining wins over unmuting.
    let (unmuted, deafened) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
    let record = unmuted.clone();
    window.on_join_unmuted(move || record.set(true));
    let record = deafened.clone();
    window.on_join_deafened(move || record.set(true));
    window.set_voice_ready(false);
    slint::platform::update_timers_and_animations();
    window.set_deafen_on_join(true);
    window.set_voice_ready(true);
    slint::platform::update_timers_and_animations();
    assert!(deafened.get() && !unmuted.get());
}

#[test]
fn starting_in_the_tray_needs_something_to_show_there() {
    let never = || -> bool { panic!("the credential store is read only when it matters") };
    assert!(!starts_hidden(false, false, never));
    assert!(!starts_hidden(false, true, never));
    assert!(starts_hidden(true, true, never));
    assert!(starts_hidden(true, false, || true));
    assert!(!starts_hidden(true, false, || false));
}

#[test]
fn each_sounds_own_volume_counts_only_while_adjusting_each() {
    i_slint_backend_testing::init_no_event_loop();
    let defaults: Layout = serde_json::from_str("{}").unwrap();
    assert_eq!(defaults.sound_set, SoundSet::NewKalimba);
    assert_eq!(defaults.sound_gain(Sound::Join), 1.0);
    let mut layout = Layout {
        sound_set: SoundSet::Rhodes,
        sound_volume: Some(50),
        each_sound: [(Sound::Mute, 50)].into(),
        ..Default::default()
    };
    assert!((layout.sound_gain(Sound::Mute) - 0.25).abs() < 1e-6);
    layout.adjust_each_sound = true;
    assert!((layout.sound_gain(Sound::Mute) - 0.0625).abs() < 1e-6);
    assert!((layout.sound_gain(Sound::Join) - 0.25).abs() < 1e-6);

    let window = YapfastWindow::new().unwrap();
    layout.apply(&window);
    assert_eq!(window.get_sound_set(), 3);
    assert_eq!(window.get_sound_volume(), 50);
    assert!(window.get_adjust_each_sound());
    let each: Vec<i32> = window.get_each_sound().iter().collect();
    assert_eq!(each, [100, 100, 50, 100, 100, 100, 100, 100, 100, 100]);
    let saved = serde_json::to_value(&layout).unwrap();
    assert_eq!(saved["sound_set"], "rhodes");
    assert_eq!(saved["each_sound"]["mute"], 50);

    // Someone who turned off the desktop's sound for notifications keeps
    // messages quiet.
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(LAYOUT),
        r#"{"notification_sound":false}"#,
    )
    .unwrap();
    let read = Layout::read(directory.path());
    assert_eq!(read.sound_gain(Sound::Message), 0.0);
    assert_eq!(read.sound_gain(Sound::Mention), 0.0);
    assert!(read.sound_gain(Sound::Join) > 0.0);
    let saved = serde_json::to_value(&read).unwrap();
    assert!(saved.get("notification_sound").is_none());
}

#[test]
fn the_renderer_setting_is_remembered() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    // Software unless chosen.
    assert!(!window.get_gpu_rendering());
    window.set_gpu_rendering(true);
    window.invoke_preferences_changed();
    assert!(Layout::read(directory.path()).gpu_rendering);
}

#[test]
fn sound_settings_are_remembered() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.set_sound_set(4);
    window.set_sound_volume(40);
    window.set_adjust_each_sound(true);
    window.invoke_preferences_changed();
    window.invoke_each_sound_changed(2, 30);
    window.invoke_each_sound_changed(6, 0);
    let read = Layout::read(directory.path());
    assert_eq!(read.sound_set, SoundSet::Glow);
    assert_eq!(read.sound_volume, Some(40));
    assert!(read.adjust_each_sound);
    assert_eq!(read.each_sound(Sound::Mute), 30);
    assert_eq!(read.sound_gain(Sound::Message), 0.0);
    let each: Vec<i32> = window.get_each_sound().iter().collect();
    assert_eq!(each[2], 30);
    // Back at 100%, a sound's own volume is forgotten.
    window.invoke_each_sound_changed(2, 100);
    assert!(
        !Layout::read(directory.path())
            .each_sound
            .contains_key(&Sound::Mute)
    );
}

#[test]
fn embed_fields_share_rows_three_inline_at_a_time() {
    let field = |name: &str, inline| EmbedField {
        name: name.into(),
        value: String::new(),
        inline,
    };
    let rows = field_rows(&[
        field("a", true),
        field("b", true),
        field("c", true),
        field("d", true),
        field("e", false),
        field("f", true),
    ]);
    let names: Vec<Vec<slint::StyledText>> = rows
        .iter()
        .map(|row| row.fields.iter().map(|field| field.name).collect())
        .collect();
    let expected: Vec<Vec<slint::StyledText>> = [&["a", "b", "c"][..], &["d"], &["e"], &["f"]]
        .iter()
        .map(|row| row.iter().map(|name| bold(name)).collect())
        .collect();
    assert_eq!(names, expected);
}

/// A window showing one styled message from Sam, with the composer.
fn one_message(body: &str) -> YapfastWindow {
    messages(&[("Sam", body)])
}

/// A window showing styled messages, each starting a group, with the composer.
fn messages(lines: &[(&str, &str)]) -> YapfastWindow {
    showing(
        lines
            .iter()
            .map(|(author, body)| ChatMessage {
                author: (*author).into(),
                body: (*body).into(),
                formatted: true,
                ..Default::default()
            })
            .collect(),
    )
}

/// A window showing these messages, each starting a group and sent at 19:2 and its
/// index, with the composer.
fn showing(chat: Vec<ChatMessage>) -> YapfastWindow {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    copy_whole_messages(&window);
    // Without the adapter's clipboard reading, Ctrl+V pastes text, as it
    // does when the clipboard holds no files or picture.
    let weak = window.as_weak();
    window.on_paste_into_composer(move || weak.unwrap().invoke_paste_composer_text());
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    window.set_composer_enabled(true);
    // A device that may write, so the verification banner stays away.
    window.set_trusted(true);
    let rows: Vec<MessageRow> = chat
        .into_iter()
        .enumerate()
        .map(|(index, message)| {
            message_row_plain(&ChatMessage {
                id: format!("m{index}"),
                item: format!("i{index}"),
                time: format!("19:2{index}"),
                ..message
            })
        })
        .collect();
    window.set_messages(ModelRc::new(VecModel::from(rows)));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    settle(&window);
    window
}

fn settle(window: &YapfastWindow) {
    for _ in 0..5 {
        slint::platform::update_timers_and_animations();
        let _ = window.invoke_timeline_extent();
    }
}

/// For each height up the window, presses there beside the text, drags up past the top
/// of the list, turning the wheel `wheel` times on the way, lets go and presses Ctrl+C,
/// until a copy changes the clipboard, which it returns. The first drag to start on a
/// message starts at the end of the lowest, so it selects up to the end of that message.
/// The list returns to the newest message before each.
fn copy_upwards(window: &YapfastWindow, wheel: usize) -> Option<String> {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    let dispatch = |event| window.window().dispatch_event(event);
    let at = |x, y| slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    // The composer's single line would turn line breaks into spaces, so
    // read the clipboard itself.
    let clipboard = || {
        i_slint_core::window::WindowInner::from_pub(window.window())
            .context()
            .platform()
            .clipboard_text(i_slint_core::platform::Clipboard::DefaultClipboard)
    };
    let before = clipboard();
    for y in (100..700).step_by(4).rev() {
        window.invoke_scroll_to_latest();
        settle(window);
        let y = y as f32;
        dispatch(WindowEvent::PointerMoved {
            position: at(900., y),
        });
        dispatch(WindowEvent::PointerPressed {
            position: at(900., y),
            button,
        });
        dispatch(WindowEvent::PointerMoved {
            position: at(900., y - 8.),
        });
        dispatch(WindowEvent::PointerMoved {
            position: at(900., 60.),
        });
        for _ in 0..wheel {
            dispatch(WindowEvent::PointerScrolled {
                position: at(900., 60.),
                delta_x: 0.,
                delta_y: 400.,
            });
            settle(window);
        }
        dispatch(WindowEvent::PointerReleased {
            position: at(900., 60.),
            button,
        });
        dispatch(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        dispatch(WindowEvent::KeyPressed { text: "c".into() });
        dispatch(WindowEvent::KeyReleased { text: "c".into() });
        dispatch(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        let copied = clipboard();
        if copied != before {
            return copied;
        }
    }
    before
}

#[test]
fn a_selection_runs_across_messages_with_their_names_and_times() {
    let window = messages(&[
        ("Sam", "Thinking **co-op** after nine."),
        ("Riley", "Yep. I should be free after dinner."),
    ]);
    assert_eq!(
        copy_upwards(&window, 0).as_deref(),
        Some(
            "Sam 19:20\nThinking co-op after nine.\nRiley 19:21\nYep. I should be free after dinner."
        )
    );
}

#[test]
fn pictures_on_their_way_hold_their_place_as_skeletons() {
    let attachment = |name: &str, preview_failed| yapfast_core::chat::Attachment {
        name: name.into(),
        size: Some(1000),
        image: true,
        dimensions: Some((1600, 900)),
        preview: None,
        preview_failed,
        placement: Placement::Message,
        gif: None,
        spoiler: false,
        video: false,
        url: None,
    };
    let row = message_row_plain(&ChatMessage {
        attachments: vec![attachment("wide.png", false)],
        ..Default::default()
    });
    let cell = row.images.row_data(0).unwrap();
    assert!(cell.loading);
    // As the 360 by 270 preview will be: 1600 by 900 scaled to fit.
    assert_eq!((row.gallery_width, row.gallery_height), (360, 203));
    assert_eq!(row.files.row_count(), 0, "not a file card meanwhile");
    // One whose preview failed is a file card instead.
    let row = message_row_plain(&ChatMessage {
        attachments: vec![attachment("broken.png", true)],
        ..Default::default()
    });
    assert_eq!(row.images.row_count(), 0);
    assert_eq!(row.files.row_count(), 1);
}

#[test]
fn a_selection_copies_the_pictures_it_runs_through() {
    let picture = Arc::new(Picture {
        width: 40,
        height: 30,
        rgba: yapfast_core::chat::Pixels::new(40, 30, vec![128; 40 * 30 * 4]),
        ..Default::default()
    });
    let window = showing(vec![
        ChatMessage {
            author: "Sam".into(),
            body: "Look at this.".into(),
            attachments: vec![yapfast_core::chat::Attachment {
                name: "cat.png".into(),
                size: None,
                image: true,
                dimensions: Some((400, 300)),
                preview: Some(picture),
                preview_failed: false,
                placement: Placement::Message,
                gif: None,
                spoiler: false,
                video: false,
                url: None,
            }],
            ..Default::default()
        },
        ChatMessage {
            author: "Riley".into(),
            body: "Nice.".into(),
            ..Default::default()
        },
    ]);
    assert_eq!(
        copy_upwards(&window, 0).as_deref(),
        Some("Sam 19:20\nLook at this.\n[Picture]\nRiley 19:21\nNice.")
    );
}

#[test]
fn a_selection_keeps_the_messages_it_scrolls_past() {
    let chat: Vec<ChatMessage> = (0..40)
        .map(|index| ChatMessage {
            author: "Sam".into(),
            body: format!("Message {index}."),
            ..Default::default()
        })
        .collect();
    let window = showing(chat);
    let expected: Vec<String> = (0..40)
        .map(|index| format!("Sam 19:2{index}\nMessage {index}."))
        .collect();
    assert_eq!(copy_upwards(&window, 40), Some(expected.join("\n")));
}

#[test]
fn a_click_on_a_link_in_selectable_text_opens_it() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let window = one_message("See https://example.com/yapfast");
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let record = opened.clone();
    window.on_open_link(move |link| record.borrow_mut().push(link.into()));
    let button = PointerEventButton::Left;
    // The link starts after "See ", a little way into the text column.
    for y in (100..700).step_by(4) {
        let position = slint::LogicalPosition::new(440., y as f32);
        window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    }
    let opened = opened.borrow();
    assert!(!opened.is_empty(), "a click on the link opens it");
    assert!(
        opened
            .iter()
            .all(|link| link == "https://example.com/yapfast")
    );
}

#[test]
fn a_link_showing_other_text_asks_before_it_opens() {
    use slint::platform::{PointerEventButton, WindowEvent};
    let window = one_message("See [example.com/yapfast](https://evil.example/yapfast)");
    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let record = opened.clone();
    window.on_open_link(move |link| record.borrow_mut().push(link.into()));
    let button = PointerEventButton::Left;
    for y in (100..700).step_by(4) {
        let position = slint::LogicalPosition::new(440., y as f32);
        window
            .window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    }
    let opened = opened.borrow();
    assert!(!opened.is_empty(), "a click on the link reaches Yapfast");
    assert!(
        opened
            .iter()
            .all(|link| link == "yapfast:leave/https://evil.example/yapfast"),
        "{opened:?}"
    );
}

#[test]
fn styled_messages_select_by_dragging_and_copy_as_plain_text() {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    let window = one_message("Thinking **co-op** after nine, if people are up for it.");
    let dispatch = |event| window.window().dispatch_event(event);
    let button = PointerEventButton::Left;
    let at = |x, y| slint::LogicalPosition::new(x, y);
    let ctrl = |key: &str| {
        dispatch(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        dispatch(WindowEvent::KeyPressed { text: key.into() });
        dispatch(WindowEvent::KeyReleased { text: key.into() });
        dispatch(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    };

    // Clicking the message without selecting leaves typing in the composer.
    let click = |x, y| {
        dispatch(WindowEvent::PointerPressed {
            position: at(x, y),
            button,
        });
        dispatch(WindowEvent::PointerReleased {
            position: at(x, y),
            button,
        });
    };
    let type_key = |key: &str| {
        dispatch(WindowEvent::KeyPressed { text: key.into() });
        dispatch(WindowEvent::KeyReleased { text: key.into() });
    };
    click(600., 712.);
    type_key("a");
    for y in (100..700).step_by(4) {
        click(500., y as f32);
    }
    type_key("b");
    assert_eq!(window.get_draft(), "ab");
    window.set_draft(SharedString::default());

    // Without element lookups in this build, drag down the timeline row by
    // row: from right of the text, past its end, to left of the timeline,
    // past its start. A drag that lands on the message selects all of it,
    // and copies it.
    for y in (100..700).step_by(4) {
        let y = y as f32;
        dispatch(WindowEvent::PointerMoved {
            position: at(900., y),
        });
        dispatch(WindowEvent::PointerPressed {
            position: at(900., y),
            button,
        });
        dispatch(WindowEvent::PointerMoved {
            position: at(600., y),
        });
        dispatch(WindowEvent::PointerMoved {
            position: at(200., y),
        });
        dispatch(WindowEvent::PointerReleased {
            position: at(200., y),
            button,
        });
        ctrl("c");
    }

    // Pasting shows what was copied: the text as read, without its Markdown.
    click(600., 712.);
    ctrl("v");
    assert_eq!(
        window.get_draft(),
        "Thinking co-op after nine, if people are up for it."
    );
}

#[test]
fn pointer_input_reaches_navigation_buttons() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let click = |x, y| {
        let position = slint::LogicalPosition::new(x, y);
        let button = slint::platform::PointerEventButton::Left;
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
    };
    click(32., 36.);
    assert!(window.get_dms_selected());
    assert_eq!(window.get_sidebar_title(), "Direct messages");
    click(32., 101.);
    assert!(!window.get_dms_selected());
    assert_eq!(window.get_sidebar_title(), "Friends");
    window.hide().unwrap();
}

#[test]
fn menus_list_their_choices_and_carry_them_out() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    let texts = |window: &YapfastWindow| -> Vec<String> {
        window
            .get_menu_rows()
            .iter()
            .map(|row| row.text.to_string())
            .collect()
    };
    window.invoke_prepare_menu("sam".into(), false);
    assert_eq!(texts(&window), ["Profile"]);
    window.invoke_prepare_menu("lounge".into(), false);
    assert_eq!(texts(&window), ["Join voice"]);
    window.invoke_prepare_menu("gone".into(), false);
    assert!(texts(&window).is_empty());

    window.invoke_choose_menu("sam".into(), false, "profile".into());
    assert!(adapter.borrow().app.state.profile.is_some());
    window.invoke_choose_menu("lounge".into(), false, "join-voice".into());
    assert_eq!(
        adapter.borrow().app.state.voice.room.as_deref(),
        Some("lounge")
    );

    let clipboard = || {
        i_slint_core::window::WindowInner::from_pub(window.window())
            .context()
            .platform()
            .clipboard_text(i_slint_core::platform::Clipboard::DefaultClipboard)
    };
    window.invoke_choose_menu("discord:10:21".into(), false, "copy-link".into());
    assert_eq!(
        clipboard().as_deref(),
        Some("https://discord.com/channels/10/21")
    );
    window.invoke_choose_menu("discord:10:21".into(), false, "copy-id".into());
    assert_eq!(clipboard().as_deref(), Some("21"));
    window.invoke_choose_menu("discord:10".into(), false, "copy-link".into());
    assert_eq!(
        clipboard().as_deref(),
        Some("https://discord.com/channels/10")
    );

    // Someone in the open conversation: their card, and the DM with them.
    window.invoke_navigate_server("friends".into());
    window.invoke_select_room("general".into());
    window.invoke_prepare_menu("@sam:preview.local".into(), true);
    assert_eq!(texts(&window), ["Profile", "Message"]);
    window.invoke_choose_menu("@sam:preview.local".into(), true, "message".into());
    assert_eq!(adapter.borrow().app.state.selected.as_deref(), Some("sam"));
}

#[test]
#[cfg(feature = "discord")]
fn a_server_members_menu_follows_permissions_and_ticks_roles() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    // Alice is Staff, below Leads; Dave, her friend, is below her.
    adapter.borrow_mut().app = yapfast_core::application::member_server(&["30"]);
    window.invoke_prepare_menu("4".into(), true);
    let rows: Vec<MenuRow> = window.get_menu_rows().iter().collect();
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Profile",
            "Mention",
            "Message",
            "Start call",
            "Add note",
            "Add friend nickname",
            "Change nickname",
            "Remove friend",
            "Ignore",
            "Block",
            "Roles",
            "Time out Dave",
            "Kick Dave",
            "Ban Dave",
            "Copy ID",
        ]
    );
    let marked = |pick: fn(&MenuRow) -> bool| -> Vec<&str> {
        rows.iter()
            .filter(|row| pick(row))
            .map(|row| row.text.as_str())
            .collect()
    };
    assert_eq!(
        marked(|row| row.separated),
        ["Change nickname", "Roles", "Time out Dave", "Copy ID"]
    );
    assert_eq!(
        marked(|row| row.danger),
        [
            "Remove friend",
            "Block",
            "Time out Dave",
            "Kick Dave",
            "Ban Dave"
        ]
    );

    // Roles: every one, ticked where held, greyed where out of reach.
    let roles = window.get_menu_submenus().row_data(10).unwrap();
    let shown: Vec<(String, bool, bool)> = roles
        .iter()
        .map(|row| (row.text.to_string(), row.checked, row.disabled))
        .collect();
    let row = |name: &str, checked, disabled| (name.to_owned(), checked, disabled);
    assert_eq!(
        shown,
        [
            row("Leads", false, true),
            row("Staff", false, true),
            row("Crew", true, false),
            row("Bot", false, true),
            row("Wardens", false, false),
        ]
    );
    assert!(roles.iter().all(|row| row.checkbox));
    window.invoke_choose_menu("4".into(), true, "role:20".into());
    assert!(
        !window
            .get_menu_submenus()
            .row_data(10)
            .unwrap()
            .row_data(2)
            .unwrap()
            .checked,
        "Crew unticks at once, and the menu stays open"
    );

    // Leads and the owner get no moderation.
    window.invoke_prepare_menu("3".into(), true);
    assert!(
        !window
            .get_menu_rows()
            .iter()
            .any(|row| row.danger && row.text.starts_with("Kick"))
    );

    // A ban asks how much to delete, and why.
    window.invoke_choose_menu("4".into(), true, "ban".into());
    assert!(window.get_person_open());
    let prompt = window.get_person_prompt();
    assert_eq!(prompt.title, "Ban Dave?");
    assert_eq!(prompt.field, "Reason (optional)");
    let options: Vec<String> = prompt
        .options
        .iter()
        .map(|option| option.to_string())
        .collect();
    assert_eq!(
        options,
        [
            "Don't delete any",
            "The last hour",
            "The last 6 hours",
            "The last 12 hours",
            "The last 24 hours",
            "The last 3 days",
            "The last 7 days",
        ]
    );
    window.invoke_choose_person_option(2);
    assert_eq!(window.get_person_prompt().chosen, 2);
    window.invoke_close_person();
    window.invoke_choose_menu("4".into(), true, "timeout".into());
    let options: Vec<String> = window
        .get_person_prompt()
        .options
        .iter()
        .map(|option| option.to_string())
        .collect();
    assert_eq!(
        options,
        [
            "60 seconds",
            "5 minutes",
            "10 minutes",
            "1 hour",
            "1 day",
            "1 week"
        ]
    );
    window.invoke_close_person();

    // Mention puts their name in the composer.
    window.invoke_choose_menu("3".into(), true, "mention".into());
    assert_eq!(adapter.borrow().app.state.draft(), "@Cara ");
}

#[test]
#[cfg(feature = "discord")]
fn invite_to_server_lists_the_servers_in_a_submenu() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    let mut app = yapfast_core::application::member_server(&["30"]);
    yapfast_core::application::with_clubs(&mut app);
    adapter.borrow_mut().app = app;
    window.invoke_prepare_menu("4".into(), true);
    let rows: Vec<MenuRow> = window.get_menu_rows().iter().collect();
    let at = rows
        .iter()
        .position(|row| row.text == "Invite to server")
        .unwrap();
    assert_eq!(rows[at - 1].text, "Change nickname");
    assert_eq!(rows[at + 1].text, "Remove friend");
    assert!(
        !rows[at].separated,
        "in the group of how she stands with them"
    );
    let servers = window.get_menu_submenus().row_data(at).unwrap();
    let servers: Vec<(String, String)> = servers
        .iter()
        .map(|row| (row.id.to_string(), row.text.to_string()))
        .collect();
    assert_eq!(
        servers,
        [("invite-to:discord:50".to_owned(), "Club".to_owned())],
        "not the server she's shown in, nor one that doesn't let Alice invite"
    );
}

#[test]
fn menus_give_each_row_its_own_submenu_and_tick_checkboxes() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    let row = |id: &str, checkbox: bool| MenuRow {
        id: id.into(),
        text: id.into(),
        checkbox,
        ..Default::default()
    };
    menus::show(
        &window,
        (
            vec![
                row("profile", false),
                row("mute", false),
                row("notifications", false),
                row("hide-muted", true),
            ],
            vec![
                vec![],
                vec![row("mute-15", false)],
                vec![row("all", false), row("suppress-roles", true)],
                vec![],
            ],
        ),
    );
    let submenus = window.get_menu_submenus();
    assert_eq!(submenus.row_count(), 4);
    assert_eq!(submenus.row_data(0).unwrap().row_count(), 0);
    assert_eq!(submenus.row_data(1).unwrap().row_count(), 1);
    assert_eq!(submenus.row_data(2).unwrap().row_count(), 2);

    // Choosing a checkbox ticks it where it is, in the menu or a submenu.
    assert_eq!(menus::toggle(&window, "hide-muted"), Some(true));
    assert!(window.get_menu_rows().row_data(3).unwrap().checked);
    assert_eq!(menus::toggle(&window, "suppress-roles"), Some(true));
    let ticked = |window: &YapfastWindow| {
        window
            .get_menu_submenus()
            .row_data(2)
            .unwrap()
            .row_data(1)
            .unwrap()
            .checked
    };
    assert!(ticked(&window));
    assert_eq!(menus::toggle(&window, "suppress-roles"), Some(false));
    assert!(!ticked(&window));
    assert_eq!(menus::toggle(&window, "profile"), None, "not a checkbox");

    // Without submenus, the menu reserves no room for one.
    menus::show(&window, (vec![row("profile", false)], vec![vec![]]));
    assert_eq!(window.get_menu_submenus().row_count(), 0);
}

#[test]
fn the_input_caret_opens_its_menu_and_measures_only_while_open() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    // Joined muted, so only the caret could unmute it.
    window.set_unmute_on_join(false);
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    adapter
        .borrow_mut()
        .action(&window, Action::RequestJoinVoice("lounge".into()));
    let click = |x, y| {
        let position = slint::LogicalPosition::new(x, y);
        let button = slint::platform::PointerEventButton::Left;
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
    };
    // The caret sits right of the microphone toggle in the dock.
    click(206., 732.);
    assert!(adapter.borrow().meter.is_some());
    assert!(window.get_muted());
    click(800., 200.);
    assert!(adapter.borrow().meter.is_none());
    window.hide().unwrap();
}

#[test]
fn dragging_the_sidebar_edge_resizes_it_within_limits_and_remembers_it() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let drag = |from: f32, to: f32| {
        let window = window.window();
        let button = slint::platform::PointerEventButton::Left;
        let at = |x| slint::LogicalPosition::new(x, 400.);
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at(from) });
        window.dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: at(from),
            button,
        });
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at(to) });
        window.dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: at(to),
            button,
        });
    };
    assert_eq!(window.get_sidebar_shown(), 240.);
    drag(312., 362.);
    assert_eq!(window.get_sidebar_shown(), 290.);
    assert_eq!(Layout::read(directory.path()).sidebar_width, Some(290.));
    // The sidebar stops at 380px, and at 180px.
    drag(362., 700.);
    assert_eq!(window.get_sidebar_shown(), 380.);
    drag(452., 0.);
    assert_eq!(window.get_sidebar_shown(), 180.);
    // In a narrow window, dragging keeps 420px for the conversation,
    // beside the member list when it shows.
    window
        .window()
        .set_size(slint::LogicalSize::new(800., 560.));
    drag(252., 700.);
    assert_eq!(window.get_sidebar_shown(), 180.);
    window.set_member_list(false);
    drag(252., 700.);
    assert_eq!(window.get_sidebar_shown(), 308.);
    drag(380., 252.);
    // A new window starts at the stored width.
    let next = YapfastWindow::new().unwrap();
    adapter.borrow_mut().attach(&next);
    assert_eq!(next.get_sidebar_width(), 180.);
    window.hide().unwrap();
}

#[test]
fn a_window_made_again_shows_what_the_last_one_did() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    adapter
        .borrow_mut()
        .action(&window, Action::SelectRoom("general".into()));
    let topic = Arc::new(yapfast_core::chat::Topic {
        body: "Plans for the weekend".into(),
        ..Default::default()
    });
    adapter.borrow_mut().apply(Event::Topic {
        room: "general".into(),
        topic: Some(topic),
    });
    adapter.borrow_mut().refresh(&window);
    let shown = |window: &YapfastWindow| {
        (
            window.get_sidebar_title(),
            window.get_filter_placeholder(),
            window.get_has_topic(),
            i_slint_core::yapfast_selection::plain(&window.get_room_topic()),
        )
    };
    let before = shown(&window);
    assert_eq!(before.0, "Friends");
    assert!(!before.1.is_empty());
    assert_eq!(before.3, "Plans for the weekend");
    drop(window);
    let next = YapfastWindow::new().unwrap();
    adapter.borrow_mut().attach(&next);
    bind_callbacks(&next, &adapter);
    adapter.borrow_mut().refresh(&next);
    assert_eq!(shown(&next), before);
}

#[test]
fn dragging_the_thread_beside_resizes_it_within_limits_and_remembers_it() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1400., 760.));
    window.set_aside_open(true);
    let drag = |from: f32, to: f32| {
        let window = window.window();
        let button = slint::platform::PointerEventButton::Left;
        let at = |x| slint::LogicalPosition::new(x, 400.);
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at(from) });
        window.dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: at(from),
            button,
        });
        window.dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at(to) });
        window.dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: at(to),
            button,
        });
    };
    assert_eq!(window.get_aside_shown(), 420.);
    // Its edge, 980px in, widens it leftwards.
    drag(980., 880.);
    assert_eq!(window.get_aside_shown(), 520.);
    assert_eq!(Layout::read(directory.path()).aside_width, Some(520.));
    // It keeps 320px, and leaves the conversation 420px.
    drag(880., 1300.);
    assert_eq!(window.get_aside_shown(), 320.);
    drag(1080., 0.);
    assert_eq!(window.get_aside_shown(), 1400. - 72. - 240. - 420.);
    // A new window starts at the stored width.
    let next = YapfastWindow::new().unwrap();
    adapter.borrow_mut().attach(&next);
    assert_eq!(next.get_aside_width(), 668.);
    window.hide().unwrap();
}

#[test]
fn the_member_list_shows_for_text_channels_and_remembers_being_closed() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    assert!(window.get_members_available() && window.get_member_list());
    let roster = window.get_roster();
    assert_eq!(roster.row_count(), 4);
    assert_eq!(roster.row_data(0).unwrap().heading, "ADMINS — 1");
    assert_eq!(roster.row_data(2).unwrap().heading, "MEMBERS — 2");
    assert!(roster.row_data(3).unwrap().heading.is_empty());
    let click = |x, y| {
        let position = slint::LogicalPosition::new(x, y);
        let button = slint::platform::PointerEventButton::Left;
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerPressed { position, button });
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased { position, button });
    };
    // The toggle ends the conversation header.
    click(1080., 24.);
    assert!(!window.get_member_list());
    assert!(Layout::read(directory.path()).members_hidden);
    click(1080., 24.);
    assert!(window.get_member_list());
    // Voice channels and one-to-one DMs have no member list.
    window.invoke_select_room("lounge".into());
    assert!(!window.get_members_available());
    window.invoke_navigate_dms();
    assert!(!window.get_members_available());
    window.invoke_select_room("party".into());
    assert!(window.get_members_available());
    assert_eq!(window.get_roster().row_count(), 3);
    window.hide().unwrap();
}

#[test]
fn forwarding_and_frequent_reactions_reach_the_window() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    let glyphs =
        |model: ModelRc<EmojiCell>| model.iter().map(|cell| cell.glyph).collect::<Vec<_>>();
    assert_eq!(glyphs(window.get_bar_reactions()), ["👍", "❤️", "😂"]);
    let message = window.get_messages().row_data(0).unwrap();
    window.invoke_react(message.item.clone(), "🍕".into());
    assert_eq!(
        glyphs(window.get_menu_reactions()),
        ["🍕", "👍", "❤️", "😂"]
    );
    assert_eq!(Layout::read(directory.path()).reactions.get("🍕"), Some(&1));

    window.invoke_start_forward(message.item.clone());
    assert!(window.get_forward_open());
    assert_eq!(window.get_forward_preview(), "Anyone around tonight?");
    let targets = window.get_forward_targets();
    assert!(
        targets.iter().all(|target| target.id != "lounge"),
        "voice is no target"
    );
    window.set_forward_query("gam".into());
    window.invoke_forward_search();
    assert_eq!(window.get_forward_targets().row_count(), 1);
    window.invoke_forward_to("games".into());
    assert!(window.get_forward_targets().row_data(0).unwrap().sent);
    window.invoke_close_forward();
    assert!(!window.get_forward_open());

    window.invoke_emoji_search("pizza".into());
    assert_eq!(window.get_emoji_rows().row_count(), 1);
}

#[test]
fn scrolling_up_leaves_the_latest_message() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    let rows: Vec<MessageRow> = (0..200)
        .map(|index| MessageRow {
            id: format!("m{index}").into(),
            item: format!("i{index}").into(),
            author: "Bob".into(),
            body: if index % 2 == 0 {
                format!("Message {index}").into()
            } else {
                format!("Message {index}, a longer line that wraps across the timeline at ordinary window widths, and then some more words").into()
            },
            first: index % 5 == 0,
            ..Default::default()
        })
        .collect();
    window.set_messages(ModelRc::new(VecModel::from(rows)));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let settle = || {
        for _ in 0..5 {
            slint::platform::update_timers_and_animations();
            let _ = window.invoke_timeline_extent();
        }
    };
    settle();
    window.invoke_scroll_to_latest();
    settle();
    assert!(
        window.invoke_timeline_at_end(),
        "starts at the latest message"
    );
    for _ in 0..8 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: 50.,
            });
        settle();
    }
    assert!(
        !window.invoke_timeline_at_end(),
        "scrolling up leaves the latest message"
    );
    window.hide().unwrap();
}

#[test]
fn a_wheel_spun_fast_scrolls_farther_for_each_notch() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    window.set_history_start(true);
    let rows: Vec<MessageRow> = (0..300)
        .map(|index| {
            let mut row = message_row_plain(&ChatMessage {
                id: format!("m{index}"),
                item: format!("i{index}"),
                author: "Bob".into(),
                body: format!("Message {index}"),
                formatted: true,
                ..Default::default()
            });
            row.first = true;
            row
        })
        .collect();
    window.set_messages(ModelRc::new(VecModel::from(rows)));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let wait = |time: Duration| {
        i_slint_backend_testing::mock_elapsed_time(time);
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(1100., 740.),
            });
    };
    // Scrolls up by ten notches, `apart` from each other, and tells how many
    // rows that went.
    let spin = |apart: Duration| {
        window.invoke_scroll_to_latest();
        for _ in 0..20 {
            wait(Duration::from_millis(50));
        }
        let start = window.get_top_index();
        for _ in 0..10 {
            // As the winit backend sends a notch: `PointerScrolled` would
            // jump without the wheel's animation.
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::internal(
                    i_slint_core::input::BackendMouseEvent::Wheel {
                        position: i_slint_core::lengths::LogicalPoint::new(700., 380.),
                        delta_x: 0.,
                        delta_y: 60.,
                        phase: i_slint_core::input::TouchPhase::Moved,
                    },
                ));
            wait(apart);
        }
        for _ in 0..20 {
            wait(Duration::from_millis(50));
        }
        start - window.get_top_index()
    };
    let slow = spin(Duration::from_millis(300));
    let fast = spin(Duration::from_millis(20));
    assert!(slow > 0);
    assert!(
        fast as f32 > slow as f32 * 1.5,
        "fast notches went {fast} rows, slow ones {slow}"
    );
    window.hide().unwrap();
}

#[test]
fn older_messages_go_in_above_without_moving_those_shown() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    window.set_loading_history(true);
    // The first row carries the loading notice and its day; once older
    // messages from that day are above it, it carries neither.
    // Each from someone else, so each row shows its author, all on one day.
    let messages = |from: usize| -> Vec<Arc<ChatMessage>> {
        (from..200)
            .map(|index| {
                let body = if index % 3 == 0 {
                    format!("Message {index}, a longer line that wraps across the timeline at ordinary window widths, and then some more words")
                } else {
                    format!("Message {index}")
                };
                Arc::new(ChatMessage {
                    id: format!("m{index}"),
                    item: format!("i{index}"),
                    author: format!("Person {index}"),
                    body,
                    formatted: true,
                    // Noon on 1 October 2026, UTC.
                    timestamp: 1_790_856_000 + index as u64,
                    ..Default::default()
                })
            })
            .collect()
    };
    let revealed = HashSet::new();
    let marks = Marks {
        first_new: usize::MAX,
        first_unread: None,
        revealed: &revealed,
        restyle: false,
    };
    let model = Rc::new(VecModel::default());
    let mut rows = Vec::new();
    let shown = messages(150);
    show_timeline(&model, &mut rows, &[], &shown, &marks, message_row_plain);
    window.set_messages(ModelRc::from(model.clone()));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    // As drawing does, the pointer goes through the list, which lays it out.
    let settle = || {
        for _ in 0..5 {
            slint::platform::update_timers_and_animations();
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(1100., 740.),
                });
        }
    };
    settle();
    window.invoke_scroll_to_latest();
    settle();
    for _ in 0..40 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: 400.,
            });
        settle();
    }
    // The first message, with the loading notice and its day above it, is
    // partly scrolled under the top.
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(700., 380.),
            delta_x: 0.,
            delta_y: -90.,
        });
    settle();
    assert_eq!(window.get_top_index(), 0);
    let edge = window.get_top_edge();
    show_timeline(
        &model,
        &mut rows,
        &shown,
        &messages(100),
        &marks,
        message_row_plain,
    );
    settle();
    assert_eq!(
        (window.get_top_index(), window.get_top_item().as_str()),
        (50, "i150"),
        "the same message is at the top"
    );
    assert!(
        (window.get_top_edge() - edge).abs() < 0.5,
        "its text stays where it was: {edge} then {}",
        window.get_top_edge()
    );
    assert!(!window.invoke_timeline_at_end());
    // A message deleted above leaves it where it is too.
    model.remove(10);
    settle();
    assert_eq!(
        (window.get_top_index(), window.get_top_item().as_str()),
        (49, "i150"),
        "the same message is at the top after a deletion above"
    );
    assert!(
        (window.get_top_edge() - edge).abs() < 0.5,
        "its text stays where it was after a deletion above: {edge} then {}",
        window.get_top_edge()
    );
    // Scrolling up shows those that went in.
    for _ in 0..2 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: 200.,
            });
        settle();
    }
    assert!(window.get_top_index() < 50);
    window.hide().unwrap();
}

#[test]
fn scrolling_up_through_styled_messages_stays_up() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    let rows: Vec<MessageRow> = (0..200)
        .map(|index| {
            let body = match index % 4 {
                0 => format!("**Message** {index}"),
                1 => format!("Message {index} with https://example.com/{index} and words that wrap across the timeline at ordinary widths, and then *some* more"),
                2 => format!("line one {index}\nline two\nline three"),
                _ => format!("Message {index}"),
            };
            let mut row = message_row_plain(&ChatMessage {
                id: format!("m{index}"),
                item: format!("i{index}"),
                author: "Bob".into(),
                body,
                formatted: true,
                ..Default::default()
            });
            row.first = index % 5 == 0;
            row
        })
        .collect();
    window.set_messages(ModelRc::new(VecModel::from(rows)));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let settle = || {
        for _ in 0..5 {
            slint::platform::update_timers_and_animations();
            let _ = window.invoke_timeline_extent();
        }
    };
    settle();
    window.invoke_scroll_to_latest();
    settle();
    assert!(
        window.invoke_timeline_at_end(),
        "starts at the latest message"
    );
    for step in 0..8 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: 50.,
            });
        settle();
        assert!(
            !window.invoke_timeline_at_end() && window.invoke_timeline_below() > 0.,
            "step {step} stays scrolled up"
        );
    }
    // Scrolling back down resumes following, and later rows stay in view.
    for _ in 0..40 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: -200.,
            });
        settle();
    }
    assert!(
        window.invoke_timeline_at_end(),
        "back at the latest message"
    );
    let messages = window.get_messages();
    let model = messages
        .as_any()
        .downcast_ref::<VecModel<MessageRow>>()
        .unwrap();
    model.push(message_row_plain(&ChatMessage {
        id: "new".into(),
        item: "new".into(),
        author: "Ann".into(),
        body: "A *new* message with https://example.com\nand a second line".into(),
        formatted: true,
        ..Default::default()
    }));
    settle();
    assert_eq!(window.invoke_timeline_below(), 0., "the new message shows");
    window.hide().unwrap();
}

#[test]
fn callbacks_preserve_drafts_scope_voice_and_clear_session_inputs() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    assert!(window.get_workspace() && window.get_preview());
    let server = adapter.borrow().app.state.servers[0].id.clone();
    window.invoke_navigate_server(server.clone().into());
    let rooms = adapter
        .borrow()
        .app
        .state
        .rooms
        .iter()
        .filter(|room| adapter.borrow().app.state.in_scope(room))
        .cloned()
        .collect::<Vec<_>>();
    let text_rooms = rooms.iter().filter(|room| !room.voice).collect::<Vec<_>>();
    window.invoke_select_room(text_rooms[0].id.clone().into());
    window.invoke_draft_edited("Draft survives navigation".into());
    window.invoke_select_room(text_rooms[1].id.clone().into());
    window.invoke_draft_edited("Different channel draft".into());
    window.invoke_select_room(text_rooms[0].id.clone().into());
    assert_eq!(window.get_draft(), "Draft survives navigation");
    assert!(window.get_can_send());
    let count = window.get_messages().row_count();
    window.invoke_send();
    assert_eq!(window.get_messages().row_count(), count + 1);
    assert!(window.get_draft().is_empty());
    // Calls start with the microphone on unless that's turned off. Out of a
    // call, the toggles change how the next one starts, and are kept.
    assert!(window.get_unmute_on_join() && !window.get_deafen_on_join());
    window.invoke_toggle_deafen();
    assert!(window.get_deafen_on_join() && adapter.borrow().layout.deafen_on_join);
    window.invoke_toggle_mute();
    assert!(window.get_unmute_on_join() && !window.get_deafen_on_join());
    window.invoke_toggle_mute();
    assert!(!window.get_unmute_on_join() && adapter.borrow().layout.unmute_on_join == Some(false));
    let voice_room = rooms.iter().find(|room| room.voice).unwrap();
    // Clicking a voice channel joins it.
    window.invoke_select_room(voice_room.id.clone().into());
    assert!(window.get_voice_here() && window.get_voice_active() && window.get_muted());
    window.invoke_navigate_dms();
    assert!(window.get_voice_active() && window.get_voice_visible());
    assert!(!window.get_voice_here());
    assert!(window.get_rooms().iter().all(|row| {
        adapter
            .borrow()
            .app
            .state
            .rooms
            .iter()
            .any(|room| room.id.as_str() == row.id.as_str() && room.direct)
    }));
    window.invoke_toggle_mute();
    assert!(!window.get_muted());
    window.invoke_toggle_deafen();
    assert!(window.get_muted() && window.get_deafened());
    window.set_password("secret that must be cleared".into());
    window.set_recovery("recovery that must be cleared".into());
    window.set_settings_open(true);
    window.invoke_logout();
    assert!(!window.get_workspace() && !window.get_voice_active());
    assert!(window.get_password().is_empty() && window.get_recovery().is_empty());
    assert!(!window.get_settings_open());
}

#[test]
fn message_actions_reach_the_shared_application_in_the_preview() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    let row = |item: &SharedString| {
        window
            .get_messages()
            .iter()
            .find(|row| row.item == *item)
            .unwrap()
    };
    let own = window.get_messages().iter().find(|row| row.own).unwrap();
    let other = window.get_messages().iter().find(|row| !row.own).unwrap();
    assert!(own.can_edit && !other.can_edit && !window.get_can_attach());

    window.invoke_react(other.item.clone(), "👍".into());
    let reactions = row(&other.item).reactions;
    assert_eq!(reactions.row_count(), 1);
    assert!(reactions.row_data(0).unwrap().mine);
    window.invoke_react(other.item.clone(), "👍".into());
    assert_eq!(row(&other.item).reactions.row_count(), 0);

    window.invoke_reply(other.item.clone());
    assert!(window.get_compose_label().starts_with("Replying to Sam"));
    window.invoke_draft_edited("**Sure**".into());
    window.invoke_send();
    let reply = window.get_messages().iter().last().unwrap();
    assert!(reply.has_reply && reply.reply_author == "Sam");
    assert!(window.get_compose_label().is_empty() && window.get_draft().is_empty());

    window.invoke_draft_edited("Unsent draft".into());
    window.invoke_edit(own.item.clone());
    assert_eq!(window.get_draft(), own.body);
    assert_eq!(window.get_compose_label(), "Editing your message");
    window.invoke_draft_edited("Changed".into());
    window.invoke_send();
    let edited = row(&own.item);
    assert!(edited.edited && edited.body == "Changed");
    assert_eq!(window.get_draft(), "Unsent draft");

    window.invoke_request_delete(other.item.clone());
    assert!(
        window.get_confirm_delete().is_empty(),
        "only own messages can be deleted"
    );
    window.invoke_request_delete(own.item.clone());
    assert_eq!(window.get_confirm_delete(), own.item);
    window.invoke_confirm_delete_message();
    assert!(row(&own.item).deleted && window.get_confirm_delete().is_empty());
}

#[test]
fn signed_in_controls_keep_trust_and_plaintext_sending_gates() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    {
        let mut adapter = adapter.borrow_mut();
        adapter.app.state.preview = false;
        adapter.app.state.account = Some("@test:example.invalid".into());
        adapter.app.state.trusted = false;
        adapter.refresh(&window);
    }
    window.invoke_draft_edited("Keep encrypted".into());
    assert!(!window.get_can_send());
    window.invoke_send();
    assert!(!window.get_sending());
    {
        let mut adapter = adapter.borrow_mut();
        adapter.app.state.trusted = true;
        let id = adapter.app.state.selected.clone().unwrap();
        adapter
            .app
            .state
            .rooms
            .iter_mut()
            .find(|room| room.id == id)
            .unwrap()
            .encrypted = false;
        adapter.refresh(&window);
    }
    assert!(!window.get_composer_enabled() && !window.get_can_send());
    window.invoke_send();
    assert_eq!(window.get_draft(), "Keep encrypted");
    assert!(!window.get_sending());
}

/// What a list told the window about its rows, as `changed 3`, `added 0+2` and
/// `removed 1+1`.
#[derive(Default)]
struct Notices(RefCell<Vec<String>>);

impl i_slint_core::model::ModelChangeListener for Notices {
    fn row_changed(self: std::pin::Pin<&Self>, row: usize) {
        self.0.borrow_mut().push(format!("changed {row}"));
    }
    fn row_added(self: std::pin::Pin<&Self>, index: usize, count: usize) {
        self.0.borrow_mut().push(format!("added {index}+{count}"));
    }
    fn row_removed(self: std::pin::Pin<&Self>, index: usize, count: usize) {
        self.0.borrow_mut().push(format!("removed {index}+{count}"));
    }
    fn reset(self: std::pin::Pin<&Self>) {
        self.0.borrow_mut().push("reset".into());
    }
}

#[test]
fn rows_with_ids_change_only_where_they_changed() {
    use i_slint_core::model::ModelChangeListenerContainer;
    let model = VecModel::from(vec![("b", 1), ("c", 1), ("d", 1)]);
    let notices = Box::pin(ModelChangeListenerContainer::new(Notices::default()));
    model
        .model_tracker()
        .attach_peer(notices.as_ref().model_peer());
    let update = |rows: Vec<(&'static str, i32)>| {
        update_model_by_id(&model, rows.clone(), |row| row.0.into());
        assert_eq!(model.iter().collect::<Vec<_>>(), rows);
        std::mem::take(&mut *notices.0.borrow_mut())
    };
    // Older rows come in above, and those below stay as they were.
    assert_eq!(
        update(vec![("a", 1), ("b", 1), ("c", 1), ("d", 1)]),
        ["added 0+1"]
    );
    assert_eq!(
        update(vec![("a", 1), ("b", 1), ("c", 2), ("d", 1)]),
        ["changed 2"]
    );
    assert_eq!(
        update(vec![("a", 1), ("b", 1), ("c", 2), ("d", 1), ("e", 1)]),
        ["added 4+1"]
    );
    assert_eq!(
        update(vec![("a", 1), ("c", 2), ("d", 1), ("e", 1)]),
        ["removed 1+1"]
    );
    assert_eq!(
        update(vec![("a", 1), ("x", 1), ("y", 1), ("e", 2)]),
        [
            "removed 1+1",
            "removed 1+1",
            "added 1+1",
            "added 2+1",
            "changed 3"
        ]
    );
    assert_eq!(update(vec![]).len(), 4, "each row goes");
}

#[test]
fn typing_and_the_call_show_without_building_the_rest_again() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    let mut adapter = adapter.borrow_mut();
    let room = adapter.app.state.selected.clone().unwrap();
    let reach = adapter.apply(Event::Typing {
        room,
        names: vec!["Sam".into()],
    });
    assert!(
        !reach
            .and(adapter.apply(Event::Voice(Default::default())))
            .everything
    );
    adapter.show(&window, reach);
    assert_eq!(window.get_typing(), "Sam is typing...");
    // A refresh that changes nothing the sidebar shows doesn't build it again,
    // so a row marked here stays marked.
    let mut row = adapter.models.rooms.row_data(0).unwrap();
    row.name = "Marked".into();
    adapter.models.rooms.set_row_data(0, row);
    adapter.refresh(&window);
    assert_eq!(adapter.models.rooms.row_data(0).unwrap().name, "Marked");
    adapter.app.state.rooms[0].name = "Renamed".into();
    adapter.refresh(&window);
    assert_ne!(adapter.models.rooms.row_data(0).unwrap().name, "Marked");
}

#[test]
fn collapsed_categories_keep_the_open_unread_and_occupied_channels() {
    let row = |id: &str, heading: &str| RoomRow {
        id: id.into(),
        heading: heading.into(),
        ..Default::default()
    };
    let rows = vec![
        row("a", "TEXT"),
        RoomRow {
            selected: true,
            ..row("b", "")
        },
        RoomRow {
            unread: true,
            ..row("c", "")
        },
        row("d", ""),
        row("e", "VOICE"),
        row("f", ""),
        row("g", "OTHER"),
        row("h", ""),
    ];
    let collapsed = std::collections::BTreeSet::from(["TEXT".to_owned(), "VOICE".to_owned()]);
    let folded = refresh::fold_collapsed(rows, &collapsed, Some("f"));
    let shown: Vec<_> = folded
        .iter()
        .map(|row| {
            (
                row.id.as_str(),
                row.heading.as_str(),
                row.collapsed,
                row.hidden,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            // A category's first channel goes, and its heading stays alone,
            // naming the channel for the heading's menu.
            ("a", "TEXT", true, true),
            ("b", "", true, false),
            ("c", "", true, false),
            ("e", "VOICE", true, true),
            // The voice channel Yapfast is in stays.
            ("f", "", true, false),
            ("g", "OTHER", false, false),
            ("h", "", false, false),
        ]
    );
}

#[test]
fn clicking_a_category_collapses_it() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    let server = adapter.borrow().app.state.servers[0].id.clone();
    window.invoke_navigate_server(server.into());
    let heading = window
        .get_rooms()
        .iter()
        .map(|row| row.heading)
        .find(|heading| !heading.is_empty())
        .expect("the preview server has categories");
    assert!(!window.get_rooms().iter().any(|row| row.collapsed));
    // Nothing the app keeps changes, only how the sidebar shows it.
    window.invoke_toggle_category(heading.clone());
    assert!(window.get_rooms().iter().any(|row| row.collapsed));
    window.invoke_toggle_category(heading);
    assert!(!window.get_rooms().iter().any(|row| row.collapsed));
}

#[test]
fn opening_at_the_new_divider_puts_it_at_the_top_and_the_bar_jumps_back() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    window.set_history_start(true);
    let rows = || -> Vec<MessageRow> {
        (0..100)
            .map(|index| {
                let mut row = message_row_plain(&ChatMessage {
                    id: format!("m{index}"),
                    item: format!("i{index}"),
                    author: "Bob".into(),
                    body: format!("Message {index}"),
                    formatted: true,
                    ..Default::default()
                });
                row.first = true;
                row.unread_start = index == 60;
                row
            })
            .collect()
    };
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let settle = || {
        for _ in 0..10 {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(1100., 740.),
                });
        }
    };
    settle();
    // The jump comes with the rows, before the list has measured any.
    window.set_messages(ModelRc::new(VecModel::from(rows())));
    window.set_unread_index(60);
    window.set_unread_label("40 new messages".into());
    messages::show_message(&window, 60, "i60", false);
    settle();
    assert!(
        (59..=60).contains(&window.get_top_index()),
        "the divider is at the top, not row {}",
        window.get_top_index()
    );
    assert!(!window.get_unread_above() && !window.get_unread_below());
    assert!(!window.invoke_timeline_at_end());

    // Scrolled on down, the divider is above, and the bar takes the view back.
    for _ in 0..3 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: -400.,
            });
        settle();
    }
    assert!(window.get_top_index() > 60);
    assert!(window.get_unread_above());
    messages::show_message(&window, 60, "i60", false);
    settle();
    assert!((59..=60).contains(&window.get_top_index()));
    assert!(!window.get_unread_above());
    // Scrolled up past it, it's below.
    for _ in 0..4 {
        window
            .window()
            .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(700., 380.),
                delta_x: 0.,
                delta_y: 400.,
            });
        settle();
    }
    assert!(window.get_unread_below());
}

#[test]
fn rows_mark_mentions_and_the_new_bar_counts_what_others_sent() {
    let message = |own: bool, mentions_me: bool| ChatMessage {
        own,
        mentions_me,
        time: "14:05".into(),
        // Today, so the bar leaves the date out.
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        ..Default::default()
    };
    assert!(message_row_plain(&message(false, true)).highlighted);
    assert!(!message_row_plain(&message(false, false)).highlighted);
    let unread = [
        message(false, true),
        message(true, false),
        message(false, false),
    ]
    .map(Arc::new);
    assert_eq!(
        refresh::unread_label(&unread, false),
        "2 new messages since 14:05",
        "Alice's own reply isn't new to her"
    );
    assert_eq!(
        refresh::unread_label(&unread[..1], false),
        "1 new message since 14:05"
    );
    assert_eq!(
        refresh::unread_label(&unread[..1], true),
        "1+ new messages since 14:05",
        "more came after a window of older messages"
    );
    let older = ChatMessage {
        timestamp: 1_791_329_400,
        ..message(false, false)
    };
    assert!(
        refresh::unread_label(&[Arc::new(older)], false)
            .starts_with("1 new message since 14:05 on "),
        "with the date when it wasn't today"
    );
}

#[test]
fn virtualized_lists_request_only_nearby_avatars() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    window.set_workspace(true);
    window.set_room_selected(true);
    window.set_room_ready(true);
    window.set_members_available(true);
    let requested = Rc::new(RefCell::new(HashSet::new()));
    let requests = requested.clone();
    window
        .global::<PictureStore>()
        .on_picture(|_, _| slint::Image::default());
    window.global::<PictureStore>().on_request(move |url| {
        requests.borrow_mut().insert(url.to_string());
    });
    window.set_servers(ModelRc::new(VecModel::from(
        (0..500)
            .map(|index| ServerRow {
                id: index.to_string().into(),
                name: format!("Server {index}").into(),
                picture_url: format!("server/{index}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    window.set_roster(ModelRc::new(VecModel::from(
        (0..500)
            .map(|index| MemberRow {
                id: index.to_string().into(),
                name: format!("Member {index}").into(),
                picture_url: format!("member/{index}").into(),
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    window.show().unwrap();
    window
        .window()
        .set_size(slint::LogicalSize::new(1120., 760.));
    let settle = || {
        for _ in 0..10 {
            i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(50));
            window
                .window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(1100., 740.),
                });
        }
    };
    settle();
    let first = requested.borrow().clone();
    let servers = first
        .iter()
        .filter(|url| url.starts_with("server/"))
        .count();
    let members = first
        .iter()
        .filter(|url| url.starts_with("member/"))
        .count();
    assert!(
        (1..30).contains(&servers),
        "requested {servers} of 500 server icons"
    );
    assert!(
        (1..35).contains(&members),
        "requested {members} of 500 member avatars"
    );
    window
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(1000., 380.),
            delta_x: 0.,
            delta_y: -600.,
        });
    settle();
    assert!(
        requested
            .borrow()
            .iter()
            .any(|url| !first.contains(url) && url.starts_with("member/"))
    );
    assert!(
        requested.borrow().len() < 100,
        "scrolling does not instantiate the whole list"
    );
    window.hide().unwrap();
}

#[test]
fn replacing_a_temporary_item_reports_the_view_without_changing_row_count() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    adapter.borrow_mut().action(&window, Action::Preview);
    let reported = Rc::new(Cell::new(0));
    let reports = reported.clone();
    window.on_media_viewport(move || reports.set(reports.get() + 1));
    let count = window.get_messages().row_count();
    let mut state = adapter.borrow_mut();
    let room = state.app.state.selected.clone().unwrap();
    let mut messages = (*state.app.state.messages).clone();
    Arc::make_mut(&mut messages[0]).item = "e:$confirmed".into();
    state.apply(Event::Timeline { room, messages });
    state.refresh(&window);
    assert_eq!(window.get_messages().row_count(), count);
    assert_eq!(
        window.get_messages().row_data(0).unwrap().item,
        "e:$confirmed"
    );
    assert!(
        reported.get() > 0,
        "the backend receives the new item ids without scrolling"
    );
}

#[test]
fn an_unread_dm_shows_on_the_rail_above_the_servers_and_opens() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    adapter.borrow_mut().action(&window, Action::Preview);
    let rail = |window: &YapfastWindow| -> Vec<(String, bool, bool)> {
        window
            .get_servers()
            .iter()
            .map(|row| (row.id.to_string(), row.dm, row.divider))
            .collect()
    };
    let servers = adapter.borrow().app.state.servers.len();
    let read = rail(&window);
    assert_eq!(read.len(), servers + 1);
    assert!(read[0].2, "the divider leads the servers");
    {
        let mut adapter = adapter.borrow_mut();
        let riley = adapter
            .app
            .state
            .rooms
            .iter_mut()
            .find(|room| room.id == "riley")
            .unwrap();
        riley.unread = 2;
        riley.mentions = 2;
        adapter.refresh(&window);
    }
    let unread = rail(&window);
    assert_eq!(unread[0], ("riley".into(), true, false));
    assert!(unread[1].2);
    assert_eq!(&unread[1..], &read[..]);
    let dm = window.get_servers().row_data(0).unwrap();
    assert_eq!((dm.name.as_str(), dm.badge.as_str()), ("Riley", "2"));
    window.invoke_open_conversation("riley".into());
    let adapter = adapter.borrow();
    assert_eq!(adapter.app.state.navigation, Navigation::DirectMessages);
    assert_eq!(adapter.app.state.selected.as_deref(), Some("riley"));
}

#[test]
fn a_folder_on_the_rail_holds_its_servers_until_opened() {
    let folder = yapfast_core::chat::ServerFolder {
        id: "discord-folder:7".into(),
        name: None,
        color: Some(0xff0000),
    };
    let server = |id: &str, folder: Option<&yapfast_core::chat::ServerFolder>| {
        yapfast_core::chat::ChatServer {
            id: id.into(),
            name: id.into(),
            channels: vec![],
            discord: true,
            muted: false,
            unavailable: false,
            icon: Some(format!("{id}.png")),
            folder: folder.cloned(),
            hide_muted: false,
            pinned: Vec::new(),
        }
    };
    let mut state = AppState::default();
    state.servers = vec![
        server("Alpha", None),
        server("Bravo", Some(&folder)),
        server("Charlie", Some(&folder)),
        server("Delta", None),
    ];
    state.navigation = Navigation::Server("Charlie".into());
    let shown = |open: &[&str]| {
        let open = open.iter().map(|&id| id.to_owned()).collect();
        refresh::rail_rows(&state, &HashMap::new(), &open)
    };
    let rows = |rows: &[ServerRow]| -> Vec<(String, bool, bool, bool, bool)> {
        rows.iter()
            .map(|row| {
                (
                    row.id.to_string(),
                    row.folder,
                    row.nested,
                    row.last,
                    row.selected,
                )
            })
            .collect()
    };
    let closed = shown(&[]);
    assert_eq!(
        rows(&closed),
        [
            ("Alpha".into(), false, false, false, false),
            // Closed, it shows that one of its servers is selected.
            ("discord-folder:7".into(), true, false, false, true),
            ("Delta".into(), false, false, false, false),
        ]
    );
    // Without a name, it's named by its servers.
    assert_eq!(closed[1].name, "Bravo, Charlie");
    assert_eq!(closed[1].tint, slint::Color::from_rgb_u8(0xff, 0, 0));
    let preview: Vec<_> = closed[1]
        .preview
        .iter()
        .map(|server| server.picture_url.to_string())
        .collect();
    assert_eq!(preview, ["Bravo.png", "Charlie.png"]);

    let open = shown(&["discord-folder:7"]);
    assert_eq!(
        rows(&open),
        [
            ("Alpha".into(), false, false, false, false),
            ("discord-folder:7".into(), true, false, false, true),
            ("Bravo".into(), false, true, false, false),
            ("Charlie".into(), false, true, true, true),
            ("Delta".into(), false, false, false, false),
        ]
    );
    assert!(open[1].open && open[3].tint == open[1].tint);
}

#[test]
fn closing_the_viewer_lets_go_of_its_picture() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    let mut adapter = adapter.borrow_mut();
    adapter.app.state.messages = Arc::new(vec![Arc::new(ChatMessage {
        item: "i0".into(),
        attachments: vec![yapfast_core::chat::Attachment {
            name: "a.png".into(),
            size: None,
            image: true,
            dimensions: Some((40, 30)),
            preview: Some(Arc::new(Picture {
                width: 40,
                height: 30,
                rgba: yapfast_core::chat::Pixels::new(40, 30, vec![128; 40 * 30 * 4]),
                ..Default::default()
            })),
            preview_failed: false,
            placement: yapfast_core::chat::Placement::Message,
            gif: None,
            spoiler: false,
            video: false,
            url: None,
        }],
        ..Default::default()
    })]);
    adapter.app.state.viewer = Some(yapfast_core::application::Viewer {
        item: "i0".into(),
        index: 0,
        pictures: HashMap::new(),
    });
    adapter.refresh(&window);
    assert_eq!(window.get_viewer_image().size().width, 40);
    adapter.app.state.viewer = None;
    adapter.refresh(&window);
    assert!(!window.get_viewer_open());
    assert_eq!(window.get_viewer_image().size().width, 0);
}

#[test]
fn a_pictures_menu_links_to_it_when_it_has_an_address() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    let file = |url: Option<&str>| yapfast_core::chat::Attachment {
        name: "a.png".into(),
        size: None,
        image: true,
        dimensions: None,
        preview: None,
        preview_failed: false,
        placement: yapfast_core::chat::Placement::Message,
        gif: None,
        spoiler: false,
        video: false,
        url: url.map(Into::into),
    };
    adapter.borrow_mut().app.state.messages = Arc::new(vec![Arc::new(ChatMessage {
        item: "i0".into(),
        attachments: vec![file(None), file(Some("https://cdn.example/a.png"))],
        ..Default::default()
    })]);
    assert_eq!(window.invoke_attachment_link("i0".into(), 0), "");
    assert_eq!(
        window.invoke_attachment_link("i0".into(), 1),
        "https://cdn.example/a.png"
    );
    assert_eq!(window.invoke_attachment_link("i0".into(), 2), "");
    assert_eq!(window.invoke_attachment_link("i0".into(), -1), "");
}

/// A Discord server with a category holding #general, and #rules outside
/// it, muted with its own level; Alice is "1".
#[cfg(feature = "discord")]
fn discord_server_adapter(
    window: &YapfastWindow,
    directory: &std::path::Path,
) -> Rc<RefCell<Adapter>> {
    use yapfast_discord::{
        notifications::{Level, Mute, Notifications as Settings, Override},
        ready,
        service::{Snapshot, Status},
    };
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.into(),
        window,
    )
    .unwrap();
    bind_callbacks(window, &adapter);
    let mut server = ready::server(&serde_json::json!({
        "id": "10", "owner_id": "99",
        "properties": {"name": "Test", "default_message_notifications": 1},
        "roles": [{"id": "10", "name": "@everyone", "position": 0, "permissions": "3072"}],
        "channels": [
            {"id": "4", "type": 4, "name": "Talk", "position": 0},
            {"id": "11", "type": 0, "name": "general", "position": 0, "parent_id": "4"},
            {"id": "12", "type": 0, "name": "rules", "position": 0},
        ],
    }))
    .unwrap();
    server.notifications = Arc::new(Settings {
        overrides: [(
            "12".to_owned(),
            Override {
                mute: Mute {
                    muted: true,
                    until: None,
                },
                level: Some(Level::Nothing),
                flags: 0,
            },
        )]
        .into(),
        ..Default::default()
    });
    server.text_channels[1].muted = true;
    adapter
        .borrow_mut()
        .app
        .apply(yapfast_core::chat::Event::Discord(Arc::new(Snapshot {
            status: Status::Online,
            user_id: Some("1".into()),
            servers: vec![Arc::new(server)],
            ..Default::default()
        })));
    adapter
}

#[test]
#[cfg(feature = "discord")]
fn the_bell_and_channel_menus_offer_the_same_notification_choices() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = discord_server_adapter(&window, directory.path());
    let rows = |window: &YapfastWindow| -> Vec<(String, String, bool)> {
        window
            .get_menu_rows()
            .iter()
            .map(|row| (row.text.into(), row.subtitle.into(), row.checked))
            .collect()
    };
    let row = |text: &str, subtitle: &str, checked| (text.into(), subtitle.into(), checked);

    // The header's bell: muting, with its times, then the levels.
    window.invoke_prepare_menu("notifications:discord:10:11".into(), false);
    assert_eq!(
        rows(&window),
        [
            row("Mute channel", "", false),
            row("Use category default", "Only @mentions", true),
            row("All messages", "", false),
            row("Only @mentions", "", false),
            row("Nothing", "", false),
        ]
    );
    let submenus = window.get_menu_submenus();
    assert_eq!(
        submenus.row_data(0).unwrap().row_count(),
        yapfast_core::application::MUTE_TIMES.len()
    );
    assert_eq!(submenus.row_data(1).unwrap().row_count(), 0);

    // A muted channel outside a category unmutes, and follows the server.
    window.invoke_prepare_menu("notifications:discord:10:12".into(), false);
    let shown = rows(&window);
    assert_eq!(shown[0], row("Unmute channel", "", false));
    assert_eq!(shown[1], row("Use server default", "Only @mentions", false));
    assert_eq!(shown[4], row("Nothing", "", true));
    assert_eq!(window.get_menu_submenus().row_count(), 0, "no row has one");

    // Its own menu has the same, the levels in a submenu.
    window.invoke_prepare_menu("discord:10:12".into(), false);
    let shown = rows(&window);
    assert!(shown.contains(&row("Unmute channel", "", false)));
    assert!(shown.contains(&row("Notification settings", "Nothing", false)));
    let at = shown
        .iter()
        .position(|shown| shown.0 == "Notification settings")
        .unwrap();
    let levels = window.get_menu_submenus().row_data(at).unwrap();
    assert_eq!(levels.row_count(), 4);
    assert!(levels.row_data(3).unwrap().checked);

    // A category's heading names its first channel; its menu is the same.
    assert_eq!(
        window.invoke_category_of("discord:10:11".into()),
        "discord:10:4"
    );
    assert_eq!(window.invoke_category_of("discord:10:12".into()), "");
    window.invoke_prepare_menu("discord:10:4".into(), false);
    let shown = rows(&window);
    assert_eq!(shown[0], row("Mute category", "", false));
    assert_eq!(
        shown[1],
        row("Notification settings", "Only @mentions", false)
    );

    // What each choice changes.
    use yapfast_core::chat::{Muting, NotificationChange, NotifyLevel};
    let adapter = adapter.borrow();
    let general = menus::Notify::of(&adapter, "discord:10:11").unwrap();
    assert_eq!(
        general.change("level-mentions"),
        Some(NotificationChange::Level(Some(NotifyLevel::Mentions)))
    );
    assert_eq!(
        general.change("level-default"),
        Some(NotificationChange::Level(None))
    );
    assert_eq!(
        general.change("mute-60"),
        Some(NotificationChange::Mute(Muting::Minutes(60)))
    );
    assert_eq!(general.change("channel-mute"), None, "its times choose");
    let rules = menus::Notify::of(&adapter, "discord:10:12").unwrap();
    assert_eq!(
        rules.change("channel-mute"),
        Some(NotificationChange::Mute(Muting::Off))
    );
    assert!(
        menus::Notify::of(&adapter, "discord:@me:30").is_none(),
        "a DM"
    );
}

#[test]
#[cfg(feature = "discord")]
fn a_server_search_names_the_server_and_heads_results_with_their_channels() {
    use yapfast_core::chat::{Event, SearchHit};
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = discord_server_adapter(&window, directory.path());
    let hit = |room: &str, item: &str| SearchHit {
        room: room.into(),
        message: ChatMessage {
            item: item.into(),
            body: "plan".into(),
            ..Default::default()
        },
    };
    {
        let mut adapter = adapter.borrow_mut();
        let app = &mut adapter.app;
        app.dispatch(Action::Navigate(Navigation::Server("discord:10".into())));
        app.dispatch(Action::SelectRoom("discord:10:11".into()));
        app.dispatch(Action::Search("plan".into()));
        app.apply(Event::Found {
            place: "discord:10".into(),
            query: "plan".into(),
            offset: 0,
            result: Ok((
                3,
                vec![
                    hit("discord:10:12", "d:9"),
                    hit("discord:10:12", "d:8"),
                    hit("discord:10:11", "d:7"),
                ],
            )),
        });
        adapter.refresh(&window);
    }
    assert_eq!(window.get_search_placeholder(), "Search Test");
    assert!(window.get_search_open());
    let rows: Vec<(String, String)> = window
        .get_search_results()
        .iter()
        .map(|row| (row.room.into(), row.heading.into()))
        .collect();
    assert_eq!(
        rows,
        [
            ("discord:10:12".into(), "rules".into()),
            ("discord:10:12".into(), String::new()),
            ("discord:10:11".into(), "general".into()),
        ]
    );
}

/// A window and adapter showing a Discord server, "10", where Alice, "1",
/// holds @everyone's permissions: a category "Talk" holding #general and
/// the Lounge, an empty category "Later", and one only Mods see, "Staff".
/// Her settings for the server hide muted channels and suppress role
/// mentions; #general mentions her.
#[cfg(feature = "discord")]
fn discord_server_window() -> (YapfastWindow, Rc<RefCell<Adapter>>, tempfile::TempDir) {
    discord_server_window_granting(0)
}

/// [`discord_server_window`], with everyone in the server allowed `extra`
/// permissions too.
#[cfg(feature = "discord")]
fn discord_server_window_granting(
    extra: u64,
) -> (YapfastWindow, Rc<RefCell<Adapter>>, tempfile::TempDir) {
    discord_server_window_changed(extra, |_| {})
}

/// [`discord_server_window_granting`], with `change` made to the server.
#[cfg(feature = "discord")]
fn discord_server_window_changed(
    extra: u64,
    change: impl FnOnce(&mut yapfast_core::yapfast_discord::ready::Server),
) -> (YapfastWindow, Rc<RefCell<Adapter>>, tempfile::TempDir) {
    use yapfast_core::yapfast_discord::{
        notifications::Notifications,
        ready::{self, permission},
        service::{Snapshot, Status},
    };
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    let bits = |bits: u64| bits.to_string();
    let hidden = serde_json::json!([{"id": "10", "type": 0, "allow": "0",
                                     "deny": bits(permission::VIEW_CHANNEL)}]);
    let mut server = ready::server(&serde_json::json!({
        "id": "10", "owner_id": "99",
        "properties": {"name": "Test", "default_message_notifications": 1},
        "roles": [{"id": "10", "name": "@everyone", "position": 0,
                   "permissions": bits(permission::VIEW_CHANNEL | permission::SEND_MESSAGES | extra)}],
        "channels": [
            {"id": "4", "type": 4, "name": "Talk", "position": 0},
            {"id": "5", "type": 4, "name": "Later", "position": 1},
            {"id": "6", "type": 4, "name": "Staff", "position": 2, "permission_overwrites": hidden},
            {"id": "11", "type": 0, "name": "general", "position": 0, "parent_id": "4"},
            {"id": "14", "type": 2, "name": "Lounge", "position": 1, "parent_id": "4"},
        ],
    }))
    .unwrap();
    server.notifications = Arc::new(Notifications {
        hide_muted_channels: true,
        suppress_roles: true,
        ..Default::default()
    });
    server.text_channels[0].mentions = 1;
    change(&mut server);
    adapter
        .borrow_mut()
        .app
        .apply(Event::Discord(Arc::new(Snapshot {
            status: Status::Online,
            user_id: Some("1".into()),
            servers: vec![Arc::new(server)],
            ..Default::default()
        })));
    (window, adapter, directory)
}

#[test]
#[cfg(feature = "discord")]
fn a_discord_servers_menus_show_and_change_its_notifications() {
    use yapfast_core::chat::{Muting, NotificationChange, NotifyLevel};
    let (window, adapter, _directory) = discord_server_window();
    let texts = |rows: ModelRc<MenuRow>| -> Vec<String> {
        rows.iter().map(|row| row.text.to_string()).collect()
    };
    window.invoke_prepare_menu("discord:10".into(), false);
    let rows = window.get_menu_rows();
    assert_eq!(
        texts(rows.clone()),
        [
            "Mark as read",
            "Mute server",
            "Notification settings",
            "Hide muted channels",
            "Edit server profile",
            "Leave server",
            "Copy link",
            "Copy ID"
        ]
    );
    let notifications = rows.row_data(2).unwrap();
    assert_eq!(
        notifications.subtitle, "Only @mentions",
        "the server's default"
    );
    let hide = rows.row_data(3).unwrap();
    assert!(hide.checkbox && hide.checked);
    let submenus = window.get_menu_submenus();
    assert_eq!(
        submenus.row_data(1).unwrap().row_count(),
        6,
        "the mute times"
    );
    let levels = submenus.row_data(2).unwrap();
    assert_eq!(
        texts(levels.clone()),
        [
            "All messages",
            "Only @mentions",
            "Nothing",
            "Suppress @everyone and @here",
            "Suppress all role @mentions"
        ]
    );
    let checked: Vec<bool> = levels.iter().map(|row| row.checked).collect();
    assert_eq!(checked, [false, true, false, false, true]);

    // The header's dropdown has the same rows, in Discord's order.
    window.invoke_prepare_menu("header:discord:10".into(), false);
    assert_eq!(
        texts(window.get_menu_rows()),
        [
            "Notification settings",
            "Edit server profile",
            "Hide muted channels",
            "Leave server"
        ]
    );
    assert_eq!(window.get_header_server(), "", "until the server is open");
    window.invoke_navigate_server("discord:10".into());
    assert_eq!(window.get_header_server(), "discord:10");

    // Each choice is the change it names; a switch turns over.
    let app = &adapter.borrow().app;
    let change = |id: &str| menus::server_change(app, "discord:10", id);
    assert_eq!(
        change("mute-60"),
        Some(NotificationChange::Mute(Muting::Minutes(60)))
    );
    assert_eq!(
        change("unmute-server"),
        Some(NotificationChange::Mute(Muting::Off))
    );
    assert_eq!(
        change("level-nothing"),
        Some(NotificationChange::Level(Some(NotifyLevel::Nothing)))
    );
    assert_eq!(
        change("suppress-roles"),
        Some(NotificationChange::SuppressRoles(false))
    );
    assert_eq!(
        change("suppress-everyone"),
        Some(NotificationChange::SuppressEveryone(true))
    );
    assert_eq!(
        change("hide-muted"),
        Some(NotificationChange::HideMuted(false))
    );
    assert_eq!(change("copy-id"), None);
}

#[test]
#[cfg(feature = "discord")]
fn discord_categories_head_their_channels_and_open_their_menus() {
    let (window, adapter, _directory) = discord_server_window();
    window.invoke_navigate_server("discord:10".into());
    let rows: Vec<RoomRow> = adapter.borrow().models.rooms.iter().collect();
    let shown: Vec<(&str, String, bool)> = rows
        .iter()
        .map(|row| {
            let category = window.invoke_category_of(row.id.clone()).to_string();
            (row.heading.as_str(), category, row.hidden)
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("TALK", "discord:10:4".into(), false),
            ("", "discord:10:4".into(), false),
            ("LATER", "discord:10:5".into(), true),
        ],
        "an empty category shows its heading alone, named by the category; \
         Staff is hidden from her"
    );
    window.invoke_prepare_menu("discord:10:4".into(), false);
    let texts: Vec<String> = window
        .get_menu_rows()
        .iter()
        .map(|row| row.text.to_string())
        .collect();
    assert_eq!(
        texts,
        [
            "Mark as read",
            "Mute category",
            "Notification settings",
            "Copy ID"
        ]
    );
}

#[test]
#[cfg(feature = "discord")]
fn pinned_channels_leave_their_categories_for_the_top() {
    use yapfast_core::yapfast_discord::notifications::{Notifications, Override};
    let (window, adapter, _directory) = discord_server_window_changed(0, |server| {
        let mut settings = Notifications::clone(&server.notifications);
        settings.overrides.insert(
            "11".into(),
            Override {
                flags: 6144,
                ..Default::default()
            },
        );
        server.notifications = Arc::new(settings);
    });
    {
        // A category of that name folding doesn't fold the pinned ones.
        adapter
            .borrow_mut()
            .layout
            .collapsed
            .entry("discord:10".into())
            .or_default()
            .insert("PINNED CHANNELS".into());
    }
    window.invoke_navigate_server("discord:10".into());
    let rows: Vec<RoomRow> = adapter.borrow().models.rooms.iter().collect();
    let shown: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                row.heading.as_str(),
                row.name.as_str(),
                row.pinned,
                row.collapsed,
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("PINNED CHANNELS", "general", true, false),
            ("TALK", "Lounge", false, false),
            ("LATER", "", false, false),
        ]
    );
    window.invoke_prepare_menu("discord:10:11".into(), false);
    let texts: Vec<String> = window
        .get_menu_rows()
        .iter()
        .map(|row| row.text.to_string())
        .collect();
    assert_eq!(
        texts,
        [
            "Mark as read",
            "Unpin channel",
            "Copy link",
            "Mute channel",
            "Notification settings",
            "Copy ID"
        ]
    );
}

#[test]
fn empty_categories_go_where_discords_order_puts_them() {
    let category = |id: &str, empty: bool| ListedCategory {
        id: id.into(),
        name: id.into(),
        empty,
    };
    let row = |heading: &str, id: &str| RoomRow {
        id: id.into(),
        heading: heading.into(),
        ..Default::default()
    };
    // Each heading names its category's first channel.
    let rows = vec![row("", "x"), row("B", "b1"), row("", "y"), row("D", "d1")];
    let categories = [
        category("a", true),
        category("b", false),
        category("c", true),
        category("d", false),
        category("e", true),
    ];
    let category_of = |id: &str| id.strip_suffix('1').map(str::to_owned);
    let listed: Vec<String> = refresh::with_empty_categories(rows, &categories, category_of)
        .iter()
        .map(|row| format!("{}{}", row.heading, if row.hidden { "!" } else { "" }))
        .collect();
    assert_eq!(listed, ["", "A!", "B", "", "C!", "D", "E!"]);
}

#[test]
#[cfg(feature = "discord")]
fn the_invite_dialog_shows_its_link_and_how_long_it_lasts() {
    let (window, adapter, _directory) = discord_server_window();
    let show = |invite: yapfast_core::application::InviteDialog| {
        let mut adapter = adapter.borrow_mut();
        adapter.app.state.invite = Some(invite);
        adapter.refresh(&window);
    };
    let invite = yapfast_core::application::InviteDialog {
        server: "Test".into(),
        room: "discord:10:14".into(),
        channel: "Lounge".into(),
        voice: true,
        max_age: 43_200,
        max_uses: 25,
        link: None,
        making: true,
        error: None,
    };
    show(invite.clone());
    assert!(window.get_invite_open());
    let prompt = window.get_invite_prompt();
    assert_eq!(prompt.title, "Invite friends to Test");
    assert_eq!(prompt.detail, "They'll land in Lounge.");
    assert!(prompt.making && prompt.link.is_empty() && prompt.note.is_empty());
    assert_eq!(
        (window.get_invite_age(), window.get_invite_uses()),
        (3, 4),
        "its choices"
    );

    // Choosing others stays while the dialog is open.
    window.set_invite_age(6);
    show(yapfast_core::application::InviteDialog {
        link: Some("https://discord.gg/abc".into()),
        making: false,
        ..invite.clone()
    });
    let prompt = window.get_invite_prompt();
    assert_eq!(prompt.link, "https://discord.gg/abc");
    assert_eq!(
        prompt.note,
        "This link expires in 12 hours and works 25 times."
    );
    assert_eq!(window.get_invite_age(), 6);

    show(yapfast_core::application::InviteDialog {
        max_age: 0,
        max_uses: 1,
        channel: "general".into(),
        voice: false,
        link: Some("https://discord.gg/abc".into()),
        making: false,
        ..invite
    });
    let prompt = window.get_invite_prompt();
    assert_eq!(prompt.note, "This link never expires and works once.");
    assert_eq!(prompt.detail, "They'll land in #general.");
    window.invoke_close_invite();
    assert!(!window.get_invite_open());
}

#[test]
#[cfg(feature = "discord")]
fn channel_menus_and_their_dialog_make_change_and_delete_channels() {
    use yapfast_core::{
        application::{ChannelAsk, ChannelDialog},
        chat::{ChannelKind, ChannelSettings},
        yapfast_discord::ready::permission,
    };
    let (window, adapter, _directory) = discord_server_window_granting(permission::MANAGE_CHANNELS);
    let texts = |rows: ModelRc<MenuRow>| -> Vec<String> {
        rows.iter().map(|row| row.text.to_string()).collect()
    };
    window.invoke_prepare_menu("discord:10:14".into(), false);
    let rows = window.get_menu_rows();
    assert_eq!(
        texts(rows.clone()),
        [
            "Join voice",
            "Pin channel to top",
            "Copy link",
            "Edit channel",
            "Duplicate channel",
            "Create voice channel",
            "Delete channel",
            "Copy ID"
        ]
    );
    let delete = rows.row_data(6).unwrap();
    assert!(delete.danger && delete.separated);
    window.invoke_prepare_menu("discord:10:4".into(), false);
    assert_eq!(
        texts(window.get_menu_rows())[1..],
        [
            "Mute category",
            "Notification settings",
            "Edit category",
            "Delete category",
            "Copy ID"
        ]
    );
    window.invoke_prepare_menu("header:discord:10".into(), false);
    assert_eq!(
        texts(window.get_menu_rows())[..2],
        ["Create channel", "Create category"]
    );

    // Choosing one opens its dialog.
    window.invoke_choose_menu("discord:10".into(), false, "create-channel".into());
    assert!(window.get_channel_open());
    let prompt = window.get_channel_prompt();
    assert_eq!(prompt.title, "Create channel");
    assert_eq!(prompt.detail, "In Test");
    assert!(prompt.kinds && prompt.named && !prompt.settings && !prompt.danger);
    window.set_channel_name("Plans".into());
    window.set_channel_kind(1);
    window.invoke_save_channel();
    {
        let adapter = adapter.borrow();
        let dialog = adapter.app.state.channel_dialog.as_ref().unwrap();
        assert!(dialog.saving, "{dialog:?}");
        assert_eq!(dialog.kind, ChannelKind::Voice);
    }
    window.invoke_close_channel();
    assert!(!window.get_channel_open());

    let show = |dialog: ChannelDialog| {
        let mut adapter = adapter.borrow_mut();
        adapter.app.state.channel_dialog = Some(dialog);
        adapter.refresh(&window);
    };
    let edit = ChannelDialog {
        ask: ChannelAsk::Edit("discord:10:11".into()),
        server: "discord:10".into(),
        server_name: "Test".into(),
        name: "general".into(),
        kind: ChannelKind::Text,
        settings: None,
        saving: false,
        error: None,
    };
    show(edit.clone());
    let prompt = window.get_channel_prompt();
    assert!(prompt.loading && prompt.settings);
    assert_eq!(prompt.confirm, "Save changes");
    // Its fields fill once Discord sent its settings.
    show(ChannelDialog {
        settings: Some(ChannelSettings {
            name: "general".into(),
            topic: "Say hi".into(),
            slowmode: 300,
            nsfw: true,
        }),
        ..edit.clone()
    });
    assert!(!window.get_channel_prompt().loading);
    assert_eq!(window.get_channel_topic(), "Say hi");
    assert_eq!(window.get_channel_slowmode(), 7);
    assert!(window.get_channel_nsfw());
    // What's written there stays while the dialog is open.
    window.set_channel_topic("Say hello".into());
    adapter.borrow_mut().refresh(&window);
    assert_eq!(window.get_channel_topic(), "Say hello");

    show(ChannelDialog {
        ask: ChannelAsk::Delete("discord:10:4".into()),
        name: "Talk".into(),
        kind: ChannelKind::Category,
        settings: Some(ChannelSettings::default()),
        ..edit
    });
    let prompt = window.get_channel_prompt();
    assert_eq!(prompt.title, "Delete category");
    assert_eq!(
        prompt.detail,
        "Are you sure you want to delete Talk? This cannot be undone."
    );
    assert!(prompt.danger && !prompt.named);
}

#[test]
#[cfg(feature = "discord")]
fn leaving_a_server_asks_first_in_red() {
    let (window, adapter, _directory) = discord_server_window();
    window.invoke_prepare_menu("discord:10".into(), false);
    let leave = window
        .get_menu_rows()
        .iter()
        .find(|row| row.text == "Leave server")
        .unwrap();
    assert!(leave.danger && leave.separated);
    window.invoke_choose_menu("discord:10".into(), false, "leave-server".into());
    assert!(window.get_leave_open());
    let prompt = window.get_leave_prompt();
    assert_eq!(prompt.title, "Leave 'Test'");
    assert_eq!(prompt.confirm, "Leave server");
    assert!(prompt.danger && !prompt.named && !prompt.saving);
    window.invoke_leave_server();
    assert!(adapter.borrow().app.state.leave.as_ref().unwrap().leaving);
    window.invoke_close_leave();
    assert!(!window.get_leave_open());
}

#[test]
#[cfg(feature = "discord")]
fn edit_server_profile_opens_the_per_server_profile_editor() {
    use yapfast_core::yapfast_discord::ready::permission;
    let (window, adapter, _directory) = discord_server_window_granting(permission::CHANGE_NICKNAME);
    window.invoke_prepare_menu("header:discord:10".into(), false);
    let texts: Vec<String> = window
        .get_menu_rows()
        .iter()
        .map(|row| row.text.to_string())
        .collect();
    assert!(
        texts.contains(&"Edit server profile".to_owned()),
        "{texts:?}"
    );
    window.invoke_choose_menu(
        "header:discord:10".into(),
        false,
        "edit-server-profile".into(),
    );
    assert!(window.get_server_profile_open());
    assert!(window.get_server_profile_prompt().loading);
    {
        let mut adapter = adapter.borrow_mut();
        adapter.app.apply(Event::ServerProfile {
            server: "discord:10".into(),
            result: Ok(yapfast_core::chat::ServerProfile {
                pronouns: "she/her".into(),
                main: yapfast_core::chat::Profile {
                    name: "Alice".into(),
                    handle: "alice".into(),
                    bio: "Main bio".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        });
        adapter.refresh(&window);
    }
    let prompt = window.get_server_profile_prompt();
    assert!(!prompt.loading && prompt.may_rename && !prompt.nitro && !prompt.changed);
    assert_eq!(window.get_server_profile_pronouns(), "she/her");
    assert_eq!(prompt.nick_placeholder, "alice");
    assert_eq!(prompt.preview.name, "Alice", "the main profile's name");
    assert_eq!(prompt.preview.handle, "alice · she/her");
    assert!(prompt.preview.has_bio, "the main profile's bio");
    assert_eq!(prompt.bio_left, 190);

    // Typing changes the preview and nothing else.
    window.set_server_profile_nick("Ally".into());
    window.invoke_write_server_profile(0, "Ally".into());
    let prompt = window.get_server_profile_prompt();
    assert_eq!(prompt.preview.name, "Ally");
    assert!(prompt.changed);
    window.invoke_write_server_profile(0, "x".repeat(33).into());
    assert_eq!(
        window.get_server_profile_prompt().too_long,
        "A nickname can be up to 32 characters."
    );

    window.invoke_close_server_profile();
    assert!(!window.get_server_profile_open());
    assert_eq!(window.get_server_profile_prompt().preview.name, "");
}

#[test]
fn a_threads_activity_reads_as_discords_list_of_threads_says_it() {
    let now = 100 * 86_400_000;
    let said = |ago: u64| refresh::ago(now - ago, now);
    assert_eq!(said(5_000), "5s ago");
    assert_eq!(said(5 * 60_000), "5m ago");
    assert_eq!(said(3 * 3_600_000), "3h ago");
    assert_eq!(said(30 * 86_400_000), "30d ago");
    assert_eq!(said(31 * 86_400_000), ">30d ago");
    assert_eq!(refresh::ago(0, 400 * 86_400_000), ">1y ago");
    assert_eq!(refresh::ago(now + 1_000, now), "0s ago", "a clock behind");
}

#[test]
fn a_theme_paints_the_window_and_the_mentions_in_its_rows() {
    i_slint_backend_testing::init_no_event_loop();
    let window = YapfastWindow::new().unwrap();
    assert_eq!(
        theme::Theme::shown(&window),
        theme::YAP,
        "the Slint defaults are Yap's, for the first frame"
    );
    let message = ChatMessage {
        body: "hi @Bob".into(),
        formatted: true,
        mentions: vec![yapfast_core::mention::Mentioned {
            text: "@Bob".into(),
            target: yapfast_core::mention::Target::User("2".into()),
            color: None,
        }],
        ..Default::default()
    };
    let yap_row = message_row_plain(&message);
    let light = theme::Theme {
        dark: false,
        chat: slint::Color::from_rgb_u8(0xfa, 0xf8, 0xf5),
        link: slint::Color::from_rgb_u8(0x1a, 0x5f, 0xb4),
        ..theme::YAP
    };
    assert!(light.apply(&window), "mentions change colour");
    assert_eq!(theme::Theme::shown(&window), light);
    assert!(message_row_plain(&message).styled != yap_row.styled);
    assert!(!light.apply(&window), "nothing to build again");
    assert!(theme::YAP.apply(&window));
    assert!(message_row_plain(&message).styled == yap_row.styled);
}

#[test]
fn a_chosen_theme_is_kept_and_survives_a_window_made_again() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    assert_eq!(window.get_theme_id(), "yap");
    assert_eq!(window.get_themes().row_count(), theme::BUILT_IN.len());
    let nord = theme::find("nord").unwrap().theme;
    window.invoke_choose_theme("nord".into());
    assert_eq!(window.get_theme_id(), "nord");
    assert_eq!(theme::Theme::shown(&window), nord);
    assert_eq!(
        Layout::read(directory.path()).theme.as_deref(),
        Some("nord")
    );

    // Closing to the tray drops the window; the next one starts in Nord.
    let again = YapfastWindow::new().unwrap();
    adapter.borrow_mut().attach(&again);
    assert_eq!(theme::Theme::shown(&again), nord);
    assert_eq!(again.get_theme_id(), "nord");

    // An id Yapfast doesn't know changes nothing, and a stored one it no longer
    // has shows Yap.
    again.invoke_choose_theme("missing".into());
    assert_eq!(theme::Theme::shown(&again), nord);
    let fresh = YapfastWindow::new().unwrap();
    Layout {
        theme: Some("gone".into()),
        ..Default::default()
    }
    .apply(&fresh);
    assert_eq!(theme::Theme::shown(&fresh), theme::YAP);
    assert_eq!(fresh.get_theme_id(), "yap");
}

#[test]
fn built_in_themes_have_their_own_ids_and_say_whether_they_are_dark() {
    let mut ids = std::collections::HashSet::new();
    for built_in in &theme::BUILT_IN {
        assert!(ids.insert(built_in.id), "{} twice", built_in.id);
        let chat = built_in.theme.chat;
        let light =
            u32::from(chat.red()) + u32::from(chat.green()) + u32::from(chat.blue()) > 3 * 128;
        assert_eq!(built_in.theme.dark, !light, "{}", built_in.name);
    }
    assert_eq!(theme::BUILT_IN[0].theme, theme::YAP, "Yap comes first");
}

#[test]
fn a_colour_is_written_as_slint_writes_one() {
    let parse = |text| theme::parse(text).map(theme::hex);
    assert_eq!(parse("#abc").as_deref(), Some("#aabbcc"));
    assert_eq!(parse(" #A0B1C2 ").as_deref(), Some("#a0b1c2"));
    assert_eq!(parse("#ffbf4726").as_deref(), Some("#ffbf4726"));
    assert_eq!(parse("#123456ff").as_deref(), Some("#123456"));
    for wrong in [
        "", "#", "abc", "a0b1c2", "#ab", "#abcd", "#12345", "#1234567", "#ggg", "#+12", "#-12345",
    ] {
        assert_eq!(theme::parse(wrong), None, "{wrong:?}");
    }
    assert_eq!(theme::parse("#ffbf4726"), Some(theme::YAP.accent_soft));
}

#[test]
fn a_custom_theme_keeps_its_colours_by_name_and_falls_back_to_its_base() {
    let stored = serde_json::json!({
        "id": "custom:1",
        "name": "Mine",
        "base": "nord",
        "dark": false,
        "colours": {
            "chat": "#fafafa",
            "row-hover": "#abc",
            "link": "not a colour",
            "a-colour-from-later": "#123456",
        },
    });
    let custom: theme::Custom = serde_json::from_value(stored).unwrap();
    let nord = theme::find("nord").unwrap().theme;
    assert_eq!(custom.base, "nord");
    assert!(!custom.theme.dark);
    assert_eq!(custom.theme.chat, theme::parse("#fafafa").unwrap());
    assert_eq!(custom.theme.row_hover, theme::parse("#aabbcc").unwrap());
    assert_eq!(custom.theme.link, nord.link, "a wrong colour is Nord's");
    assert_eq!(
        custom.theme.accent, nord.accent,
        "a missing colour is Nord's"
    );

    // It's written back with every colour, by the names ui/theme.slint has.
    let written = serde_json::to_value(&custom).unwrap();
    assert_eq!(written["colours"]["row-hover"], "#aabbcc");
    assert_eq!(written["colours"]["accent-soft"], "#88c0d026");
    assert_eq!(
        written["colours"].as_object().unwrap().len(),
        theme::Theme::NAMES.len()
    );
    assert_eq!(
        serde_json::from_value::<theme::Custom>(written).unwrap(),
        custom
    );

    // A base Yapfast doesn't know is Yap.
    let unknown: theme::Custom =
        serde_json::from_value(serde_json::json!({ "base": "gone" })).unwrap();
    assert_eq!((unknown.base, unknown.theme), ("yap", theme::YAP));
}

#[test]
fn custom_theme_ids_stay_apart_from_built_in_ones_and_each_other() {
    for built_in in &theme::BUILT_IN {
        assert!(!built_in.id.contains(':'), "{}", built_in.id);
    }
    let custom = |id: &str| theme::Custom {
        id: id.into(),
        name: id.into(),
        base: "yap",
        theme: theme::YAP,
    };
    let mut themes = vec![
        custom("custom:2"),
        custom("nord"),
        custom("custom:2"),
        custom(""),
        custom("custom:x"),
    ];
    theme::tidy(&mut themes);
    let ids: Vec<_> = themes.iter().map(|theme| theme.id.as_str()).collect();
    assert_eq!(
        ids,
        ["custom:2", "custom:3", "custom:4", "custom:5", "custom:6"]
    );
    assert_eq!(theme::new_id(&themes), "custom:7");
    assert_eq!(theme::new_id(&[]), "custom:1");
}

#[test]
fn a_custom_theme_is_made_edited_and_deleted_in_appearance() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    let nord = theme::find("nord").unwrap().theme;
    window.invoke_choose_theme("nord".into());

    // A new theme starts as a copy of the one shown, and shows each change.
    window.invoke_new_theme();
    assert!(window.get_theme_editor());
    assert_eq!(window.get_theme_name(), "Nord copy");
    assert_eq!(window.get_theme_start(), 4);
    assert_eq!(window.get_theme_starts().row_count(), theme::BUILT_IN.len());
    let colours = window.get_theme_colours();
    assert_eq!(colours.row_count(), theme::Theme::NAMES.len());
    let chat = theme::Theme::NAMES
        .iter()
        .position(|name| *name == "chat")
        .unwrap();
    assert_eq!(colours.row_data(chat).unwrap().hex, theme::hex(nord.chat));
    window.invoke_theme_colour_edited(chat as i32, "#102030".into());
    assert_eq!(
        theme::Theme::shown(&window).chat,
        theme::parse("#102030").unwrap()
    );
    assert!(window.get_theme_can_save());

    // A field without a colour says so and holds Save back; the window keeps
    // the last colour.
    window.invoke_theme_colour_edited(chat as i32, "#1020".into());
    assert!(colours.row_data(chat).unwrap().wrong);
    assert!(!window.get_theme_can_save());
    assert_eq!(
        theme::Theme::shown(&window).chat,
        theme::parse("#102030").unwrap()
    );
    window.invoke_save_theme();
    assert!(window.get_theme_editor(), "Save waits for a colour");
    window.invoke_theme_colour_edited(chat as i32, "#123".into());
    window.invoke_theme_dark_toggled(false);
    window.set_theme_name("  Midnight  ".into());
    window.invoke_save_theme();

    // Saved, it's chosen, listed after the built-in themes and kept.
    assert!(!window.get_theme_editor());
    assert_eq!(
        window.get_theme_colours().row_count(),
        0,
        "rows go with the editor"
    );
    assert_eq!(window.get_theme_id(), "custom:1");
    let themes = window.get_themes();
    assert_eq!(themes.row_count(), theme::BUILT_IN.len() + 1);
    let card = themes.row_data(theme::BUILT_IN.len()).unwrap();
    assert_eq!(
        (card.id.as_str(), card.name.as_str(), card.custom),
        ("custom:1", "Midnight", true)
    );
    let midnight = theme::Theme {
        dark: false,
        chat: theme::parse("#112233").unwrap(),
        ..nord
    };
    assert_eq!(theme::Theme::shown(&window), midnight);
    let layout = Layout::read(directory.path());
    assert_eq!(layout.theme.as_deref(), Some("custom:1"));
    assert_eq!(layout.custom_themes.len(), 1);
    assert_eq!(layout.custom_themes[0].theme, midnight);
    assert_eq!(layout.custom_themes[0].base, "nord");

    // A window made again starts in it.
    let again = YapfastWindow::new().unwrap();
    bind_callbacks(&again, &adapter);
    adapter.borrow_mut().attach(&again);
    assert_eq!(theme::Theme::shown(&again), midnight);
    assert_eq!(again.get_theme_id(), "custom:1");
    let window = again;

    // Cancel puts back the theme from before editing, and the name.
    window.invoke_edit_theme("custom:1".into());
    assert!(window.get_theme_editing_saved());
    assert_eq!(window.get_theme_name(), "Midnight");
    window.invoke_theme_colour_edited(chat as i32, "#ff0000".into());
    window.set_theme_name("Red".into());
    assert_eq!(
        theme::Theme::shown(&window).chat,
        theme::parse("#ff0000").unwrap()
    );
    window.invoke_cancel_theme();
    assert_eq!(theme::Theme::shown(&window), midnight);
    assert_eq!(adapter.borrow().layout.custom_themes[0].name, "Midnight");

    // A new theme from another start takes its colours and name.
    window.invoke_new_theme();
    assert_eq!(window.get_theme_name(), "Midnight copy");
    window.invoke_theme_start_chosen(9);
    assert_eq!(window.get_theme_name(), "Sakura copy");
    assert_eq!(
        theme::Theme::shown(&window),
        theme::find("sakura").unwrap().theme
    );
    window.set_theme_name("Blossom".into());
    window.invoke_theme_start_chosen(0);
    assert_eq!(window.get_theme_name(), "Blossom", "a name given stays");
    window.invoke_cancel_theme();
    assert_eq!(theme::Theme::shown(&window), midnight);

    // Renamed, then deleted while chosen: Yap shows.
    window.invoke_edit_theme("custom:1".into());
    window.set_theme_name("Dusk".into());
    window.invoke_save_theme();
    assert_eq!(
        window
            .get_themes()
            .row_data(theme::BUILT_IN.len())
            .unwrap()
            .name,
        "Dusk"
    );
    window.invoke_edit_theme("custom:1".into());
    window.invoke_delete_theme();
    assert!(!window.get_theme_editor());
    assert_eq!(window.get_theme_id(), "yap");
    assert_eq!(theme::Theme::shown(&window), theme::YAP);
    assert_eq!(window.get_themes().row_count(), theme::BUILT_IN.len());
    let layout = Layout::read(directory.path());
    assert_eq!(layout.theme.as_deref(), Some("yap"));
    assert!(layout.custom_themes.is_empty());
}

#[test]
fn a_window_made_again_while_editing_drops_the_edit() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_new_theme();
    let link = theme::Theme::NAMES
        .iter()
        .position(|name| *name == "link")
        .unwrap();
    window.invoke_theme_colour_edited(link as i32, "#ff0000".into());
    assert_eq!(theme::mention_color(), 0xff0000);

    let again = YapfastWindow::new().unwrap();
    adapter.borrow_mut().attach(&again);
    assert!(adapter.borrow().theme_editor.is_none());
    assert!(
        adapter.borrow().restyle,
        "rows lose the edited mention colour"
    );
    assert!(!again.get_theme_editor());
    assert_eq!(theme::Theme::shown(&again), theme::YAP);
    assert_eq!(theme::mention_color(), 0x7cc4ff);
    assert!(Layout::read(directory.path()).custom_themes.is_empty());
}

#[test]
fn a_reactions_tooltip_names_up_to_three_as_discords_client_does() {
    let names = |list: &[&str]| {
        list.iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    let tip = |list: &[&str], count| messages::reaction_tip(":tada:", &names(list), count);
    assert_eq!(tip(&["Ann"], 1), ":tada: reacted by Ann");
    assert_eq!(tip(&["Ann"], 2), ":tada: reacted by Ann and 1 other");
    assert_eq!(tip(&["Ann", "Bo"], 2), ":tada: reacted by Ann and Bo");
    assert_eq!(
        tip(&["Ann", "Bo"], 5),
        ":tada: reacted by Ann, Bo, and 3 others"
    );
    assert_eq!(
        tip(&["Ann", "Bo", "Cy"], 3),
        ":tada: reacted by Ann, Bo, and Cy"
    );
    assert_eq!(
        tip(&["Ann", "Bo", "Cy"], 4),
        ":tada: reacted by Ann, Bo, Cy, and 1 other"
    );
    // Discord's names for emoji: a word, not `:+1:`, and a custom one's own.
    assert_eq!(emoji::shortcode("👍"), ":thumbsup:");
    assert_eq!(emoji::shortcode("🎉"), ":tada:");
    assert_eq!(emoji::shortcode(":talk:"), ":talk:");
}

#[test]
fn the_thread_beside_keeps_its_rows_when_a_new_layout_builds_them_again() {
    i_slint_backend_testing::init_no_event_loop();
    let directory = tempfile::tempdir().unwrap();
    let window = YapfastWindow::new().unwrap();
    let adapter = Adapter::new(
        Arc::new(BackgroundRuntime::new().unwrap()),
        directory.path().into(),
        &window,
    )
    .unwrap();
    bind_callbacks(&window, &adapter);
    window.invoke_explore_preview();
    let messages: Vec<_> = (0..3)
        .map(|n| {
            Arc::new(ChatMessage {
                id: format!("t{n}"),
                item: format!("t{n}"),
                author: "Ann".into(),
                body: format!("Reply {n}"),
                ..Default::default()
            })
        })
        .collect();
    {
        let mut adapter = adapter.borrow_mut();
        let aside = adapter.app.state.aside.insert(Default::default());
        aside.room = "thread".into();
        aside.ready = true;
        aside.messages = Arc::new(messages);
    }
    adapter.borrow_mut().refresh(&window);
    assert_eq!(window.get_aside_messages().row_count(), 3);
    // As a theme's colours do, which the rows carry.
    adapter.borrow_mut().restyle = true;
    adapter.borrow_mut().refresh(&window);
    assert_eq!(window.get_aside_messages().row_count(), 3);
}
