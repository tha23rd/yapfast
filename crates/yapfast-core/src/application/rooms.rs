//! Moving between servers, DMs and rooms, and starting DMs.

use super::*;
use crate::chat::Muting;

/// What a conversation's or server's menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuChoice {
    MarkRead,
    JoinVoice,
    /// The other person's profile, in a one-to-one DM.
    Profile,
    /// Open the DM with someone, starting it if need be.
    Message,
    /// Send someone a Discord friend request, accept theirs, or cancel
    /// this account's.
    AddFriend,
    AcceptFriend,
    CancelFriendRequest,
    /// Keep a DM at the top of the DM list on this device, or stop.
    Pin,
    Unpin,
    /// Start a DM's call, or join the one going on.
    Call,
    /// Write this account's note on a Discord DM's other person, or the
    /// nickname it gives them as a friend.
    AddNote,
    AddNickname,
    EditNickname,
    /// Close a one-to-one Discord DM.
    CloseDm,
    /// Send someone an invite to one of this account's Discord servers, in
    /// a submenu of them; see `places.rs`.
    InviteToServer,
    Unfriend,
    Ignore,
    Unignore,
    Block,
    Unblock,
    /// Mute a Discord DM for one of [`MUTE_TIMES`], or unmute it.
    Mute,
    Unmute,
    /// Copy a link to it that its network's apps open.
    CopyLink,
    /// Copy its id as its network knows it.
    CopyId,
    // A Discord server member's; see `members.rs`.
    /// Mention them in the open channel's composer.
    Mention,
    /// Change their nickname in the server, or this account's own.
    ServerNickname,
    /// Give them roles or take them away, in a submenu of the server's.
    Roles,
    Timeout,
    RemoveTimeout,
    Kick,
    Ban,
    // A member's in a voice channel, as its menu there offers them.
    /// Mute someone for this account alone, as a box to tick.
    LocalMute,
    /// This account's own mute and deafen in the call it's in here.
    SelfMute,
    SelfDeafen,
    /// Move them to another voice channel, in a submenu of those.
    MoveTo,
    /// Mute or deafen them for everyone, as boxes to tick, or take them
    /// out of voice.
    ServerMute,
    ServerDeafen,
    Disconnect,
    // A Discord server channel's or category's notifications: muting it for
    // one of [`MUTE_TIMES`], or unmuting it, and which of its messages notify.
    ChannelMute,
    ChannelNotifications,
    // A Discord server's, category's and channel's own; see `places`.
    /// Mute a Discord server for one of [`MUTE_TIMES`], or unmute it.
    MuteServer,
    UnmuteServer,
    /// A Discord server's notification level and its switches, in a submenu.
    ServerNotifications,
    /// A box to tick that keeps a Discord server's muted channels out of
    /// its channel list.
    HideMuted,
    /// Make an invite link to a Discord server or channel.
    Invite,
    /// Pin a Discord server's channel to the top of its channel list, for
    /// this account wherever it signs in, or unpin it.
    PinChannel,
    UnpinChannel,
    /// Make a channel in a Discord server, or one like a channel in its
    /// category; make a category.
    CreateChannel,
    CreateCategory,
    /// Change a Discord channel or category, copy a channel, or delete one.
    EditChannel,
    DuplicateChannel,
    DeleteChannel,
    /// Leave a Discord server this account doesn't own.
    LeaveServer,
    /// Join a Discord thread this account isn't in, or leave one it is.
    JoinThread,
    LeaveThread,
    /// Change this account's nickname in a Discord server, through a
    /// member's nickname dialog.
    EditServerProfile,
}

/// How long [`MenuChoice::Mute`] mutes for, as Discord's client offers.
pub const MUTE_TIMES: [Muting; 6] = [
    Muting::Minutes(15),
    Muting::Minutes(60),
    Muting::Minutes(3 * 60),
    Muting::Minutes(8 * 60),
    Muting::Minutes(24 * 60),
    Muting::Always,
];

