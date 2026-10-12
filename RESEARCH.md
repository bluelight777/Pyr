# Research: building a better Discord for voice, webcams and screen sharing

Compiled 2026-10-10/11 from web research (four research passes: Discord's weak points and
internals; competitors and media technology; the Rust library ecosystem, with
cross-compile tests; names). Every claim has a source link. **UNVERIFIED** marks claims
no primary or fetched source confirmed. "Derived" means our own arithmetic.

Contents:
1. [Summary: where Discord can be beaten](#1-summary-where-discord-can-be-beaten)
2. [Why people complain about or leave Discord](#2-why-people-complain-about-or-leave-discord)
3. [How Discord's voice and video work](#3-how-discords-voice-and-video-work)
4. [The landscape](#4-the-landscape)
5. [Media technology findings](#5-media-technology-findings)
6. [The Rust stack](#6-the-rust-stack)
7. [Naming](#7-naming)
8. [What Ingle took from this](#8-what-ingle-took-from-this)

---

## 1. Summary: where Discord can be beaten

1. **Free high-quality screen share.** Free Discord streams are capped at 720p30; 1080p60
   and 4K need Nitro or server boosts ([Go Live FAQ](https://support.discord.com/hc/en-us/articles/360040816151-Go-Live-and-Screen-Share)).
2. **One weak viewer shouldn't degrade everyone.** Discord caps a stream to its slowest
   viewer ([Discord engineering blog](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology));
   a 2021 report saw camera bitrate drop to ~150 kbps for everyone when one viewer with a
   poor connection joined ([support thread](https://support.discord.com/hc/ja/community/posts/1500000784202-Video-quality-dropping-to-150-Kbps-when-someone-with-poor-connection-joins)).
3. **Better voice without boosts.** Unboosted servers cap voice at 96 kbps
   ([How-To Geek](https://www.howtogeek.com/714745/how-to-adjust-the-audio-bitrate-on-discord/));
   Opus goes to 510 kb/s ([opus-codec.org](https://opus-codec.org/)).
4. **A small native app.** Discord's Electron client can pass 4 GB of RAM in some
   sessions; Discord tested auto-restarting it above 4 GB
   ([PCWorld](https://www.pcworld.com/article/3005317/discord-admits-windows-11-app-hogs-ram-tries-solving-it-with-auto-restarts.html)).
5. **No ID checks, no accounts to leak.** Discord's 2025 vendor breach exposed about 70,000
   government-ID photos ([Discord](https://discord.com/press-releases/update-on-security-incident-involving-third-party-customer-service)),
   and global age assurance began rolling out in September 2026
   ([ID Tech](https://idtechwire.com/discord-begins-global-age-assurance-rollout-with-new-verification-options/)).
6. **Self-hosting as one binary**, so a community isn't hostage to someone else's outages,
   policy or ads.
7. **First-class Linux** (PipeWire capture, Wayland push-to-talk, VAAPI) where Discord
   and its community clients still have gaps.
8. **Table stakes people miss when leaving**: drop-in voice channels next to text,
   one-link onboarding, push-to-talk, low-latency screen broadcast, reactions, history
   ([HN thread](https://news.ycombinator.com/item?id=46949564), [LWN](https://lwn.net/SubscriberLink/1058319/7f10cd1d82956e9f/)).

---

## 2. Why people complain about or leave Discord

**Age verification**
- April 2025: face-scan and ID checks tested in the UK and Australia
  ([MediaNama](https://www.medianama.com/2025/04/223-discord-experimental-age-verification-system-uk-australia/)).
- UK, 25 July 2025: Online Safety Act duties took effect; users passed the face check
  with a video game's photo mode ([PinkNews](https://www.thepinknews.com/2025/07/25/discord-video-game-characters-age-verification-checks-uk-online-safety-act)).
- 9 Feb 2026: "teen-by-default" announced for every account worldwide
  ([Discord](https://discord.com/press-releases/discord-launches-teen-by-default-settings-globally)).
  The [HN thread](https://news.ycombinator.com/item?id=46945663) drew 2,046 points; US
  searches for "Discord alternatives" rose ~10,000% in 48 hours
  ([Windows Central](https://www.windowscentral.com/software-apps/discord-alternative-search-10000-percent-stoat));
  TeamSpeak hit hosting capacity ([Kotaku](https://kotaku.com/discord-alternative-teamspeak-age-verification-check-rivals-2000669693)).
- 24 Feb 2026: rollout delayed to H2 2026; Discord said 90%+ of users would never be asked
  ([Discord blog](https://discord.com/blog/getting-global-age-assurance-right-what-we-got-wrong-and-whats-changing),
  [TechCrunch](https://techcrunch.com/2026/02/24/discord-delays-global-rollout-of-age-verification-after-backlash/)).
- From the week of 23 Sep 2026: an age model from account signals, plus card, app-store,
  wallet, on-device face estimation or ID + selfie
  ([ID Tech](https://idtechwire.com/discord-begins-global-age-assurance-rollout-with-new-verification-options/)).

**The 2025 vendor breach.** Disclosed 3 Oct 2025 (vendor 5CA): ~70,000 ID photos, names,
emails, IPs, partial payment data, support messages
([Discord](https://discord.com/press-releases/update-on-security-incident-involving-third-party-customer-service)).
5CA denied responsibility ([SecurityWeek](https://www.securityweek.com/customer-service-firm-5ca-denies-responsibility-for-discord-data-breach/));
the attackers claimed far larger numbers, which Discord called incorrect
([Security Affairs](https://securityaffairs.com/183143/cyber-crime/discord-denies-massive-breach-confirms-limited-exposure-of-70k-id-photos.html)).

**Paywalls on call quality**
- Go Live: free 720p/30fps, Nitro up to 4K/60; 50 viewers; one stream watched at a time
  ([Go Live FAQ](https://support.discord.com/hc/en-us/articles/360040816151-Go-Live-and-Screen-Share)).
  Boost level 1 gives 720p60, level 2 1080p60 ([Boosting FAQ](https://support.discord.com/hc/en-us/articles/360028038352-Server-Boosting-FAQ)).
- Voice: 64 kbps default, 8–96 kbps without boosts; 128/256/384 kbps at boost levels
  ([How-To Geek](https://www.howtogeek.com/714745/how-to-adjust-the-audio-bitrate-on-discord/),
  [Audio Bitrate FAQ](https://support.discord.com/hc/en-us/articles/11635925354775-Audio-Bitrate-FAQ)).
- Uploads: free 20 MB since August 2026 ([Discord](https://support.discord.com/hc/en-us/articles/25444343291031-File-Attachments-FAQ)).
  Nitro $9.99/month, Basic $2.99 ([discord.com/nitro](https://discord.com/nitro)).

**Resource use of the Electron app**
- Normal use "under 1 GB", but sessions past 4 GB
  ([Windows Latest](https://www.windowslatest.com/2026/09/27/electron-is-one-of-windows-11s-biggest-ram-offenders-and-discords-game-mode-doesnt-fix-it/));
  auto-restart test above 4 GB ([PCWorld](https://www.pcworld.com/article/3005317/discord-admits-windows-11-app-hogs-ram-tries-solving-it-with-auto-restarts.html)).
- TechRadar: 780 MB rising to 1.4 GB after a few minutes of streaming
  ([TechRadar](https://www.techradar.com/computing/memory/some-windows-11-apps-have-a-massive-ram-problem-and-this-app-is-the-worst-offender)).
- Electron 42 (Sep 2026): median CPU −17%, median RAM +4%
  ([PCGH](https://www.pcgameshardware.de/RAM-Hardware-154108/News/Discord-CPU-Last-Performance-1554217/)).
  Startup improved ~650 ms in June 2026 ([patch notes](https://discord.com/blog/discord-patch-notes-june-4-2026)).
  No rigorous independent benchmark found (UNVERIFIED).

**Linux**
- Wayland screen share with audio reached stable in Jan 2025, PulseAudio apps only, with
  choppy software encoding reports ([GamingOnLinux](https://www.gamingonlinux.com/2025/01/discord-screen-sharing-with-audio-on-linux-wayland-is-officially-here)).
- Dec 2025–May 2026: game capture, AMD VAAPI encoding, Gamescope zero-copy, packages
  ([PCGH](https://www.pcgameshardware.de/Linux-Software-26761/News/Das-Jahr-des-Linux-Desktops-1534559/));
  NVIDIA/Intel hardware encoding not confirmed ([pbxscience](https://pbxscience.com/discord-declares-year-of-the-linux-desktop-with-native-fedora-arch-packages-and-major-performance-overhaul/)).
- Electron apps on native Wayland lose global keybinds
  ([Hyprland discussion](https://github.com/hyprwm/Hyprland/discussions/10351)); the
  community client Vesktop has no global keybinds ([Vesktop](https://github.com/Vencord/Vesktop),
  [declined PR](https://github.com/Vencord/Vesktop/pull/326)).

**Ads, outages, regulation, privacy**
- Rewarded video ads ("Quests") and an auction-priced Ads Manager launched 29 Sep 2026
  ([ppc.land](https://ppc.land/discord-opens-auction-priced-ad-buying-to-studios-of-any-size-by-early-2027/)).
- Voice outages on 19 Mar 2026 ([Techloy](https://www.techloy.com/discord-voice-not-working-heres-why-thousands-of-users-were-affected.md)),
  8 May 2026 ([GV Wire](https://gvwire.com/2026/05/08/discord-goes-down-for-tens-of-thousands-of-users-downdetector-shows/)),
  22 Jul 2026 ([GV Wire](https://gvwire.com/2026/07/22/discord-goes-down-for-thousands-of-users-wednesday-downdetector-shows/)).
  Discord's own blog admits >1 s call latency at peak in one region during its edge
  migration ([Discord](https://discord.com/blog/how-we-moved-discord-voice-to-the-edge)).
- Brazil ordered Go Live suspended on 12 Aug 2026 ([Al Jazeera](https://www.aljazeera.com/news/2026/8/12/brazil-orders-discord-to-suspend-livestreams-after-teens-death));
  video and screen share were then off in Brazil ([Social Day](https://socialday.live/features/discord-suspends-screensharing-and-video-in-brazil-after-child-safety-order)).
  Current status UNVERIFIED.
- A zero-click location leak via cached avatars ([write-up](https://gist.github.com/hackermondev/45a3cdfa52246f1d1201c1e8cdef6117)).

**What people want from an alternative**: persistent drop-in voice channels, text and
voice together, one-link onboarding, low-latency high-fps screen broadcast,
push-to-talk, history, reactions, the ability to delete their data
([HN 46949564](https://news.ycombinator.com/item?id=46949564), [HN 46945663](https://news.ycombinator.com/item?id=46945663)).
Matrix lacks game streaming, PTT and voice channels ([Matrix](https://matrix.org/blog/2026/02/welcome-discord/));
Mumble has no video; TeamSpeak had no built-in video
([TechCrunch](https://techcrunch.com/2026/03/01/best-discord-alternatives-age-verification-identity-privacy)).
No quantitative survey found (UNVERIFIED).

---

## 3. How Discord's voice and video work

**Architecture**
- 2018: no peer-to-peer; all media through Discord's servers (hides IPs, absorbs DDoS).
  Elixir signaling, a homegrown C++ SFU, a native media engine on WebRTC, Salsa20 instead
  of DTLS/SRTP, nothing sent in silence; 850+ voice servers, 2.6M concurrent users
  ([Discord](https://discord.com/blog/how-discord-handles-two-and-half-million-concurrent-voice-users-using-webrtc)).
- 2025–26: voice moved to Cloudflare's edge (300+ cities) on a new **Rust SFU** (Tokio, 8
  workers each with their own UDP socket, one call per SFU instance, ~25,000 hosts).
  Frankfurt ping −34%, loss −42% ([Discord](https://discord.com/blog/how-we-moved-discord-voice-to-the-edge)).

**Codecs**
- Voice: Opus 48 kHz stereo ([voice docs](https://docs.discord.com/developers/topics/voice-connections)).
- Video: VP8 and H.264, plus HEVC and AV1 on some platforms; one codec every viewer can
  decode; streams not synced with voice; streamer bitrate capped by the slowest viewer
  ([Discord](https://discord.com/blog/how-it-all-goes-live-an-overview-of-discords-streaming-technology)).
  AV1 on RTX 40 since Feb 2023 ([NVIDIA](https://www.nvidia.com/en-us/geforce/news/nvidia-studio-discord-av1/)).
- Fixing AMD keyframe forcing cut sessions below 50 fps from 39% to 18%
  ([Discord](https://discord.com/blog/from-blocky-to-brilliant-improving-video-quality-on-discord-go-live-on-amd-gpus)).
- Simulcast is undocumented; reverse-engineering shows one layer capped at 2,500 kbps,
  30 fps, ~720p ([Spacebar docs](https://docs.spacebar.chat/contributing/reverse-engineering/voice/webrtc/), UNVERIFIED).

**DAVE end-to-end encryption**
- Frames encrypted on the sender's device; MLS (RFC 9420) group keys with the voice
  gateway as delivery service; per-call P-256 device keys by default
  ([Discord](https://discord.com/blog/meet-dave-e2ee-for-audio-video)).
- AES-128-GCM with 8-byte tags and 32-bit nonce; codec headers left in clear. Documented
  weaknesses: not key-committing, unauthenticated clear-range offsets, the SFU can inject
  silence, removed members can decrypt for ~2 s ([daveprotocol.com](https://daveprotocol.com/)).
- Mandatory since 1 Mar 2026 except Stage channels
  ([support](https://support.discord.com/hc/en-us/articles/38749827197591-A-V-E2EE-Enforcement-for-Non-Stage-Voice-Calls));
  audited by Trail of Bits, libdave open source ([Discord](https://discord.com/blog/bringing-dave-to-all-discord-platforms)).
  No plans to encrypt text ([Discord](https://discord.com/blog/every-voice-and-video-call-on-discord-is-now-end-to-end-encrypted)).

**Voice processing**: Krisp noise suppression on-device since 2020; it switches itself off
when CPU use is high ([Krisp FAQ](https://support.discord.com/hc/en-us/articles/360040843952-Krisp-FAQ)),
which is exactly when people are gaming. Echo cancellation / AGC presumably from WebRTC
(UNVERIFIED). No published latency figures (UNVERIFIED).

---

## 4. The landscape

| Product | State (Oct 2026) | Notes |
|---|---|---|
| **TeamSpeak 6** | beta 6.0.0-beta4.1; 1:1 calls in alpha ([It's FOSS](https://itsfoss.com/news/teamspeak-direct-calls-teased/)) | Screen/camera share "up to 4K or full source", H.264 via NVENC or OpenH264. Complaints: 5 fps streams on an RTX 4090, ~196 ms monitor capture ([forum](https://community.teamspeak.com/t/screensharing-severe-lag-capture-time-200ms-stuck-at-5fps-loud-audio-spike-on-voice-start/63900)), RAM leak report ([forum](https://community.teamspeak.com/t/ts6-client-uses-a-lot-of-ram-after-a-short-while/55919)). Self-host server beta with a 32-slot licence ([GitHub](https://github.com/teamspeak/teamspeak6-server)). |
| **Mumble** | 1.5.915 (Jul 2026), 1.6 RC on Qt6 + RNNoise 0.2 ([blog](https://www.mumble.info/blog/mumble-1.5.915/)) | Low-latency Opus voice, positional audio, self-hosted; server holds keys (not E2EE), known OCB2 weaknesses ([issue](https://github.com/mumble-voip/mumble/issues/4219)); no video or screen share. |
| **Stoat** (ex-Revolt) | renamed Oct 2025 ([Wikipedia](https://en.wikipedia.org/wiki/Stoat_(software))) | Rust backend, Electron desktop; voice on LiveKit; video/screen share shipped Jul 2026 capped at **720p30** ([release notes](https://stoat.chat/release-notes/93eb1f4ec77444ca9fb13617e0e6c0b1)). |
| **Fluxer** | public since Jan 2026, ~500k users ([blog](https://canary.fluxer.app/blog/how-i-built-fluxer-a-discord-like-chat-app)) | Erlang + Rust, AGPL, self-host; moving from LiveKit to **RTP in QUIC datagrams**, WebTransport web client, DAVE mandatory ([roadmap](https://fluxer.app/blog/roadmap-2026)). |
| **Root** | $9M seed ([Game Developer](https://www.gamedeveloper.com/press-release/root-raises-9m-in-funding-to-build-a-next-gen-discord)) | Proprietary; voice/video/app share; per-window app audio ([changelog](https://www.rootapp.com/changelogs/feb-4-2026)); no self-host. |
| **Element Call** | default in Element ([talk](https://media.ccc.de/v/matrix-conf-2025-74592-element-call-redefining-conferencing-for-privacy-scale-and-sovereignty)) | E2EE via MatrixRTC; self-hosting needs LiveKit + JWT service + .well-known ([Element](https://element.io/blog/end-to-end-encrypted-voice-and-video-for-self-hosted-community-users/)), and fails in practice ([issue](https://github.com/element-hq/element-x-android/issues/6544)); no voice channels or PTT ([gist](https://gist.github.com/SturmB/0c75dfbf61b9416370310dcc4b065733)). |
| **Jitsi Meet** | JVM SFU ([GitHub](https://github.com/jitsi/jitsi-videobridge)) | AV1 + L3T3 SVC by default, CPU-adaptive fallback; AV1 best for screen share ([Jitsi](https://jitsi.org/blog/av1-and-more-how-does-jitsi-meet-pick-video-codecs/)); screen share defaults to 5 fps ([config.js](https://github.com/jitsi/jitsi-meet/blob/master/config.js)). |
| **Signal** | up to 75 per call ([support](https://support.signal.org/hc/en-us/articles/360052977792-Group-Video-Calling)) | Rust SFU (AGPL), googcc, VP8 3-layer simulcast, per-sender frame keys rotated on join/leave ([Signal](https://signal.org/blog/how-to-build-encrypted-group-calls/), [GitHub](https://github.com/signalapp/Signal-Calling-Service)). |
| **Guilded** | shut down 19 Dec 2025 ([AlternativeTo](https://alternativeto.net/news/2025/11/guilded-to-shut-down-on-december-19-2025-as-focus-shifts-to-roblox-communities/)) | |
| **Spacebar**, **Sharkord** | early | Spacebar: AGPL Discord-backend clone ([GitHub](https://github.com/spacebarchat/server)); Sharkord: Bun + mediasoup alpha ([GitHub](https://github.com/Sharkord/sharkord)). |
| **Parsec** | game streaming | Own UDP protocol with congestion control co-designed with the encoder; latency > fps > quality ([Parsec](https://parsec.app/blog/a-networking-protocol-built-for-the-lowest-latency-interactive-game-streaming-1fd5a03a6007)); 4–8 ms encode-to-render on LAN at 240 fps (vendor test, [Parsec](https://parsec.app/blog/parsec-game-streaming-total-latency-at-240-frames-per-second-c0818cc0daa5)). |
| **Moonlight/Sunshine** | game streaming | NVENC P1 / AMF ultra-low-latency / x264 zerolatency, 20% Reed-Solomon FEC ([docs](https://docs.lizardbyte.dev/projects/sunshine/latest/md_docs_2configuration.html)), single-frame VBV, reference-frame invalidation before IDR ([docs](https://docs.lizardbyte.dev/projects/sunshine/latest/classnvenc_1_1nvenc__base.html)). |
| **RustDesk** | remote desktop | VP8/VP9/AV1 software, H.264/H.265 hardware, self-hosted relay ([blog](https://rustdesk.com/de/blog/rustdesk-vs-vnc-nat-traversal-codecs-verschlusselung)). |

Others: Steam voice is WebRTC + Opus relayed through Valve
([HotHardware](https://amp.hothardware.com/news/valve-overhauls-steam-chat-client-group-tools-friends-list));
Slack huddles allow two concurrent screen shares with drawing
([Slack](https://slack.com/help/articles/4402059015315-Use-huddles-in-Slack)).

---

## 5. Media technology findings

### Voice

- **Latency budget** (ITU-T G.114): ≤150 ms one-way is essentially transparent; 400 ms is
  the planning ceiling ([G.114](https://www.cs.columbia.edu/~andreaf/new/documents/other/T-REC-G.114-200305.pdf)).
  Contributors: Opus frame + look-ahead (CELT 2.5 ms, SILK 5 ms, [RFC 6716](https://www.rfc-editor.org/rfc/rfc6716));
  Windows 10 ms default buffers, down to ~2.66 ms with IAudioClient3 ([Microsoft](https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/low-latency-audio));
  PipeWire default quantum ≈21 ms (derived, [docs](https://docs.pipewire.org/devel/page_man_pipewire_conf_5.html));
  WebRTC's jitter buffer starts at 80 ms ([NetEQ](https://webrtc.googlesource.com/src/+/HEAD/modules/audio_coding/neteq/delay_manager.cc)).
- **Opus settings**: 20 ms frames; fullband speech 28–40 kbps, stereo music 64–128 kbps;
  prefer VBR; FEC only works in speech modes ([RFC 7587](https://www.rfc-editor.org/rfc/rfc7587)).
  FEC reacts late and is capped; RED at distance 2 cut concealment at 20% loss from 18% to
  0.7% ([webrtcHacks](https://webrtchacks.com/red-improving-audio-quality-with-redundancy/)).
- **Opus 1.5/1.6 neural features**: DRED (up to 1 s of redundancy in padding), deep PLC,
  LACE/NoLACE (near-transparent 9 kb/s); off by default
  ([Opus 1.5](https://opus-codec.org/demo/opus-1.5)). Opus 1.6 shrank DRED 3× with an
  incompatible format ([Opus 1.6](https://opus-codec.org/demo/opus-1.6/)); DRED is still a
  draft ([IETF](https://datatracker.ietf.org/doc/draft-ietf-mlcodec-opus-dred/)).
  Verdict: decoder-side features can ship now; DRED's wire format isn't final.
- **Jitter buffer**: NetEQ targets the 95th-percentile arrival delay and time-stretches
  (accelerate / preemptive expand) instead of dropping
  ([NetEQ](https://webrtc.googlesource.com/src/+/HEAD/modules/audio_coding/neteq/g3doc/index.md)).
- **Noise / echo / gain**: RNNoise is ~60× real time with 10 ms look-ahead
  ([demo](https://jmvalin.ca/demo/rnnoise/)); DeepFilterNet3 is better but has 40 ms
  latency ([paper](https://arxiv.org/pdf/2305.08227)); WebRTC AEC3 works on 64-sample
  blocks ([source](https://webrtc.googlesource.com/src/+/HEAD/modules/audio_processing/aec3/aec3_common.h)),
  and `sonora` is a pure-Rust port of AEC3/NS/AGC2 ([docs.rs](https://docs.rs/crate/sonora/0.1.0)).

### Screen share and webcam

- **Codecs**: AV1 has screen-content tools (palette, IntraBC) ([paper](https://arxiv.org/pdf/2008.06091));
  Meta saw up to 80% BD-rate savings vs H.264 at ~5× CPU ([@Scale](https://atscaleconference.com/av1-improving-rtc-video-quality-at-scale/));
  Chrome's AV1 screen share is 45% smaller and 35% faster than VP9 ([Chrome](https://developer.chrome.com/blog/av1)).
  OpenH264 has a screen-content mode ([GNOME MR](https://gitlab.gnome.org/GNOME/gnome-shell/-/merge_requests/3259)).
- **Bitrates** (YouTube live, high motion): 1080p60 6–17 Mbps H.264, 4–12 Mbps AV1/HEVC
  ([YouTube](https://support.google.com/youtube/answer/2853702)). For text, keep
  resolution and drop fps ([W3C content hints](https://www.w3.org/TR/mst-content-hint/)).
- **Low-latency encoding**: NVENC ultra-low-latency CBR, intra refresh or reference
  invalidation for recovery ([NVIDIA](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/nvenc-video-encoder-api-prog-guide/index.html));
  AMF ULTRA_LOW_LATENCY ([AMD](https://github.com/GPUOpen-LibrariesAndSDKs/AMF/blob/master/amf/doc/AMF_Video_Encode_AV1_API.md));
  VideoToolbox low-latency mode saves up to 100 ms ([WWDC21](https://developer.apple.com/videos/play/wwdc2021/10158/)).
- **Capture**: DXGI duplication gives GPU surfaces plus dirty rects ([Microsoft](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/desktop-dup-api));
  per-app audio via process loopback, Windows build 20348+ ([Microsoft sample](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)).
- **Congestion control**: GCC (delay-based Kalman over-use detector plus loss)
  ([draft](https://datatracker.ietf.org/doc/html/draft-ietf-rmcat-gcc-02)) with
  transport-wide feedback ([draft](https://datatracker.ietf.org/doc/html/draft-holmer-rmcat-transport-wide-cc-extensions-01));
  SCReAMv2 adds L4S ([draft](https://datatracker.ietf.org/doc/html/draft-ietf-ccwg-rfc8298bis-screamv2-01));
  BBR lingers in probing for app-limited media ([draft](https://www.ietf.org/archive/id/draft-huitema-ccwg-bbr-realtime-00.html)).
  Recovery: NACK/PLI ([RFC 4585](https://datatracker.ietf.org/doc/rfc4585/)), RTX ([RFC 4588](https://datatracker.ietf.org/doc/rfc4588/)), FlexFEC ([RFC 8627](https://datatracker.ietf.org/doc/rfc8627/)).
- **Group calls**: VP8/H.264 only do temporal layers plus simulcast; AV1/VP9 add spatial
  SVC ([W3C](https://www.w3.org/TR/webrtc-svc/)); AV1's Dependency Descriptor lets an SFU
  drop layers without reading the payload, so it works under E2EE ([AV1 RTP spec](https://aomediacodec.github.io/av1-rtp-spec/)).

### Transport

- **QUIC datagrams** (RFC 9221): unreliable, congestion-controlled, no fragmentation
  ([RFC 9221](https://rfc-editor.org/rfc/rfc9221)). **RTP over QUIC** draft: one stream per
  frame avoids head-of-line blocking; resets drop stale media
  ([draft](https://datatracker.ietf.org/doc/html/draft-ietf-avtcore-rtp-over-quic)).
- **MoQ**: transport draft-22 (Oct 2026) still a working-group draft; 131 of 350 interop
  tests passed at IETF 126 ([recap](https://moqtap.com/blog/moqt-ietf-126-recap/));
  Cloudflare's relay in beta ([Cloudflare](https://blog.cloudflare.com/moq-relays/)).
  Not ready to bet a product on yet.
- **WebTransport** is in every major browser including Safari 26.4
  ([caniuse](https://caniuse.com/webtransport), [WebKit](https://webkit.org/blog/17862/webkit-features-for-safari-26-4/)),
  but you bring your own congestion control and audio processing
  ([webrtc.ventures](https://webrtc.ventures/2026/04/webtransport-is-now-baseline-what-it-means-for-real-time-media/)).
- **E2EE standards**: SFrame (RFC 9605, ~22 B overhead per frame, key management out of
  scope, [RFC](https://www.rfc-editor.org/rfc/rfc9605)); MLS (RFC 9420, forward and
  post-compromise secrecy, [RFC](https://www.rfc-editor.org/rfc/rfc9420)); Rust: openmls.

### Design recommendations from the research (priority order)

1. A p95 mouth-to-ear target of ≤150 ms (≤100 ms within a region); measure every stage.
2. Small audio device buffers (IAudioClient3, PipeWire quantum) instead of defaults.
3. Opus voice 20 ms VBR with FEC driven by loss reports; stream audio stereo 96–128 kbps;
   RED under loss.
4. Ship Opus decoder-side ML (deep PLC, NoLACE); put DRED behind version negotiation.
5. A NetEQ-style jitter buffer (95th percentile target, time-stretching).
6. AEC3 + AGC2 + RNNoise by default; DeepFilterNet3 as an opt-in mode.
7. Screen codecs: AV1 hardware → H.264 hardware → OpenH264 screen mode; free 1080p60.
8. Fastest encoder presets, ~1-frame VBV, no B-frames; recover with reference
   invalidation/LTR/intra refresh rather than keyframes.
9. Zero-copy GPU capture; skip encoding unchanged frames; process-loopback app audio;
   watch capture time (where TeamSpeak 6 struggles).
10. GCC-style delay + loss control; avoid nested controllers.
11. NACK/RTX on short round trips, FEC on long ones.
12. An SFU forwarding AV1 SVC layers; simulcast for H.264.
13. E2EE on by default with verification codes.
14. QUIC: audio in datagrams, one stream per video frame, a WebTransport web client
    later; skip MoQ until it's an RFC.
15. Self-host as one binary; avoid multi-service setups that break.

---

## 6. The Rust stack

Survey date 2026-10-10. Versions and dates from crates.io. **XC✅** = cross-compiled and
linked a test .exe for `x86_64-pc-windows-gnu`; **XCc** = compiled, not linked;
**SRC✗** = build script can't cross-compile as-is; **?** = untested.

| Area | Crate | Notes | Build |
|---|---|---|---|
| Audio I/O | [cpal](https://crates.io/crates/cpal) 0.18.2 | WASAPI, CoreAudio, ALSA; native PipeWire/PulseAudio hosts new in 0.18 (opt-in) | XC✅ |
| | [wasapi](https://crates.io/crates/wasapi) 0.25.0 | Per-process loopback | XCc |
| Opus | [opus](https://crates.io/crates/opus) 0.4.0 | Built on opusic-sys (libopus 1.6.1); FEC, DTX, loss % | cmake · XC✅ |
| | [opusic-c](https://crates.io/crates/opusic-c) 1.6.1 | Adds DRED, multistream | cmake · XC✅ |
| | [unsafe-libopus](https://crates.io/crates/unsafe-libopus) 0.2.0 | Pure-Rust c2rust port of 1.3.1, ~20% slower | XCc |
| Noise | [nnnoiseless](https://crates.io/crates/nnnoiseless) 0.5.2 | RNNoise port; use `default-features = false` | XC✅ |
| Echo, NS, AGC | [sonora](https://crates.io/crates/sonora) 0.2.0 | Pure-Rust WebRTC APM M145 (AEC3, NS, AGC2, HPF); passes upstream tests; MSRV 1.91 | XC✅ |
| | [webrtc-audio-processing](https://crates.io/crates/webrtc-audio-processing) 2.1.0 | C++ via meson | SRC✗ |
| Resampling | [rubato](https://crates.io/crates/rubato) 5.0.1 | API changed: `Fft`, `Async`, `Slip` | XC✅ |
| Screen capture | [windows-capture](https://crates.io/crates/windows-capture) 2.0.1 | WGC + DXGI; D3D11 texture, dirty regions, CPU buffer | XCc |
| | [xcap](https://crates.io/crates/xcap) 0.9.8 | Screenshots solid; video recorder WIP | XC✅ |
| | [screencapturekit](https://crates.io/crates/screencapturekit) 11.0.0 | macOS 13+, zero-copy, system audio | macOS |
| | [ashpd](https://crates.io/crates/ashpd) + [pipewire](https://crates.io/crates/pipewire) | Wayland portal ScreenCast, Camera, GlobalShortcuts | Linux |
| Webcam | [nokhwa](https://crates.io/crates/nokhwa) 0.10.11 | MSMF/V4L2/AVFoundation; sporadic maintenance | XC✅ |
| Video codec | [openh264](https://crates.io/crates/openh264) 0.9.8 | Builds from source without nasm (3× faster with it); `ScreenContentRealTime`, rate control, `force_intra_frame` | C++ · XC✅ |
| | [ffmpeg-next](https://crates.io/crates/ffmpeg-next) 9.0.0 | Needs FFmpeg libraries + libclang | UNVERIFIED |
| | [rav1e](https://crates.io/crates/rav1e) 0.8.1 | Fastest speed only "near real-time" | ? |
| Pixels | [yuv](https://crates.io/crates/yuv) 0.8.19, [fast_image_resize](https://crates.io/crates/fast_image_resize) 6.1.0 | SIMD color conversion and scaling | XC✅ |
| Transport | [quinn](https://crates.io/crates/quinn) 0.11.12 | RFC 9221 datagrams; ring or aws-lc-rs | XC✅ |
| | [str0m](https://crates.io/crates/str0m) 0.24.1 | Sans-IO WebRTC with SFU example, TWCC, simulcast | XC✅ |
| | [livekit](https://crates.io/crates/livekit) 0.9.4 | Prebuilt MSVC libwebrtc | SRC✗ |
| | MoQ ([repo](https://github.com/moq-dev/moq)) | moq-net, moq-tokio, hang; pre-1.0 | XC✅/XCc |
| GUI | [egui/eframe](https://crates.io/crates/egui) | 0.36 needs Rust 1.95; wgpu default since 0.34; per-tile textures | 0.33.3 XC✅ |
| | [slint](https://crates.io/crates/slint) 1.18.1 | GPL / royalty-free / commercial | 1.16.1 XC✅ |
| PTT | [global-hotkey](https://crates.io/crates/global-hotkey) 0.8.0 | Grabs the key (other apps don't see it) | XC✅ |
| | [device_query](https://crates.io/crates/device_query) 4.0.1 | Polls keys and mouse buttons without consuming them; X11 only on Linux | XC✅ |
| | [handy-keys](https://crates.io/crates/handy-keys) 0.3.4 | evdev, works on Wayland (needs udev rule) | ? |
| E2EE | [sframe](https://crates.io/crates/sframe) 2.0.0, [openmls](https://crates.io/crates/openmls) 0.9.1, [chacha20poly1305](https://crates.io/crates/chacha20poly1305) | RFC 9605, RFC 9420 | XC✅/XCc |

Reference apps: [moq-dev/moq](https://github.com/moq-dev/moq) (closest blueprint: cpal +
Opus + sonora, WGC/MF, SCK/VT, PipeWire/VAAPI with an OpenH264 fallback),
[iroh-live](https://github.com/n0-computer/iroh-live), [Zed](https://github.com/zed-industries/zed)
(LiveKit fork), [RustDesk](https://github.com/rustdesk/rustdesk), [Cap](https://github.com/CapSoftware/Cap).

Gotchas found while testing:
1. OpenH264 is C++, so a MinGW `.exe` imports `libstdc++-6.dll` unless you set
   `CXXSTDLIB_x86_64_pc_windows_gnu=static=stdc++` and add the GCC library path (verified).
2. Watch default features (nnnoiseless pulls CLI deps; str0m pulls a web server) and the
   rustls/quinn crypto-provider mismatch (rustls defaults to aws-lc-rs, quinn to ring).
3. MSRVs: egui 0.36 needs 1.95, slint 1.92, sonora/openmls 1.91.

---

## 7. Naming

Brief: warm / campfire, a name that isn't taken. Checked 2026-10-11 with RDAP lookups
(each method tested first against names known to exist and names known to be free),
GitHub profile pages, crates.io and US trademark search (Trademarkia).

| # | Name | Why | .com | .app | .chat | GitHub | crates.io | Conflicts | Risk |
|---|---|---|---|---|---|---|---|---|---|
| 1 | **Ingle** | Scots/archaic for the hearth fire; sounds like *mingle* | taken; inglechat.com free | listed for sale ([ingle.app](https://ingle.app)) | free | `ingle` taken (person), `inglechat` free | free | Canadian travel insurer ([source](https://www.insurancebusinessmag.com/ca/news/breaking-news/ingle-international-rolls-out-travel-insurance-platform-64585.aspx)); a pulled dating app ([AppBrain](https://www.appbrain.com/app/ingle-meet-match-monetize/com.ingle.app)); all 15 US INGLE marks dead ([Trademarkia](https://www.trademarkia.com/search/trademarks?query=ingle)) | Low |
| 2 | **Firefolk** | "the people around the fire" | taken; firefolkchat.com free | taken (Mar 2026, unknown) | free (.dev free) | `firefolkchat` free | free | an art gallery, a band, a fan D&D race; 0 US marks ([Trademarkia](https://www.trademarkia.com/search/trademarks?query=firefolk)) | Low |
| 3 | **Irori** | Japanese sunken hearth people gather round | taken | pending delete | free | taken | free | Irori Inc., Tokyo startup ([source](https://www.vcbacked.co/company/irori)) | Low–med |
| 4 | **Ingleside** | "the fireside" | taken | taken | free | taken | free | marks in senior housing, finance, wine | Low |
| 5 | **Hobfire** | coinage: the hob beside the hearth; a hob is a hearth spirit | **free** | **free** | free | free | free | none found; 0 US marks | Low (less obvious) |
| 6 | **Bothy** | Scottish open hut where strangers share the stove; mirrors self-hosting | taken | taken | free | taken | free | GIN BOTHY | Low |
| 7 | **Kokko** | Finnish midsummer bonfire | taken | taken | free | taken | free | "Kokko Party" chat app (pulled 2025) | Low–med |
| 8 | **Firelit** | "lit by fire" | taken (Firelit LLC) | a live "Firelit" macOS app | free | taken | free | that app; close to Firelight/FMOD | Med |

Ruled out: Emberhall (a game renamed to it on 2026-10-09), Hearthline (a game in beta),
Campwire (a course platform; close to 37signals' Campfire), Firenook (a Rust project),
Noroshi (a Japanese messaging app), Inglenook (live marks), Hearthkin (registered 2026),
Fogata (all taken).

Caveats: "free" means an RDAP 404 (likely available), not a price check; `.gg` couldn't be
checked; GitHub API lookups were blocked, so profile pages were used; trademark search was
US federal only and loose. **This is not legal clearance.**

Recommendation: **Ingle** (short, warm, a real word for the hearth, ownable), with
**Firefolk** as runner-up.

---

## 8. What Ingle took from this

| Finding | What Ingle does now | Later |
|---|---|---|
| Free 720p30 cap is Discord's sorest paywall | 720p30 → 1080p60 / 1440p30 presets, free | AV1, hardware encode, 4K |
| One slow viewer drags everyone down | Per-viewer queues; frames dropped and keyframes requested for that viewer only | Simulcast / SVC layers |
| Stream audio must not echo the call | Process loopback that excludes Ingle's own process; window shares capture just that app | Linux (PipeWire), macOS (SCK) |
| Voice quality paywalled at 96 kbps | Up to 192 kbps for everyone; 64 kbps default | RED, DRED, NoLACE |
| Krisp turns off under load | RNNoise by default, and it never switches itself off; plus WebRTC AEC3/AGC2 via sonora | DeepFilterNet as an option |
| E2EE is now expected (DAVE) | Always-on E2EE for voice, video and stream sound; key rotation on join/leave; call code | MLS for large calls |
| DAVE's documented weaknesses | ChaCha20-Poly1305 with full 16-byte tags; header authenticated except the server-written session id | SFrame interop |
| Electron bloat | Native Rust, egui, one ~28 MB exe | GPU texture path |
| ID checks and breaches | No accounts: a key pair is the identity; servers never see IDs or emails | |
| Self-hosting is painful elsewhere (Element) | One server binary, one UDP port, links with pinned certificates | Docker image, systemd unit |
| QUIC datagrams + stream-per-frame is where the field is heading (Fluxer, MoQ, RTP-over-QUIC) | Exactly this, over quinn | WebTransport web client |
| PTT on Wayland is broken in Electron | Polling PTT on Windows/X11 that doesn't steal the key | XDG GlobalShortcuts portal |

---

## 9. Hosting: which server to rent

The owner of a community needs a server that is always on and reachable on one UDP port.
For most people that means renting a small VPS, so the app can rent one for them.
Requirements are tiny (the server is one static program using a few tens of MB of
memory); what matters is price, reliability, an easy sign-up and an API.

Small plans, checked October 2026 (prices change; the app shows live prices from the
provider's API before anything is bought):

| Provider | Cheapest useful plan | Notes |
|---|---|---|
| **DigitalOcean** | $4/mo: 512 MiB, 1 vCPU, 10 GiB disk, 500 GiB transfer. $6/mo: 1 GiB, 25 GiB, 1,000 GiB ([pricing](https://www.digitalocean.com/pricing/droplets)) | Card or PayPal sign-up; regions in North America, Europe, Asia and Australia; simple REST API with scoped tokens |
| Vultr | $5/mo: 1 GB, 25 GB, 1 TB; cheaper $2.50/$3.50 plans are limited or IPv6-only ([pricing](https://www.vultr.com/pricing/)) | IPv6-only would leave many home connections out |
| Akamai (Linode) | Nanode $5/mo: 1 GB, 1 TB ([comparison](https://kimmo.suominen.com/blog/2026/08/august-2026-cloud-pricing-comparison/)) | |
| Hetzner | CX23 €5.49/mo + €0.50 for an IPv4 address after the June 15, 2026 price rise ([article](https://privatedevops.com/news/hetzner-june-2026-cloud-price-increase-what-to-do)) | The most memory per euro, in Europe |

The brief was "cheap and most stable", so the app rents from **DigitalOcean**: the $6
plan is recommended (room to spare) and the $4 plan offered for small groups. Transfer is
what a busy group uses up first: voice is small (five people talking send about 0.6 GB an
hour in total through the server), but a 1080p60 screen share (~6 Mbps) watched by three
people sends about 8 GB an hour. The $6 plan's 1 TB a month covers around four hours of
that every day; the $4 plan's 500 GB about half as much.

Any other provider works through **Use a server I already have**, which logs in over SSH.

How the rental works, from DigitalOcean's API documentation:

- A personal access token is made in the control panel (API → Generate New Token) with
  Full Access or custom scopes ([how](https://docs.digitalocean.com/reference/api/create-personal-access-token/),
  [scopes](https://docs.digitalocean.com/reference/api/scopes/)). Creating droplets
  (`droplet:create`) also needs `droplet:read`, `regions:read`, `sizes:read`,
  `actions:read` and `image:read`; tagging needs `tag:create`, so Ingle doesn't tag
  ([droplet:create](https://docs.digitalocean.com/reference/api/scopes/droplet/create/)).
- `POST /v2/account/keys` adds Ingle's SSH public key; `POST /v2/droplets` with
  `name`, `region`, `size`, `image` (`ubuntu-24-04-x64`) and `ssh_keys` creates the
  server; `GET /v2/droplets/{id}` reports `status` (`new` → `active`) and
  `networks.v4[].ip_address` with `type: public`; `DELETE /v2/droplets/{id}` removes it
  ([droplets](https://docs.digitalocean.com/reference/api/reference/droplets/),
  [SSH keys](https://docs.digitalocean.com/reference/api/reference/ssh-keys/)).
- After that it's a normal SSH install, the same as for a server you already have.

What couldn't be checked here: the live DigitalOcean API (this research sandbox can't
reach it), so the app's DigitalOcean code was tested against a stand-in that follows the
documented requests and responses, with a real Ubuntu 24.04 system playing the droplet.
