# How Ingle works

```
              ┌──────────────── ingle.exe ─────────────────┐
              │ app (egui UI)                              │
              │   └─ engine: state, E2EE, audio, video     │
              └──────────────┬─────────────────────────────┘
                             │ one QUIC connection per server (UDP 7766)
                             │  • control stream: length-prefixed JSON
                             │  • audio: QUIC datagrams
                             │  • video: one unidirectional stream per frame
              ┌──────────────┴─────────────────────────────┐
              │ ingle-server                               │
              │   SQLite (members, channels, messages)     │
              │   SFU: forwards encrypted media, never     │
              │   decrypts it                              │
              └────────────────────────────────────────────┘
```

| Crate | What it holds |
|---|---|
| `crates/proto` | Wire types (`control.rs`), binary media headers (`media.rs`), identities and E2EE primitives (`crypto.rs`), invite links (`invite.rs`), QUIC endpoint helper with a plain-UDP fallback (`udp.rs`). |
| `crates/server` | `ingle-server`: TLS identity, SQLite storage, login, chat, roles, the media relay (`hub.rs`, `conn.rs`). |
| `crates/engine` | Everything the client does except drawing: connections and reconnects, state the UI reads, calls, E2EE key management, the audio engine, video capture/encode/decode, push-to-talk. |
| `crates/app` | The desktop UI (eframe/egui): server rail, channels, chat with markdown, call stage, dialogs, theme and icons. |

## Transport

- **QUIC** (quinn 0.11, rustls + ring, TLS 1.3, ALPN `ingle/1`) on one UDP port. One
  connection per server carries text, voice and video, so there's a single port to forward
  and congestion control sees all traffic together.
- **Server identity without a CA.** The server makes a self-signed certificate on first
  run. Invite links carry its SHA-256 fingerprint
  (`ingle://host:port/CODE?fp=…`), and the client accepts that certificate only.
- **Control stream.** The client opens one bidirectional stream, writes the preamble
  `ING\x01`, then exchanges length-prefixed JSON frames: requests with ids, replies and
  pushed events (`Hello`, `Ready`, `MessageCreated`, `VoiceUpserted`, `E2eeKey`, …).
- **Audio** goes in QUIC datagrams (RFC 9221): never retransmitted, so a late packet never
  delays newer ones. 14-byte header: kind, track, flags, epoch, sequence, session.
- **Video** sends each encoded frame on its own unidirectional stream (22-byte header plus
  ciphertext), the approach of the RTP-over-QUIC and MoQ drafts: a lost packet only delays
  its own frame, and a frame that's too old is abandoned with a stream reset.
- **Fallback socket.** quinn turns on ECN and packet-info socket options. If the OS refuses
  (Wine does; some old stacks might), `proto::udp::bind_endpoint` retries with a plain UDP
  socket: same QUIC, minus ECN and batching.

## Identity and login

- Your identity is an Ed25519 key pair in `identity.key`. The base64 public key is your
  user id; there are no accounts, emails or passwords.
- Login: the server sends a random nonce in `Hello`. The client signs
  `"ingle-login-v1" ‖ nonce ‖ TLS exporter ‖ user id`. The TLS exporter ties the signature
  to this exact connection, so a login can't be replayed or relayed elsewhere.
- New people need an invite code (from the link) the first time; members can always log
  back in. Invites can be limited by uses and expiry. The owner link is a one-use invite
  that grants the owner role. Roles: owner, admin, member. Bans are by identity.

## End-to-end encryption

The server relays media it cannot read. The design is a simplified cousin of
SFrame + MLS (Discord's DAVE and Signal's group calls do similar things):

1. On joining a voice channel, each participant makes a fresh X25519 **session key** and
   signs it, together with the channel id, with their identity key. Everyone checks these
   signatures, so the server can't swap in its own keys.
2. Each sender picks a random 32-byte **media secret** per epoch and sends it to each other
   participant, wrapped with X25519(session, session) → HKDF-SHA256 → ChaCha20-Poly1305.
3. **Rotation**: whenever someone joins or leaves, every sender starts a new epoch, after a
   300 ms delay so the new key arrives first. Old epochs stay valid for 6 seconds to
   cover packets in flight. People who left can't decrypt new media; newcomers can't
   decrypt old media. A sender doesn't transmit until its key has been distributed.
4. Each packet or frame is sealed with ChaCha20-Poly1305 under a key derived from
   (media secret, track), with the sequence number as nonce. The header's first 10 bytes
   are authenticated; the session field, written by the server, isn't (a wrong value just
   fails to decrypt).