impl AppState {
    /// What the menu of a listed server or conversation offers, in order;
    /// nothing for one that isn't listed.
    pub fn menu_choices(&self, id: &str) -> Vec<MenuChoice> {
        if self.servers.iter().any(|server| server.id == id) {
            self.server_choices(id)
        } else if self.is_category(id) {
            self.notification_choices()
        } else {
            self.room_choices(id)
        }
    }

    /// Whether an id names a category of a listed Discord server: a room id
    /// in the server that names none of its rooms. Only a category's heading
    /// asks for a menu by one.
    pub fn is_category(&self, id: &str) -> bool {
        discord::parse_room(id).is_some_and(|(guild, _)| {
            guild != discord::DM_GUILD
                && self
                    .servers
                    .iter()
                    .any(|server| discord::parse_server(&server.id) == Some(guild))
                && !self.rooms.iter().any(|room| room.id == id)
        })
    }

    /// A Discord server channel's or category's notification choices.
    fn notification_choices(&self) -> Vec<MenuChoice> {
        if self.preview || self.discord_user.is_none() {
            return vec![];
        }
        vec![MenuChoice::ChannelMute, MenuChoice::ChannelNotifications]
    }

    /// What someone's menu offers, by Discord user id or Matrix ID: their
    /// profile when Yapfast knows them, a DM, and Discord friendship; in their
    /// DM, what the DM's menu offers.
    pub fn person_choices(&self, user: &str) -> Vec<MenuChoice> {
        // In their own DM, someone's menu is the DM's, less what's about it
        // as a conversation in the list; its ID is theirs.
        if let Some(dm) = self
            .selected_room()
            .filter(|room| room.direct && room.person.as_deref() == Some(user))
        {
            let mut choices = self.room_choices(&dm.id);
            choices.retain(|choice| !matches!(choice, MenuChoice::MarkRead | MenuChoice::CopyLink));
            return choices;
        }
        let me =
            self.discord_user.as_deref() == Some(user) || self.account.as_deref() == Some(user);
        let mut choices = vec![];
        // A profile comes from a conversation: the open one, or a friend's DM.
        let listed = self.selected_room().is_some() && self.person(user).is_some();
        let friend = self.friends.iter().any(|friend| friend.id == user)
            && (self.selected.is_some() || self.dm_with(user).is_some());
        if listed || friend {
            choices.push(MenuChoice::Profile);
        }
        if !me && self.can_message(user) {
            choices.push(MenuChoice::Message);
        }
        // Matrix IDs start with `@`; Discord user ids are numbers.
        if !me && !self.preview && !user.starts_with('@') && self.discord_user.is_some() {
            match self.relation(user) {
                Relation::Stranger => choices.push(MenuChoice::AddFriend),
                Relation::Incoming => choices.push(MenuChoice::AcceptFriend),
                Relation::Outgoing => choices.push(MenuChoice::CancelFriendRequest),
                Relation::Friend | Relation::Me => {}
            }
        }
        if !self.preview {
            choices.push(MenuChoice::CopyId);
        }
        choices
    }

    /// Whether Yapfast can open a DM with someone: one that's listed, or a new
    /// one with a Discord friend, a member of the open Discord server's
    /// channel, or a Matrix user.
    pub(super) fn can_message(&self, user: &str) -> bool {
        self.dm_with(user).is_some()
            || (!self.preview
                && (user.starts_with('@')
                    || self.discord_friends.iter().any(|friend| friend == user)
                    || (self.discord_user.is_some()
                        && self.discord_server_open()
                        && self.person(user).is_some())))
    }

    /// Whether the open conversation is a Discord server's channel.
    pub(super) fn discord_server_open(&self) -> bool {
        self.selected
            .as_deref()
            .and_then(discord::parse_room)
            .is_some_and(|(guild, _)| guild != discord::DM_GUILD)
    }

