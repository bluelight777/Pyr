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

`ingle-data/ingle.db` (SQLite, WAL): meta (name, default invite), users (id, name, role,
banned), channels, messages (with reply, edit time), reactions, invites. TLS key and
certificate sit next to it. Voice state lives only in memory.

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
  with that key, poll until it's `active` with a public IPv4 address, then install over
  SSH as root, retrying while the droplet boots. Prices and regions come from the API.
  A droplet that exists after a failure is reported back so the UI can retry the install
  or delete it. The token stays in memory only.
- **Own server** (`install.rs`, `ssh.rs`): `russh` (pure Rust, `ring` crypto) with a
  password (falling back to keyboard-interactive) or an OpenSSH/PEM/PKCS#8 key with an
  optional passphrase. Host keys are trusted on first use and their SHA-256 fingerprint
  is logged. Passwords aren't stored.
- **Check** (`probe.rs`): a QUIC handshake from this PC to the new server, pinned to the
  fingerprint in the owner link. A timeout means something in front of the server (usually
  the provider's firewall) blocks UDP, and the UI says where to open it.
- Ingle's own key (`ssh/ingle_ed25519`) and a list of set-up servers
  (`hosted-servers.json`) live in the settings folder.

The app joins the new server with the owner link as soon as the setup finishes.

## Threat model in short

- A malicious **server** can read and alter text chat, see metadata, drop or delay media,
  and refuse service. It can't decrypt or forge voice/video/stream sound, can't
  impersonate an identity (logins are signatures), and can't substitute call keys without
  the call code changing.
- A malicious **participant** can record what they receive, like in any call.
- Losing `identity.key` loses that identity; anyone who copies it becomes you.

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
9. Attachments and image embeds, GIF picker, custom emoji, DMs and a friends list,
   threads, search.
10. A web client over WebTransport + WebCodecs, then mobile.
11. H.264 licensing: OpenH264 here is compiled from source, which Cisco's patent licence
    doesn't cover. Before shipping widely, either download Cisco's prebuilt OpenH264 at
    runtime or default to AV1.