5. The **call code** is a hash of everyone's identity and session keys. If everyone sees
   the same digits, nobody is in the middle.

Not covered: text chat is not end-to-end encrypted (the server stores it, like Discord);
the server sees who talks to whom and when (metadata) and packet sizes.

## The relay (SFU)

- **Voice**: datagrams fan out to everyone in the channel except the sender and deafened
  people. Muted people's packets are dropped. The server rewrites only the session field.
  A moderator's **server mute** and **server deafen** (`MUTE_MEMBERS`, `DEAFEN_MEMBERS`) are
  enforced here too, whatever the person's app does: a server muted person's packets are
  dropped (their stream's sound too), and a server deafened person is sent none and their
  microphone's packets are dropped too (like deafening yourself). Only someone ranked above them undoes it (a moderator can't undo their own;
  the owner can). Both are kept per member in the server's settings, so they last through
  rejoins and restarts.
- **Soundboard**: a sound played is a small control message (`PlaySound` → `SoundPlayed`
  to everyone in the channel), not audio: each app mixes the sound itself, from Ingle's own
  sounds (made by the app) or the Opus file the sound is on its server (cached). Rate-limited
  per person; server muted people's sounds don't play.
- **Stream sound** (track 3) goes only to people watching that person's screen share.
- **Video**: people subscribe per track. Cameras are subscribed automatically; screen
  shares only when you click *Watch Stream*. Each (viewer, track) has its own queue: if
  the oldest unacknowledged frame is older than 350 ms + RTT, the server skips frames for
  that viewer until the next keyframe and asks the sender for one (rate-limited). So one
  slow viewer doesn't slow down everyone else's stream.
- Keyframe requests from viewers are passed to the sender, also rate-limited.

## Audio pipeline (client)

One real-time thread (MMCSS "Pro Audio" priority on Windows) runs in 10 ms steps:

```
mic (cpal) → resample to 48 kHz → AEC3 echo canceller → noise suppression
  (RNNoise "Strong" or WebRTC NS "Standard"; or "Voice isolation": DeepFilterNet3 cleans
  the sound, 30 ms behind, while RNNoise's speech detector on the raw mic decides when you
  talk and a level check keeps only the close voice) → AGC2 → level + voice detection
  → gate (voice activity with 400 ms hangover and 20 ms pre-roll, or push-to-talk)
  → Opus 20 ms, mono, VBR, in-band FEC, 64 kbps default → seal → datagram

datagram → open → per-person jitter buffer (adaptive target 40–200 ms, FEC recovery,
  packet-loss concealment, talkspurt re-buffering, latency trimming; after a stall or a
  long loss burst it jumps back to live instead of catching up) → Opus decode
  → stereo mixer (per-person and per-stream volume, soft clip) → speakers (cpal)
         └→ also fed to the echo canceller as the far-end signal
```

**Sound devices that stop.** Windows can stop calling a stream back without a word (a
virtual device such as NVIDIA Broadcast or a GoXLR restarting or changing format, a USB
hiccup), or report that the device went away. Each stream counts its callbacks; the audio
thread reopens a microphone or speakers that went a second without one (three for a new
stream) or reported they're gone, on a thread of its own, retrying after 0.5, 1, 2 and then
every 5 s. Meanwhile the mixer keeps running by the clock, so the speaking rings go on and
nothing piles up. After 1.5 s the call screen says so. If the device chosen stays away for
10 s, the default one stands in until it's back (checked every 5 s). Notification sounds
reopen stuck speakers the same way.

Libraries: `cpal` (WASAPI/ALSA/CoreAudio), `sonora` (pure-Rust port of WebRTC's audio
processing: AEC3, NS, AGC2, high-pass), `nnnoiseless` (RNNoise), `deep_filter` with `tract`
(DeepFilterNet3, its model built in), `opus` (libopus 1.6).

**Stream sound** has its own thread: 48 kHz stereo capture, Opus 128 kbps stereo
(`Application::Audio`), paused during silence. On Windows it uses WASAPI *process
loopback*: whole-screen shares capture every process except Ingle's own process tree,
window shares capture just that window's process tree.