    /// The listed one-to-one DM with someone.
    pub(super) fn dm_with(&self, user: &str) -> Option<&ChatRoom> {
        self.rooms
            .iter()
            .find(|room| room.direct && room.person.as_deref() == Some(user))
    }

    /// What a listed conversation's menu offers. The preview's rooms have no
    /// network to read them on or link to.
    fn room_choices(&self, id: &str) -> Vec<MenuChoice> {
        let Some(room) = self.rooms.iter().find(|room| room.id == id) else {
            return vec![];
        };
        let mut choices = vec![];
        if !self.preview && unread(room) {
            choices.push(MenuChoice::MarkRead);
        }
        let connected = self.voice.active() && self.voice.room.as_deref() == Some(id);
        if room.voice && !connected && self.can_join_voice(id) {
            choices.push(MenuChoice::JoinVoice);
        }
        if room.person.is_some() {
            choices.push(MenuChoice::Profile);
        }
        if room.direct && !room.request && !self.preview {
            choices.push(if self.pinned.contains(&room.id) {
                MenuChoice::Unpin
            } else {
                MenuChoice::Pin
            });
        }
        if room.direct && !connected && self.can_join_voice(id) {
            choices.push(MenuChoice::Call);
        }
        let discord = room.network == Network::Discord
            && room.direct
            && !self.preview
            && self.discord_user.is_some();
        if discord && let Some(person) = &room.person {
            choices.push(MenuChoice::AddNote);
            let friend = self.friends.iter().find(|friend| &friend.id == person);
            match friend.map(|friend| friend.nickname.is_some()) {
                Some(false) => choices.push(MenuChoice::AddNickname),
                Some(true) => choices.push(MenuChoice::EditNickname),
                None => {}
            }
            if !room.request {
                choices.push(MenuChoice::CloseDm);
            }
            choices.extend(self.relationship_choices(person));
        }
        if discord && !room.request {
            choices.push(if room.muted {
                MenuChoice::Unmute
            } else {
                MenuChoice::Mute
            });
        }
        // A thread's notifications are its own, which Yapfast doesn't change yet.
        if room.network == Network::Discord
            && !room.direct
            && !room.voice
            && !room.locked
            && room.parent.is_none()
        {
            choices.extend(self.notification_choices());
        }
        if !self.preview {
            choices.extend([MenuChoice::CopyLink, MenuChoice::CopyId]);
        }
        choices
    }

    /// How a DM's or a server member's menu changes how this Discord
    /// account stands with someone: friendship, ignoring and blocking.
    pub(super) fn relationship_choices(&self, person: &str) -> Vec<MenuChoice> {
        let has = |list: &[String]| list.iter().any(|listed| listed == person);
        let friendship = match self.relation(person) {
            Relation::Friend => Some(MenuChoice::Unfriend),
            // Discord takes no request to someone blocked.
            Relation::Stranger if !has(&self.discord_blocked) => Some(MenuChoice::AddFriend),
            Relation::Incoming => Some(MenuChoice::AcceptFriend),
            Relation::Outgoing => Some(MenuChoice::CancelFriendRequest),
            Relation::Stranger | Relation::Me => None,
        };
        let ignore = if has(&self.discord_ignored) {
            MenuChoice::Unignore
        } else {
            MenuChoice::Ignore
        };
        let block = if has(&self.discord_blocked) {
            MenuChoice::Unblock
        } else {
            MenuChoice::Block
        };
        friendship.into_iter().chain([ignore, block]).collect()
    }

    /// What a listed server's menu offers.
    fn server_choices(&self, id: &str) -> Vec<MenuChoice> {
        let mut choices = vec![];
        if self.preview {
            return choices;
        }
        if self.unread_channels(id).next().is_some() {
            choices.push(MenuChoice::MarkRead);
        }
        choices.extend([MenuChoice::CopyLink, MenuChoice::CopyId]);
        choices
    }

