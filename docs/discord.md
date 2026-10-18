# Discord in Yapfast

Status on 2026-10-07: steps 1 to 4 of the [order of work](#order-of-work) work
on Linux, and step 5, mirroring into Matrix, hasn't started. Decisions were
recorded on 2026-10-06.

| Area | Works on Linux | Open |
| --- | --- | --- |
| [Linking](#account-linking) | QR code from Settings, or the sign-in of a build without Matrix; unlinking; Discord's captcha, in the browser | A captcha solved against real Discord |
| [Voice](#voice) | Server voice channels, mute, deafen, per-person volume, voice gate with its sensitivity, [DeepFilterNet noise suppression](#microphone) and its strength, taking a room's echo out, [camera video](#camera-video) | A check of Windows audio and cameras on real Windows; Discord's push-to-talk; soundboard |
| [DM calls](#dm-calls) | Starting, ringing, answering and declining DM and group DM calls, with cameras | A check by hand through real Discord |
| [Go Live](#screen-sharing) | Sharing a screen with its sound, watching streams | A check on real Windows |
| [Channels and messages](#servers-channels-and-messages) | History, live updates, sending, replies, edits, reactions, files, embeds, custom emoji, stickers, spoilers, unread state, notifications and their settings, member lists, pins and search, a channel's topic and header, server, category and [channel menus](#a-channels-menu), pinning channels to the top, invites, making and changing channels, threads, a channel's list of them and starting them, [forums](#forums), leaving servers | Posting in forums, pinning messages, forwarding, server settings and privacy |
| [DMs](#dms) | DMs, group DMs, message requests and spam, starting a DM, [a DM's menu](#a-dms-menu): pinning, muting and closing | Leaving a group DM |
| [Friends](#friends-and-requests) and [profiles](#profiles) | Friends list, friend requests, adding by username, profile cards and the DM panel, custom status, notes, friend nicknames, removing friends, ignoring and blocking | A list of the blocked and ignored |
| [Mirroring](#mirroring-into-matrix) | | Everything |

The [later list](#later) has the rest.

Yapfast connects to Discord directly, so a person who links their Discord account
doesn't need the Discord desktop app. Yapfast joins Discord voice as that person,
shows their servers, channels and DMs, and sends and receives messages. Yapfast must still be complete without Discord: nothing
Discord-related appears until someone links an account, and Discord work doesn't
replace beta features of Yapfast itself.

Protocol references: Discord doesn't document what its own client sends. The
community's
[discord-userdoccers](https://github.com/discord-userdoccers/discord-userdoccers)
does, for example in `pages/gateway/gateway-events.mdx`,
`pages/topics/read-state.mdx`, `pages/resources/relationships.mdx`,
`pages/remote-authentication/desktop.mdx` and `pages/topics/cloud-uploads.mdx`.
The settings protobuf is in
[discord-protos](https://github.com/discord-userdoccers/discord-protos).

## Risk to accounts

Discord's terms don't allow third-party clients to use personal accounts.
Bans are uncommon, but they happen, and Discord can change its protocol and
break Yapfast at any time. Linking is opt-in per person. Yapfast shows this warning
before linking, and each person decides for their own account.

Behave like the official client to keep the risk low. Respect rate limits,
never act without a user action behind it, and don't poll. Keep all Discord code
in one crate, so a protocol change breaks only that part and Yapfast can disable it
without affecting Matrix.

### Staying within Discord's limits

Yapfast follows the rules Discord documents for every client:

- **HTTP rate limits.** Each request's route maps to the bucket Discord names in
  `X-RateLimit-Bucket`. Discord counts a bucket apart for each major ID, the
  channel, server or webhook, so Yapfast keys its count by both: sending in one
  channel doesn't hold back another. Yapfast waits for an empty bucket to reset
  before it asks again. It also waits after a global 429, marked so in the
  headers or the body. If Discord still answers 429, Yapfast waits the body's
  `retry_after`, or `Retry-After`, or a second when Discord names neither, and
  tries once more. A wait longer than a minute fails at once, with a message to
  try again later.
- **Invalid requests.** Discord bans an IP that sends 10,000 requests in a
  10-minute window answered 401, 403 or 429. Yapfast doesn't ask for what it knows
  will be refused: a channel the account's roles hide shows a notice instead
  of fetching messages.
- **Gateway resume.** When the connection drops, Yapfast resumes the session at
  READY's `resume_gateway_url` with op 6 and the last sequence number, and
  Discord replays what was missed. Yapfast also resumes after op 7, after op 9
  with `d: true`, and when a heartbeat goes unacknowledged. It identifies again
  only when Discord won't resume: op 9 with `d: false`, close codes 4007 and
  4009, or three failed attempts. After op 9 it waits a random one to five
  seconds first, or longer when sessions keep ending. Opening a connection,
  Discord's HELLO and each send give up after 10 seconds, so a dead connection
  fails rather than hangs. Yapfast drops a connection without a close frame,
  because a close with 1000 or 1001 ends the session.
- **Gateway send limit.** A connection may send 120 commands a minute and is
  closed with 4008 past that. Yapfast holds commands back at 110, so heartbeats
  always fit. The connection runs in a task of its own, so heartbeats go on
  time while commands wait.
- **Backoff.** Reconnects back off from 2 to 60 seconds, and start over only
  once a session has stayed up for a minute. Each wait is a random 50% to 100%
  of the step, so clients cut off together don't return together. Yapfast sends
  at most 10 IDENTIFYs an hour: an account that keeps identifying gets flagged,
  so past that Yapfast stops reconnecting and says so until it restarts. The first
  heartbeat comes after a random part of the interval, as Discord asks.
- **Typing.** Mention search waits 300 ms after the last key before it asks
  Discord for members, so typing a name sends one search, not one per letter.
- **Cookies.** Yapfast keeps the cookies Discord's servers set and sends them
  back where they apply, as any HTTP client with a cookie jar does. These are
  Discord's device cookies and Cloudflare's rate limiting and bot management
  cookies. One jar serves the whole process. Persistent cookies are saved,
  readable only by the user, in `discord-cookies.json` in the data directory,
  so Yapfast stays the same device across restarts. Yapfast can't pass a Cloudflare
  challenge or captcha, so it never has a `cf_clearance` cookie.
- **Downloads.** Pictures and files from Discord's CDN download eight at a
  time, over one HTTP/2 connection, as a browser's requests to one host go.
  Opening the emoji picker queues its pictures in the picker's order instead
  of asking for hundreds at once.

## Why not an existing bridge

[mautrix-discord](https://github.com/mautrix/discord) bridges messages through
a service hosted next to Synapse. It has no voice, and voice matters most here.
It also adds a Matrix user for every Discord contact, so the homeserver learns
who each person talks to. Beeper uses the same bridge and also has no Discord
calls.

A Discord bot relaying voice into a Matrix room, like
[matrix-discord-voice-bridge](https://github.com/lukacsi/matrix-discord-voice-bridge),
follows Discord's rules. But Discord users would hear every Yapfast user as one bot,
it only works in servers that install the bot, and bots can't join DM calls.

## Account linking

Link by QR code, the method mautrix-discord uses. Yapfast shows a code from
Discord's remote login service, and the person scans it with the Discord phone
app. Pasting a token and email/password login aren't offered. A build with
Matrix links from Settings, under Discord; one without Matrix links on its
sign-in screen, and signing out there unlinks. Either way, Yapfast first says
that Discord doesn't allow third-party clients, so linking puts the account at
risk, and shows the code only when asked.

The remote login gateway (`wss://remote-auth-gateway.discord.gg/?v=2`, with
Discord's web origin) works as Discord's own login page does,
`yapfast_discord::remote_auth`:

1. Yapfast makes a 2048-bit RSA key and sends its public half, as base64 DER.
2. Discord sends a nonce encrypted to it (RSA-OAEP with SHA-256), and Yapfast
   answers with the SHA-256 of the nonce, base64url, which proves it holds the
   key.
3. Discord sends a fingerprint, and the code carries
   `https://discord.com/ra/{fingerprint}`.
4. Scanning it sends who scanned (`id:discriminator:avatar:username`, encrypted
   to the key), and Yapfast shows "Confirm on your phone".
5. Confirming sends a ticket, which `POST /users/@me/remote-auth/login` trades
   for the login, encrypted to the key again.

The gateway closes the connection about two minutes after the code shows, and
Yapfast connects again for a new code while the code is on screen. Saying no on
the phone stops linking, with "Try again". The key lives only in memory for the
one attempt. The `rsa` crate carries the Marvin timing advisory
(RUSTSEC-2023-0071), which needs an attacker to time many decryptions; Yapfast
decrypts three messages from Discord per attempt. Linking connects Discord at
once, without restarting: voice reaches it through a slot the chat backend
fills. When Discord can't start then, or as Yapfast starts, such as when the
credential store can't be read, Yapfast says why. What waits on Discord, such as
a friend request, the custom status or a profile, fails with that reason
rather than waiting, as it does when the chat backend stops.

Discord may ask for a captcha when trading the ticket, as it does of logins
from devices it doesn't know
([captcha handling](https://docs.discord.food/topics/captcha-handling)): a
400 with `captcha_key`, an hCaptcha site key that changes, and often
`captcha_rqdata`, `captcha_rqtoken` and `captcha_session_id`. Yapfast has no web
view, so `captcha.rs` serves a page with the challenge on `127.0.0.1`, at a
random path, and opens it in the browser; the dialog says so and can open it
again. The page loads hCaptcha's `api.js` with `host=discord.com`, as
hCaptcha's mobile SDKs name the host they stand for, sets the rqdata, and
posts the solution back. Yapfast trades the ticket again with `X-Captcha-Key`,
`X-Captcha-Rqtoken` and `X-Captcha-Session-Id`, keeping the login gateway's
heartbeat going meanwhile, and shows a new captcha up to three times if
Discord doesn't take one. The listener lives only while the page waits, so
nothing stays loaded afterwards. hCaptcha's site check passed Discord's site
key for `127.0.0.1` on 2026-10-13; whether Discord takes a solution made this
way awaits a real captcha. The fake server asks for one after
`POST /_control/qr-captcha`.

Store the Discord token in the OS credential store, next to the Matrix session.
The token gives full control of the account, so it never goes into Matrix, not
even encrypted. A device that links appears in Discord as its own session.
Devices that only use the mirror don't need to link.

Unlinking a device, from Settings or by signing out of a build without Matrix,
leaves any Discord call, ends the session with `POST /auth/logout`, and deletes
its token. Removing
the mirrored rooms is a separate step, described under [Unlinking](#unlinking).

## Voice

Scope for the beta:

- Server voice channels: join, leave, a participant list, speaking indicators,
  mute, deafen and per-person volume.
- DM and group DM calls, both incoming with ringing and outgoing.
- One connection at a time across Yapfast and Discord. Joining Discord voice asks
  to leave a Yapfast call, and the other way round.
- The same devices, push-to-talk, voice activation, echo cancellation and
  voice leveling as Yapfast calls. Noise suppression is DeepFilterNet, or
  RNNoise, rather than WebRTC's, as described under Microphone below.

Clicking a voice channel joins through the same voice dock and one-call rule
as Matrix voice. Calls start with the microphone on unless Settings turns
that off, mute and deafen show in Discord, and a
voice gate sends audio only during speech. The owner and a friend talked
through it on 2026-10-06. Audio goes through `audio_io.rs`, PulseAudio on
Linux, WASAPI on Windows and Audio Queue Services on macOS, and libwebrtc's
processing module; see the
[Windows guide](windows-beta.md#discord-voice-on-windows) and the
[macOS guide](macos.md#sound) for what's been checked there. davey's change for Go Live groups is described in
`vendor/davey/README.yapfast.md`.

Discord may move a call to another voice server, saying so with a new
`VOICE_SERVER_UPDATE` while the call is connected. Yapfast connects the call
there and it goes on. The old server may drop the connection before that
word comes, so a dropped connection waits five seconds for it before the
call ends. A stream moves the same way, with a new `STREAM_CREATE` and
`STREAM_SERVER_UPDATE`: sharing sends to the new server, after waiting up to
five seconds for it when the old connection ends, and watching connects again.

The account may be in voice on another client too, such as Discord's app on
a phone. Yapfast tells that client's voice state from its own by the
`session_id` it carries: READY's is this gateway session's, and a voice
state with another is the other client's. The voice channel lists the
account there as Discord does: dimmed until hovered, without its icons, with
the tooltip "Connected on another client". Joining here takes the call over,
and the join waits for a voice state of this session. When the other client
joins voice, Yapfast's call ends without sending op 4 with no channel, which
would end the other client's call too; Yapfast sends that only for a join under
way or a voice state of its own session.

The group uses screen sharing often, so sharing a screen and watching
streams (Go Live) came before the beta; see [Screen sharing](#screen-sharing).
Camera video works in server voice channels and DM calls; see
[Camera video](#camera-video).

Since March 2026, Discord voice requires its end-to-end encryption, DAVE, and
[rejects clients without it](https://support.discord.com/hc/en-us/articles/38749827197591-A-V-E2EE-Enforcement-for-Non-Stage-Voice-Calls).
[davey](https://github.com/Snazzah/davey) (MIT) implements DAVE in Rust.
Calls first ran on [songbird](https://github.com/serenity-rs/songbird) (ISC),
a voice connection for bots built on it; they now run on Yapfast's own, the one
Go Live uses. [How calls carry video](#how-calls-carry-video) says why.

Yapfast's current audio path runs inside LiveKit's libwebrtc. Discord voice needs
its own Opus encoding and decoding plus the same audio processing. libwebrtc's
device module can't be reused outside a LiveKit room: it hands captured audio
straight to WebRTC and only plays remote WebRTC streams. Its audio processing
module can, through `livekit::webrtc::native::apm::AudioProcessingModule`. So
Discord voice gets a small path of its own: `audio_io.rs` for the microphone and
speakers at 48 kHz, the processing module for echo cancellation and gain
(noise suppression became DeepFilterNet; see [Microphone](#microphone)), then
Yapfast's voice connection for Opus and DAVE. Feed the speaker mix to
the processing module's reverse stream so echo cancellation has its reference.

Opus comes from the libopus inside Yapfast's libwebrtc archive, called directly
(`yapfast_discord::opus`). songbird and Go Live's sound used `opus2`, which
builds a libopus of its own; two copies of the same symbols risk a clash when
linking, particularly with MSVC on Windows, and now there's one. The Windows
build gave `opus2` an empty library so that it used libwebrtc's; with `opus2`
gone, it needs none.

### Connection details

As in Discord's voice panel, the dock's status has a signal icon graded by
the call's connection, and hovering it or the status shows the last ping
("42 ms"). The same icon sits in the call's conversation header. A click on
either opens the details, as Discord's popout from its panel has them: a
Connection tab with a graph of the ping over the last five minutes, the
average and last ping, and the outbound packet loss rate; and while the call
is end-to-end encrypted with someone else in it, a Privacy tab with the voice
privacy code in six groups of five digits, to compare with the others, and
a button that copies it. Discord's client shows its graph only in developer
mode; Yapfast shows it to everyone. A notice at the bottom says "End-to-end
encrypted" while DAVE encrypts the call's frames, and "Secure connection"
otherwise, as Discord's does; the dock's detail line says the same. It said
end-to-end encrypted before DAVE was ready, and on the fake, which has none.

The ping is the voice gateway's heartbeat round trip, from op 3's nonce,
the time it was sent, until op 6 echoes it, as Discord's web client
measures it. Heartbeats go every 5 s rather than the 13.75 s the server
asks, as that client caps them, and the call keeps the last 60 pings, 120
bytes. The average is over all of them; Discord's client averages its
oldest 20. The loss rate comes from the RTCP receiver reports Discord's
server sends about this account's audio and camera: the packets reported
lost between two reports, out of those expected, summed over both, as
Discord's client sums its outbound streams. It's 0 while nothing is sent,
and unknown until two reports came. The grades are Discord's: bad past
500 ms or 10% lost, average past 250 ms or 5%, fine below; the icon is
grey until the first ping.

The call's thread writes these into `yapfast_discord::call_stats::CallStats`.
Yapfast copies the pings out only when the tooltip opens and each second while
the popout is open. The state the UI refreshes from changes only when the
grade or the encryption does. The fake answers heartbeats and sends a
receiver report about a call's audio every 50 packets;
`POST /_control/voice-link?ping=300&loss=7` slows its ACKs and reports
loss, and `talks_to_the_fake_server` checks both are measured.

Matrix calls don't have these yet. LiveKit's SDK measures its signal
connection's round trip but doesn't expose it, and `Room::get_stats`
serialises all of WebRTC's statistics to JSON and parses them back, too much
to run every few seconds for a graph. A Matrix call's detail line says
"End-to-end encrypted" while LiveKit encrypts its frames.

### How calls carry video

Camera video in calls needed a choice: add video to songbird, or move calls
onto the connection Go Live already uses (`voice_connection.rs`, shared by
`stream_connection.rs` and `call_connection.rs`). Yapfast moved calls.

| | Video in songbird | Yapfast's own connection |
| --- | --- | --- |
| Threads per call | songbird's mixer thread, its disposal thread and tasks on Yapfast's runtime, then video beside them | One thread with its own small runtime, as each Go Live stream has |
| Code | 19,000 vendored lines, patched in the gateway task, the mixer and the UDP receiver for video, and its DAVE for video frames | About 800 new lines with tests (the call's media loop and jitter buffer, and Opus); the gateway code moved out of Go Live's; songbird's vendor copy and its dependencies go |
| Voice gateway | Version 4, older than Discord's clients | Version 8, as Discord's clients and Go Live use: sequence acknowledgements and numbered binary messages |
| DAVE | songbird's, for Opus only | Yapfast's (`dave.rs`, on davey), already encrypting Go Live's video and sound |
| Video pieces | New: packetizing, reassembly, NACK and RTX, keyframe requests | Go Live's, tested against the fake and Discord |
| libopus | Two copies, songbird's `opus2` and libwebrtc's | libwebrtc's alone |
| Risk to calls that work | Low for audio until the patch grows | The audio path is new: Opus in and out, a jitter buffer, loss concealment, speaking |

songbird's audio works, and keeping it was the lower risk for audio alone.
But video would have meant patching most of songbird's driver, and keeping
those patches through its releases, while Yapfast already had a version 8
connection carrying video with DAVE. The new audio path keeps the API
`yapfast_core` uses (`Call`, `Microphone`, `Track`, `Listener`), so
`discord_audio.rs` and the audio I/O beneath it (`audio_io.rs`) don't change, and the fake
voice server now exchanges audio both ways to test it.

A call's connection sends Opus as songbird did, stereo at 128 kbit/s in its
music mode, 20 ms a packet, with five silent frames when the microphone
pauses, as Discord's clients do. It marks speaking with op 5 when
it starts. The RTP timestamp goes on 20 ms every tick, sending or not, as a
microphone's clock does. Until 2026-10-17 it went on only with each packet
sent. WebRTC's jitter buffer (NetEq), which Discord's clients play others
through, compares packets' arrival times with their timestamps over the last
2 s of timestamps, so after a pause every packet looked as late as the pause
was long. Its target delay rose to match, and after a long concealment it
waits until half its target is buffered before playing: the start of speech
after a pause was held back, by up to half the pause. That's from NetEq's source
(`decision_logic.cc`, `packet_arrival_history.cc`); a real call hasn't
confirmed it yet. Received audio waits in a jitter buffer per speaker: playing
starts once 60 ms has arrived, a missing packet plays as Opus's loss
concealment (or from the next packet's forward error correction), and a
speaker who stays silent for 100 ms drops out of the mix until they speak
again. When the voice gateway drops, the connection resumes with op 7, and
connects again from scratch if resuming fails.

Measured on 2026-10-13 in a call on the fake server, Discord-only `fast`
build, Skia Software, averaged over 10 s: muted, songbird took 18 threads
and 14 MiB of anonymous memory, Yapfast's connection 17 and 13 to 15 MiB, both
at 0.4% of a core. Unmuted, with the microphone's processing running, both
took 34 MiB; songbird 7.2% of a core and Yapfast's connection 6.2%.

### Camera video

Turning the camera on in a call does what Discord's clients do: the main
gateway's voice state says `self_video`, and the call's connection announces
the camera with op 12, its video and RTX SSRCs (those op 2 assigned) and one
stream at rid "100", at most 1280×720 and 30 frames a second. Discord's
clients offer a second layer at quality 50; Yapfast sends one, as its Go Live
does. Turning it off sends op 12 again with a video SSRC of 0 and no
streams. Yapfast offers H264 to encode, as for Go Live, and both H264 and VP8 to
decode. The camera's pictures go to OpenH264 on the camera's own thread, at
1.5 Mbit/s, and each frame to the call's thread, which encrypts it with DAVE
and sends it as Go Live's packetizer does: sender reports, a resend from RTX
for each NACK, and a keyframe when someone asks with a PLI. A frame the
call's thread can't take in time is dropped, and the next one is a keyframe.

The others' cameras arrive the same way round. Op 12 says who films on which
SSRC, and Discord forwards a camera only to those who ask for it with op 15
("media sink wants"): `{"any": 0, "<video SSRC>": quality, "pixelCounts":
{...}}`, quality from 0 to 100. Yapfast asks for each camera whose tile is in
sight, at 100 for a tile at least 360 pixels tall and 50 for a smaller one,
with the tile's pixels; for any other, 0. Frames come together as Go Live's
do (reordering, NACK, a PLI after a loss) and are decrypted with the
sender's DAVE key. Whole frames go to one thread, `yapfast-discord-video`,
which keeps an FFmpeg H264 or libvpx VP8 decoder on one thread for each
camera in sight, and writes each picture straight into the tile's feed at the
tile's size. A tile that leaves the screen loses its decoder within a second,
and its buffers at once; a new decoder starts from a keyframe. The others'
voice states say `self_video`, which marks them with a camera in the channel
list and gives their tile a feed.

The fake voice server announces Bob's, Carol's and Dave's cameras with op 12
(`POST /_control/camera?who=bob`, `&off` to stop), sends the recording in
testing/fixtures to a call that asks for it with op 15, losing a packet and
reordering another as it does for Go Live, and counts the frames of Alice's
camera (`GET /_control/call-video`). `talks_to_the_fake_server` checks
cameras both ways in a server's voice channel and in a DM call.

Measured on 2026-10-13 in a four-person call on the fake server, with 720p
cameras at 30 frames a second (`testing/measure_discord_cameras.py`): each
remote camera took about 5.5 MiB and 5% of a core, Yapfast's own camera about
11 MiB and six threads, and all of it went when the cameras went off. See
[measurements](measurements/README.md).

### DM calls

A DM or group DM has a call, as in Discord. The phone in its header starts
one, or joins the one going on, through the same voice dock and one-call
rule as a voice channel. Joining sends op 4 without a server, naming the DM
as the channel; VOICE_STATE_UPDATE and VOICE_SERVER_UPDATE come back without
a server too, the latter with the channel's id, and the voice connection
identifies with the DM's id where a server's call gives the server's. Yapfast
keeps those voice states under the server `@me`, as DM room ids do.

Discord sends CALL_CREATE, CALL_UPDATE and CALL_DELETE for the DMs' calls,
including one for each call going on when Yapfast connects (the
AUTO_CALL_CONNECT capability Yapfast already asks for). Each names whom the call
rings. Starting a call, rather than joining one with someone in it, rings the
others with `POST /channels/{id}/call/ring` and no recipients, once Discord
has reported the call: it rings only in a call it knows. Calling and ringing
are allowed separately. In a DM, Yapfast rings a friend at once and anyone else
only when `GET /channels/{id}/call` says they're `ringable`, as discord.py-self
does; otherwise, or when Discord refuses, the call says so until someone
comes, and the other person sees the call only in the DM. Yapfast plays a soft
ringback while nobody has come. People
being rung show in the call faded, "Calling...", until they answer or Discord
stops ringing them.

A call ringing this account shows a card at the top of the window with the
DM's picture, Decline and Accept, and Yapfast rings: a rising arpeggio
synthesized in `ring.rs` and played through `audio_io.rs` on the call's speakers
until the call is answered, declined or stops ringing. While Yapfast isn't in
front a desktop notification says who is calling. On Do Not Disturb the card
shows without the ring or the notification. Accepting opens the DM and
joins, leaving any other call without asking, since answering already chose.
Declining posts `POST /channels/{id}/call/stop-ringing` with this account,
which leaves the call to the others. The DM's own call shows the same choice
above its messages when it's open.

While this account is in a DM's call, the call's stage shows above the DM's
messages, taking less than half the conversation, more while watching a
stream. Without it, a DM with a call going on shows who is in it and a button
to join, and its row in the DM list a green phone. Discord's call messages
(type 3) read "started a call.", and once Discord adds when it ended, "started
a call that lasted 5 minutes.". Go Live works in DM calls too: op 18 with the
type `call` and no server, and the stream key `call:{channel}:{user}`. The
fake server checks all of this and answers for Bob, who picks up after three
seconds when Alice rings him; in a group nobody answers, and Dave, who isn't
her friend, can't be rung. Message requests
have no call until accepted. A DM call through real Discord, and Go Live in
one, haven't been checked by hand yet.

### Microphone

Discord's own client suppresses noise with Krisp, a neural network, and
WebRTC's suppressor sounded different from it. So Yapfast's Discord microphone
runs, in 10 ms blocks: WebRTC's echo cancellation and high-pass filter; if
asked, taking the room's echo out; a noise suppressor, which also tells the
gate which blocks are speech; WebRTC's gain control on what's left; then
Yapfast's leveling.

The suppressor is DeepFilterNet 3, through libDF, its Rust implementation on
tract (crate `deep_filter`; the code and the model's weights are MIT or
Apache-2.0). crates.io has only libDF 0.2.5, from before DeepFilterNet 3, so
Yapfast takes the repository's last commit, `d375b2d`, and its `default-model`
feature builds the 8 MB model into Yapfast. It runs both of its stages on every
block, as libDF's command line does. The library's defaults skip deep
filtering above 20 dB SNR, which dulled clean speech (wideband PESQ 4.53 fell
to 3.91) and took up to 5 dB off speech over a fan.

RNNoise, through `nnnoiseless` (BSD-3-Clause), was the suppressor until
2026-10-12. On real calls it let keyboard clicks through, and they held the
gate open. Settings → Voice & video → "Noise suppression in Discord calls"
still chooses it, to compare the two, as does Off, for a quiet room: RNNoise
then only tells the gate which blocks are speech. Yapfast keeps the choice in
`layout.json` as `noise_suppression`. A call going on switches at its next
block, after DeepFilterNet's 0.2 s load if switching to it.

"Noise suppression strength" limits how far noise is turned down: Full takes
out all it can, Medium at most 30 dB and Light 20 dB (`noise_strength`, Medium
unless changed). Until 2026-10-18 Full was the default, and it cut people's
laughs: DeepFilterNet takes high-pitched giggles (a pitch of 440-760 Hz) for
noise, with its SNR estimate at -15 dB as for typing, and puts out silence for
them. Low laughs (120-170 Hz) and mumbling came through at 73-100% at every
strength. Over two rooms, Full sent 0-31% of two giggles, Medium 47-66% and
Light 85-100%, with sighs and breathy laughs in step (a one-off test, with
Wikimedia Commons recordings). With mouse clicks and chair creaks in the
pauses of speech, as below, one more of 11 creaks opened the gate at Medium
and Light, and no click did; over a minute of steady loud creaking, the gate opened 4
times at Full, 11-13 at Medium and 23-24 at Light. libDF's threshold for
silencing a block, a pitch detector and RNNoise's speech probability as a
second opinion didn't save the giggles: the network's own gains silence them,
and the last two found at most half of each while firing on typing over a
hum. libDF
mixes the microphone back into its output spectrum to keep to the limit;
RNNoise's output gets the same mix of the input it lines up with, a block
earlier. Below 20 dB, typing that DeepFilterNet left got past the gate's level
after gain control, so there's no lighter step. PESQ falls as the limit does,
since it counts what's left of the noise against clean speech:

| DeepFilterNet's limit | Full | 30 dB | 20 dB | 12 dB |
| --- | --- | --- | --- | --- |
| PESQ, typing | 3.05 | 2.81 | 2.62 | 2.30 |
| PESQ, fan | 3.37 | 3.33 | 3.21 | 2.97 |
| PESQ, clean speech | 4.53 | 4.54 | 4.56 | 4.59 |
| Typing left in pauses, mean and loudest block | -79, -53 dBFS | -67, -52 | -58, -45 | -50, -37 |
| Pauses sent with typing | 6.3-6.4% | 6.3-6.4% | 6.5-11% | 81-86% |
| Times typing opened the gate in pauses, each mix | 0 | 0 | 0-2 | 17-19 |

The voice gate opens on three blocks in a row that the suppressor takes for
speech and that are louder than the input sensitivity, -60 dBFS unless
changed, or that are 18 dB louder still. While open, each such block holds it
another 400 ms. Until 2026-10-17 one block opened it. What DeepFilterNet
leaves of a mouse click comes out over two blocks, since its frames overlap,
louder than the sensitivity, and its estimate takes it for speech. With
recorded mouse clicks and office chair creaks in the pauses of speech over a
quiet room (a one-off test, from Wikimedia Commons recordings), one block
opened the gate on 7 of 33 clicks, two on 4, and three on none, and on one
creak of 11. On the mixes below, three cost at most 0.2 points of speech sent.
Under speech, DeepFilterNet turns clicks down only 8-13 dB, rather than about
40 in pauses; the declicker, below, takes out more. Levels are RMS over the block, after gain
control. "Input sensitivity in Discord calls" sets it from -80 to 0 dBFS
(`input_sensitivity`); behind its handle, the microphone's level goes from
green through orange at -18 dBFS to red at -6, with whether the gate is open.
While the setting shows, Yapfast runs the chosen microphone through a call's
processing on a thread of its own, `yapfast-gate-meter`, and sends nothing; in a
call that's a second copy of the processing. DeepFilterNet takes a block
for speech when its own estimate of the block's SNR is at least 5 dB, RNNoise
when its speech probability is at least 0.6. In a simulation of the gate on
the mixes below, a 0 dB threshold let typing open it in pauses, and 10 dB
missed quiet speech. RNNoise's probability beside DeepFilterNet's output did
as well as 5 dB (99.4% of speech sent rather than 99.5%, no opening in pauses
rather than one), so Yapfast doesn't run both. The
gate keeps the last 100 ms while shut and sends them when it opens, so a soft
start such as an "h" isn't cut off; speech goes out that much later until the
gate closes. It stays shut for the first 200 ms after the microphone opens,
while the suppressor learns the noise. Opus stays at 128 kbps in its music
mode, as songbird sent it.

The declicker (`crates/yapfast-core/src/declick.rs`, on unless
`noise::set_declick` turns it off) takes out what DeepFilterNet leaves of mouse
clicks while someone speaks. A click is a burst above 4 kHz whose band above
12 kHz jumps at least 25 dB within half a millisecond and comes within 15 dB of
the whole signal; an "s" or a "t" comes on slower and is weaker up there. It's
found in DeepFilterNet's input, and when it comes out 30 ms later the band
above 4 kHz is held for up to 30 ms to the level it had just before. A
linear-phase filter splits the bands, which puts the output 1 ms later; one
that shifts phase left much of a click between 4 and 10 kHz. With recorded
mouse clicks over speech (a one-off test, as above), measured over the 10 ms
from each of 121 transients, the band above 4 kHz came out louder than without
the click for 9% of them rather than 23%, and quieter, by 4.3 dB at the median
rather than 2, for 75% rather than 60%. It found 107 of the transients. On eight
other readers' recordings it turned the band down 1.2 times a minute (0-3 each),
and never on the mixes' two readers; PESQ on the mixes below moved by at most 0.01. It
costs 11 µs a block. Rebuilding the click's samples from the speech around them
(least-squares AR interpolation) did worse: at 48 kHz an order of 32 spans less
than a pitch period, and a 1 ms gap in speech came back only 7 dB below it.

"Take out your room's echo in Discord calls" (`reduce_room_echo`, off unless
turned on) is for a voice that sounds far away in a bare room: the reverb the
room puts on it, which echo cancellation doesn't touch, since that only takes
out what the speakers play. It's online weighted prediction error (WPE;
Yoshioka and Nakatani 2012, in Caroselli et al.'s 2017 recursive least
squares form), in `crates/yapfast-core/src/dereverb.rs`: in each band up to
5 kHz, the reverb in a 20 ms frame is predicted from the 200 ms of frames
starting 20 ms before it, and subtracted. The first 20 ms of the room, which
the voice needs to sound whole, stay. DeepFilterNet takes some of a room out
on its own, more than RNNoise, but its short view of the past limits it on
late reverb. Its post-filter, tried at
0.05-0.2, cut tails by up to 3 dB more without bringing PESQ closer to the dry
speech, and cost 0.1-0.6 PESQ on the noise mixes, so Yapfast leaves it off. A
search in October 2026 found no open model that takes a room out at 48 kHz in
real time on a processor; LocalVQE does at 16 kHz.

`scripts/compare-room-echo` runs the noise comparison's speech through
synthetic rooms (a direct sound, then a decaying diffuse tail) and the whole
microphone. Results on 2026-10-12, as above:

| Room's reverberation time, reverb against direct sound | 0.4 s, -6 dB | 0.6 s, -3 dB | 0.8 s, 0 dB |
| --- | --- | --- | --- |
| PESQ against dry speech: unprocessed | 2.94 | 2.31 | 1.89 |
| Room echo left (DeepFilterNet alone takes some out) | 3.15 | 2.66 | 2.24 |
| Room echo taken out | 3.44 | 2.86 | 2.40 |
| Reverb 50-150 ms after phrases, unprocessed, left, out, dB | -21, -29, -32 | -14, -24, -26 | -10, -18, -19 |

On the noise mixes, PESQ fell 0.01-0.08 with it on (hiss 3.51 to 3.43), and
on dry speech 0.05. It takes 0.04-0.08 ms more a block, about a fifth more,
and puts the voice 10 ms later. Working up to 24 kHz rather than 5 kHz took
no more out at 1.7 times the cost; 100 ms of history rather than 200 took a
third less out.

Echo cancellation is WebRTC's AEC3 with its defaults, fed the speakers' mix
as it's written to PulseAudio. Fed a recording of a friend's voice through a
simulated room into the microphone (`YAPFAST_AUDIO_FAR` below), it took 42-48 dB
of echo out with the speakers 60-400 ms behind, 37 dB at 500 ms, and none at
600 ms, past what its delay search covers. A room reverberating for 0.65 s or
more left enough of the echo (21 dB taken out) for the gate to send some of
it, which longer filters in AEC3's configuration could help.

`scripts/compare-noise-suppression [LIMIT]` compares the two, and Off through
the whole microphone, at a strength's limit in dB, on 90 s of speech, two
Spoken Wikipedia readings at -26 dBFS with 1.5-4 s pauses, mixed with four
keyboard recordings (95% of blocks below -32 dBFS), a recorded fan high-passed
as WebRTC does (-42 dBFS), pink hiss (-45 dBFS), and pairs of them; for the
gate, the pairs again 14 dB quieter. PESQ is wideband, against the clean
speech. Results on 2026-10-12, with the `fast` profile on a 12-core desktop CPU:

| | RNNoise | DeepFilterNet |
| --- | --- | --- |
| Time per 10 ms block, suppressor alone, mean (p99) | 0.043 ms (0.05) | 0.34 ms (0.46) |
| Time per block, the whole microphone | 0.06 ms | 0.35 ms |
| Microphone thread in a call, fake Discord | 1.4% of a core | 5.9% of a core |
| Output behind input | 10 ms | 31 ms |
| Memory | | 39 MiB more, loaded in 0.2 s |
| PESQ, clean speech | 4.53 | 4.53 |
| PESQ, typing (unprocessed 1.61) | 1.99 | 3.05 |
| PESQ, fan (2.23) | 2.60 | 3.37 |
| PESQ, hiss (1.72) | 2.88 | 3.49 |
| PESQ, typing and fan (1.52) | 1.93 | 2.78 |
| Typing left in pauses, mean and loudest block | -47, -29 dBFS | -79, -53 dBFS |
| Pauses sent with typing, the gate's 400 ms hold included | 58-81% | 6.3-6.4% |
| Times typing opened the gate in pauses, five typing mixes | 2-5 each | 0 |
| Speech sent, all mixes | 99.6-100% | 99.0-100% |

The gate's rows are from 2026-10-17, with the gate opening on three blocks in
a row.

DeepFilterNet is 21 ms later than RNNoise. Both lag by half their 20 ms
window, 10 ms, DeepFilterNet also looks two blocks ahead, and the declicker
adds 1 ms. With the gate's 100 ms, speech goes out 131 ms late. The speech it doesn't send is mostly the first second after
the microphone opens, while its normalization and WebRTC's gain control
settle, and 10-50 ms at the quiet ends of words. These are mixes, not a
microphone in a room, and nothing compared Krisp.

To hear the processing, run a recording through it: raw mono 16-bit 48 kHz in
`YAPFAST_AUDIO_IN`, with `YAPFAST_AUDIO_OUT` and optionally `YAPFAST_AUDIO_SENT`, then
`cargo test -p yapfast-core processes_a_recording -- --ignored --nocapture`.
`YAPFAST_SUPPRESSOR=rnnoise` or `off` chooses the suppressor,
`YAPFAST_NOISE_LIMIT` limits it in dB, `YAPFAST_DEREVERB=on` takes the room's echo
out and `YAPFAST_LEVELING=off` turns leveling off. `YAPFAST_AUDIO_FAR`, in the same
format and as long, is what the speakers played, for echo cancellation;
`YAPFAST_AUDIO_ECHOED` then gets the microphone with only echo cancelled.

### Screen sharing

Go Live streams come through Discord's gateway: op 18 starts sharing, op 20
watches someone, op 22 pauses and op 19 ends either. Discord answers with
STREAM_CREATE, naming the RTC server, then STREAM_SERVER_UPDATE with an
endpoint and token. The stream gets a voice connection of its own to that
server, reusing the call's voice session id; a stream waits for that id too,
if this account's voice state arrives after both answers. `yapfast_discord::stream` names
streams by key (`guild:{server}:{channel}:{user}`) and
`Discord::{go_live, watch, end_stream, pause_stream}` run the signalling. Voice
states carry `self_stream`, which shows as a LIVE badge beside the person.
Sharing starts from the call's controls or from the voice panel's Share
button, which works wherever the window is; the panel says when the stream
is live and stops it. While the call is off screen, the stream being watched,
or else this account's own, shows in a corner (`StreamPip`), as Discord's
picture in picture does. STREAM_CREATE and STREAM_UPDATE name a stream's
viewers in `viewer_ids`; Yapfast keeps them for the stream it shares and the one
it watches, and shows them over the stream (`ViewerStack`) and as a count in
the voice panel; a click on it returns to the call. This account's
own stream opens large from its tile, as others' do, without watching it
through Discord: it shows the worker's preview.

Capture runs in a worker process, `yapfast --stream-worker`, so its buffers take
memory only while sharing; the process exits when sharing stops. Playing a
video file runs there too (`Job::Play`, `video_player`), which also keeps
FFmpeg's reading of files people sent out of the window's process. Its
frames carry their time, which the seek bar shows.
`stream_worker` starts it with one job as a JSON line and reads its reports
back. On Linux it starts `/proc/self/exe`, the running Yapfast, since Yapfast's path
names a deleted file once a rebuild or update replaces it. Frames reach the UI through `frames`, a triple buffer in a memory-mapped
file in the runtime directory. The file is unlinked once the worker opens it.
Windows won't delete a mapped file, so there it's a temporary file that
Windows deletes once both processes close it.
On Linux, libwebrtc's capturer asks the screen sharing portal what to share.
The portal answers over D-Bus through GLib, so the worker runs GLib's main
context while it waits for the dialog and for frames. Yapfast tells Discord it's
live only once frames flow, so a cancelled dialog shares nothing. Windows has
no such dialog, so the share menu lists the screens and windows libwebrtc's
capturers find (`stream_worker::shareable`), and the worker captures the one
picked by its id; see the
[Windows guide](windows-beta.md#go-live-on-windows).
`YAPFAST_STREAM_TEST_PATTERN=1` shares moving colour bars instead of a screen.

Once Discord has started the stream, Yapfast unpauses it with op 22, as
Discord-video-stream does, and the worker gets its server and sends the video
there (`yapfast_discord::stream_connection`). The connection is a call's
(`voice_connection.rs`), on voice gateway version 8 as Discord's clients
use: heartbeats acknowledge the last sequence number, and binary DAVE
messages start with one. It identifies with the stream's RTC server and `video: true`,
discovers its address, then uses `aead_aes256_gcm_rtpsize` transport
encryption. Yapfast offers H264 only and announces its size with op 12, again
whenever someone connects. Frames come from OpenH264 in libwebrtc's archive
through `native/h264.c`: constrained baseline, realtime and tuned for screen
content, with a keyframe at least every 3 s and whenever RTCP asks for one.
Yapfast also has VP8 (`native/vp8.c`, still built and tested); every Go Live
sender known to work sends H264. H264 goes as whole NAL units or FU-A
fragments, and past a frame's first 8 packets, 4 go out each millisecond: a
1080p keyframe of 200 sent at once can overflow buffers on the way. Yapfast keeps
the last 2048 video packets, and when a viewer reports some lost with a
generic NACK (RTCP 205), resends up to 128 of them as Discord's client does:
on the RTX SSRC with RTX's payload type and numbering, the original sequence
number before the payload (RFC 4588), and the repaired stream id (extension
12) in place of the stream id. The fake server reports one packet lost and
checks what comes back. An RTCP sender report goes out every second.

Before sharing, the share button's menu picks a resolution and a frame rate,
as Discord's client asks for them: 720p, 1080p or the screen's own size (up to
4K), at 15, 30 or 60 fps. Yapfast remembers the choice beside the chosen audio
devices (`devices.json`), and starts at 1080p30. Larger screens, and those of
another shape, are scaled down to fit. Yapfast sends them at the bitrates
Discord's client uses: at most 3.5 Mbit/s up to 720p and 9 Mbit/s above, the
cap op 12 announces, with the frame rate as its `max_framerate`.
Like Discord's client it also follows the server's bandwidth estimate
(REMB): when that drops below the cap, the encoder aims for the estimate
instead, though never below 500 kbit/s, and it changes only when the two
differ by more than a tenth. Discord's client allows accounts without Nitro
720p30, but trials 1080p30 at 3.5 or 5 Mbit/s for them (its
"2026-05-frontier-tuning" experiment). The worker converts captured
frames to I420 with libyuv, scales them with libyuv's filtering scaler, and
encodes those; the preview is the same picture scaled down to 720p.

Discord shows a still of each stream, in the call and to people outside it.
Like Discord's client, Yapfast makes one once the stream connects and the
picture isn't black any more (a screen is black until what's shared draws),
then every 5 minutes: the picture scaled down to fit 512 by 288, a JPEG at
quality 92. The worker reports it as a data URI, and Yapfast uploads it with
`POST /streams/{stream key}/preview`. A failed upload is tried again a minute
later, up to three times; a newer still replaces an upload not done yet, and
stopping the share cancels it. The fake server takes previews of Alice's
stream only, and lists them at `/_control/video`.

A shared screen carries the sound every other application plays, as Discord's
client and Vesktop share it; Yapfast's own sound stays out, so viewers never hear
the call echoed back (`stream_audio`, with `audio_io::others` hearing the
applications). On Linux, PulseAudio or PipeWire's
PulseAudio server monitors each application's playback stream on its own
(`pa_stream_set_monitor_stream`, as pavucontrol's level meters do), and
streams from Yapfast's processes are skipped by process id. So are other Discord
clients' (Discord, Vesktop, Equibop, WebCord, Legcord, ArmCord, by application
name or program): with one of them in the call too, its sound echoed the call
back to the people in it. The worker mixes them every 20 ms, waiting until an
application has 40 ms queued and keeping at most 200 ms of each, and encodes
Opus at 64 kbit/s, what Discord's client gives a stream's sound. Frames go out
as plain Opus RTP on the stream's audio SSRC, DAVE-encrypted, with sender
reports, and op 5 marks them as shared sound (speaking flag 2). Nothing is
sent while no application plays. On Windows, WASAPI's process loopback hears
everything but Yapfast's own process tree as one stream, from Windows 10's 2004
update on; it can't leave other Discord clients out. On macOS, from 14.2 on, a
Core Audio process tap hears every process using sound as one stream too, but
Yapfast's own and other Discord clients', by process, program or bundle id,
once macOS has asked whether Yapfast may record it; see
[the macOS guide](macos.md#go-lives-sound). Other systems share video only
for now, and so does the test pattern. Two ignored tests check the capture against the
machine's sound server with a quiet tone: another process's is heard, and
Yapfast's own is not.

Each video packet carries the one-byte extensions a Discord client sends: a
transport-wide sequence number (id 5), playout delay (6), screen content
type (7) and the stream id `100` (11). The stream id is what ties a packet to
the stream op 12 declared. These ids aren't the ones in Discord's WebRTC
SDP, where 5 is playout delay. Yapfast once sent only playout delay as id 5, and
Discord's server dropped every packet: its receiver reports said it had
received nothing, while it did read our RTCP, and viewers timed out with
error 2012. concord and IMDiscord, two clients that broadcast over this
transport, use the same set. The fake server drops video without the stream
id as well.

DAVE follows songbird's handshake (`dave.rs`) and encrypts each frame once the
group is ready. A stream's MLS group isn't the voice channel's: its id is the
RTC server id minus one, which Discord-video-stream found too. If the server's
proposals or commits name another group, Yapfast moves to it. The vendored
davey's `set_channel_id` keeps the key package already sent, so the streamer's
welcome still opens. Yapfast accepts the people op 11 names into the group, and
also those named in video attributes (op 12) or in the stream key.

With `YAPFAST_LOG=debug`, Yapfast logs every 5 s while sharing what it sent and the
RTCP that came back (`stream sent`): receiver reports with the highest
sequence number Discord's server got and its echo of our sender reports
(`lsr`), its bitrate estimate (REMB), and keyframe requests, and each change
of bitrate. The voice connection logs the voice server's RTCP too, to compare
with a sender Discord accepts. Messages the stream server sends are logged
too, except those carrying keys.

Watching works the same way in reverse. Clicking someone's LIVE badge, or
their tile in the call, sends op 20 and starts a worker with a watch job. The
worker connects as a viewer, offers H264 and VP8 for decoding only, and asks
for video with op 15. It learns the streamer's SSRCs and codec from op 12 and
op 14, then puts RTP packets back in order (`rtp::Reorder`) and together
into frames (`rtp::Assembler`: VP8 descriptors, and H264 single units,
STAP-A and FU-A as Annex B). As Discord's client does, it asks for a missing
packet again at once with a generic NACK, again every 80 ms up to three
times, and takes the resend from the streamer's RTX SSRC (RFC 4588); padding
there, which probes the path, is dropped. Packets after a gap wait for it at
most 300 ms. A frame still missing a packet is dropped, and so is everything
after it until the next keyframe. So are frames while none has come yet,
and after one the decoder was too far behind to take. The connection asks
for a keyframe with a picture loss indication, at most every 500 ms. It
sends a receiver report every second (packets lost, the highest received,
and the streamer's last sender report echoed). With `YAPFAST_LOG=debug` it logs
what it received every 5 s (`stream received`): packets out of order,
resent, recovered, given up on, and keyframes asked for. The fake server
loses one packet of each stream it sends a viewer and delivers another late,
and resends what's asked for. The worker decodes with
libvpx or with the FFmpeg H264 decoder in libwebrtc's archive
(`native/decode.c`) and writes RGBA at up to 1080p into the shared frames.
Watching is possible only in the call you're in, so clicking someone sharing
in another channel joins it first, once asked, and watches once the call is
connected; Discord drops a watch sent before. The stream's sound, Opus on
the streamer's audio SSRC, is DAVE-decrypted as audio and played by the worker
through `audio_io.rs` on the call's speakers (`stream_audio::Player`), about 100
ms queued; being deafened silences it, which Yapfast tells the worker with a
`hear` control. Yapfast's workers play as "Yapfast", which sharing leaves out, so a
stream you watch never reaches one you share. Watching a stream from Discord's
own client works: H264 with DAVE, confirmed by hand.

The fake server checks the transport, packetization, sender reports and frames
that Yapfast sends, the sound's Opus packets and soundshare flag, reading an H264
picture's size from its SPS, without DAVE. To viewers it streams the
recordings in `testing/fixtures` (VP8, or H264 after `POST
/_control/stream-codec`) once they ask with op 15. Checked by hand against
real Discord: Discord's client shows Yapfast's stream at 1080p with its sound,
without the call's, and Yapfast shows streams from Discord's client.

## Servers, channels and messages

Yapfast shows Discord live from its own Discord connection. Every server, channel
and DM is visible without copying anything into Matrix.

Yapfast lists the linked account's Discord servers in the rail with their
channels. Channels the account's roles and channel overwrites keep it out of
show greyed out with a lock, as in Discord; channels hidden from it appear the
same way only with the "Show channels you can't access" setting, which is off
by default. Server text channels are listed under their Discord categories and
open in Yapfast's timeline. Clicking a category's heading collapses it, leaving
the open channel, unread ones and voice channels with people in them, as in
Discord; Yapfast remembers which categories are collapsed, per server. History
loads from Discord 50 messages at a time (`GET /channels/{id}/messages` with
`before` the oldest loaded message and `limit=50`) as the timeline scrolls
near its top, one page at a time per channel, until a short page shows the
channel's first message is there; and new messages, edits, deletions
and reactions arrive live. A jump to a message that isn't loaded, from a
reply's quote, a pin or a search result, asks for the 50 around it
(`around` its snowflake), as Discord's client does, and they show in place
of the timeline as a window of older messages; see [jumping to a
message](#jumping-to-a-message). Opening a channel subscribes to its server, as the
web client does, because Discord sends large servers' messages only to
subscribers. Yapfast sends, replies, edits, deletes,
reacts and sends files through Discord's cloud uploads. Mentions and timestamps
show as text, custom emoji and stickers as pictures, every attachment shows,
images with previews scaled by Discord's media proxy, and channels the account
can't send in are read-only. Bots and webhooks have a BOT tag after their
name. An author's name takes their highest coloured role's colour, once their
roles are known: live messages carry them, and for history Yapfast asks Discord
for the authors' member profiles.

Yapfast keeps the open channel's messages in memory, and those of the 40
channels open before it, so going back to one shows it at once. A closed
channel keeps its newest 200 messages as metadata and follows new messages,
edits, deletions and reactions while it's kept. All channels share one
8 MiB cache for decoded preview pixels, with least recently used eviction.
Only messages in or near the timeline viewport request previews; evicted
previews can be decoded again from the disk cache. Opening it again loads the
latest page in the background and fits it in: messages kept within the page's span that it lacks were deleted
meanwhile, and when a full page starts after everything kept, more came than
a page holds, so only the page stays. The UI keeps the same 40 channels'
last timeline and member list, so going back shows them before the chat side
answers, without "Loading messages...". It keeps what the chat side keeps of
each timeline, the newest 200 messages as metadata. It drops decoded preview
handles when closing a channel, so remembered UI timelines do not pin pixels
outside the shared cache. Previews show as loading until the viewport requests
them. Channels also go to disk, as the startup cache below describes, so a
channel opened after a restart shows its saved messages while the latest page loads
and fits in the same way.

The open channel renders each message for the timeline once, with its
author's name, colour and avatar and its mentions as names, and keeps it. A
new message, an edit, a reaction or a preview that arrived renders only that
message again; a change to someone's name, avatar or roles, to the
server's roles or channel names, or to who's blocked, renders them all. A closed channel drops its rendered messages,
since they hold its previews.

Unread dots and mention counts follow Discord's read states, mutes and
@everyone and role suppression, including passive updates from large servers.
A message that calls on this account is highlighted in the timeline, as in
Discord: one that mentions it, one of its roles in the server, or @everyone
or @here, from a message's `mentions`, `mention_roles` and
`mention_everyone`, which count only what the author may mention. The
account's settings for the server can suppress @everyone and @here, and
roles. A reply to its message is highlighted too, whether it pings or not,
but only while it's new: after the last message read when the channel or
thread opened, by snowflake. A ping reply puts the account in `mentions`,
so it's a mention only when its `content` names it, `<@id>` or `<@!id>`.
Each server in the rail shows its mentions as a count, and the DM button
counts DM mentions and the friend requests waiting for an answer.
Showing an unread channel marks it read, and with the "Mark messages read on
Discord" setting, on by default, acknowledges it on Discord within a second.
Reads within that second go together: one channel as its own
acknowledgement, several, as when a server is marked read from its menu, in
one `POST /read-states/ack-bulk` of up to 100 channels, as the official
client does.
Before it does, Yapfast notes the channel's last read message, from READY's read
state or the last acknowledgement since, and the "New" divider goes above the
first message after it from someone else. Snowflakes order messages, so the
divider finds its place even when the last read message was deleted. When
every loaded message is unread, the 50 around the last read one load in
place of the newest, as a window of older messages that opens at the
divider; when even those are all newer, the divider goes above the oldest.
The open text channel lists its members on the right as Discord's sidebar
does: under hoisted roles, then online, then offline, with offline members
faded. Each shows a status dot for online, idle or do not disturb, their name
in their highest coloured role's colour, and their custom status or what
they're playing below it. Yapfast shows the list's first 100 rows, headings
included. Discord keeps one list for all channels with the same readers, and
its id says who they are: `everyone` when @everyone sees the channel and no
overwrite hides it, else a MurmurHash3 of the overwrites that allow or deny
seeing it, as Discord's client computes it. Yapfast keys its lists by that id, so
moving between such channels shows the same list and asks Discord for
nothing, and an update applies only to the list it names. Yapfast keeps the last
40 lists. An INVALIDATE, by which Discord stops sending rows, leaves them as
they were, to show when their channel is again. One Discord still sends, or
sent within 10 minutes, shows without asking again. An older one shows at
once while Yapfast subscribes, and Discord's sync replaces it. A list
on screen that Discord doesn't send is asked for again once it's 10 minutes
old. When no list comes within 8 seconds of opening a channel, the panel
says Discord didn't send one instead of loading on, and the header says
"Members" until a count is known. A build without Matrix shows the Discord
account's name, avatar and connection in the bottom-left panel.

Yapfast starts from Discord as it last saw it, kept in the data directory by
`yapfast_discord::cache`, and Discord's answers replace it as they arrive:

- `discord-account.json` holds the account's name and avatar, its servers
  with their channels, roles and emoji, its DMs, friends, friend requests and
  status. It also holds the names Yapfast learned (users, server nicknames,
  server avatars and members' roles), which messages and member lists show
  people by, and the member lists shown lately. Every message Discord sends
  from any server teaches Yapfast its author, so it keeps the 20,000 people it
  learned most lately and forgets a quarter of them, those learned longest
  ago, once there are more. It's written at most every 20
  seconds while connected. Calls and who is in voice aren't saved. On the
  next start Yapfast shows it all at once, identifies with the saved status, and
  READY replaces it. A saved member list shows at once, even before Yapfast
  connects, and Discord's sync replaces it. The list on screen is kept with
  them when the connection drops, so it shows again at once after
  reconnecting.
- `discord-messages/` holds, for each channel shown lately, its newest 100
  messages, written within 20 seconds of a change, when the channel leaves
  the 40 kept in memory, and when Yapfast closes. The 300 channels used last are
  kept. Opening a channel shows its saved messages, and the latest page fits
  in as it does for a channel kept in memory.
- `discord-pictures/` holds avatars, server icons, emoji and stickers, and the
  media proxy's previews of attachments and links, up to 8 MiB each. Discord
  gives each version of a picture its own address, so a kept one is never
  stale; the proxy's signature (`ex`, `is` and `hm`) isn't part of the key,
  so a picture signed again is found. Past 256 MiB the least recently shown
  go. Attachments' own files aren't kept.

The files are readable only by the user and aren't encrypted. Each names its
login by a SHA-256 hash of the token, so another linked account starts empty,
and unlinking deletes them all. While Discord is unreachable, an open channel
shows what was saved, waits, and loads once Yapfast is back, rather than showing
an error.

- Discord servers appear in the server rail with a Discord badge, and Discord
  DMs appear in the DM list with the same badge. A build without Matrix leaves
  the badge off, since everything in it is Discord's.
- A build without Matrix leaves out the composer's line about encryption and
  keys, since everything in it is Discord's; what stops sending still shows
  there. The account's dot in the bottom-left shows its Discord status: idle,
  do not disturb, or the grey ring when invisible or disconnected.
- Discord servers keep the order the account's server list has in Discord,
  from `guild_folders` in its settings, and follow it when it changes
  elsewhere. A server the settings don't list yet, such as one just joined,
  goes first.
- Folders show in the rail as in Discord. Closed, a folder is a tile of its
  colour holding its first four servers' icons, with the pill, unread dot
  and mention count of all its servers; its tooltip is its name, or its
  servers' names when it has none. Clicking opens it: a folder icon in its
  colour, its servers below it on a strip of that colour. Yapfast remembers the
  open folders on this device, in `layout.json`, as Discord's client keeps
  them per device. Settings entries with an id are folders, read with their
  name and colour (`GuildFolder` fields 2 to 4, each in a wrapper); one
  without is a server on its own. Folders can't be made or changed in Yapfast
  yet. `--folders` gives the fake Discord's filler servers two folders.
- A server out of reach in a Discord outage (`GUILD_DELETE` with
  `unavailable`, or READY listing it so) stays in the rail, faded, as last
  known, until its `GUILD_CREATE` brings it back. Only a `GUILD_DELETE`
  without `unavailable`, as when the account leaves, removes it.
- Server icons and avatars come from Discord's CDN, which needs no login.
  Yapfast asks for each one once it shows: in the rail, beside people in a
  voice channel of the open server, in the call, or beside a message. A
  member's server avatar wins over their own. Yapfast decodes them at 128px
  before the UI sees them. Once more than 400 have arrived, those off
  screen go, and Yapfast asks for them again, from disk, when they show.
  Custom emoji stay, since the picker asks for hundreds at once.
  Initials show until a picture arrives or when it fails. A download that
  may work later, when the CDN is out of reach, fails or answers 429, is
  tried up to six times; every download then waits together, 10 seconds
  and doubling up to five minutes, so a CDN in trouble hears from Yapfast
  rarely. One Discord refuses, such as a 404, isn't tried again. A server
  without an icon keeps its initials, and a person without
  an avatar gets Discord's default avatar for their account.
- Who reacted comes as Discord's client asks for it: `GET
  /channels/{c}/messages/{m}/reactions/{emoji}?limit=3&type=0` for a
  reaction's tooltip, and `limit=100` with `after`, the last user id, for each
  page of the Reactions window. Users come by id; those not known in the
  server are asked for over the gateway first, so the names are server
  nicknames, as Discord's client shows them. A moderator removes someone's
  reaction with `DELETE …/reactions/{emoji}/0/{user}?location=Message&burst=false`.
  Super reactions aren't listed apart yet.
- Messages support text, replies, edits, deletions, reactions, mentions and
  attachments. Up to ten files go with a message, uploaded to Discord's
  storage first and then named in the message. Each file is limited as
  Discord limits it since 13 August 2026: 20 MB without Nitro, 50 MB with
  Nitro Basic or the old Nitro Classic, and 500 MB with Nitro, by the
  account's `premium_type`. A server boosted to level 2 allows 50 MB and one
  at level 3 100 MB to everyone, whichever is higher. Discord sends the
  server's `premium_tier` in READY and GUILD_UPDATE, and the account's
  `premium_type` in READY and USER_UPDATE. The web client's own constant
  without Nitro is still 10 MiB, but `getMaxFileSize` raises every limit to
  at least 20 MiB (`Math.max(0x1400000, …)`) before it checks a file or names
  the limit, so 20 MB is what it enforces.
- Link previews and bots' cards show from the embeds Discord adds to a
  message: Discord's servers read the linked page's Open Graph tags, often a
  moment after sending, and the preview arrives in a message update. A card
  shows the site, author, title, description, fields and footer behind a bar
  of its colour, with its picture large for articles and videos and small
  beside the text otherwise. A video the media proxy has (`video.proxy_url`)
  plays in place, as an attached one does, under the card's thumbnail as its
  poster, whatever the card's type: vxtwitter's posts come as `rich` cards
  with one. Others, such as YouTube's, open in the browser. Other pictures open in the image viewer. A linked image or GIF
  shows with the message's images, without its address when that's all the
  message says. Pictures come through Discord's
  media proxy (`images-ext-*.discordapp.net`), so Yapfast never contacts the
  linked site, and embeds of a message marked to hide them don't show.
- Animated GIFs, APNGs and animated WebPs play, in messages, as stickers and
  in the image viewer. The media proxy keeps GIFs as GIFs; other pictures
  come as WebP, asked for with `animated=true` as Discord's client does, so
  an animated one, such as vxtwitter's AVIF GIFs, stays animated rather than
  turning into its first frame. An animation keeps its first frame in
  memory, and its file in `discord-pictures/`, or in memory when it isn't
  kept there, as one over 8 MB isn't. Its frames decode one at a time on the
  UI thread as they show, reading the file, and a second after it last
  showed, as when its row scrolls away, it lets its decoder go. Decoded
  whole, a few seconds of a GIF took tens of megabytes, and an open channel
  kept every one it had loaded. Frames shorter than 20 ms play at 100 ms, as
  browsers do. Tenor and Giphy links arrive as MP4 "gifv" embeds, which play
  the same way, silent and looping, from the proxy's copy of the MP4
  (`video.proxy_url`): up to a minute of it, decoded by the FFmpeg, libvpx
  and dav1d inside libwebrtc (`native/video.c`). One that can't be decoded
  shows its still picture. It's named and saved as its video, not its
  picture.
- Attached videos show their poster, which the media proxy makes from the
  first frame (`proxy_url?format=webp`), with a play button; they play in
  place, with sound, under a bar to pause, move through them, mute and show
  them full screen. Full screen, as the image viewer shows a picture, the
  video is enlarged to fit the window with its bar under it; Space plays or
  pauses it, the arrow keys move five seconds, and Escape brings it back to
  its cell. The worker draws its pictures at the size they show: playing,
  the next ones come at a new size; paused, the one showing is redrawn,
  which moving back to it would hold up while playing. MP4, MOV and WebM
  play, with H264, VP8, VP9 or AV1 pictures in 8-bit 4:2:0 and AAC, Opus,
  MP3, Vorbis or FLAC sound; HEVC, which iPhones record, doesn't, as
  libwebrtc's FFmpeg lacks it. The file downloads into `discord-videos/`
  beside the account file, and plays as it arrives: the worker's reads wait
  for the download. It goes when the video stops, as when the channel
  closes. Sound plays on the call's speakers through `audio_io.rs`, on
  Linux and Windows. Over the mute button a slider sets how loud, as does the
  wheel; its travel is squared, so its middle sounds about half as loud.
  Turning it to nothing mutes, and unmuting brings back the level it had.
  The next video plays as loud, until Yapfast closes.
- Spoilers, `||like this||`, show under an opaque cover that a click lifts.
  The cover is drawn by Yapfast's patched Slint text renderer over a link to
  `yapfast:spoiler/{item}/{n}`; see [building](building.md). A spoiler in a reply
  or a notification reads "(spoiler)". Attachments named `SPOILER_...` or
  flagged as spoilers, and pictures linked inside a spoiler, show blurred with
  a "Spoiler" label until clicked. Revealing lasts while Yapfast runs. The eye
  on a file waiting in the composer marks it as a spoiler, which blurs its
  thumbnail; Yapfast uploads it as `SPOILER_` and its name, as Discord's client
  does. Matrix has no spoiler for a file, so its rooms don't offer the eye.
- Custom emoji show as pictures: in messages, as reactions and in the emoji
  picker, which lists them by server after the frequently used ones, the open
  server's first. In messages Yapfast draws them as text: each emoji whose
  picture arrived gets a code point in the Supplementary Private Use Area B,
  and each batch of them a colour bitmap (CBDT) font made on the fly, which
  text falls back to after Satoshi, so they wrap and select like letters.
  The picker, suggestions and reaction buttons show the picture itself, so
  the hundreds a picker lists add no fonts; a quiet square holds each place
  until its picture arrives. A message of only
  emoji, up to 30, shows them large, as Discord does. Without Nitro, the
  picker offers only the open server's emoji, none animated, and sending
  turns `:name:` into Discord's `<:name:id>` only for those. Animated emoji
  show their first frame.
- Stickers show as pictures at 160px, PNG and APNG stickers from the CDN and
  GIF ones from the media proxy. Lottie stickers, which are vector
  animations, show their name.
- Messages from people this account blocked show collapsed as "Blocked
  message", which "Show it" opens, and they don't notify or show typing.
  Yapfast learns the blocked from READY's relationships (type 2) and keeps them
  current from RELATIONSHIP_ADD and RELATIONSHIP_REMOVE.
- A reply's quoted line jumps to the original and highlights it for a
  moment, as described under [jumping to a message](#jumping-to-a-message).
- Desktop notifications follow the person's Discord settings: each channel's,
  category's or server's level (every message, mentions only or nothing),
  falling back to the server's default, with mutes leaving only mentions. DMs
  notify unless muted; message requests and spam don't. Apps' answers to
  slash commands and context menu commands notify as people's messages do;
  joins, pins and call notices don't. Nothing notifies while the account's
  status is Do Not Disturb. They show while Yapfast
  isn't in front, titled with who wrote where, and clicking one opens the
  conversation. On Linux they go to the freedesktop notification service.
  Each also plays Yapfast's message sound, or its mention sound, unless it's in
  the conversation in front; see [voice](voice.md#sounds).
- Mentions in messages are links on a light fill, in the role's colour or the
  person's name colour, as Discord shows them. A person's opens their profile
  card beside the mention, and a role's lists the members Yapfast has seen with it (from member lists, messages
  and member requests), each opening their card in turn.
- Typing `@` suggests who to mention: the channel's listed members, those who
  wrote lately, and server members a search by name finds (the gateway's
  member request with a query, as Discord's client does), then the roles
  anyone may mention, and every role with @everyone and @here for those
  allowed to mention everyone. Sending turns each `@Name` into Discord's
  `<@id>` or `<@&id>`.
- Who else is typing shows above the composer, for ten seconds after Discord
  last said so or until their message arrives. Writing tells Discord Alice is
  typing, at most every eight seconds, unless a setting turns it off.
- Reading a message in Yapfast marks it read on Discord, so the phone app agrees.
  A setting turns this off.
- Clicking the account's avatar or name, bottom left, opens a menu of Online,
  Idle, Do Not Disturb and Invisible, as in Discord. Yapfast saves the choice in
  the settings below and sends it with opcode 3. Do Not Disturb holds back
  Yapfast's message notifications. Closing Yapfast takes the person offline, like
  closing the Discord app.
- The custom status shows under the account's name. "Change custom status" in
  that menu, or "Set a custom status" in My account, sets it: an emoji from the
  picker, words, and when it clears (never, 30 minutes, an hour, four hours,
  or at midnight). Discord keeps it in the account's settings, a protobuf
  (`PreloadedUserSettings`, field 11), which arrive in READY as
  `user_settings_proto` and change in USER_SETTINGS_PROTO_UPDATE. Yapfast reads
  only the status and custom status from it, and changes either with `PATCH
  /users/@me/settings-proto/1`, whose body sets nothing else. Each session
  shows the status to others itself, so Yapfast sends it with opcode 3 once
  connected, whenever the settings change, and clears it when it ends. A
  custom emoji in a status needs Nitro.

### GIFs

The GIF button left of the emoji button, or Ctrl+G, opens a picker with two
tabs, GIFs and Emoji, as Discord's does; Ctrl+E opens it on Emoji. Discord
finds GIFs through Klipy, and Yapfast asks Discord as its desktop client does:

- The home is `GET /gifs/trending?media_format=tinywebp&locale=en-US`:
  trending terms, each with a picture, and one trending GIF. It shows a
  Favorites tile, a "Trending GIFs" tile and the terms, 110px tall, two to a
  row. A term searches for itself; Trending GIFs lists `GET
  /gifs/trending-gifs`.
- Typing searches with `GET /gifs/search?q=` a quarter second after the last
  key, as Discord's does. Results come in Discord's masonry: columns at
  least 200px wide with 12px gutters, each GIF going to the shortest. Within
  1180px of the end, `GET /gifs/suggest?q=&limit=5` offers other terms, as
  buttons under the results ("Your perfect GIF is in another castle").
- A click sends the GIF's page URL as a message, keeping a reply that was
  being written, and a search's pick is reported with `POST /gifs/select`
  `{id, q}`. Discord unfurls the URL into a gifv embed, whose picture Yapfast
  plays. Shift and a click puts the URL in the message instead.
- Only the GIFs in sight, and a screen either side, load and play; they come
  from Klipy's or Tenor's media hosts, at the size they show, and go when the
  picker closes.
- Favourites are kept as Discord's are, in the second settings protobuf
  (`FrecencyUserSettings`, field 2): a map from each GIF's page to its
  picture, size, format and order. `GET /users/@me/settings-proto/2` reads
  them, the first time the picker opens or a star is pointed at, and `PATCH`
  writes all of them back; USER_SETTINGS_PROTO_UPDATE with type 2 brings
  changes made elsewhere. A star at a tile's top right, or at the top left of
  a GIF in a message, adds or takes one out; a new one gets the highest
  order, and the Favorites view lists the newest first, searchable by name.
  A message's picture has the star when it moves: a gifv embed, or an
  attachment or image embed whose picture Discord flags `IS_ANIMATED` or
  serves as `image/gif`, or one named `.gif`. As in Discord's client, the
  star keys it by the gifv embed's page, an image embed's own link, as when
  a favourite saved under a direct link to its file is sent, or the
  attachment's address. An address on Discord's CDN or media proxy under
  `/attachments/` or `/ephemeral-attachments/` loses its `ex`, `is` and
  `hm`, the signature that expires; any other stays as it is.
- Favourites show at once, without waiting on Discord or the GIF providers.
  Yapfast keeps the list on disk, shows it when the picker opens and replaces
  it when Discord answers; a star used meanwhile shows at once and is
  applied to Discord's list once it comes, so a change made on another
  device stays. Each favourite shown gets a thumbnail in `discord-gifs/`,
  50 MB at most, the least lately shown going first:
  - Tenor's own small MP4 when the GIF is Tenor's: its links name the
    format with a code, `AAAP1` for that MP4, which took 106 KB at the
    median on Tenor's GIFs, to the full GIF's few megabytes.
  - The picture itself, when it's a video or no larger than 64 KB, which
    keeps a small GIF's transparency.
  - Otherwise a VP9 WebM of it at the tiles' width, made once, off the UI
    thread, with the VP9 encoder already in Yapfast's libwebrtc. Measured on
    Tenor's full GIFs at 231px, they took 14-297 KB, 40 KB at the median: a
    fifth of an animated WebP at the same quality, and a fifth of its
    decoding work. A transparent GIF goes over the panel's tone, as VP9
    keeps no transparency.

  The picture downloads to disk as it comes, so a full-size GIF of tens of
  megabytes takes no memory, and the thumbnail plays from its file.

### Jumping to a message

A reply's quote, a pin, a search result and the bar over the "New" divider
jump to a message. A loaded one scrolls into view. Otherwise Yapfast asks for
`GET /channels/{id}/messages?around={id}&limit=50`, as Discord's client does,
and the messages around it show in place of the timeline: a window of older
messages, with "You're viewing older messages" and "Jump to present" in a bar
over its end. Discord answers around a snowflake even when no message has it,
so a deleted message's place shows, and Yapfast says it couldn't find the
message. A jump into another channel opens that channel at the message.

Scrolling down the window loads the 50 `after` its newest message, and up,
the 50 `before` its oldest, a page at a time. Discord doesn't say how it
splits a page around a snowflake, so Yapfast learns where the channel starts
from a page before, and that the window reached the newest message from a
short page after, or once it reaches the newest messages Yapfast kept.

While the window shows, the channel's newest messages wait beside it, at
most 200 as a closed channel keeps, and follow new messages, edits,
deletions and reactions; edits, deletions and reactions show in the window
too, but new messages don't go in it. The channel stays unread, and isn't
marked read, until the newest show. Jump to present, sending a message, or a
page that reaches them shows them again without asking Discord, and the
window goes. Leaving the channel lets the window go too, and only the
newest go to disk. Pages loading for a window that a newer jump or the
present replaced are dropped when they come.

The window holds what was asked for, not every page between it and the
present. Growing upwards past 200 messages, it lets go of its newest,
below the screen, which load again; growing downwards it keeps its oldest,
which are above the rows on screen and would move them if they went, until
it joins the newest messages.

### Notification settings

Discord keeps the account's notification settings per server, and one set
for DMs: READY lists them in `user_guild_settings`, and
USER_GUILD_SETTINGS_UPDATE replaces a server's
(`yapfast_discord::notifications`). Each holds the server's mute, with
`mute_config`'s `end_time` when it ends, its level (every message, mentions
only, nothing, or the server's own default, `default_message_notifications`),
whether @everyone and @here, and role mentions, call on the account, whether
muted channels hide, and `channel_overrides`: each channel's and
category's own mute and level, where 3 means the category's or server's.
Yapfast publishes each server's settings with the server, shared between
snapshots until Discord changes them, and marks each channel muted when it
or its category is. A muted channel shows faint, as a muted DM does, and a
server set to hide muted channels leaves them out of the channel list, but
for the open one and those that mention the account, whose count the rail
shows. A mute with an end lifts by itself then: the session wakes at the
soonest end and publishes again.

`PATCH /users/@me/guilds/{guild}/settings` changes them, with `@me` for DMs.
The body names only what changes, and Discord keeps the rest; a channel's or
category's goes in `channel_overrides`, keyed by its id rather than listed:
`{"channel_overrides": {"<id>": {"muted": true, "mute_config":
{"selected_time_window": 900, "end_time": "..."}}}}`. A mute for good is
`selected_time_window` -1 and no end, as Discord's client sends it, and a
level is `message_notifications`. `Action::ChangeNotifications` asks for a
change to a server, by its server id, or to a channel or category, by its
room id: a mute for one of the DM menu's times, a level, or one of the
server's switches. A server's menu and its header's dropdown offer the
server's own. The fake keeps each server's settings and answers with
USER_GUILD_SETTINGS_UPDATE.

A server text channel and a category offer their own mute and level, as
Discord's do, built alike in `src/ui/menus.rs` (`Notify`):

- The bell in the channel's header opens a menu of "Mute channel", whose
  submenu mutes for 15 minutes, 1, 3, 8 or 24 hours, or until unmuted, or
  "Unmute channel" with when the mute ends, then the levels: "Use category
  default", or "Use server default" outside a category, under which the
  level it follows shows, then All messages, Only @mentions and Nothing,
  the chosen one checked. The bell is slashed while the channel is muted,
  itself or through its category.
- Right-clicking the channel offers the same: "Mute channel" with its times,
  or "Unmute channel", and "Notification settings", whose second line is the
  level that applies and whose submenu lists the levels.
- Right-clicking a category's heading, or the Menu key on it, offers "Mute
  category" or "Unmute category" and "Notification settings", for the
  category's own override; a muted category's channels show muted. The
  heading names its first channel, and Yapfast finds the channel's category
  (`Application::category_of`); an empty category's heading names the
  category itself.

### Permissions and roles

What the account may do in a server is its @everyone role's permissions
and those of its roles, everything for the owner and administrators
(`Server::server_permissions`); in a channel, the channel's overwrites for
@everyone, the roles and the member then apply
(`Server::channel_permissions`). Acting on another member, to kick, ban,
time out, rename or change their roles, also needs to outrank them: nobody
outranks the owner, the owner outranks everyone else, and otherwise the
highest role must be higher (`Server::outranks`). A role can be given or
taken by those who may manage roles and whose highest role is above it,
or by the owner, but never @everyone or one an integration manages
(`Server::may_grant`). The core asks through `Application::may`,
`outranks` and `may_grant`, and `server_roles` lists a server's roles,
highest first, with their colours, from the server the latest snapshot
shares. A category keeps its overwrites too, so `may` answers for one by
its room id, and the channel list leaves out an empty category its
permissions hide. Members' roles are learned with their names; member list rows and
`ChatMember::roles` share each member's list with them rather than copying
it.

### A server member's menu

Right-clicking someone in a server channel's member list, or a message's
author there, opens their menu in the order of Discord's: Profile, Mention,
Message, Call, Add note and the friend nickname; Change nickname, [Invite to
server](#invite-to-server), friendship, Ignore and Block; Roles; in red,
Time out, Kick and Ban, each naming them; then Copy ID, last as in
Discord's `GuildChannelUserContextMenu`. In a voice channel it has more; see
[Someone's menu in a voice channel](#someones-menu-in-a-voice-channel).
What shows depends on this account's permissions and on its
highest role against theirs, checked when the menu opens
(`application/members.rs`). The member's roles and timeout come from the
service's names, which each snapshot shares as `SharedNames` rather than
copying them.

- Mention puts `@Name ` in the composer, which sending turns into `<@id>`,
  when the channel can be written in.
- Message opens the DM with them, or starts one with any member through `POST
  /users/@me/channels`. Call opens the DM and starts its call; with a friend
  Yapfast has no DM with, it starts the DM first and calls once it opens.
- Add note works for any member, and the friend nickname for friends, without
  a DM.
- Change nickname: another's needs Manage Nicknames and outranking them,
  and is `PATCH /guilds/{id}/members/{user}` with `{"nick": ...}`; empty
  resets it. One's own entry offers Edit server profile in its place, as
  Discord's menu does, which opens the [per-server profile
  editor](#editing-ones-server-profile).
- Roles needs Manage Roles. Its submenu lists every role but @everyone,
  highest first, ticked where they hold it. Roles at or above this account's
  highest, and those an integration manages, show greyed, as Discord shows
  them. Ticking one is `PUT` or `DELETE
  /guilds/{id}/members/{user}/roles/{role}`; the box changes at once and the
  menu stays open.
- Time out needs Moderate Members and outranking them, and isn't offered for
  administrators, whom Discord doesn't let anyone time out. Its dialog offers
  Discord's lengths, 60 seconds to a week, and a reason, then `PATCH
  /guilds/{id}/members/{user}` with `communication_disabled_until`. Someone
  timed out is offered Remove timeout instead, which sends null, and the
  member list marks them with a red clock beside their name.
- Kick needs Kick Members and outranking them: a confirmation with a reason,
  then `DELETE /guilds/{id}/members/{user}`.
- Ban needs Ban Members and outranking them: how far back their messages go
  (none, the last hour, 6, 12 or 24 hours, 3 or 7 days) and a reason, then
  `PUT /guilds/{id}/bans/{user}` with `delete_message_seconds`.
- Reasons go to the server's audit log in the `X-Audit-Log-Reason` header,
  URL-encoded and cut to the 512 characters Discord keeps.
- One's own entry has Profile, Change nickname, Roles and Copy ID. Nobody
  outranks the owner, so nobody kicks, bans or times out the owner.

### Someone's menu in a voice channel

Right-clicking someone in a server's call, or under a voice channel in the
channel list, opens their volumes, as before, above their menu as a member
of the server, with what Discord's client adds there, in its order:

- After Add note: Mute, a box that mutes them for this account alone and
  keeps their volume for when it's unticked, which Yapfast remembers. One's own
  entry, in the call Yapfast is in there, has Mute and Deafen boxes for this
  account's own instead, and no volume.
- After Roles: Move to, a submenu of the server's other voice channels.
  Someone else's lists those this account may move them to: it needs Move
  Members in their channel and in the other, which it or they may join. Its
  own lists those it may join, and choosing one joins it there, as Discord's
  client does. Choosing one for someone else is `PATCH
  /guilds/{id}/members/{user}` with `{"channel_id": ...}`.
- In red, before Time out: Server mute and Server deafen, boxes ticked from
  their voice state's `mute` and `deaf`, which need Mute Members and Deafen
  Members, and send `{"mute": ...}` or `{"deaf": ...}`; then Disconnect,
  which needs Move Members and sends `{"channel_id": null}`, without asking,
  as Discord's doesn't. Neither needs outranking them, as on Discord.

The tracker keeps a voice state's own `self_mute` and `self_deaf` apart from
the server's `mute` and `deaf`, so the channel list and the call show a
server mute or deafen in red, as Discord's client does. Discord's Disable
Video, Mute Soundboard and Apps aren't offered yet. In a DM's call or a
Matrix room's, the right click still sets only the volume.

Discord answers each change with GUILD_MEMBER_UPDATE, or for a kick with
GUILD_MEMBER_REMOVE, and for a ban with GUILD_BAN_ADD as well, and sends the
member list's subscribers its operations. Yapfast learns the member's
nickname, roles and `communication_disabled_until`, shows the open member
list again when it lists them, and renders the open channel again, so names,
colours and the list follow. A member who leaves is forgotten. A nickname,
timeout, kick or ban that fails says why in its dialog; a role says so in a
notice.

Discord's menu also has Invite to listen (Spotify), Apps and Open in
moderator view, which Yapfast leaves out for now.

### Invites

Invite to server, in a server's menu or its header, and Invite to channel,
in a channel's, make an invite with `POST /channels/{id}/invites` and a
body of `max_age`, in seconds or 0 for good, `max_uses`, 0 for any number,
and `temporary` false; the answer's `code` makes `https://discord.gg/{code}`.
The dialog asks for one at once with the defaults of Discord's dialog,
7 days and any number of uses, and makes another when asked with its
other choices. Discord answers 204, with no invite, when a server paused
invites, which the dialog says. A channel's invite lands people in it. A
server's lands them in the open channel when that's in the server, else
in the first of its channels in Discord's order; each only where the
account may see the channel and create invites (`CREATE_INSTANT_INVITE`),
and the choice shows only then. docs.discord.food doesn't say which
channel Discord's client picks for a server, so Yapfast takes the one it
would open. The fake makes invites to "Server"'s channels, sends
INVITE_CREATE, and lists them at `/_control/invites`; its @everyone may
invite, as Discord's is by default.

### Invite to server

Someone's menu has an Invite to server submenu, as Discord's has, in a
server member's menu after Change nickname, and in a DM's, a group DM
member's and a friend's before friendship, Ignore and Block. It lists, in
the rail's order, the Discord servers where this account may create invites
(`CREATE_INSTANT_INVITE` on the server) and has a channel to invite to,
leaving out only the server they're shown as a member of. Discord's client
lists the rest whether they're in them or not, as it doesn't ask which
servers it shares with them, and Yapfast does the same. Nobody is offered it
for themselves, a bot, or someone blocked or ignored. Yapfast knows bots from
the `bot` field of user objects, kept with the service's names. The list is
built when the menu opens, from the servers the snapshot shares.

Choosing a server picks its channel as Discord's client does: of the text
and announcement channels this account may see and invite to, the first by
`position`, not the channel list's order, which goes by category first;
failing those, such a voice channel. Then three requests:

- `POST /channels/{id}/invites` with `{"max_uses": 1, "unique": true}`, so
  it works once and lasts Discord's default day, and an
  `X-Context-Properties` location of "User Invite Context Menu".
- `POST /users/@me/channels` with their id, which finds the DM or starts it.
- `POST /channels/{dm}/messages` with the link, `https://discord.gg/{code}`,
  as its content, and a location of "context_menu".

Discord's client shows nothing more: the DM's CHANNEL_CREATE and the
message's MESSAGE_CREATE list the DM and show the link in it. Where
Discord's says nothing on a failure, Yapfast says why in a notice. Discord's
client also offers a server without the permission when it has a vanity
URL, and sends that instead; Yapfast doesn't keep servers' vanity codes, so it
leaves those out. The fake makes invites to "Server"'s channels and its
`--servers` fillers', and `/_control/invites` lists each with its location.

### Channels and categories

Where the account may manage channels (`MANAGE_CHANNELS`, on the server
for one made there, on the category for one made in it, on the channel
for the rest), Create channel and Create category make one with
`POST /guilds/{id}/channels` and a body of its `name`, `type` (0 text, 2
voice, 4 category), `parent_id` for a category, and for text its `topic`,
`rate_limit_per_user` and `nsfw`. Edit channel reads a text channel with
`GET /channels/{id}` and saves with `PATCH /channels/{id}`: its name, and
for text its topic, slowmode, one of Discord's from 0 to 21600 seconds,
and age restriction; a voice channel or a category changes only its name.
Duplicate channel reads the channel and makes another with its type,
category, permission overwrites, topic, age restriction, slowmode and
voice settings under the name given, as Discord's client does. Delete
channel uses `DELETE /channels/{id}`; deleting a category leaves its
channels outside any, each with a CHANNEL_UPDATE before the
CHANNEL_DELETE. Yapfast changes its list only from CHANNEL_CREATE, UPDATE
and DELETE, which the service already applies to the one server they
name, so the sidebar rebuilds that server's rows and keeps the others.
Names are 1 to 100 characters and topics up to 1024, checked before
asking. The fake lets Alice's Crew role manage "Server"'s channels and
sends each event.

### A channel's menu

Right-clicking a server's channel offers, in order, as Discord's menu
does: Mark as read when it has unread messages, Join voice for a voice
channel, then Invite to channel, Pin channel to top or Unpin channel, and
Copy link, then Mute channel and Notification settings for a text
channel, then Edit channel, Duplicate channel, Create channel and Delete
channel where the account may manage channels, and Copy ID. A thread's
menu has none of inviting, pinning or managing.

#### Pinning a channel

Pin channel to top, as Discord's client has it, moves a text or voice
channel out of its category into a section at the top of the server's
channel list, headed "Pinned channels", above every category and the
channels outside any. The channel shows only there, with its threads under
it. Pinned channels list by their own `position`, voice channels after
text ones, not in the order they were pinned. The section has no menu and
doesn't fold, and a line ends it. Once a channel is pinned its menu says
Unpin channel. Discord's client sets no limit on how many, and offers it
for every channel but threads and categories.

The pin belongs to the account, so it follows it to every client: it's
the `FAVORITED` flag, `1 << 11`, of the channel's override in the user
guild settings, the same `channel_overrides` that hold its mute and level
(see [Notification settings](#notification-settings)). Discord's client
pins with `PATCH /users/@me/guilds/settings` and a body of
`{"guilds": {"<guild>": {"channel_overrides": {"<channel>": {"flags":
6144}}}}}`: the override's flags with `FAVORITED` set, and with
`OPT_IN_ENABLED`, `1 << 12`, too if it wasn't. Unpinning sends the flags
with `FAVORITED` cleared and the rest as they were. Discord answers with
each server's settings, and USER_GUILD_SETTINGS_UPDATE tells every session.
`yapfast_discord::notifications` reads the flags with each override and
writes the body (`Override::pin_flags`, `flags_body`); Yapfast changes the
list when the update comes, as it does for a mute.

Yapfast works out the pinned channels when it lists a server's rooms, which
happens only when the server's snapshot changes, and keeps them in the
server's `ChatServer::pinned`; the sidebar rebuilds only when the servers
it hashes change. Discord's "Move to…", which reorders the server's
channels for everyone, and its Favorites server, kept in the settings
protobuf, are different features; Discord's client only hides "Move to…"
and dragging for a pinned channel, which Yapfast has neither of. The fake
keeps each override's flags and answers the bulk endpoint as Discord does.

### Threads

A text channel's threads (types 10, 11 and 12) are channels of their own,
with a `parent_id` and `thread_metadata`. A user account's READY and
GUILD_CREATE carry only the active threads it's in, each with its own
`member`; THREAD_CREATE, UPDATE, DELETE and LIST_SYNC change them, and
THREAD_MEMBER_UPDATE and THREAD_MEMBERS_UPDATE say when the account joins or
leaves one. The service keeps them in the server's snapshot, newest first,
with the thread's owner, privacy, archiving, locking, message count (which
leaves out the message a thread was started from) and the account's
membership: when it joined, its mute and its notification level (flags 2,
4 and 8 for all messages, mentions only and nothing). A thread takes its
permission overwrites from its parent: it shows with View Channel, a
private one only to its members or with Manage Threads, and writing in it
takes Send Messages in Threads, which a locked thread also asks Manage
Threads for.

The sidebar lists each active thread the account is in under its channel,
joined to it by a line, with its own unread state and mentions, read at
join time until Discord's read state says otherwise. A thread notifies
as its own level says, else as its channel does; one the account isn't in
notifies only of mentions, and a muted thread not at all. Opening one shows
it in place of the channel, its header reading "general › Plans" with the
channel's name a link back, and the channel's member list beside it.

As in Discord's client (`ActiveJoinedThreadsStore`), a thread goes quiet
its `auto_archive_duration` after its newest message, its start or its
`archive_timestamp`, whichever came last, though Discord may archive it
much later. A quiet thread lists only while it's unread and not muted,
mentions the account, or is on screen. The service wakes when the next
thread goes quiet and publishes a snapshot, so the row goes with nothing
else happening (`Tracker::next_quiet`). The fake's `/_control/thread-quiet`
makes Plans go quiet, and `?reset` lists it again.

A message that started a thread carries it as `thread`, and shows a chip
with its name and count under it; the THREAD_CREATED notice (type 18)
links to the thread through its `message_reference`, and a thread's first
message, THREAD_STARTER_MESSAGE (type 21), shows the message it was started
from. Clicking a chip opens the thread beside the channel, as Discord
does, in place of the member list: its messages, its history and a
composer of its own, which replies, edits and reacts as the timeline does.
Its button shows it in place of the channel, and closing it, or opening
another channel, brings the member list back. A thread the account isn't
in is read with `GET /channels/{id}` and listed while it shows, so it
reads and marks read like any other; writing in it joins it, as Discord
does. The fake has "Plans", from Bob's welcome, and its controls write in
it, start "Movie night" and archive "Plans".

A thread's menu offers Join thread, or Leave thread for one the account is
in, after Mark as read, as Discord's client does (its `thread-context`
menu). So does the More button (…) in a thread's header, after its member
list, and in the header of a thread beside its channel: the same menu less
Mark as read, as Discord's "Toolbar Overflow" and "Sidebar Overflow" menus
start with joining or leaving; their other items, such as closing or
locking the thread, Yapfast doesn't have yet. Discord's client checks
nothing else: not archiving, locking or permissions. It sends `POST
/channels/{id}/thread-members/@me` to join, though docs.discord.food says
PUT, and `DELETE` to leave, each with the menu's `location` ("Context
Menu", "Toolbar Overflow" or "Sidebar Overflow") and no body; Discord
answers with THREAD_MEMBERS_UPDATE. A thread left stays on screen, as in Discord's
client, and stops listing in the sidebar once it's off it. The fake
answers both, refuses an archived thread (50083), and
`/_control/thread-members` lists each request.

#### A channel's list of threads

The Threads button comes first in a text or announcement channel's header,
and in a thread's for its channel, as in Discord's client, which no
permission hides. It opens the channel's threads under it, as Discord's
popout does (`application/threads.rs`, `ThreadsPopover`): "N joined
threads", the active ones the account is in, then "N other active
threads", then "Older threads", the archived ones, each section with the
newest message first. An active thread's row shows its newest message, its
author's avatar and name in their role colour, and how long ago ("5m ago",
">30d ago"), or "No recent messages"; an archived one's shows "Started by"
its owner and when it was last active. On the right are a few of its
members' avatars and how many are in it. Clicking a row opens the thread in
place of the channel, and Shift-click beside it, as Discord's does; a
thread the account isn't in opens as any other does, without joining it.
Sized as in Discord's client (its CSS and list in the web bundle): each
thread is a card 72px tall with 8px under it, the first section's heading
32px tall and later ones 64px, and the popout 35% of the window wide, from
480px to 600px, and as tall as its rows, from 400px to 80% of the window.

What fills it, as Discord's web client does it: the active threads come
from the gateway, never a request. Subscribing to a server with `threads`
(op 37), which Yapfast does for the server on screen, brings THREAD_LIST_SYNC
with its active threads the account may see and each one's newest message
(`most_recent_messages`). The service keeps each thread's newest message
(its text cut to 200 bytes), its member count and preview ids in the
server's snapshot, and follows them with MESSAGE_CREATE and
THREAD_MEMBERS_UPDATE; the list reads them from the snapshot as it shows,
without copying the server. Archived threads come only with Read Message
History, when the list opens and then a page of 25 at a time, from `GET /channels/{id}/threads/search` with
`archived=true&sort_by=last_message_time&sort_order=desc&limit=25`
and an `offset`, as Discord's client asks; it never calls the
`/threads/archived/` endpoints. The next page loads only as a scroll by
hand ends within 200px of the list's end, as Discord's client does it, so
a list too short to scroll loads no more. Archived threads the snapshot
knows are merged in where the pages reach, and none show before the first
page answers. Search, which also needs Read Message
History, runs on Enter with `name=` and no `archived`, so it finds active
and archived threads, and its results replace the sections until it's
cleared; the first search of a channel can answer 202 while Discord indexes
it, and Yapfast asks again. A private thread shows only to its members and
those with Manage Threads. The pages, the search's results and the channel
objects the list found, from which a thread opens without another `GET
/channels/{id}`, are dropped when the list closes or another room opens.

The fake syncs "Plans", "Book club", which Alice isn't in, and leaves out
"Surprise for Bob", a private thread she isn't in. #general has 30 older,
archived threads, more than a page, and `/_control/thread-searches` lists
each search.

#### Starting a thread

A thread starts as in Discord's client, from three places: Create Thread in
a message's menu, the list of threads' Create button (and its empty
list's Create Thread), and Create Thread in the composer's + menu, which
then also holds Upload a File. Each opens Discord's "New Thread" form beside
the channel, in place of its member list (`application/threads.rs`,
`NewThreadPanel` in `ui/thread.slint`): Thread Name, up to 100 characters;
Private Thread, "Only people you invite and moderators can see"; and the
first message, "Enter a message to start the conversation!", which takes
the channel composer's draft. From a message the form shows that message,
the name is optional and the thread public; an empty name becomes
Discord's default: the message's embed title, else its first line without
leading `#`, `-` or spaces, grown word by word up to 40 characters and
then "...", else "Thread". Without a message a name is required ("Thread
Name is required"), and a first message always is ("Starter Message is
required"). Closing a form with something written asks "Are you sure you
want to go?" first.

Discord's client offers a public thread with Create Public Threads and
Read Message History, and a message's Create Thread only on a delivered
message without a thread, whose menu offers View Thread instead; a
private one takes Create Private Threads, in a text channel only, and
where only that is allowed, the form starts one. Creating sends `POST
/channels/{id}/messages/{message}/threads` or `POST
/channels/{id}/threads` with the `name`, the `type` (11, 12 for a
private one, 10 in an announcement channel), the channel's
`default_auto_archive_duration`, else 4320 minutes, and the `location`
Discord's client gives ("Message", "Thread Browser Toolbar", "Thread
Browser Empty State", "Plus Button"). Discord then sends THREAD_CREATE
with `newly_created` and THREAD_MEMBERS_UPDATE with the account in it,
updates the message, which now carries the thread, and posts a
THREAD_CREATED notice in the channel for a public thread started without a
message or from an older one. As Discord's client does, Yapfast opens the
thread beside the channel and then sends the form's first message as an
ordinary message in it.

The fake starts threads as Discord does, with those events and notices,
refuses a second thread on a message (160004) and a name outside 1 to 100
characters (50035), and `/_control/thread-creates` lists what each request
sent.

### Forums

A forum (type 15) lists among the text channels, by its position, with its
own icon, and shows its posts in place of messages, as Discord's client
does (`application/forum.rs`, `ForumPanel`). Its posts are public threads
whose parent is the forum, so everything above holds for them: posts the
account is in list under the forum in the sidebar, the active ones come
with THREAD_LIST_SYNC once the server is subscribed (with `threads`, but
no channel, since a forum has no member list), and archived ones come a
page of 25 at a time from `GET /channels/{id}/threads/search`, as a
channel's list of threads asks. The service keeps the forum's tags
(`available_tags`), whether a post must take one (flag 16), its default
sort order and its default reaction, and each post's `applied_tags` and
whether it's pinned (flag 2). A post's first message has the post's own id
and isn't counted in its `message_count`.

The forum shows its active posts, the pinned one first, then by latest
message, or by when they were made if the forum sorts by creation, then
"Older posts", the archived ones by latest message, then what a search of
their titles finds while one shows. Each card shows, as Discord's list
layout does: a pin for a pinned post and its first three tags, "+N" for
more; its title, bright when the account opened it and has messages to
read there, and "NEW" for a post made since the forum was last read that
the account never opened (it has no read state of its own); the first
message's author in their role colour and its text, or "Original message
was deleted"; the reaction most given, with super reactions, or the
forum's default reaction without a count; the replies; how long ago it was
last active, or made; and its first picture, a 72px thumbnail from a file
or an embed. Clicking a card opens the post beside the forum, as Discord's
does, and Shift-click in its place.

First messages, as Discord's client gets them: an archived page or a
search brings its posts' `first_messages`; the active posts' come from
`POST /channels/{id}/post-data` with `{"thread_ids": [...]}`, at most ten at
a time, which answers with each post's `first_message`, `most_recent_message`
and `owner`. Yapfast asks for the first 25 cards' when the forum opens and
for 25 more each time the list scrolls near its end, one request at a time,
as Discord's client asks for the cards it shows. The cards, pages and first
messages are kept only while the forum shows. Opening the forum marks it
read, which acknowledges its newest message, as Discord's client does; a
forum's newest message is its newest post, so a post made since makes it
unread (THREAD_CREATE with `newly_created`).

The fake's #ideas has a pinned welcome, "Dark theme for the sidebar",
which Alice is in, with reactions and replies, a crash report with a
screenshot and two tags, a newer post she never opened, and three archived
ones. It answers `post-data` (`/_control/post-data` lists each request's
ids), gives first messages with a forum's search, and `/_control/forum-post`
makes Carol post.

### Editing one's server profile

Edit server profile, in a server's menu, its header's dropdown and one's
own entry in its member list, opens Yapfast's version of Discord's "Per-server
Profiles" settings page (`application/server_profile.rs`, the dialog in
`ui/server.slint`). Discord's client offers it to everyone; it reads the
profile with `GET /users/@me/profile?guild_id=...` and shows, in its
order:

- Server Nickname, up to 32 characters, greyed with "You don't have
  permission to change your nickname in this server." without Change
  Nickname or Manage Nicknames.
- Pronouns, up to 40.
- Avatar, with Change Avatar and Reset Avatar.
- Profile Banner, with Change Banner and Reset Banner, and Discord's hint:
  "Upload a PNG, JPG, or GIF under 10MB. Images should be at least
  680x240."
- Bio, Markdown up to 190 characters, with what's left counted under it.

A profile card beside them previews the profile as the server will show
it, the main profile's name, pronouns, bio, avatar and banner standing in
for each field left empty, as Discord merges them: an empty text and a
missing picture both go by the main profile. A Reset button shows only
when the server has a picture of its own, as it stands or newly chosen.

The nickname and pronouns are free. The avatar, banner and bio take
Discord's full Nitro: the client checks `canUsePremiumProfileCustomization`,
a feature of the Nitro SKU alone, so `premium_type` 2 and not Basic (3) or
Classic (1). Without it Discord still shows them, greyed, under an offer:
"Make unique profiles for each server", "Customize your profile for every
server, from avatar decorations to bios. Only with Nitro.", and "Unlock
with Nitro", or "Upgrade Nitro" with a lesser Nitro, which opens
`https://discord.com/nitro`. Yapfast shows them the same way.

Change Avatar and Change Banner use the file chooser attachments use. Yapfast
reads the picture off the UI thread, as Discord's cropper sends it: the
middle square of an avatar, at most 1024 pixels, and the middle 17 by 6 of
a banner, at most 2400 by 848, from a PNG, JPG, GIF or WebP under 10 MB.
It keeps only that file, a PNG where the picture has transparency and
otherwise a JPEG, and a preview at the card's size, twice over; an
animated GIF goes as it is, so it stays animated. The full decoded picture
is dropped at once, and closing the editor drops the rest. With two
4000 by 3000 photos chosen, the heap in use (window-control `stats`) went
from 11.26 to 12.97 MiB and back to 11.28 once the editor closed.

Save Changes sends what changed, as the client does, in up to two
requests, each only with fields to send:

1. `PATCH /guilds/{id}/members/@me` with `nick` and `avatar`. Discord
   answers with the member and sends GUILD_MEMBER_UPDATE, which Yapfast's
   names already take in, so the nickname and server avatar show in the
   member list and on messages.
2. `PATCH /guilds/{id}/profile/@me` with `banner`, `bio` and `pronouns`.
   No gateway event follows; the client reads its answer.

A picture goes as a data URI, `data:image/png;base64,...`, or `null` to go
back to the main profile's; a cleared text goes as `""`. Discord's docs
say the member endpoint also takes `bio`, `banner` and `pronouns`, but the
client never sends them there.

Discord's editor also has a profile theme, an avatar decoration, display
name styles, a nameplate, a profile effect and a frame, all Nitro and most
bought from its shop. Yapfast leaves them out, as it shows none of them.

A member's server profile shows wherever Yapfast shows them in that server:
their server avatar in the member list and on their messages, from their
member object, and their server banner, bio and pronouns on their profile
card, from `GET /users/{id}/profile?guild_id=...`, whose
`guild_member_profile` wins over their own where it isn't empty. Yapfast
keeps a fetched profile for five minutes, but fetches it again once
GUILD_MEMBER_UPDATE changes their nickname or server avatar, and after
this account saves its own.

The fake keeps a server profile for each member of "Server": Carol has a
server avatar and Bob a server bio. It takes both requests, refuses the
avatar, banner and bio with Discord's `PREMIUM_REQUIRED` error unless
Alice has Nitro (`--nitro`, or `POST /_control/nitro`, with `?off` to take
it away, which USER_UPDATE tells her session), serves an uploaded picture
from its CDN as it came, and shows hers at `GET /_control/server-profile`.
`scripts/ui-session do 'pick avatar PATH'` chooses a picture in the editor,
as its file chooser would.

### Leaving a server

Leave server, in a server's menu or at the end of its header's dropdown,
asks first, with Discord's words, then sends
`DELETE /users/@me/guilds/{id}` with `{"lurking": false}`; Discord
answers 204 and sends GUILD_DELETE, without `unavailable`, and
GUILD_MEMBER_REMOVE. The tracker already drops a server on such a
GUILD_DELETE, and Yapfast goes to the DMs if it was open. The owner isn't
offered it: Discord has them transfer or delete the server instead. The
fake lets Alice leave "Server", which Bob owns, leaves it out of READY
afterwards, and `/_control/rejoin` brings her back with a GUILD_CREATE.

A server's privacy settings, which Discord's menu also has, are left for
later: they live in the settings protobuf's guild entries, which Yapfast
would have to read and write back whole, beyond a menu's reach.

### DMs

DMs and group DMs are listed under the DM button after Matrix DMs, the latest
conversation first as in Discord, with the other person's avatar or the
group's icon and a Discord badge. They open, send and keep their messages
like server channels, under the room id `discord:@me:<channel>`, which takes
`@me` for the server as Discord's links do. Every unread message in a DM
counts as a mention, and a muted DM shows neither, as in Discord. A DM
without a read state counts as read, since Discord keeps one for each DM
with something to read. A one-to-one DM shows its person's status dot, with
their custom status or what they're doing under the name. Discord sends
presence only for friends, so anyone else shows the grey offline ring, as in
Discord's DM list. These follow PRESENCE_UPDATE live. The header of an open
one-to-one DM shows the same dot on their avatar, and their custom status or
what they're doing in a bubble beside the name. A group DM
shows how many are in it. A group DM lists its members by name, with the
presence of those who are friends. Yapfast downloads the pictures of the first
150 DMs listed; the rest keep their initials.

Friend requests, message requests and spam have their own screen. A
"Requests" entry below Friends at the top of the DM list opens it in place of the
conversation, with a badge counting the incoming friend requests (spam
excluded) and message requests. Message requests come from people outside the
account's DMs. Those Discord marks as spam are listed too, under "Spam", so a
person Discord misjudged can still be found. The entry stays highlighted
while a request from the screen is open. Opening one shows its messages with Accept and Ignore in place of the
composer: replying waits for accepting, as in Discord. Accepting makes it an
ordinary DM; ignoring removes it, and the sender isn't told. Requests count
no mentions, spam isn't marked unread, and Yapfast never opens a request on its
own when it picks a conversation. Forwarding is still open.

The pen in the DM list's header starts a DM: it lists the account's friends
(from READY's relationships), then anyone else Yapfast knows by name, and
opening one posts to `/users/@me/channels`, which finds the DM with them or
starts one. Yapfast opens it once its CHANNEL_CREATE lists it.

### Friends and requests

"Friends", at the top of the DM list, opens the friends list in place of the
conversation. It lists the account's friends from READY's relationships by
name, each with their status dot and custom status or activity, under two
tabs in the header: Online, for those online, idle or on do not disturb, and
All. A search field filters it by name, and "Add Friend" in the header opens
the add-friend dialog. Discord sends presence for friends in READY's
`merged_presences` and in PRESENCE_UPDATE, so the list keeps up with them.
Clicking a friend, or the message button on their row, opens the DM with
them, or starts one as the pen does and opens it once Discord lists it.

The person beside the pen, in the same header, adds a friend by username: `POST
/users/@me/relationships` with `{"username": ..., "discriminator": null}` and
`{"location":"Add Friend"}` as the context. Discord's codes 80004 (nobody by
that name), 80000 (they take no requests from strangers) and 80007 (already
friends) get their own words.

Friend requests come first on the requests screen, from READY's
relationships (type 3 sent to this account, 4 sent by it) and kept current by
RELATIONSHIP_ADD and RELATIONSHIP_REMOVE. One sent to this account is
accepted with the check or ignored with the cross; one it sent is cancelled
with the cross. A profile card offers "Send friend request" to someone who
isn't a friend, or "Accept friend request" when they asked first, and the
header of a DM with them offers the same. Sending or accepting is
`PUT /users/@me/relationships/{id}` with an empty body, and ignoring or
cancelling is `DELETE` on the same path, each with the X-Context-Properties
header Discord's client sends: `{"location":"User Profile"}` when sending
from a profile, `{"location":"Friends"}` otherwise, as base64. Discord
doesn't let a client send many requests quickly, and Yapfast only sends one when
the person clicks. Requests Discord took for spam are listed too, marked
"Maybe spam".

### A DM's menu

Right-clicking a DM offers, besides Profile and copying, these:

- Pin keeps a DM or group DM at the top of the list. Discord's desktop client
  doesn't pin DMs, so Yapfast keeps the pins in `layout.json`, on this device.
- Start call joins the DM's call, as the phone in its header does.
- Add note writes this account's note on the other person:
  `GET /users/@me/notes/{id}` fills the dialog, a 404 meaning there's none,
  and `PUT` with `{"note": ...}` saves it, an empty one removing it.
- Add friend nickname, for a friend, is `PATCH
  /users/@me/relationships/{id}` with `{"nickname": ...}`, or null to remove
  it. RELATIONSHIP_UPDATE carries it back, and Yapfast shows a friend by their
  nickname in the DM list and the friends list, as Discord does.
- Close DM is `DELETE /channels/{id}`; Discord answers with CHANNEL_DELETE,
  and the DM returns when either side writes.
- Remove friend is `DELETE /users/@me/relationships/{id}`, and Block is `PUT`
  on the same path with `{"type": 2}`, each after the person confirms, with
  `{"location":"ContextMenu"}` as the context. Unblock is the same `DELETE`.
- Ignore is `PUT /users/@me/relationships/{id}/ignore`, and `DELETE` stops it.
  Discord marks the relationship `user_ignored`, which Yapfast tracks to offer
  the opposite.
- Mute is `PATCH /users/@me/guilds/@me/settings` with the channel's override:
  `muted` and a `mute_config` of `selected_time_window`, in seconds or -1 for
  good, and its `end_time`. Unmute sets `muted` false. USER_GUILD_SETTINGS_UPDATE
  brings the change back, and a muted DM shows faint and counts nothing unread.

- [Invite to server](#invite-to-server) sends them an invite to one of this
  account's servers, chosen in its submenu.

Someone who isn't a friend is offered Add friend, Accept friend request or
Cancel friend request in place of Remove friend, unless they're blocked.
Right-clicking the other person's avatar or name in the DM's messages opens
the same menu. The person's choices show only in a one-to-one DM. A message request offers
neither pinning, closing nor muting, as it waits for an answer.

### A DM's header

A DM or group DM's header has, as Discord's does, its call, its pinned
messages, adding friends, the profile or member list, and a search. Video
calls aren't there yet. A server's text channel has, after its icon and name,
its topic, then on the right Discord's buttons in Discord's order: the bell
of its [notification settings](#notification-settings), its pinned
messages, the member list's toggle, bright while the list shows, and the
search. The list of a channel's threads, which Discord's header has before
the bell, is still to come.

- The topic is the channel's `topic`, from READY and GUILD_CREATE, which
  CHANNEL_UPDATE changes. Yapfast keeps it with the channel, shared between the
  server's snapshots (`Option<Arc<str>>`), and when a channel opens, renders
  its Discord markup as a message's: mentions by name, custom emoji,
  Markdown and links. It sends the result (`Event::Topic`) again only when
  the topic changes, or when it mentions someone and the names Yapfast knows
  change. The header shows it on one line, after a rule, cut at the last
  word that fits; clicking it shows it in full under the header, with its
  links. A channel without one shows none. The fake's #general has one, and
  `POST /_control/topic` changes it.

- Pinned messages open under the pin, the latest pinned first, from `GET
  /channels/{id}/messages/pins`, whose `items` carry each `message`. Yapfast asks
  each time they open, and for the first 50.
- Search, as in Discord, looks through the DM in a DM, with `GET
  /channels/{id}/messages/search`, and through the whole server in a server
  channel, with `GET /guilds/{id}/messages/search`; the box says "Search"
  and the DM's or the server's name. Results come 25 a page, newest first,
  each of `messages` a list whose first is the one found; they take the
  member list's or the profile's place, and Show more loads the next page.
  Discord indexes a DM or a server the first time it's searched, answering
  202 with code 110000 and a `retry_after`; Yapfast asks again up to five
  times.
- The box takes Discord's filters with the words (`yapfast_discord::search`):
  `from:` and `mentions:` someone, `in:` a channel of the server, `has:`
  link, embed, poll, file, video, image, sound, sticker or forward,
  `before:`, `after:` and `during:` a day such as 2026-10-13, and
  `pinned:true`. A value with spaces goes in quotes. Yapfast sends them as
  Discord's client does, each repeated for several: `author_id`, `mentions`,
  `channel_id`, `has` (`snapshot` for forwards), `min_id` and `max_id`, the
  snowflakes of the day's local midnights, and `pinned`. `in:` names a text
  channel by its name; `from:` and `mentions:` find the member through the
  gateway's member search in a server, as the box's suggestions do, and
  among the people Yapfast knows in a DM, someone called exactly that first. A
  name nobody has, or a filter Discord doesn't take, says so in place of
  results.
- A server's results are headed by the channel they're in, and the search
  stays open in the server's channels. Clicking a result in another
  channel opens it on the messages around the result
  (`Action::JumpInRoom`). In an unread channel the "New" divider still goes
  in, but neither takes the view from the result nor loads the messages
  around the last read one in place of its window. Another server, a DM or
  a voice channel closes the search.
- In a server, pins and results carry no member profiles, so Yapfast asks the
  gateway for those of authors it doesn't know yet and waits up to five
  seconds for them; then they show with their server nickname, avatar and
  role colour.
- Clicking a pin or a result jumps to it in the timeline, as a reply's
  original does; see [jumping to a message](#jumping-to-a-message).
  Pins and results show their text and name their files, without
  downloading them.
- Adding friends to a one-to-one DM makes a new group of them and the other
  person with `POST /users/@me/channels` and their ids in `recipients`, as
  Discord's client does, and opens it; the DM stays as it was. In a group,
  each is `PUT /channels/{id}/recipients/{user}`, which CHANNEL_RECIPIENT_ADD
  confirms. A group holds ten people, this account included, so the picker
  stops there, and it offers only friends who aren't there already.

### Profiles

Clicking someone in a channel's member list, or under a voice channel,
shows their profile in a small card beside it, which "View full profile" enlarges into a centred one;
right-clicking a DM and choosing "Profile" opens the full one. A one-to-one
DM shows its other person's profile in a panel on the right, with the bio in
full and the mutual servers and friends. Mutual servers show their icon and
the person's nickname there; clicking one opens a card beside it with the
server's banner, member count (from READY's `member_count`), channel count
and "Go to server". Mutual friends show their avatar and status dot;
clicking one opens their profile card beside it, or their full profile from
a full profile; the person icon in the DM's header
hides or shows it, and Yapfast remembers the choice in `layout.json`. Yapfast fetches the
profile once each time the open DM changes to someone else. Yapfast fetches it
from `GET /users/{id}/profile` with the open server's id, so the server's
nickname, avatar, banner, bio and pronouns win over the person's own, and the
card lists their roles in role order, when they joined Discord and the server,
and the servers and friends the account shares with them. Profiles are kept for
five minutes, at most 32 of them. Presence and activity come from the member
list, since the profile doesn't carry them. When Discord refuses, the card
keeps what the member list knew and says so. Matrix members and one-to-one
Matrix DMs get the same card with their name, Matrix ID, avatar and room role.

### Later

In rough order. The browser covers these until then.

- Posting in forums, sorting and filtering a forum's posts by tag, its
  gallery layout, and media channels.
- Pinning and unpinning messages.
- Forwarding Discord messages, which Discord marks as forwarded, rather than
  sending their text again.
- Making and changing server folders.
- Joining servers by invite.
- A server's privacy settings: direct messages, message requests and
  activity status from its members.
- Discord's push-to-talk, and its voice settings.
- The soundboard.
- Polls, activities, and server administration.
- [Mirroring into Matrix](#mirroring-into-matrix).
- The blocked on the friends page, beside Online and All.
- Blocking and unblocking from a profile.
- Animated custom emoji, which show their first frame.
- Copying custom emoji: copied text has their private code points, not
  `:name:`.
- A custom status's end while Yapfast is closed, or set from a device whose
  clock is off: Yapfast trusts the time Discord gives.
- Opening embed pictures from a card in the viewer with their neighbours.

## Mirroring into Matrix

Mirroring copies Discord DMs and chosen server channels into Matrix rooms on the
person's homeserver. It does two things:

- Devices without a Discord login can read and reply. Only one device needs
  to link Discord.
- History builds up on the homeserver from the moment of linking, independent
  of Discord.

Mirroring is on by default after linking. All DMs and group DMs are mirrored.
Server channels are chosen per server or per channel, because large servers
would fill the homeserver with other people's history. A setting turns
mirroring off completely. It's the last phase, because everything else works
without it. If it turns out not to be useful, it can be dropped.

### Layout

One space, named "Discord (Yapfast)", holds every mirrored room directly. Yapfast
groups the rooms by Discord server using the encrypted details below. Without
nested spaces, the homeserver can't count servers or channels per server. Each
mirrored room has only the person as a member.

Element Web hides rooms that belong to a space from its Home list, unless they
are DMs or the "Show all rooms in Home" setting is on
([`SpaceStore.showInHomeSpace`](https://github.com/element-hq/element-web/blob/develop/apps/web/src/stores/spaces/SpaceStore.ts)).
Yapfast never adds mirrored rooms to `m.direct`, so Element doesn't treat them as
DMs. Element X filters by space only on request, so it may still list them.
To keep other clients quiet:

- Mirrored messages use a custom event type, so other clients show nothing.
- Yapfast posts them as the person, so the homeserver sends no notifications
  for them and other clients show no unread badges.
- Mirrored rooms carry the `m.lowpriority` tag.
- The space topic says "Mirrored from Discord by Yapfast. Open in Yapfast to read."

### What the homeserver can see

Room names, topics and avatars stay empty. Yapfast stores each room's Discord
server name, channel name and IDs in a custom state event,
`app.yapfast.yapfast.discord.channel`. The event content is ciphertext. The space
carries the same kind of event for the Discord account. Element shows the rooms
as unnamed.

Yapfast creates one random key per Matrix account and stores it in secret storage
under a custom name through the SDK's `SecretStore::put_secret`. Any device
that enters the recovery key gets it, and Yapfast keeps a local copy after that.
Encrypt with AES-256-GCM or XChaCha20-Poly1305 from the RustCrypto crates, with
a fresh random nonce per value. Include the room ID as associated data, so
ciphertext copied into another room fails to decrypt. Messages are ordinary
Megolm-encrypted room events.

The only plain record is the space's room ID, kept in account data so every
device finds the space. The homeserver can still see the number of mirrored
rooms, message counts, sizes and timing. That's accepted: the homeserver is the
group's own VPS.

[MSC4362](https://github.com/matrix-org/matrix-spec-proposals/pull/4362) would
encrypt state events in Matrix itself. The pinned SDK has the
`experimental-encrypted-state-events` feature, but the proposal is a draft and
needs homeserver support. Switch to it if it lands; only Yapfast reads these
events.

### Which device mirrors

Only one Yapfast instance with a Discord login runs the mirror at a time. The most
recently active linked instance takes a lease, stored in account data with an
expiry, and renews it while it runs. Instances without a Discord login never
take it. Every mirrored event carries its Discord message ID, and writers skip
IDs already in the room, so a takeover during a lease change doesn't post twice.

While no linked Yapfast instance runs, nothing is mirrored. The next instance to
take the lease fetches the missed messages from Discord.

### Devices without a Discord login

A device without a Discord login shows Discord servers and DMs from the mirror,
with the same badges as a linked device. Its Discord activity becomes custom
request events in the mirrored room: sending, replying, editing, deleting,
reacting and marking as read. The instance holding the lease carries these out
on Discord, then replaces each request with the resulting mirrored event, or
marks it failed with the reason.

Until a linked instance picks a request up, the message shows as waiting, with
the reason "Waiting for a device with Discord". Like Yapfast's offline queue, the
person can cancel it. Requests carry a client-generated ID, so a lease change
never sends one twice.

Voice, calls and status need a Discord login on the device itself. A device
without one shows Discord voice channels from the mirror, without participants,
and offers to link Discord to join.

### History and storage

Linking a channel to the mirror copies the last 100 messages or 7 days,
whichever is less. After that the mirror keeps everything. On a linked device,
scrolling past the start of the mirror loads older messages from Discord and
adds them to it. Attachments over a size cap stay on Discord: a linked device
loads them when opened, and other devices show a placeholder. Smaller ones are
re-uploaded encrypted to the homeserver.

Devices keep only a bounded local cache of recent messages and attachments, as
for Matrix rooms. Older history loads on demand from the mirror, or from Discord
for channels that aren't mirrored. The mirror is the long-term, encrypted copy
of the history. A new device reads old mirrored messages through key backup
after entering the recovery key, as in other encrypted rooms. Attachments make
the homeserver's disk grow. Synapse's media retention can expire old ones if
that becomes a problem.

A message deleted on Discord is also deleted from the mirror, matching Discord.
A mirroring setting, "Keep messages deleted on Discord", keeps them instead.
It's off by default. Edits keep their earlier versions in Matrix either way.

### Unlinking

Logging one device out of Discord leaves the mirror alone, since other devices
may use it. Turning off mirroring for a channel, or removing Discord from the
account, makes Yapfast leave and forget the affected rooms. Yapfast asks first,
because this deletes the history kept on the homeserver. Synapse deletes a room
once nobody is in it.

## Memory

The Discord connection and its server state count towards the same targets as
the rest of Yapfast: under 250 MiB settled idle and under 400 MiB in a ten-person
call. Large servers carry big member lists, so request members only for the
channel on screen. Measure settled idle and a Discord call with the method in
[measurements](measurements/README.md).

## Testing

`testing/fake_discord.ts` is a local Discord: the gateway, the parts of the
HTTP API Yapfast uses, a voice server that answers the handshake without DAVE,
Go Live signalling, and the remote login gateway. Set
`YAPFAST_DISCORD_TEST_SERVER` to it, and Yapfast uses a test token instead of the
linked account, kept in memory. With `YAPFAST_DISCORD_TEST_UNLINKED=1` too, Yapfast
starts unlinked and links against the fake, whose controls scan and confirm
the code. The fake resumes a dropped gateway session, replaying what it
missed, and can send op 7. `crates/yapfast-discord/tests/fake_server.rs`
runs against it. With `--env PATH`, the fake serves an environment's
content in place of its fixture, for demos and screenshots; see
[building](building.md#networks) and `testing/fake_discord_env.ts`.
`crates/yapfast-discord/tests/fake_env.rs` runs against
`testing/envs/example.json`. It counts the audio a call sends (`/_control/call-audio`,
and by SSRC `/_control/voice-audio`),
which the ignored `discord_audio` test `a_call_sends_what_the_microphone_hears`
checks: Yapfast joins the Lounge, unmutes and beeps into a microphone that hears
its speakers, as a private PulseAudio of null sinks does. What the fake can't show, such as media and DAVE with
Discord's servers, needs real Discord. Use a separate test account, which the
owner provides, in a private test server. Run these tests by hand, never in CI: repeated automated logins
are what gets accounts flagged. Keep the QR-linked test token in the developer's
OS credential store, like any other Discord login. Matrix-side behavior, such
as the mirror, request events and encrypted names, is tested against the local
Synapse stack with recorded Discord events instead of a live connection.

## Licences

davey is permissive, so closed-source Yapfast can use it, as songbird was
while it ran calls. GPL
clients such as [Abaddon](https://github.com/uowuo/abaddon) are reference
material only.

## Order of work

1. Prove Discord voice outside the UI. Link by QR code, then join a voice
   channel with DAVE in a command-line harness, like Milestone 1 did for Matrix
   calls. Settle the audio question above here.
2. Voice in Yapfast: the voice channel list, joining, controls, and the shared
   connection rule.
3. DM and group DM calls with ringing.
4. Live servers, channels, DMs and messages, notifications, read state and
   status.
5. Mirroring into Matrix.

Steps 1 to 4 work on Linux; step 5 hasn't started.

Screen sharing and watching streams in Discord calls (Go Live) came early,
and camera video in calls in October 2026.