Some PCs play all their sound again from another process (Voicemeeter, SteelSeries Sonar,
NVIDIA Broadcast's speaker effects), which puts the call back into the stream where
process loopback can't leave it out. So the stream thread also gets a copy of what the call
plays (`PlayedTap`, filled only while a stream with sound runs) and `stream_echo` takes it
back out: GCC-PHAT between the two at 16 kHz finds a delayed copy of the call even under
loud game sound (a peak over 50 times the median, twice in a row at the same delay), then
the way back is learnt per channel as an H1 estimate (cross-spectra over 43 ms frames,
averaged over about 30 s of talk; the game averages away), turned into a 30 ms impulse
response that's shrunk tap by tap against the noise seen far out, and subtracted with a
partitioned overlap-save filter. Nothing is suppressed or ducked, so the game sounds the
same, it only subtracts while that makes the stream quieter, and until the call is found
the stream goes out bit for bit as captured. `examples/stream_echo_eval.rs` measures it:
31-37 dB of the call removed after a few seconds of talk (about 20 dB if the other program
compresses it).

## Video pipeline (client)

```
capture → latest-frame slot (never queues stale frames) → encoder thread:
  scale (fast_image_resize) → RGB→I420 (yuv, BT.601) → OpenH264
  → seal → one QUIC stream per frame
```

- Capture: Windows Graphics Capture (monitors and windows, via `windows-capture`); X11
  `GetImage` on Linux; cameras via `nokhwa` (Media Foundation, V4L2, AVFoundation).
- OpenH264 runs in real-time mode (`CameraVideoRealTime` or `ScreenContentRealTime`),
  bitrate rate control with frame skipping, a keyframe every 10 s and on request.
- Backpressure: at most ~0.12 s of frames in flight; a static screen is re-sent at least
  every 500 ms, so a new viewer sees it.
- Receive: frames are reordered for up to 120 ms; after a gap the decoder waits for the
  next keyframe and asks for one. Decoded frames land in a store that the UI uploads as
  textures (only the newest frame).

## Server storage

`ingle-data/ingle.db` (SQLite, WAL): meta (the server's name and settings), users (id,
name, role, banned), roles and member_roles, channels, messages (with reply and edit time),
reactions, invites, files (attached pictures, profile pictures and emoji pictures, each
linked to its message; the bytes sit in `files/`), dms, groups and group_members, relations
(friends, requests, blocks), emojis and bug_reports. TLS key and certificate sit next to it. Voice state lives
only in memory. The data folder is made readable only by the server's own account (0700),
whichever command creates it first.

The owner's app backs all of it up every hour (see the README), into folders and files
readable only by them from the moment they're written. The server's private key goes into
the backup sealed to the owner (`server-key.der.sealed`: ChaCha20-Poly1305 with a key derived
by HKDF from the owner's identity), so a backup folder that leaks (a synced cloud folder,
say) can't be used to pass for the server; restoring with the server's identity takes the
owner's Ingle, or their `identity.key`.

Each member's row notes the invite they joined with: banning them deletes that invite, so
they can't walk back in through the same link with a new identity.

## Calls and groups

A direct message's call is a voice channel only the two people in it can see: the same
relay, the same end-to-end encryption and call code (`server/src/hub.rs` keeps each call's
*audience*, and tells nobody else it exists). Calling someone joins the call and sends
`Ring`; their apps ring (every device they're on) until one answers or declines, or it rings
out after 45 seconds and counts as missed (`engine/src/ring.rs`, on a thread of its own so it
rings with the window hidden). Moderators can't mute or move anyone in it, and blocking
someone ends it.

A group (up to 10 people, `GROUPS_FEATURE`) is a conversation of its own with a call of its
own, stored like a channel only its members can read. Adding people to a direct message's
call makes a group and moves the call there: the ring tells those who were in the first call
to follow (`moved_from`), so the call carries on without anyone answering again, and rings
everyone added. A group call rings for each person until they answer or say no, or it rings
out, whoever else has answered; the one calling sees those still ringing in the call as
*Calling…*. Apps say they understand
groups when they log in (`GROUPS_CAP`), and the server tells older apps nothing about them.

## Bug reports

Every copy of Ingle sends its bug reports to one central inbox, the server named when Ingle
is built (`INGLE_BUG_SERVER=host:port/<server key>`), whichever servers its user is on: a
report is sent instead of logging in, with no account (`engine/src/bugs.rs`). That server
keeps reports only while its owner has *Collect Ingle bug reports* on, and caps them per
address and per hour. The owner reads them under Bug reports in the channel list and
approves the ones to fix; their app saves the approved ones (with their logs) to
`approved-bug-reports/<server key>.json` in its settings folder, which is what Claude reads
when asked to fix them. A report is a stranger's words: it never decides anything.

A report that can't be sent stays in the Report a bug window with the reason (or comes
back from a notice, if the window was closed), and when the inbox is the owner's own server
with collecting off, sending switches it on first.

## Plugins

**The Ingle Library** is a list on the website (`ingle/library/index.json`) of programs and
add-ons, with every file's SHA-256 and size, signed with the release key under its own
context (`ingle plugin library v1`, so an update signature can't pass as a list or the other
way round) and dated like a release: the app takes no list past its date or older than one it
has seen (`proto/src/library.rs`, `app/src/plugins/library.rs`). It's made offline with
`scripts/make-library.sh` from `installer/site/library/plugins.json` and the built files.

**Installing** (`app/src/plugins/install.rs`) downloads a plugin's files into a new folder,
checks each against the list, and swaps the folder in whole; a program's files are checked
again before every launch. A plugin taken out of the list stops (an add-on) or won't open (a
program).