    /// The server's channels with messages to read.
    fn unread_channels(&self, id: &str) -> impl Iterator<Item = &ChatRoom> {
        let channels = self
            .servers
            .iter()
            .find(|server| server.id == id)
            .map(|server| server.channels.as_slice())
            .unwrap_or_default();
        self.rooms
            .iter()
            .filter(move |room| channels.contains(&room.id) && unread(room))
    }
}

pub(super) fn unread(room: &ChatRoom) -> bool {
    room.unread > 0 || room.mentions > 0
}

/// A link to a room or server that its network's apps open: Discord's own,
/// or matrix.to's, through the server a Matrix id names when it names one.
pub fn network_link(id: &str) -> String {
    match id.strip_prefix(discord::PREFIX) {
        Some(discord) => format!(
            "https://discord.com/channels/{}",
            discord.replacen(':', "/", 1)
        ),
        None => match id.split_once(':') {
            Some((_, server)) => format!("https://matrix.to/#/{id}?via={server}"),
            None => format!("https://matrix.to/#/{id}"),
        },
    }
}

/// A room's or server's id as its network knows it: a Discord channel's or
/// server's, or a Matrix room's or Space's.
pub fn network_id(id: &str) -> &str {
    match id.strip_prefix(discord::PREFIX) {
        Some(discord) => discord.rsplit(':').next().unwrap_or(discord),
        None => id,
    }
}

impl Application {
    pub(super) fn navigate(&mut self, navigation: Navigation) -> Vec<Effect> {
        if self.state.navigation == navigation {
            return vec![];
        }
        if let Navigation::Server(id) = &navigation
            && !self.state.servers.iter().any(|server| &server.id == id)
        {
            return vec![];
        }
        self.state.navigation = navigation;
        self.state.dm_screen = None;
        self.restore_selection()
    }

    pub(super) fn open(&mut self, id: String) -> Vec<Effect> {
        let Some(room) = self.state.rooms.iter().find(|room| room.id == id) else {
            // Asked for before Discord's lists came, as by a notification
            // that started Yapfast on macOS: it opens once they have it.
            if !self.state.discord_online && !self.state.preview {
                self.pending_open = Some(id);
                return vec![];
            }
            return self.open_thread(id, false);
        };
        let navigation = if room.direct {
            Some(Navigation::DirectMessages)
        } else {
            self.state
                .servers
                .iter()
                .find(|server| server.channels.contains(&id))
                .map(|server| Navigation::Server(server.id.clone()))
        };
        let mut effects = navigation
            .map(|navigation| self.dispatch(Action::Navigate(navigation)))
            .unwrap_or_default();
        effects.extend(self.select(id));
        effects
    }

    /// Opens a Discord thread that isn't listed, as a message's link to it
    /// or a notification from it does, in place of the open room or
    /// `beside` it: Discord tells a user account only of the threads it's
    /// in, so Yapfast asks for it, and opens it once it lists.
    pub(super) fn open_thread(&mut self, id: String, beside: bool) -> Vec<Effect> {
        let in_server = discord::parse_room(&id).is_some_and(|(guild, _)| {
            guild != discord::DM_GUILD
                && self
                    .state
                    .servers
                    .iter()
                    .any(|server| server.id == discord::server_id(guild))
        });
        if !in_server || self.state.preview || !self.state.discord_online {
            return vec![];
        }
        if beside {
            self.pending_aside = Some(id.clone());
        } else {
            self.pending_open = Some(id.clone());
        }
        vec![Effect::Backend(Command::Discord(
            DiscordCommand::OpenThread { room: id, beside },
        ))]
    }

    /// The thread [`Self::open_thread`] asked for couldn't be had.
    pub(super) fn thread_opened(&mut self, room: String, result: Result<(), String>) {
        if let Err(error) = result {
            let opening = self.pending_open.take_if(|pending| *pending == room);
            let beside = self.pending_aside.take_if(|pending| *pending == room);
            if opening.is_some() || beside.is_some() {
                self.state.notice = Some(error);
            }
        }
    }

    pub(super) fn select_room(&mut self, id: String) -> Vec<Effect> {
        let mut effects = self.select(id.clone());
        // Clicking a voice channel joins it, or asks before leaving another
        // call. Restoring a selection, for example on navigation, never joins.
        let voice = self.state.selected.as_ref() == Some(&id)
            && self
                .state
                .rooms
                .iter()
                .any(|room| room.id == id && room.voice);
        if voice {
            effects.extend(self.dispatch(Action::RequestJoinVoice(id)));
        }
        effects
    }

    pub(super) fn message_person(&mut self, user: String) -> Vec<Effect> {
        if let Some(room) = self.state.dm_with(&user) {
            let room = room.id.clone();
            return self.dispatch(Action::Open(room));
        }
        if !self.state.can_message(&user) {
            return vec![];
        }
        self.starting_dm = true;
        let discord = !user.starts_with('@');
        vec![Effect::Backend(Command::StartDm { id: user, discord })]
    }

    /// Marks a conversation read, or every conversation of a server or a
    /// Discord category.
    pub(super) fn mark_read(&mut self, id: String) -> Vec<Effect> {
        if !self.menu_choices(&id).contains(&MenuChoice::MarkRead) {
            return vec![];
        }
        let sync = self.state.sync_discord_reads;
        let rooms: Vec<String> = if self.state.servers.iter().any(|server| server.id == id) {
            self.state
                .unread_channels(&id)
                .map(|room| room.id.clone())
                .collect()
        } else if self.state.is_category(&id) {
            self.unread_in_category(&id)
        } else {
            vec![id]
        };
        rooms
            .into_iter()
            .map(|room| Effect::Backend(Command::MarkRead { room, sync }))
            .collect()
    }

    /// Pins a listed DM to the top of the DM list, or unpins it.
    pub(super) fn pin(&mut self, room: String, pinned: bool) -> Vec<Effect> {
        let choice = if pinned {
            MenuChoice::Pin
        } else {
            MenuChoice::Unpin
        };
        if self.state.menu_choices(&room).contains(&choice) {
            self.state.pinned.retain(|id| *id != room);
            if pinned {
                self.state.pinned.push(room);
            }
        }
        vec![]
    }

    pub(super) fn close_dm(&mut self, room: String) -> Vec<Effect> {
        if !self
            .state
            .menu_choices(&room)
            .contains(&MenuChoice::CloseDm)
        {
            return vec![];
        }
        // Closed, it isn't pinned either.
        self.state.pinned.retain(|id| *id != room);
        vec![Effect::Backend(Command::Discord(DiscordCommand::CloseDm {
            room,
        }))]
    }

    pub(super) fn mute_dm(&mut self, room: String, muting: Muting) -> Vec<Effect> {
        let choice = if muting == Muting::Off {
            MenuChoice::Unmute
        } else {
            MenuChoice::Mute
        };
        if !self.state.menu_choices(&room).contains(&choice) {
            return vec![];
        }
        vec![Effect::Backend(Command::Discord(DiscordCommand::Mute {
            room,
            muting,
        }))]
    }

    pub(super) fn open_new_dm(&mut self) -> Vec<Effect> {
        if self.state.preview {
            return vec![];
        }
        self.state.new_dm = Some(NewDm::default());
        vec![Effect::Backend(Command::FindPeople {
            query: String::new(),
        })]
    }

    pub(super) fn start_dm(&mut self, contact: Contact) -> Vec<Effect> {
        let Some(new_dm) = self.state.new_dm.as_mut().filter(|new_dm| !new_dm.starting) else {
            return vec![];
        };
        new_dm.starting = true;
        new_dm.error = None;
        vec![Effect::Backend(Command::StartDm {
            id: contact.id,
            discord: contact.discord,
        })]
    }