**Add-ons** (`app/src/plugins/addon.rs`) are WebAssembly modules run by wasmi, an
interpreter, each on a thread of its own. A module that imports anything is refused, so an
add-on can reach nothing but the JSON Ingle hands it; each call gets a fuel budget (about 300
million instructions) and the module 64 MB of memory, and one that runs out is stopped. It can
add /commands, draw a panel from a few kinds of widgets, and keep up to 64 KB of data of its
own. What it sends (4,000 characters at most) goes only to the channel that's open, as the
person's message, and only if the add-on asked for the commands permission when it was
listed. The plugin kit (`plugins/kit`) writes the module's side.

**Programs** are apps of their own, run with the user's rights like anything else they
install: the protection is that the library only lists programs Firelit Games has read and
built from source, and Ingle only opens the files the list names. On Windows a program opens
inside Ingle's window (`app/src/plugins/programs.rs`): Ingle starts it with `INGLE_EMBED`
set, takes its main window off the taskbar, makes Ingle's window its owner, removes its frame
and keeps it over the page it has in Ingle (following the window as it moves, and stepping
aside for Ingle's own menus, dialogs and notices). A window that keeps part of its frame
anyway (an undecorated one that keeps room round itself for its shadow, say) is made that much
bigger and cut down to the part it draws in, so that part fills the page to its edges.
**Pop out** gives it its frame back.

**Plugins sent in** (`engine/src/plugins.rs`): anyone's Ingle sends a plugin's Rust source
(never a built file) to the library server, which is the bug report server unless Ingle was
built with `INGLE_LIBRARY_SERVER`, before logging in and with no account. The server keeps
it only while its owner has *Collect plugins for the Ingle Library* on, after checking it
(a Cargo.toml, plain relative paths, 600 KB and 400 files at most, a few an hour per address,
a cap on how many wait), and only the owner sees them, under Plugins sent in. Approving one
saves its source to `approved-plugins/<server key>/<id>-<name>/` on the owner's PC;
building, signing and listing happen offline. What's sent in is a stranger's code and words:
it never decides anything.

## Setting up servers (`crates/host`)

**Create a server** in the app ends with the same step either way: an SSH session to a
Linux machine that uploads a static `ingle-server` (musl, built for x86-64 and ARM64 by
`scripts/build-vps-server.sh` and shipped inside both installers) plus
`install-ingle-server.sh`, and runs the script as root (directly, or through `sudo -n` /
`sudo -S`). The script is idempotent: it installs the program to `/usr/local/bin`,
creates the `ingle-server` user and `/var/lib/ingle-server`, writes a hardened systemd
unit, opens the UDP port in ufw, firewalld or iptables when one of them is active, runs
`ingle-server links` (sets up the community and prints the owner and invite links as
`INGLE_*=` lines) and starts the service. Lines starting with `==> ` become progress steps
in the app.

- **Rent** (`digitalocean.rs`): with the person's API token, add Ingle's SSH key
  (`POST /v2/account/keys`, or find it on a 422), create an `ubuntu-24-04-x64` droplet
  with that key and an SSH host key Ingle made for it (through cloud-init, so the first login
  already knows the server it's talking to), poll until it's `active` with a public IPv4
  address, then install over
  SSH as root, retrying while the droplet boots. Prices and regions come from the API.
  A droplet that exists after a failure is reported back so the UI can retry the install
  or delete it. The token stays in memory only.
- **Own server** (`install.rs`, `ssh.rs`): `russh` (pure Rust, `ring` crypto) with a
  password (falling back to keyboard-interactive) or an OpenSSH/PEM/PKCS#8 key with an
  optional passphrase. Host keys are never trusted on first use: a server seen for the first
  time isn't logged into (no password or key is sent) until the person has checked its
  SHA-256 fingerprint, which the app shows with how to check it on the server. A checked key
  is remembered, keys in this PC's `~/.ssh/known_hosts` count as checked, and a key that
  changed is refused. Passwords aren't stored.
- **Check** (`probe.rs`): a QUIC handshake from this PC to the new server, pinned to the
  fingerprint in the owner link. A timeout means something in front of the server (usually
  the provider's firewall) blocks UDP, and the UI says where to open it.
- Ingle's own key (`ssh/ingle_ed25519`) and a list of set-up servers
  (`hosted-servers.json`) live in the settings folder.

The app joins the new server with the owner link as soon as the setup finishes.

## Limits

What one person, or one place, can use up on a server (`server/src/hub.rs`, `conn.rs`):

- **Connections**: 32 at once from one place (an IPv4 address, or an IPv6 /64), 128 from an
  IPv6 /48, 6 per member, 4000 in all (`INGLE_MAX_CONNECTIONS`). Before logging in: 8 per
  place, 16 per /48 and 256 in all, and 15 seconds to log in, handshake included.
- **Requests**: a token bucket per connection (20 a second, bursts of 80; heavier requests
  cost more), messages and reactions with buckets of their own.
- **Voice**: 200 sound packets and 96 KB a second per sender (bursts of twice that), dropped
  before they're copied to anyone; video 5 MB a second per sender (bursts of 24 MB). A
  connection that stops reading is dropped once 16 MB waits for it.
- **Joining**: 10 wrong invite codes per place in 10 minutes; everyday invites let in 120
  people an hour (bursts of 30).
- **Ringing**: a call rings (or says no) 4 times in a row, then once every 5 seconds;
  starting or adding to a group costs four requests.
- **Storage**: 5 GB for a server's pictures and files (`INGLE_STORAGE_MB`), a tenth of that
  (at most 1 GB) per member, 100 MB in a member's first three days, 400 MB an hour.
- **Bug reports** (only while collecting): 5 an hour per place, 60 an hour in all, 2000
  waiting at most; the owner's list always fits in one reply.

## Threat model in short

- A malicious **server** can read and alter text chat, see metadata, drop or delay media,
  and refuse service. It can't decrypt or forge voice/video/stream sound, can't
  impersonate an identity (logins are signatures), and can't substitute call keys without
  the call code changing.
- A malicious **participant** can record what they receive, like in any call.
- Losing `identity.key` loses that identity; anyone who copies it becomes you (and can put
  back the server identity in your backups).
- **GIF search keys** are the kind every member's app has: GIPHY doesn't allow searches or
  GIF loads to go through a server ("Do not proxy requests to GIPHY"), and KLIPY also wants
  each app to search it directly, so the owner's keys are sent to every member's app. An
  owner should use keys made just for their server, and change them if they're misused.
- **Updates** are signed (Ed25519) release files with dates: an app takes no file past its
  date or older than one it has seen. On Linux the package is checked again as root, in a
  folder only root can write, and installed from there.
- **Bug reports** carry the end of the app's log, with DigitalOcean tokens, GIF keys and the
  codes in invite links taken out first.
- **Plugins**: an add-on can't reach anything outside its sandbox, and only sends what the
  person asked for to the channel they have open. A program runs with the user's rights, so
  what keeps a bad one out is the review before it's built and the signed list Ingle checks
  its files against. Whoever holds the release key could list a harmful program, as they
  could sign a harmful update.

## Roadmap

In rough priority order:

1. Field-test on Windows hardware: device hot-plug, Bluetooth headsets, multi-monitor
   capture, high-DPI, firewall prompts.
2. Hardware video encoding (Media Foundation → NVENC/AMF/QuickSync) and AV1 screen
   content coding: much better quality per bit, lower CPU.
3. Simulcast or SVC (AV1 L1T3/L3T3) so each viewer gets a quality that fits their
   connection.
4. GPU path: capture texture → encoder without CPU copies; render video with native wgpu
   textures instead of re-uploading.
5. Transport-wide congestion feedback (GCC/SCReAM-style) driving the encoder bitrate.
6. Wayland (xdg-desktop-portal ScreenCast + PipeWire, GlobalShortcuts for PTT) and macOS
   (ScreenCaptureKit); stream sound on Linux (PipeWire) and macOS.
7. Opus 1.5/1.6 neural features (deep PLC, NoLACE) and DRED once its format is final;
   RED (redundant audio) under heavy loss.
8. MLS (`openmls`) for key management in big calls; optional E2EE text channels.
9. Bots as members; attachments beyond pictures, threads, search. (Plugins came in 0.7.2,
   Rust only: the kit is a Rust crate, and plugins come as Rust source that's built from it.)
10. A web client over WebTransport + WebCodecs, then mobile.
11. H.264 licensing: OpenH264 here is compiled from source, which Cisco's patent licence
    doesn't cover. Before shipping widely, either download Cisco's prebuilt OpenH264 at
    runtime or default to AV1.