    pub(super) fn load_pictures(&mut self, urls: Vec<String>) -> Vec<Effect> {
        let wanted: Vec<String> = urls
            .into_iter()
            .filter(|url| self.requested_pictures.insert(url.clone()))
            .collect();
        if wanted.is_empty() || self.state.preview {
            vec![]
        } else {
            vec![Effect::Backend(Command::Discord(DiscordCommand::Pictures(
                wanted,
            )))]
        }
    }

    pub(super) fn select(&mut self, id: String) -> Vec<Effect> {
        // A thread beside the channel goes when another room shows.
        let changes = self.state.selected.as_ref() != Some(&id)
            && self
                .state
                .rooms
                .iter()
                .any(|room| room.id == id && self.state.in_scope(room));
        let mut effects = if changes { self.close_aside() } else { vec![] };
        effects.extend(self.show_room(id));
        effects
    }

    fn show_room(&mut self, id: String) -> Vec<Effect> {
        let Some(room) = self
            .state
            .rooms
            .iter()
            .find(|room| room.id == id && self.state.in_scope(room))
        else {
            return vec![];
        };
        let voice_room = room.voice;
        let locked = room.locked;
        let forum = room.forum;
        // Where the "New" divider goes, from before showing the room reads it.
        let read_marker = (room.unread > 0 || room.mentions > 0)
            .then(|| room.last_read.clone())
            .flatten();
        // Messages after the last one read are new.
        let seen = room.last_read.clone();
        self.remember_view();
        self.last_selected
            .insert(self.state.navigation.clone(), id.clone());
        self.state.dm_screen = None;
        // Showing another room forgets a stream to watch in another call.
        self.state.pending_watch.take_if(|(room, _)| *room != id);
        self.state.selected = Some(id.clone());
        self.state.confirm_delete = None;
        self.state.forwarding = None;
        self.state.viewer = None;
        self.state.video = None;
        self.state.profile = None;
        self.state.room_ready = self.state.preview || voice_room;
        self.state.loading_history = false;
        // The preview's samples are all there is.
        self.state.history_start = self.state.preview;
        self.state.history_failed = false;
        self.state.detached = false;
        self.state.loading_later = false;
        self.state.later_failed = false;
        self.state.jump = None;
        self.seeking = None;
        self.focusing = false;
        self.state.first_unread = None;
        self.read_marker = read_marker.map(|marker| (marker, false));
        self.state.seen = seen;
        self.placing_seen = true;
        self.state.notice = None;
        self.state.messages = Arc::new(vec![]);
        self.visible_messages.clear();
        self.state.members = None;
        if !self.state.preview
            && let Some(index) = self.shown.iter().position(|shown| shown.room == id)
        {
            let shown = self.shown.remove(index);
            self.state.messages = shown.messages;
            self.state.members = shown.members;
            self.state.room_ready = true;
        }
        self.state.selection_version = self.state.selection_version.wrapping_add(1);
        // Typing is followed only in the open conversation, so what's known of others is stale.
        self.state.typing.clear();
        self.state.mention_query = None;
        self.state.mention_groups.clear();
        self.state.found_members.clear();
        self.state.role = None;
        self.state.pins = None;
        // A server's search stays open in its channels.
        if self.state.search.is_some() && self.state.search_shown().is_none() {
            self.state.search = None;
        }
        self.state.add_people = None;
        self.state.topic = None;
        self.searched.clear();
        // The list of threads goes, as the popout does in Discord's client,
        // and so does a new thread's form.
        let mut effects = self.close_threads();
        effects.extend(self.close_forum());
        self.state.new_thread = None;
        if self.state.preview {
            if !voice_room {
                self.state.messages = Arc::new(self.samples.get(&id).cloned().unwrap_or_default());
                self.state.members = Some(Ok(preview_members(&id)));
            }
        } else if voice_room {
            effects.push(Effect::Backend(Command::CloseRoom));
        } else if forum && !locked {
            // Its posts show in place of messages, which it has none of.
            self.state.room_ready = true;
            effects.push(Effect::Backend(Command::CloseRoom));
            effects.extend(self.show_forum(id));
            effects.extend(self.mark_discord_read());
        } else if locked {
            // Discord would refuse its messages, and refusals count against
            // the account, so Yapfast doesn't ask.
            self.state.room_ready = true;
            self.state.notice =
                Some("Your roles on this Discord server don't let you read this channel.".into());
            effects.push(Effect::Backend(Command::CloseRoom));
        } else {
            effects.push(Effect::Backend(Command::Open(id)));
        }
        effects
    }

    pub(super) fn restore_selection(&mut self) -> Vec<Effect> {
        let remembered = self.last_selected.get(&self.state.navigation);
        let next = self
            .state
            .rooms
            .iter()
            .find(|room| Some(&room.id) == remembered && self.state.in_scope(room))
            // A message request opens only when chosen.
            .or_else(|| self.state.scoped_rooms().find(|room| !room.request))
            .map(|room| room.id.clone());
        if let Some(id) = next {
            self.select(id)
        } else {
            self.deselect()
        }
    }

    /// Closes the conversation on screen, leaving none selected.
    pub(super) fn deselect(&mut self) -> Vec<Effect> {
        let mut effects = self.close_aside();
        effects.extend(self.close_room());
        effects
    }

    fn close_room(&mut self) -> Vec<Effect> {
        let mut effects = self.close_threads();
        effects.extend(self.close_forum());
        self.remember_view();
        self.state.selected = None;
        self.state.confirm_delete = None;
        self.state.forwarding = None;
        self.state.viewer = None;
        self.state.video = None;
        self.state.room_ready = false;
        self.state.loading_history = false;
        self.state.history_start = false;
        self.state.history_failed = false;
        self.state.detached = false;
        self.state.loading_later = false;
        self.state.later_failed = false;
        self.seeking = None;
        self.focusing = false;
        self.state.first_unread = None;
        self.read_marker = None;
        self.state.seen = None;
        self.placing_seen = false;
        self.state.notice = None;
        self.state.messages = Arc::new(vec![]);
        self.visible_messages.clear();
        self.state.members = None;
        self.state.topic = None;
        self.state.selection_version = self.state.selection_version.wrapping_add(1);
        if !self.state.preview {
            effects.push(Effect::Backend(Command::CloseRoom));
        }
        effects
    }

    /// Shows the Matrix and Discord lists together, leaving a server or
    /// conversation that disappeared.
    pub(super) fn show_lists(&mut self) -> Vec<Effect> {
        #[cfg(feature = "discord")]
        let (discord_rooms, discord_servers) =
            (self.discord_index.rooms(), self.discord_index.servers());
        #[cfg(not(feature = "discord"))]
        let (discord_rooms, discord_servers) = (std::iter::empty(), std::iter::empty());
        chat::sync_list(
            &mut self.state.rooms,
            self.matrix_rooms.iter().chain(discord_rooms),
        );
        chat::sync_list(
            &mut self.state.servers,
            self.matrix_servers.iter().chain(discord_servers),
        );
        if let Navigation::Server(id) = &self.state.navigation
            && !self.state.servers.iter().any(|server| &server.id == id)
        {
            self.state.navigation = Navigation::DirectMessages;
        }
        // A screen of the DM list stands in for a conversation.
        if self.state.dm_screen.is_some() && self.state.navigation == Navigation::DirectMessages {
            return vec![];
        }
        if !self
            .state
            .rooms
            .iter()
            .any(|room| Some(&room.id) == self.state.selected.as_ref() && self.state.in_scope(room))
        {
            return self.restore_selection();
        }
        vec![]
    }
}
