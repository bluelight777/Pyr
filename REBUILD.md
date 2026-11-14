# Rebuilding Pyr 0.13.1 to 0.13.28

On November 10, 2026 the cloud workspace that held Pyr's only copy of the source from after 0.13.0
was wiped, with the release key. This repository starts again from 0.13.0 (GitHub,
`bluelight777/pyr`, tag v0.13.0) and rebuilds what 0.13.1 to 0.13.28 did, then adds what's new.
Nothing is released until the rebuild does everything 0.13.28 did: everyone installs the first
rebuilt version by hand once (it carries a new release key), and nobody should lose a feature.

## What we go by

- Every release's notes: `rebuild/notes-0.13.x.txt` (from each release's `latest.json`).
- The 0.13.28 programs (the Linux `.deb`: `pyr`, `pyr-server`, `pyr-mcp`, `pyr-bot`): run them
  beside the rebuild and compare screens and messages; their strings give the exact wording and
  the protocol's names (`rebuild/strings-*.txt`).
- The art: embedded in the 0.13.28 app (carved out into `rebuild/assets-0.13.28/`), and Blue's
  `meshy-mcp\pyr-casino` folder.
- The sounds: all 213 of 0.13.28's (INGS files: the soundboard redrawn, the ten ringtones, the
  casino's `c_*`, and each slot machine's band `s_<machine>_*`), carved out of the app into
  `rebuild/sounds-0.13.28/` and named from the app's own tables (the pointer next to each).
- The plugin kit and Library as 0.13.28 published them (`pyr-upload-0.13.28`).
- This session's transcript for 0.13.27 (programs update in one click) and 0.13.28 (AFK).

## Ground rules

- **Talk to 0.13.28 as it did.** Same feature names, message names and fields, so a rebuilt app
  works with a 0.13.28 server and the other way round. Check against the 0.13.28 binaries.
- **The echo.** Voice isolation and echo cancellation stay as 0.13.23 / 0.13.25 had them: with
  isolation, echo cancellation stays on, headphones or not. Twice a fix for "my voice went up and
  down" (0.13.11, 0.13.24) let the others' voices back through for people on speakers. Nothing
  ships that changes what the echo canceller does without Pyr's own echo test, speakers into the
  microphone, A/B against 0.13.28.
- **Voice and screen sharing: identical to 0.13.28** (Blue, while the rebuild was under way:
  "please make it identical to what it is now"). What the released rebuild does by itself in a
  call and in a share is what 0.13.28 does: its numbers and settings read out of its program,
  calls tried both ways (rebuilt with 0.13.28). A fix that changes the voice path (the
  headphones one for reports 657 to 675, say) stays in behind a switch and off, until Blue asks
  for it after the echo test. What can't be shown in the container (real speakers into a real
  microphone, a graphics card's encoder) is tried on Blue's PC, A/B against 0.13.28, before
  release; whatever can't be shown to match is said, not shipped.
- **Back up every session:** a git bundle to `Downloads\pyr-source` on Blue's PC and to the
  outputs folder. Keys live in `Downloads\pyr-keys` (never in the repository).

## The key change

- New release key (`pyr-release-key-2.txt`). 0.13.28 apps keep reading `latest.json` and
  `library/index3.json` (old key, can't change); rebuilt apps read new names signed with the new
  key (`latest2.json`, `library/index4.json`). The Keeper keeps its key; the new release key
  certifies the same one, and the Keeper writes both lists.
- The rebuilt server tells apps older than the rebuild, once, that a new Pyr is out and where
  to get it (pyr.gg), since their Update button can't offer it.

Done (2026-11-14): the app's `RELEASE_KEY` is the new key's public half and it reads
`latest2.json` and `library/index4.json` (`index2.json` before the Keeper's is up); it says
`release_key_2` in its login, and a server tells an app without it, three seconds after it logs
in, "There's a new Pyr, and this one can't update to it by itself: get it from pyr.gg." (an
error with no request: shown as a refusal, never reported). The Keeper, given `release_key2`
and `cert2_file`, keeps a second list (`library2` in its state, beside 0.13.28's) and puts up
`index4.json` with it: the same add-ons and skins, and programs as the new key endorses them;
`index3.json` goes on as 0.13.28's apps believe it (programs as the key before endorsed them;
the new key's are left out of it). Lists signed with either key are taken in. Run end to end
with test keys (the Keeper binary, the library tool, a 0.13.28 Keeper's state). The release
scripts write `latest2.json` (never `latest.json` again) and the page and `publish.ps1` read it.
The notice seen on a 0.13.28 app (Olive, on the rebuilt server): its error card, "Something
went wrong" over the words, with Copy and Dismiss, until dismissed; again at its next login.
Still to do: the swap itself.

On the day (keys by path, never printed):
1. `library endorse --key pyr-release-key-2.txt --trust J0EGQDSoDplSzp3aFrK7EA8C5Of4w4D40kaO-g7iJcE
   --in index3.json --out index-key2.json` (the live list, its programs endorsed again with the
   new key);
2. `library certify --key pyr-release-key-2.txt --keeper <the Keeper's key, the "key" in its
   cert.json on the VPS> --out keeper-cert-2.json`, onto the VPS beside the Keeper's cert;
3. the Keeper's config gains `"release_key2": "O_eZ7ZYewM395zMS-dFgHOFypniuEnfr02SCJAqtwJY"`
   and `"cert2_file"`; the rebuilt `pyr-keeper` in place of 0.13.28's; `pyr-keeper import
   index-key2.json` puts up `index4.json`;
4. `KEY=pyr-release-key-2.txt scripts/make-library.sh` and `scripts/make-release.sh` as ever.

## Moving the VPS's server to the rebuild (nothing lost)

The rebuilt server opens the VPS's own database, written by 0.13.28. Whatever 0.13.28 kept
that the rebuild doesn't know yet must come through untouched:
- [x] A casino record: all 34 of 0.13.28's fields known now (and anything else kept in `extra`); one
      this Pyr can't read is never written over (it plays with a new one meanwhile, said in the
      log); one from before levels keeps its level on the new curve.
- [x] Every other JSON the server keeps carries what it doesn't know along (`#[serde(flatten)]
      extra`), so writing it back loses nothing: the casino chat's lines, each game's numbers by
      the hour, day and all time, the big wins, the staff log, the Bonfire (the fire burning
      and its draws), the bots' ratings (a bot rated again keeps what was kept with it). Stored
      JSON this Pyr can't read at all is never written over: it's kept as it was under
      `unread:<key>` in the meta table (said in the log, once), and that one starts afresh.
- [x] Before the swap, every name 0.13.28 stores is one the rebuild knows (from its server's
      serde lists, `rebuild/tools/fields.py`, read again off its binary on 2026-11-14): bot
      levels `rookie regular hard shark genius`, games `slots blackjack roulette baccarat craps
      dice keno reels dragon plinko domino`, Bonfire prizes `grand blaze spark`, staff
      `game_master ambassador`: each read and written back the same, in 0.13.28's order (a
      test). A list holding a name this Pyr can't read is still kept whole under `unread:`.
- [x] Tables 0.13.28 added: none. Its schema is 0.13.0's, every table and index (compared in
      the test copy); the rebuild adds its props tables, which 0.13.28 leaves alone.
- [x] Tried: a copy of the 0.13.28 test server's database (seven casino records with 0.13.28's
      34 fields, 27 meta keys: the Bonfire, the staff log, who's let in the control room, the
      Friday pool, Plinko's numbers...) run by the rebuilt server, 0.13.28's app on it: the card,
      missions, Ember Back, Bonfire and jackpot as they were; daily chips claimed, dice and Plinko
      played (every round checked by 0.13.28's app). Then 0.13.28's server on what the rebuilt
      one wrote, 0.13.28's app on it: everything as left (103,676 chips, 2 XP to level 2, the
      dice mission at 6 of 40, the Bonfire at 1,000,930 with Carol's 42 embers), and played
      again. No meta key, record, field, table or index lost either way (`work/ui/swap.sh`,
      `dbdump.py`, `dbdiff.py`).
- [x] Short links keep working through the swap: Brethren of Blood's own code
      (`https://pyr.gg/brethren`, open on November 11, 2026) and its listed invites stay at
      pyr.gg. The rebuilt server announces with the same protocol as 0.13.28's (`pyr directory
      announce v1`, `api/v1/announce`, signed with the server's own TLS key, which the swap
      keeps; the release key plays no part) and keeps the community's code from 0.13.28's
      database. pyr.gg itself (0.13.28's directory and site) stays as it is. Tried with a
      local directory (`pyr-directory`) and a copy of the 0.13.28 test server listed there:
      its owner gave it its own code (`campfire`) and Carol made an invite, both listed by
      0.13.28's server; the rebuilt server swapped in on the same data announced the same
      two codes (same certificate, host and port), and someone new joined it with
      `campfire`.

## Checklist

Version in brackets: where 0.13.x first had it.

### Server, protocol and chat
- [x] Pictures that arrive before their key keep them (0.13.4)
- [x] AFK channel: picked in Server settings, 5/15/30/60/120 min, a channel called AFK by
      default (0.13.4); never while sharing or watching a screen, an hour by default (0.13.25);
      away vs Idle by hand, from joining the channel, the app checks before going (0.13.28)
- [x] Invite someone to your voice channel (card with Join) (0.13.6)
- [x] Taskbar badge with mentions and DMs, 9+ (0.13.7)
- [x] Files in DMs and groups: up to 2 GB, E2EE, resumable, kept 7 days, 4 GB a week (0.13.13)
- [x] New DM said "isn't end-to-end encrypted yet" (0.13.13): a bug of 0.13.1 to 0.13.12 the
      rebuild never had (0.13.0 checks the server's sealed feature; its words are 0.13.28's, both
      of them, and no "yet" anywhere). 0.13.28's intro on screen, for the record:
      `rebuild/ref28/dm-call-intro-0.13.28.png`.
- [x] Plugins sent in with big files: parts (0.13.14)
- [x] Game invites to friends from programs (0.13.15/16), as links pyr.gg/<game>/<code> (0.13.18)

### Voice, video, sounds
- [~] Voice isolation takes the PC's own sound out of the microphone (0.13.2); headphones dips
      (0.13.11), its echo (0.13.12); double-talk on speakers (0.13.19); 0.13.24's try taken out
      (0.13.25). 0.13.28's voice is 0.13.23's: the PC's own sound to the echo canceller (0.13.2),
      the canceller on with isolation, headphones or not (0.13.12 took 0.13.11's change back
      out), 0.13.19's double-talk tuning; 0.13.24's "dipguard" taken out again (PYR_AB=dipguard
      does nothing, as in 0.13.28). Rebuilt (5b62cc1, 92bf1df; the details under reports 657 to
      675). Left: Pyr's own echo test on Blue's PC, A/B against 0.13.28.
- [x] Headset that drops out and comes back isn't an error (0.13.2): the call screen says what's
      going on at once (as before), but "Sound problem in the call: …" (an error, so reported
      by itself) only comes if the same trouble lasts 15 s; a device back before then says
      nothing. (0.13.28's words for it are the same; how long it waits couldn't be read out of
      its program: 15 s is the rebuild's.) 0.13.28's call bar has no "Audio problem:" line:
      see what it shows with the call bar (screen share items).
- [x] Streamer mode only when turned on (0.13.2): sharing the screen no longer turns it on by
      itself; 0.13.28's words in Settings, the casino's Streaming menu and its button ("It's on
      only when you turn it on.", no "(hold Space)" there). The poker table's peek line is
      "Streamer mode · hold Space to peek" (0.13.28: "hold {key}", with the keys for every game).
- [x] Sound profiles Classic / Ember; ten new ringtones; soundboard redrawn (0.13.2/3). Settings →
      Notifications → Sound profile (Classic first, everyone starts with it; saved as 0.13.28
      saves it: `sound_profile` "" or "ember"), its words and place on the page pixel for pixel.
      Ember's fifteen sounds are what 0.13.28 played, recorded off it sample for sample
      (`work/ui/alsashim.c`: a stand-in sound card that keeps time and writes down what's
      played; each set off on 0.13.28 with a second Pyr: a message, joining and leaving, someone
      joining and leaving, sharing, watching, someone watching mine, being called and calling,
      the mute shortcut), kept as 24-bit FLAC (`crates/engine/assets/ember`, 1.5 MB). Recorded
      the same way, 0.13.28's Classic message is the rebuild's chime to the sample, and the
      rebuilt Pyr's Test plays Ember's and Classic's messages as 0.13.28 does (within 2^-23).
      The ten ringtones (Sunrise, Kalimba, Music box, Island, Arcade, Night drive, Lo-fi,
      Lounge, Old phone, Daydream) and the redrawn soundboard are 0.13.28's own files (carved
      out of it, every frame checked); a ringtone is saved as 0.13.28 saves it (`ring-kalimba`,
      its name and emoji), the menu is 0.13.28's (RINGTONES between Pyr ring and SOUNDS), and
      Kalimba plays as 0.13.28 plays it, to the sample. A skin's own sounds play in place of
      either profile's.
- [x] Join chime in Classic climbs and rings (0.13.23): someone else joining my channel; me
      joining keeps the old chime. Its notes, read out of 0.13.28's program (the table just
      before Join's; Hz, start ms, length ms, loudness): (523.3, 0, 120, 0.60), (659.3, 70, 120,
      0.60), (784.0, 140, 120, 0.65), then the chord at 210 ms for 560 ms: (1046.5, 0.70),
      (659.3, 0.25), (784.0, 0.25). Every other chime's table is the rebuild's, unchanged (the
      renderer can't be read out of the program; 0.13.0's is kept). Skins' sound names as
      0.13.28's: `you_join`, `you_leave`, `viewer_join`, `viewer_leave`, `mute`, `unmute` (a
      skin without one plays its `join`, `leave`, `watch_start`, `watch_stop`); someone else
      tuning in to a stream in the call plays `viewer_join`.
- [x] Mute shortcut: any key combo, then any single key (0.13.20/22); soundboard keys the same.
      As 0.13.28 has it, tried key by key on it: Settings → Voice & Video → Mute shortcut (Set a
      shortcut, "Press the keys…", the keys as caps with Change and Remove), refused on Copy,
      Cut, Paste, Select all, Undo, Redo, Save, Find (Ctrl with the key alone), Switch windows
      and Close the window (Alt + Tab, Alt + F4), and on the push-to-talk key in Push to Talk;
      Esc stops it (not with Ctrl: Ctrl + Esc is a shortcut). A key on its own is said under it
      and just types in Pyr's own boxes (anywhere else it goes off). It mutes from anywhere,
      as the microphone button does (a moderator's mute stays, and says so), with 0.13.28's
      step down or up (its tables, after Ringback's: muted (523.3, 0, 90, 0.45), (392.0, 60,
      170, 0.45); unmuted (392.0, 0, 90, 0.45), (523.3, 60, 170, 0.45)), and the log line
      "the mute shortcut muted=…". A longer combination holds a shorter one inside it (Ctrl +
      M doesn't go off with Ctrl + Shift + M); held keys otherwise don't matter (Shift + M sets
      off M, as on 0.13.28). Soundboard keys: the same refusals, "is your mute shortcut", a key
      on its own allowed with 0.13.28's words; a mute shortcut taking a soundboard key says
      so. Saved as 0.13.28 saves it (`mute_key`), and settings this Pyr doesn't know yet are
      kept as they are (0.13.28's `casino_keys`, `games_volume`, `sound_profile`,
      `poker_auto_top_up`): a 0.13.28 config.json read and written back loses nothing.
- [x] Share picker redesign (0.13.14), written from 0.13.28's own code (its heading, tile, line,
      preview and Refresh functions read out of its program) and checked against its screen
      pixel for pixel: 1600 wide at 700, 800, 900 and 1000 tall; one, two and five screens
      (one screen alone takes half the width, 270 at most; as many across as fit 160 each, three
      at most); a window picked; a tile, a line and Refresh (with its box and hint) pointed at.
      Only the preview's picture differs (the screens behind differ). Its icons (monitor,
      window, test bars) are 0.13.28's, in proportion to their square. Made-up screens and
      windows in any build (PYR_FAKE_SCREENS / PYR_FAKE_WINDOWS; screens named "Screen 2 —
      XB273K GP" as 0.13.28 names them, the tile showing the number from the name); Windows
      lists screens by their own names ("Display 2" when none) and leaves out windows it keeps
      hidden (cloaked).
- [x] Labels in dialogs easier to read (0.13.14): `theme::readable` (WCAG contrast; towards TEXT a
      tenth at a time) at the seven places 0.13.28 has it: field labels (6:1 on SIDE; and 12
      high, spaced 0.6, as 0.13.28), the picker's subtitle and Refresh (5:1), a line's size and
      icon and a tile's monitor and size (4.5:1), and the picker's note (DIM on SIDE, 4.5:1: in
      Campfire the only one that changes, to 148,136,127). Checked against 0.13.28's Invite
      dialog (EXPIRE AFTER, MAX NUMBER OF USES) pixel for pixel.
- [x] Still screens keep their quality (0.13.21): RateControl's app-limited test (under a quarter
      of the target or of the most that got through lately, a megabit at least, over the round
      trip and a quarter second, and in what's on its way), then no queue cut and a viewer's
      only for frames skipped or a lot lost; cuts while app-limited from where it was. High
      profile (CABAC) after the encoder starts, and Catmull-Rom scaling for text, as 0.13.28
      (PYR_AB=cabac / sharp for the old). Tests: a still moment on a busy road isn't a reason to
      cut; a struggling viewer of a busy share still backs off.
- [x] A share keeps its size and lowers its frame rate instead (0.13.22): the ladder is the
      preset's size at 6/6, 5/6, 4/6 and 3/6 of its frame rate (PYR_AB=ladder for the old one).
- [x] 60 fps shares keep their 60 (0.13.26): a frame up to a quarter of a frame early is taken,
      the wait for the next is up to then (1 to 40 ms), and Windows is asked for frames twice as
      often as the frame rate.
- [x] The call's stats say what really goes out (0.13.26): "sending 1080p 57 fps · 6.0 Mbps ·
      processor encoder", the frames sent in the last second or so, " (screen still)" when only
      the last picture went out again, and the encoder (NVIDIA, AMD, Intel, the card's, or the
      processor's for a screen share). Same line as 0.13.28's, side by side with a test picture
      shared (both 49 to 59 fps here); the "screen still" case by a test (no source here stops
      sending).
- [x] Watching a share costs less (0.13.26): each decoded frame goes up as it is
      (from_rgba_premultiplied: a decoded frame is opaque), not looked at pixel by pixel.
- [~] On the graphics card end to end (0.13.26, Windows): rebuilt (video/hw.rs, wired into the
      capture and the publisher; what 0.13.28 does is in rebuild/notes-hw.md). The capture keeps
      copies on the card (three textures, frames taken twice as often), the card's video engine
      scales them and makes them NV12 (eight textures in turn), Media Foundation's hardware
      H.264 encoder of the capture's card encodes them (asynchronous ones by their events; a
      keyframe asked for, or the encoder made anew for each where it won't); the card's priority
      raised while sharing; small copies read back without waiting for the preview and the
      content watch; the processor's way (read back, OpenH264) from the first failure, and the
      card again with a new graphics device. PYR_AB=hwenc for the processor's way always. Builds
      for Windows; can't run here. To do on Blue's PC, A/B against 0.13.28: a share's stats
      line names the card's encoder (NVIDIA, AMD or Intel), the picture and colours look the
      same, keyframes come when a viewer joins, the log has "encoding on the graphics card",
      and PYR_AB=hwenc gives "processor encoder".
- [x] Automatic reports (voice went up and down, stopped responding, sound problems): 0.13.0
      has the swells (dips) detector and the watchdog; what 0.13.x added, by its notes: a
      headset dropping out and back isn't an error (0.13.2, above); the voice report in 0.13.28's
      words (657 to 675, below); stopped responding (668 to 670, below); and a table's "no" (not
      enough chips, more than it takes, the dice already rolling) shown as before but no longer
      sent in by itself (0.13.24: it was a reported error toast, now a refusal).

### The approved bug reports (657 to 675)
- [x] 668, 670 (stopped responding): false. 668 was the update's installer being checked on the
      window's thread just before quitting (now: quitting is marked first; a freeze the window
      came out of by quitting isn't reported); 670 was the window being opened again after the
      graphics card went away (not watched meanwhile, not reported).
- [ ] 669 (stopped responding): the window's thread at 60 to 100% for ten minutes, the PC's
      memory 92 to 94% in use. Now: the log says what the window was showing and at how many
      frames a second; casino pictures not drawn for 90 s are let go. Look again when it comes
      back with that.
- [ ] 657 to 667, 671 to 675 (voice down and back up, voice isolation, headphones): all with
      isolation, echo cancellation on, automatic gain on; 0.13.11's note says it's while
      something plays on the PC (a game's music) with headphones. Plan, keeping what the echo
      canceller does with others' voices as it is:
      1. pc_sound (0.13.2, WASAPI process loopback, thread `pyr-pc-sound`, `PYR_AB=pcsound`):
         the PC's own sound goes to the echo canceller only while the speakers can be heard in
         the microphone (`mic_echo`: found, or not known yet). With headphones it can't come
         back, so it isn't there to turn the voice down by.
      2. 0.13.19's `aechfcap` (double-talk on speakers): get its numbers from the 0.13.28
         binary (`tune_aec3`'s stores; field offsets from `offset_of!` on sonora's config built
         with the same compiler) rather than guess.
      3. `examples/aec_eval.rs` before and after (headphones and speakers, others talking and
         music, with the PC's sound), then Pyr's own echo test on Blue's PC: speakers into
         the microphone, A/B against 0.13.28, before anything ships.
      Done (1 and 2; 3 in the container). `aechfcap` is `bands_in_limiting_gain = 0` (0.13.28's
      `tune_aec3` at 0x1510860: aecsat, aectune, gains, aecmask, then the store to 0x1c4).
      `aec_eval`, voice isolation, others talking, a game on the PC at -10 dBFS, your voice at
      -40 dBFS (a quiet headset microphone): headphones, as 0.13.28 (`PYR_AB=pcgate`): 7 of
      133 stretches of your talking turned down, and "my voice went down and back up for the
      others 3 times within 60 s of talking (by up to 31 dB), turned down by the echo
      cancellation and automatic gain (... headphones ...)", the reports' own words; the
      rebuild: 3 of 133, all in the first 2 s (the canceller starting; both have them), none
      said. Speakers: the same either way (the PC's sound goes to the canceller; echo
      through -99 dB). Still to do: Pyr's own echo test on Blue's PC, A/B against 0.13.28. **The released rebuild
      does as 0.13.28 does** (Blue's ground rule for voice): the PC's own sound always goes to the
      echo canceller, and the fix is held back behind `PYR_FIX=pcgate` (`pyr_proto::ab::fix`, off
      unless named) until the echo test on Blue's PC and he asks for it. Checked with `aec_eval`
      (voice isolation, headphones, others talking, a game at -10 dBFS, your voice at -40 dBFS):
      as it starts, 7 of 133 turned down and 0.13.28's report word for word, as 0.13.28; with
      `PYR_FIX=pcgate`, 3 of 133 and no report.

### Library and programs
- [x] Plugins with art: up to 100 MB and 5,000 files, any Rust (0.13.6). 0.13.28's checks run
      side by side (its pyr-mcp's check_plugin on the same folders): 101 MB, 5,000 files, built
      files and other languages refused in its words, "(100 MB at most)", its tool and guide
      texts and the Make your own page word for word. Its client sends a big file in pieces of
      640,000 bytes one after another under its path (seen in the rebuilt server's frame log);
      the rebuilt server puts them together and stores exactly what 0.13.28's stores (every
      column, the same plugin from 0.13.28's pyr-mcp to each server). Fixed (0.13.28's bug): with
      a couple of thousand files its first frame went over 1 MiB with the list of files in it,
      and the plugin never arrived (4,900 sprites: refused by both servers); the rebuild keeps
      room for the list, and 0.13.28's server takes it. Owners looking at thousands of files:
      each reply fits a frame (room kept for the list of the rest), and the parts of big files
      come from the submission kept, not read again for each.
- [ ] Programs open inside Pyr, stay in their page, Opening..., NVIDIA helper windows
      (0.13.7 to 0.13.10, 0.13.17, 0.13.18). In: a few frames a second while one's on show
      (0.13.7) and "Opening {name}…" (as 0.13.0 and 0.13.28 say it); a program's windows
      watched (shown, hidden, moved: Pyr steps at once, and puts one that moved itself back);
      its windows looked at every step: one showing outside Pyr is taken in as it starts after
      a hidden first one, or once the one in its page was hidden (a browser's second window
      stays its own); taken in out of sight and shown only in its place; helper windows skipped
      by 0.13.28's list of their classes (wgpu, GLFW, SDL_app_hidden, OLE, IME, MSCTFIME,
      NVOpenGLPbuffer, GDI+, Cicero, DDEML); pyr.log says which window, in 0.13.28's words
      ("taking a program's window in", "a program shows a window of its own outside Pyr: taking
      that one in", "a program's window closed: looking for its next one"). Windows only:
      **to A/B on Blue's PC** (Torch & Conquer, Pyropoly, Chimbrowser against 0.13.28).
- [x] Programs know your Pyr name and friends; PYR_BRIDGE; invite friends (0.13.15)
- [x] Library shows plugins added since setup (0.13.7); Dominó program removed and never listed
      (0.13.12, 0.13.25). Pyr: `domino-cubano` taken off the PC as Pyr starts (its files, data
      and line, "plugins: a program Pyr took over was removed", as 0.13.28 does it, run side by
      side) and never shown whatever list it's given. The Keeper, run side by side with
      0.13.28's (`pyr-keeper-0.13.28` and the rebuilt one on the same test lists, keys and
      server database, nine steps: first run, `once`, `unlist`, `import`s): a list taken in is
      merged with what's up (what it adds, newer versions in place, never an older one; what's
      up stays), what's taken off is kept in `state.json`'s `unlisted` (`"skin:<id>"` for
      skins, written even when empty) and stays off, and a program Pyr does itself goes when
      the list is next put up ("programs Pyr does itself now taken off the list"): the same
      lists, the same log lines, the same state.
- [x] One-click update of open programs, programs can ask, invites update first, list fetched
      again, every ten minutes (0.13.27)

### Pyr Casino

How (decided while starting it):
- The real 0.13.28 runs in the workspace (`work/ui/up28.sh`: its own server and app on a
  screen of their own): every screen is matched against it, side by side. Its art and sounds
  are carved out under their own names (`rebuild/assets-0.13.28`, `rebuild/sounds-0.13.28`).
- Measuring text off 0.13.28's screen: egui sizes a font by its em (epaint's `px_scale_factor` is
  size / units per em), so a font's cap height on screen is size × capHeight / unitsPerEm (Inter
  0.7275, Anton 0.8594, Bebas 0.7). `rebuild/tools/fitsize.py` finds the size from an ink width,
  `eguitext.py` lays text out as egui does (glyphs on whole pixels with 4 sub-pixel bins, its
  "…" cutting), `tilefont.py` tells a display font by its shapes.
- A casino game's round is checked by everyone's own Pyr, replaying it from the seeds with the
  same code. A rebuilt game can't replay 0.13.28's rounds bit for bit without its source, so a
  rebuilt game doesn't take a 0.13.28 feature name it can't honor exactly: it gets the rebuild's
  own (and a 0.13.28 app on a rebuilt server plays the games from before it, which 0.13.x apps
  already do on older servers). Same look, same rules, same 99% (checked by simulation, as
  0.13.x did); new seeds and code.

- [x] Reel Row Remastered: twelve machines with features (tumbles and bombs, Hold & Win,
      wilds that grow/freeze/walk/stay), ante, Play Feature, slowing reels, splashes, plaques,
      a sound band per machine, 99% checked by simulation (0.13.2). Under the rebuild's own
      feature name (`reel_row_remastered`, `Bet::Remaster`): 0.13.28's rules, points, prices,
      odds and words as its Info pages and Play Feature show them (rebuild/notes-reelrow.md),
      new code and seeds. Every way to play pays 99%: the wild machines worked out exactly
      (99.00 ±0.02%, Bonus Chance with each machine's own odds, still 1 in 45), the tumbling and
      Hold & Win machines played twenty million times a way, tuned and then checked on other
      rounds: each within 0.2% of 99%, most within 0.1% (what's left is the runs' own noise:
      Hold & Win's Grand comes once in a quarter of a million spins). Each family filmed beside 0.13.28 and matched: the reels' timing
      (each reel's stop to a tenth of a second), tumbles (the winners aglow, a ring of lights
      flying off, the drop), bombs and the multiplier badge, Hold & Win (its splash over the
      page, the jackpot plaques across the top, RESPINS and the total in the machine's
      lettering, coins' worth on a dark pill), Hot Wilds and Espresso Shots landing where they
      landed and filling from the reel's foot, Frozen respins, the Spotlight's walk, the feature
      banner, no armed tag over the reels (0.13.28 has none), 0.13.28's symbol sheets. The
      machine's bar fits its width (0.13.28's ran Info and Turbo into the spin button under
      about 1,450 wide). The last reels slow and light up a scatter (or a coin) away. Bomb
      Spins and Super Hold & Win come with the next item.
- [x] Bonus rounds that build: Bomb Spins, Super Hold & Win (0.13.6). Play Feature's seventh
      card at 0.13.28's prices (Bomb Spins 102.80× / 97.80× / 98.60× / 98.20×, Super Hold & Win
      66.70× / 62.70× / 66.80× / 63.00×), words and order; 0.13.28's splash, "SPIN n / 10",
      "MULTIPLIER ×N AND BUILDING", "W × M = T", the tag at the end, Payers on a purple ribbon,
      jackpot coins' names on a red one, collectors landing anywhere (filmed beside 0.13.28).
      The rebuild's own numbers for 99% (`Feature::Builds`): Bomb Spins on richer reels (pay
      symbols 1.15 times as likely, a bomb in about one cell in fifteen: about one a screen, as
      0.13.28's showed), its bombs about as the machine's own; Super Hold & Win's Payers (0.5×
      to 5×, adding to every coin as they land) and Hoarders (worth everything else on the
      reels, gathered again whenever something new lands: 0.13.28's, seen in two rounds of ten)
      tuned on forty-million-round samples. A
      free spin lands at Turbo's pace (as 0.13.28's do), and its tumbles show at the player's.
- [x] Big wins: Big, Mega, Epic, Legendary, Mythic with fireworks (0.13.2/4). Measured on
      0.13.28's own rounds, chosen: a test server with a fixed house seed (a scratch build, kept
      out of the repository) let its app play rounds whose seeds were searched for in advance:
      Pyr Sevens at 247.65, 250.10, 252.15 and 316.50 times, Ember Dragon's Grand at 1,078 and
      1,079 and Minis at 20.20 and 21.20, Dice at 22 and 26.75, filmed and their sound recorded,
      then the same rounds on the rebuild side by side. Big from 10 times, Mega 25, Epic 50,
      Legendary 250 (0.13.28 went in between 248.55 and 250.10), Mythic 1,000. The count's
      length (3.65 s for one level, 2.3 more each, 11 at most; every jackpot's 11), its curve,
      the beat at each level, the plaques and their colours, the rays, the fountain of coins at
      each hit, the ring, the flash; Mythic's crown with the rays and its words turning through
      the colours, sparks, fireworks and lightning; a jackpot ends on the crown, "JACKPOT!",
      whatever it reached; the total held 2.5 s (Big) to 4.5 s (the crown). Its sounds,
      0.13.28's files at its pitches (found in its recordings by matching the waveforms): the
      tier-up a whole tone higher at each level, a slam with it from Legendary on, ticks fifty a
      second rising half a semitone at a time to an octave, the landing's slam two semitones
      under the level's; the slots' sounds at a slot, the casino's elsewhere; as loud as 0.13.28's
      at the same settings. Fixed on the way: a game's sounds were limited to six at once (a big
      win's hits were turned away under its ticks); in a call a tick waits while the call's
      mixer has four sounds going (the mixer itself unchanged). Unverified: the ticks' loudness
      at a real PC's frame rate (this machine draws the debug build at about twelve frames a
      second, and a late frame plays its latest tick only). Plinko's comes 1.25 s after the
      ball lands, once its bucket's lit (0.13.28's, filmed in autoplay: 1.15 and 1.35 s at 11
      times; the balls keep dropping under it); the rebuild showed it as the ball landed.
      A big win booked while another shows waits behind it and takes its place when its moment
      comes (0.13.28's, filmed in Plinko's autoplay; the rebuild's blanked the one showing for
      the 1.25 s); a new round (Dice and Keno playing by themselves, a spin, a deal) fades the
      one showing out over half a second (0.13.28's Dice autoplay, filmed; the rebuild cut it).
      Both filmed on the rebuild after, the same.
- [x] Pyr Sevens' Turbo as 0.13.28's: the reels land at once on their stops (they waited for
      each stop to come round on the strip, up to a second a reel, so a Turbo round of free
      spins took three times as long). The same round (same seed and number) on both apps:
      its big win 17.1 s after the click on 0.13.28, 17.4 on the rebuild (56 before), each spin
      in step; without Turbo they were in step already (68.7 and 68.5 s).
- [x] Bets to 100,000 on blackjack, baccarat, keno, slots (0.13.2/4/6); Pyr Sevens' Phoenix
      Jackpot in proportion (0.13.6); new players 100,000 (0.13.6). Every limit asked of
      0.13.28's server and answered the same, word for word, by the rebuilt one: blackjack,
      dice and keno to 100,000 a round, baccarat's Tie and pairs to 25,000, roulette 25,000 on a
      number to 500,000 on red or black and 1,000,000 a spin, Pyr Sevens' line bets to 5,000
      (0.13.28's list: 1 to 2,500 and 5,000), Reel Row and Ember Dragon's bets 20,000, 50,000 and
      100,000 added (a progressive jackpot's whole pool from 10,000 up, as before), what a round
      can pay 50,000,000,000. The Phoenix Jackpot starts at 10,000,000 and pays its pool in
      proportion to the bet; the machine shows your share (10,010 at a line bet of 5). Under
      0.13.28's names (`casino_100k`, `big_slot_bets`, `slots_100k`): 0.13.28's rounds at the top
      bets replayed by the rebuild's code (60 of them, every game), and 0.13.28's app on the
      rebuilt server bets 5,000 a line and checks its rounds. The rack as 0.13.28 lays it out
      (the 100K chip, 1K/5K/25K/100K on the chips, chips narrower to fit, the bet's words beside
      them while there's room: measured at eleven widths), except that where 0.13.28's chips ran
      under the buttons (Keno at 1,280 wide, anything narrower) they sit on a tray over the
      controls instead. Keno's ½, −chip and 2× and Dice's −chip (0.13.4) with it.
- [x] The casino's menu bar (0.13.6): the mark and name, Lobby, Games, VIP Club, Leaderboard,
      Profile (Staff and the control room for those who have them), Fairness at the right; each
      button's name by it on a bar over 1150 wide, the icon alone (its name on hovering) under
      that. The Games menu (0.13.6): every game this server has in four columns (the slots two
      wide), its picture, a word, its badge; "See all" goes to that chapter of the lobby, poker's
      lines too, "The whole lobby" to its top; a click anywhere else closes it (and does what it
      does there), as 0.13.28's. Measured off 0.13.28's screen at 1100, 1280 and 1500 wide: the
      panel (1198 wide at most, 16 in from the bar's right and the window's left), its ember and
      violet glows (over the bar too), its shadow, the caret, the columns narrowing with the
      window, names cut with "…" where they don't fit, every hover. (Keys comes with keys for
      every game; Bounty Poker and Dominó Cubano join the menu when they're in.)
- [x] Floor (0.13.2/4/6/7): in chapters (Slots, Poker, Table games, Pyr Originals), a bar of
      four buttons in their colors to jump to each (as wide as the page less 92 allows, 260 at
      most), each chapter's heading on a band of its color. Tiles like Stake's: each game's name
      big in its own lettering (0.13.28's fonts, game by game: Barlow Condensed, Anton, Bebas,
      Squada, Fjalla, Archivo Black), sized by the tile and cut down to fit 85% of its width,
      the studio tiny and spaced under it (PYR ORIGINALS, CHIMBITA GAMES), the tile darkened
      toward its foot as 0.13.28's; under each tile how many are playing it right now (a green
      dot when anyone is) and my best win; what it pays back shows when you point at it. The
      server says who's playing (everyone who played a round of it in the last five minutes, as
      0.13.28's server does, read off the wire) and the floor asks again every 45 seconds and on
      coming back to it. The casino's chat is at the table games and poker tables only (not on
      the floor), the "funny" tags gone.
      Measured off 0.13.28's screen at 900, 1100, 1280, 1500 and 1600 wide: every text's font,
      size and place (`work/textlab` draws text exactly as egui does, `work/ui/textfit.py` finds
      it on the screen), the bar, the bands, the tiles' shade, corners and spacing. (The
      machines' feature badges come with Reel Row Remastered; Bomb Pot, Bounty Poker and Dominó
      Cubano with theirs.)
- [x] Levels: 100 levels, ten tiers and crests, XP by time played, level chips, tier chests,
      frames and felts, missions (0.13.5). Every number measured on 0.13.28 (`measured/levels.md`):
      the curve, what each level pays, a round's XP at each game and stake, the long day's half,
      the house edges Ember Back is a share of, the missions and their rewards, the stars (one
      star's pay however many a round crosses, as 0.13.28). Missions are the day's, the same for
      everyone. The card's "almost there" in a level's last 15%; the burst as 0.13.28's ("LEVEL
      13", "EMBER!", the chips counting up). Fixed on the way: past 100, the card says how far the
      next star is (0.13.28 always said 0); "1 more level", not "levels"; the VIP tiers' perks fit
      their cards.
- [x] VIP Club: Ember Back, daily chips by tier, weekly bonus, VIP page (0.13.5) (a tab until the
      menu bar comes with the floor)
- [x] Plinko (0.13.5): the four difficulties' tables for 8 to 16 rows as 0.13.28 has them (every
      one pays back 99%), its round (a ball's path from the round's seed, bit for bit: 900 of
      0.13.28's own rounds replayed), its limits and words, the Edge of the Board achievement
      and the play's XP. The page as 0.13.28 draws it, measured off its screen and filmed: the
      board's size and place at three sizes, the temple as bright, pegs, ball, buckets and their
      colors, a tenth of a second a row, the bucket pressed and lit as a ball lands and its
      multiple floating up, the last ten buckets down the right. The peg and bucket sounds are
      made here. Its panel as 0.13.28's to the pixel (both tabs, measured off its screen at 1280
      by 800 and checked by a pixel difference): its rows' places, the Difficulty buttons' widths
      (61, 75, 61, 64, six apart) and corners, the sizes of its words and the Campfire palette's
      colours for them, the full-width rows to three from the right, the slider's handle inside
      its track; Auto play's words ("Number of games (0: until you stop it)", "Stop on a big win
      (10× or more)" by its switch on the left, "Start autoplay", a red "Stop autoplay (∞
      left)"), the field and switch dimmed while it runs.
- [x] The control room (0.13.5); GMs give/take chips, staff log and download (0.13.4). As 0.13.28
      has it, measured off its screen page by page: Overview (eight numbers, the VIP Club's
      players at each tier, the big wins, the latest in the log), Players (look one up, give or
      take chips: taking is clicked twice, "Take N away now"), the Chips log (filter, the first
      500, green/red/blue as given/taken/other, "Download the whole log": 0.13.28's CSV to the
      byte), Announce (a toast for everyone in the casino, big at a table), Access (the Owner of
      the Game lets people in and out). Game Masters give without a limit on a 0.13.4 server
      (prize boxes too; Ambassadors from their allowance), they and the owner take chips away
      (never more than they have), every gift and chip taken in the log with the balance before
      and after; the staff page's typed chips box, Take away and Download the whole log. Checked
      both ways: the rebuilt app on 0.13.28's server and 0.13.28's app on the rebuilt server
      (the server's home fingerprint changed in a test copy so its owner is the Owner of the
      Game), every page and every action. Today's missions and Ember Back now show on every
      casino page, as 0.13.28's do. Better than 0.13.28: a long log line stops at its card's
      edge instead of running off it.
- [x] Bomb Pot (was Flash Pot), new art, 50,000 and 100,000 (0.13.4/6); the crown win (0.13.4).
      Bomb Pot everywhere (the server's words as 0.13.28's, probed on it; its refusal lists all
      five buy-ins, 0.13.28's says three), `big_bomb_pots` with 50,000 and 100,000. The card as
      0.13.28's, measured at 1100, 1280 and 1600 wide and checked side by side on 0.13.28's
      server (glow, art and its fade, wrap at 46%, title glow, buttons and their shadows, the
      hint beside them from 1330 wide, waiting with Leave and the seats' pictures, running with
      Go to my table, "N GOING OFF NOW"). The crown win as 0.13.28's, filmed in the same games
      on both apps (a loser's and a winner's view of each): the picture pops up as a ring goes
      out, the laurel, the crown dropping and swaying (2.25° and 3.5 px, 4.8 s and 3.9 s), the
      words where 0.13.28 has them (held frames match to the pixel), the count over 3 s, 12.4 s
      in all; its art read before it's wanted. The wheel, fuse, bang and "ALL IN EVERY HAND"
      unchanged from 0.13.0 (side by side, by eye). The Games menu's jumps land where 0.13.28's
      do (6 higher); a debug build no longer panics at a tournament's table (buy-in sums
      saturate). A filled seat's gold ring and "PLAY" (a lighter brown than the amount) as
      0.13.28's (found side by side with Bounty Poker's). Not matched: 0.13.28's finale starts
      4.4 to 5.1 s after the last hand (the rebuild: 4.4); its sounds not compared (0.13.0's).
      Measurements in `rebuild/measured/bombpot.md`.
- [x] Bounty Poker (0.13.6). The server as 0.13.28's (`bounty`, `bounty_poker`): six seats,
      1,500 chips, two-minute levels, 45 s for people then bots; half of each buy-in on that
      head; a knockout pays half on the spot and puts half on the winner's head (who won the
      last pot the busted player was in; split if it was split); the winner takes their own
      with the prize; the pool the buy-ins less the bounties less 3% (2,910 for 1,000, the house
      staking the bots), to the top two; 0.13.28's refusals word for word. Checked over three
      watched games on 0.13.28's server (bounties, prizes, places) and a test of twelve games
      (every bounty paid to someone). One change: calling one off names Bounty Poker (0.13.28
      says a Bomb Pot). The card under the Bomb Pot card as 0.13.28's, side by side to the pixel
      (the description at 50%, the turning crosshair drawn as 0.13.28 draws it, its wide glow
      that darkens the Bomb card's corner, waiting with red seat rings, playing with the price
      on my head), "Five seats and one fuse, or six seats and a price on every head", the Games
      menu line with NEW and its drawn picture (the menu's pictures' corners now 8, as
      0.13.28's). At the table, the price on each head on a pill at the plate's foot (0.13.28's
      at its table's size, drawn at 0.84 it matches 0.13.28's at 1280 wide; the table's sizes
      are the scaling item's). Not seen on 0.13.28: a person's knockout (paid on the spot, its
      card says), a split pot's knockout. Measurements in `rebuild/measured/bounty.md`. A
      knockout of mine shows "BOUNTY +n" under a crosshair at the table's top for three seconds:
      in, but not yet seen side by side with 0.13.28's (it needs a knockout by me there).
- [x] High stakes and higher-than-high tables, 500K/1M/5M chips (0.13.6): three tables of each,
      0.13.28's names and bets (Hearth 10–1,000, High Limit 500–100,000, Crown Room 10,000 to
      10,000,000; Monte Carlo, Riviera High Rollers to 500,000, Grand Salle; Salon Privé, Black
      Pearl, Imperial Salon; Lucky Sevens, High Rollers, Sky Rollers). A roulette place's most is
      its own (25,000 on a number to 500,000 on red), twenty times that at the Grand Salle (500,000
      on a number, 10,000,000 on red: tried in 0.13.28's app on both servers, the same); Tie and
      the pairs to 25,000, to the table's most at the Imperial Salon; craps to 50,000, 2,500,000
      and 500,000,000 on the layout at once. The 500K, 1000K and 5000K chips (0.13.28's colors
      and words), a table's rack from the biggest chip not over its least bet (the Crown Room's
      5K to 5000K). The tables to pick from as 0.13.28's cards (0.13.4), measured: TABLES pick
      one, a card a table with YOU'RE HERE or how many seats are open, its bets and a dot a seat;
      better than 0.13.28 where its Crown Room's bets ran over the dots (said shorter there). The
      tables below as 0.13.28 lays them out (checked side by side, all four). Under 0.13.28's
      name (`high_tables`): its app plays the rebuilt server's Grand Salle and checks the spin.
- [x] Craps: lay odds (0.13.6), buy and lay (0.13.23, 0.13.26), chips stay as piled (0.13.24),
      lay win callout (0.13.25). The rules settle as 0.13.28's server does: 125 of its rolls
      replayed (`craps_0_13_28.json`), its refusals in its words and its order (read out of its
      program and tried on it: the most on the layout is 500,000,000 everywhere, its High
      Rollers took 2,700,000; buys and lays too small are said before what the point allows,
      then what the table takes). Its bots, read out of its program (`bots_bet` at 0x24b2d0)
      and watched: the pass-line-with-odds one buys the 4 and 10 one time in three, the dark
      side lays full odds half the time and the 4 (or 10) one time in four, in twenties. The
      table, side by side with 0.13.28 on its own server, same pixels watching (five players
      with every bet) and sitting (my piles, tags, the rack, Buys off / Buys working): LAY and
      BUY strips (`craps_buy_lay`), LAY ODDS on a point (`high_tables`), the OFF puck at the
      field's end, piles as put down (the last twelve chips drawn, 0.3 of a chip apart), the
      words by the pointer (wrapped at 420, kept 6 from the edge), "Can't: ..." and "You have
      N chips left to put down" as notices, the rack only chips I have left, a table's point
      on its card. A server without those features shows the layout it knows, as 0.13.28
      does there (tried with each name hidden). Better than 0.13.28: the words show over the
      chat button (it covered them); no "pick a smaller chip" when the rack has none; a bet
      put down and then standing up no longer says "my Pyr's seed wasn't used" (0.13.28 does,
      at all four table games); after a reconnect the casino table I'm at is watched again
      (0.13.28's stands still till I leave and come back). Not done: the house game's page
      (only shown by servers older than 0.12, which have neither) keeps 0.13.0's layout; the
      server takes buys and lays there as 0.13.28's does.
- [x] Dominó Cubano tables in the casino, to 10,000,000, El Presidente (0.13.11/12). The five
      standing tables (El Presidente the fifth: at 1280 wide neither app's strip shows it, nor
      scrolls), every page and sound checked side by side with 0.13.28 (`measured/domino.md`).
  - [x] The game and the server as 0.13.28's (`casino_domino`): Pairs and Solo, double-nine,
        ten tiles each (Solo two to four, drawing from what's left), capicúa half again (never
        on a double), chiva double, ¡Doblo! to ×8 within what everyone covers (answered by the
        people on the answering side, 25 s, taken if unanswered; bots take every one), 25 s a
        turn and 3 s a turn after a person's time runs out; 86 of 0.13.28's hands played again
        move for move (`crates/poker/tests/domino`), its words, its views key for key, its
        fourteen bots.
  - [x] The page as 0.13.28's, side by side to the pixel (`rebuild/measured/domino.md`): the
        tables strip and New table, the room, the table, the line laid as it lays it (turning at
        9.5 tiles' halves, doubles across), the seats, plates and turn rings, the cube, the
        rack, the controls and covers, the ¡Doblo! offer, the hand's end; the Games menu line
        (NEW); the floor's card first among the table games (NEW, PAIRS OR SOLO, its name in
        Archivo Black), side by side.
  - [x] Its sounds (`measured/domino.md`): the tiles, the opening tile, the slam and the
        shuffle made sample for sample as 0.13.28 makes them, each tile placed toward who laid
        it and sounding as it lands; the table's knock, all-in and chips for a pass, a ¡Doblo!
        and an accept, my turn's bells, a win's; heard side by side with 0.13.28 at one table,
        watching and both sitting (`measured/domino.md`). Fixed on the way: ¡Doblo! showed past
        what everyone covered (0.13.28 shows it only up to the cap), and a refused move stopped
        any other from going (the turn ran out).
        Not the same: the table's own sounds' tails (0.13.28's differ between the two sides
        after the first 25 ms), and its all-in's shove is brighter than the rebuild's (the
        poker table's sounds: to look at).
  - [x] A ¡Doblo! from both sides, side by side with 0.13.28 (Olive and Reba doubling against
        each other): the doubling side's banner on the felt, the answering side's panel, their
        words lit orange; the cube's ring gold only when it's my side's (watching, teal).
  - [x] Solo side by side (El Malecón): four in Solo pass and don't draw (the rebuild let them:
        fixed, with a test), the pozo on the felt, Draw's words, "· drew", "sitting this one
        out", the draw's sound as loud as 0.13.28's, the felt's Solo words.
  - [x] A ¡Doblo! watched (the banner, as the doubling side's) and put to me as the next in
        turn (my tiles bright, my time on the bar), side by side.
  - [ ] Not yet seen on 0.13.28: a Chiva (the game plays it as 0.13.28's rules, unseen on its
        screen: none in its 86 hands played here, and one needs an opponent who never laid a
        tile). Its words read off 0.13.28's server: "¡Chiva!", and "¡Capicúa y Chiva!" for a hand
        that's both (the rebuild said "¡Capicúa!": fixed, with a test). 0.13.28's app also has a
        line of its own with "Chiva & " and "· +N% bonus · ×M" beside the hand's end box (format
        pieces in its binary, never on screen here): the rebuild doesn't draw it.
- [x] Poker: the suits on bot faces and card pips (0.13.28's are one smooth shape each; the
      rebuild's are 0.13.0's circles and polygons), the Shark's crown and star
      (`measured/newtable.md`, `measured/cards.md`).
  - [x] Bot faces: the suits as 0.13.28's shapes (traced at 6x), one near-white shape with a dark
        shadow; a Shark (only a Shark) wears 0.13.0's fin, the violet ring and a star in a soft
        violet glow over the top, with a small one on the ring either side, drawn again with
        egui's own tessellator against 0.13.28's picture and checked in the app at 3x (Which bot?
        and a seat: the same to the pixel but for the discs' edges). Which bot? shows the faces
        as the level picked has them, as 0.13.28's does.
  - [x] The cards as 0.13.28's (`measured/cards.md`), measured with both apps watching one table
        at 3x, the same cards side by side: the body (its edge a 79th of the card, the face
        shading white to warm white), the rank in Barlow Condensed Black a fifth of the card
        high, its red (204,31,46) and black (23,25,31), pips lit from above (1.15 to 0.875 of
        their color down each), laid out as its board (rows 0.2155 in, a 7's and 8's halfway,
        a 9's and 10's four rows evenly), aces one pip 3/8 across; jacks, queens and kings in
        its gold frame (three golds down it, a fine line inside), the figure cut by a gold line
        slanting through the middle with a small gold diamond, the suit small in the corners;
        backs with the painting in a white margin. No small kind of card (a seat's shown cards,
        a history's hands: whole faces), a seat's badges give way to its shown cards. To the
        pixel but for the edges' softness and the figures (0.13.28's softer: fewer texels).
        The four-colour deck in its colours (clubs (27,128,69), diamonds (28,99,196)).
  - [x] The hand's name on the board and the bots' speech bubbles stay full size on a small
        table (0.13.28's: 1.19 times the rebuild's at 1280x800, the table's scale 0.84).
  - [x] Bots think and bluff like people (0.13.26), measured off 0.13.28's Medium bots at two
        tables (`measured/bots.md`: each turn's think by what it ended in, betting street by
        street) and played again in a simulation of the same tables: a bot's time comes from how
        hard its choice was (its hand against the price, or against a hand worth a bet) and
        what's riding on it, not from what it does; rags go in the least time; a trapper acts
        out a long think over a monster now and then. 0.13.28's medians (fold 0.60 s, check
        1.00, call 1.20, raise 1.80-1.98) and long thinks (two in five ending in a fold) come
        out within a few tenths and points. The preflop raiser bets the flop (0.13.28: 75%; the
        simulation 73-85%), draws are bet and raised, a missed draw fired on the river heads up,
        the Hard and Expert bots check-raise a bluff now and then, a maniac bluffs far more than
        a rock and a rookie hardly ever. Before the flop they play as before (VPIP and PFR as
        0.13.28's, bot by bot). Genius (Shark) keeps its own (it had all of these).
  - [~] Table sounds (0.13.6: chips that ring, cards that slide and snap, the felt, the rail, a
        better win), recorded off 0.13.28 at a table of bots and measured
        (`measured/tablesounds.md`), made again in `engine/src/table_sfx.rs`: fresh each time as
        0.13.28's are, from what each is made of (the chips' clicks and their four partials, the
        rail's 120 Hz knock, the slides, the snap, the felt; the bells' exact notes and timing),
        each measured against 0.13.28's over many takes (partials, decays, click counts and
        gaps, band levels over time) and as loud as 0.13.28's at the same settings (within 1 dB,
        at 0.13.0's cue volumes, which 0.13.28 keeps). The table's cues change everywhere they're
        used (roulette's chips are these chips on 0.13.28 too). As 0.13.28: the blinds' chips 0.1 s
        after the deal, an action's sound 40 ms after the board's card it brings, a time bank's
        a turn's last seconds ticking at one of two pitches and its time bank's at the other
        (`Cue::TickUp`; which one a turn starts on followed nothing watched, so it's a toss), and
        Time out's new knocks and bells (E5, C#5). Checked side by side: both apps watching the
        same table, sound for sound (`work/ui/ab.py`): the same latency (57 and 60 ms) and within a
        few dB in every band over a sound's first 0.3 s; and both seated, running their clocks
        out (the bells' levels by the same measure on both). The Ember profile plays them the same.
        The ticks fitted again to their own shape (F#6 and A6, each partial at a chance phase, its
        decays and level: 98.5% of each 0.13.28 tick explained so). Not checked: rabbit and Bomb
        Pot tables.
  - [x] The casino games' sounds as 0.13.28's (`measured/casinosounds.md`), heard off it game by
        game and taken apart into its own files (`work/ui/decomp.py`): the casino's cues play its
        `c_*` files (dice, Keno, roulette, craps and blackjack heard: which file, when, how loud,
        where), turned down to 0.9 when louder, placed by turning the other side down 0.7 of the
        pan; the table's own sounds (chips, cards) fresh as the poker table's. Pyr Sevens and Ember
        Dragon play the slots' band (0.13.2's "new slot sounds"): the reels' stops climbing a
        pentatonic, each sound's file and loudness as heard (the free spins, the big wins, Ember
        Respins, the dragon growing, All or Nothing). Plinko's pegs and buckets are 0.13.0's own
        helpers as 0.13.28 used them, sample for sample (a peg's pitch by its row, a bucket's by
        its place). The rebuild taken apart the same way at dice, Keno, Pyr Sevens and Plinko: the
        same files at the same gains, pitches and places. Not heard, so by the rule: the Bomb
        Pot's wheel, fuse and bang, the Bonfire's draw, Ember Dragon's big spin win and
        jackpots, a slot's All or Nothing win. Not checked: whether the Remastered machines'
        reel stops climb too (0.13.28's machines wouldn't spin here today).
  - [x] Staff titles on seats (0.13.4/0.13.6), as 0.13.28's (`measured/tablefeatures.md`): the
        seat's `badge` (`owner`, `game_master`, `ambassador`, none when hidden), the server
        sending it (with the MMRs) and `hide_title` (a change shows at once); "Show my title" in
        the table's menu for staff, its notices, the prize box after the separator as 0.13.28
        has it. The pill (GAME MASTER for the Owner too, AMBASSADOR) to the pixel at 1× and 3×.
        The emblems over the picture, traced off 0.13.28 drawn three times over with the title
        and without (the winged shields with crown or gem, the Ambassador's silver shield, the
        glows), and their moving parts filmed at 1× (a 3 s beat, nine rays turning 20° a second,
        sparkles in turns): side by side at 3× they look the same; the shapes are drawn again,
        not 0.13.28's to the pixel, and the sparkles' places are the rebuild's own. Not checked:
        titles at the casino's table games (blackjack and the rest).
  - [x] The level badge's ring in its tier's colour (Ember bronze, Flame silver, Blaze gold...),
        read off 0.13.28's seats.
  - [x] Slower wins (0.13.4), as 0.13.28's (`measured/tablefeatures.md`): its server waits 7 s
        after a showdown (3 s after a fold) and 7 s plus the board's time after an all-in; an
        all-in's board shows the first street 1.8 s on, each next 2.15 s after, the win 1.75 s
        after the river. The win: the pot's chips gather in its middle, stay 0.65 s, slide to
        the winner in 1.2 s wherever they sit; the ring and "+N" come in from 1.45 s, the "+N"
        rising 25 k; the name's pill fades from 5 s, the ring and "+N" from 5.55 s. Filmed
        beside 0.13.28's with the other apps stopped so each drew at full speed (six all-in
        hands each): the streets within a frame, the slide's fitted start and length (0.62 /
        1.19 s against 0.63 / 1.21), the "+N"'s fade and rise to the pixel and frame, the
        fades within 0.07 s. Not filmed again since: the Bomb Pot's crown win comes 4.4 s
        after the last hand's win is shown, which the slower board makes later on the clock
        (0.13.28's does too, as far as the reference measured goes).
  - [x] The casino chat's round button at a poker table, seated or watching (as at the table
        games; not on the floor or the poker room's lobby), to the pixel; its panel the same.
  - [x] The buttons before my turn (0.13.4) and Out of chips with Rebuy (0.13.22), as 0.13.28's
        (`measured/tablefeatures.md`): Check / Fold, Check, Call any; Fold, Call N, Call any
        once someone's bet, a raise taking the tick off Call N (checked at a three-handed table
        with two 0.13.28 apps); diffed with 0.13.28's at 1280×800 and 1600×1000, plain and
        ticked, to the anti-aliasing. Out of chips: the panel, Rebuy, Leave seat and the gold
        + Add chips, diffed at 1280×800 (Rebuy under the pointer too).
  - [x] The table's top bar (0.13.22) as 0.13.28's: Sit out / I'm back / Away and + Add chips,
        then icons (the hand log and Last hand, the sound, fullscreen, ☰), and the ☰ menu in
        0.13.28's order: Auto top-up, Straddle, Auto-muck, Drop a prize box (staff), Streamer
        mode..., Keys..., Leave seat, Close this table... (the owner). Diffed with 0.13.28's.
  - [x] Straddle (0.13.4, `poker_straddle`), Rabbit (0.13.4, `poker_rabbit`), Add chips any
        time and Auto top-up (0.13.22, `poker_top_up_anytime`), closing a table (0.13.4/24), as
        0.13.28 has them (`measured/tablefeatures.md`: its requests captured from its app, what
        its server sends from the rebuilt app's frame log, the timings from both):
        `straddle {table, on}` (the app keeps the box per table in memory, as 0.13.28's);
        the first after the big blind posts 2 big blinds (`posted … "what":"straddle"`), acts
        last with the option, a raise to 2 straddles at least; never heads-up. `rabbit {table,
        hand}` → `rabbit {table, hand, by, board, had}` to everyone, once; the next hand 7 s
        after it; the board is the deck's next cards with a burn before each street (the
        test checks two of 0.13.28's own shuffles give its boards). "Rabbit" in the middle
        for everyone at the table after a hand over before the river; the cards turn over
        one by one (0.45 s apart, 0.36 s each) light blue with a blue rim under "Rabbit
        hunted by …", then small in a panel when the next hand's dealt, gone 14 s after (its
        last second fading): filmed beside 0.13.28's at 30 fps, the same frames to one or
        two. `top_up` in a hand: kept on the seat (`pending`, "+400" green at the plate's
        right, for everyone), on the stack when the hand's over; the window as 0.13.28's
        (stack · most · chips, Top up to …, +20/+50/+100 big blinds that fit, the slider and
        Add, the in-a-hand line, Auto top-up). Auto top-up (`poker_auto_top_up`, 0.13.28's
        setting) tops up to the most whenever I'm not in a hand. `close` → `closed` to
        everyone ("… was closed: the chips at it went back to whoever had them."), a hand
        called off with each getting back what they started it with; the menu's "Close
        {table}?" (who's sitting, Close the table / Keep it open) and the lobby's Close →
        Close it now for the owner (`TableSummary.owner`, as 0.13.28 sends it). The 0.13.28
        app on the rebuilt server: its straddle posted and read ("Friend posts the straddle
        (40)"), its rabbit turned over, its chips added in a hand shown waiting and then on
        the stack, its Close closing. Better than 0.13.28: its server refused its own "Top up
        to the most" after a bet in the hand (it counted the stack the hand started with);
        the rebuild takes it and gives back at the hand's end whatever wouldn't fit. Found
        on the way (0.13.0's, fixed): standing up after folding in a hand gave back the
        chips already put in the pot.
  - [x] The table grows and shrinks with the window (0.13.2/0.13.6), as 0.13.28's, measured at 55
        window sizes (`measured/scaling.md`): k = (width/1150).min((height − 180)/620), 0.72 to
        1.6; the bar under the table 124k, the room for the seats 78k; pictures, plates, cards,
        badges, pills, the pot, the meter, the buttons and their words all k times (corner
        radii, shadows and the 16 to the window's edges not). 0.13.28's action bar: Fold 144,
        Call and Raise 184, 58 high, 12 apart, the words 20 / 17.5 / 15.5; the amount row 28
        high (presets 56 wide, 60 apart); my cards 104 wide (0.13.0's 96), the meter under them
        kept 7 over the bar; the board's cards a·0.22 to 150. Diffed with 0.13.28's at 1100×700,
        1280×800, 1600×800, 1600×1000 and 2560×1440: the felt, seats, plates, cards, meter and
        buttons fall on 0.13.28's pixels (what differs is the hand: cards, names, faces).
  - [x] New table (0.13.4/0.13.22) as 0.13.28's, measured: the High roller blinds (500/1K to
        25K/50K), a picture of the table with each seat's bot (who and how good, × to take one
        away, Fill the seats, Clear), and Which bot? (How good, then Anyone or one of the 20 by
        name, the ones sitting greyed "Ace's at this table already"); the same at a table's
        empty seat; a server without `poker_pick_bot` gets "Add a Medium bot", one without
        `hard_bots` no Hard. Compared with 0.13.28 to the pixel but for the suits on the faces.
        `create`/`add_bot` with `who` as 0.13.28 sends them (taken off it; Anyone sends none).
  - [x] Bot levels (0.13.7): Easy, Medium, Hard (new: between Medium and Expert, look level 20,
        first MMR 1,075), Expert, Shark; their wire names stay rookie, regular, hard, shark,
        genius, and so do the ratings' ids (`bot:shark:…` is an Expert, as 0.13.28 keeps them).
        Tags EASY/MEDIUM/HARD/EXPERT/SHARK BOT; an Expert has no fin, a Shark a violet ring
        (168,122,255) (its crown and star still to match). Apps without `casino_0_13_7` (0.13.28's
        cap) are told a Hard bot is a Medium one, as 0.13.28's server tells them; the rebuilt app
        sends it. New tournament: the five levels (Hard where the server has it).
- [x] Keys for every game (0.13.26): a Keys button on every game's top bar (lit while open, where
      0.13.28 has it: left of the sound), Keys in the casino's menu bar, Keys… in a poker table's
      menu: the Keys window as 0.13.28 has it, measured (its moves, words, keys, colors, the ×
      drawn as two strokes, every place to the pixel, the list half the window and 141 more), the
      game it was opened from first, "(this game)". Click a key and press another (Ctrl, Alt,
      Shift with it), two a move; a key another move of the same game had moves over and it says
      so; × takes one off, ↺ puts a move back, one button puts every key back; Esc stops
      waiting. Kept as 0.13.28 keeps them (`casino_keys` in the settings, only what's changed,
      0.13.28's names: tried, `{"chip.bigger":["Ctrl+Shift+W"]}` both), so a Pyr updated from
      0.13.28 keeps its keys. Every game's keys go through it: the four slot machines (spin, bet
      less and more), blackjack alone and at the tables (H S D P, Enter, R rebet or clear),
      baccarat (Enter, Backspace, R), roulette (Space or Enter, D, Backspace, R, Ctrl+Z against the
      house), craps (Space throws or says I'm ready, Backspace), Dice, Keno, Plinko, the chip rack
      (Left and Right), poker (F C R, 1 to 5 the amounts, Space held to peek in streamer mode).
      Buttons and hints show the key as it is now ("Roll  (Space)", "Hit  (H)", poker's corners,
      "· Space" under a table's button). Never while typing, nor while the window's open. Tried:
      rebinding, Up/Down, Left/Right, Space at Dice; tests. (Dominó's "Pass, or deal me in" is
      in the list and waits for Dominó.)
- [x] Emotes that move (0.13.23): fourteen (rofl, river, mindblown, rage, shades, rain, fire,
      dance, clown, fish, popcorn, salty, sleep, chicken), sent as 0.13.28 sends them (`e:<name>`,
      read off the frame log) both ways (tried: 0.13.28's to the rebuild and the rebuild's to
      0.13.28); their sheet carved from 0.13.28; the picker's "Moving" under the stickers with
      0.13.28's words, each standing still on its picture and acting when hovered; the poker bar
      as 0.13.28's (GG, Nice, rofl, rage, shades, ALL IN). Each act filmed in 0.13.28 at 60
      frames a second and fitted picture by picture (`rebuild/measured/emotes.md`): pictures and
      their pace, sway, turn, hops, shakes, the ring, the falling shades, fire's blaze, a tenth
      bigger than a sticker; the Zs as 0.13.28's to the frame; the other little things as many,
      as big, where and as fast (0.13.28's are the same every time, worked out from the time:
      so are the rebuild's, from its own numbers). Better than 0.13.28: the first one at a table
      shows its face at once (0.13.28's came without it for about a second).
- [x] New cards and backs (0.13.22): Meshy's court figures, ace of spades and six backs (carved
      from 0.13.28); every face-down card in the back picked, and a back picked while sitting
      shows at that seat at once (Blue: "dealt green after picking red")
- [x] Congrats / buy a drink on a big win (0.13.7), as 0.13.28's (`rebuild/measured/cheers.md`):
      a big win of 100 times the bet for 10,000 chips or more (a jackpot, a million up) has a
      moment for three minutes (its `cheer` on the big win); everyone else in the casino gets
      the banner for 12 seconds with Congrats! and Buy a drink · 5,000 (Congrats sent, Drink
      bought once clicked; the banner as wide, its rim 2), the winner a toast for each (in the
      casino; one away gets them on coming back, as 0.13.28's); 0.13.0's banner, 5 seconds, for
      the other big wins. 0.13.28's refusals word for word; apps before 0.13.7 are told none of
      it. Checked side by side with 0.13.28 (same server, same banners: pixel for pixel, each
      state; the tooltips; a big win without a moment; the winner's toasts), and 0.13.28's demo
      (`PYR_DEMO_CHEER=1`) is the rebuild's too.
- [x] Table invites (0.13.6): "+ Invite" on the top bar of every blackjack, roulette, baccarat and
      craps table (left of the rounds checked) and beside a poker table's Lobby, "Invite someone
      here to this table"; its list as 0.13.28's (Invite to this table, Look for someone, everyone
      here online first, "(offline)", 600 wide, scrolls past eight; never me, bots or anyone I've
      blocked); "Invited Olive to the table". 0.13.28's words both ways, taken from its app and its
      server (`invite_to_table {uid, table: {game, id}}`, `table_invite {from, table, name}`): its
      app invites on the rebuilt server, its server's invites reach the rebuilt app, and Join goes
      there. The server as 0.13.28's answers (tried on it): not to me, a bot, someone blocked
      either way, or the same person again within 20 seconds; anyone else, online or not; "that
      table's closed". Shown where the server has `voice_invite` (0.13.28 names no feature for
      it). The card in the corner (0.13.28's, measured: 332 by 126 at the bottom right, its bar
      running down the minute, Not now and Join), the same card for an invite to a voice channel
      ("invites you to 🔊 Lounge", which the rebuild had at the top right, drawn its own way), and
      Invite to 🔊 Lounge under Call in someone's menu, as 0.13.28 has it. Better than 0.13.28: two
      cards no longer overlap. Volume for the games (0.13.4): the speaker on every game's top bar
      ("Volume") and Volume at a poker table open GAME VOLUME's slider, Sound off and "Every game
      here, poker too. Pyr's other sounds follow Settings → Sounds."; kept as 0.13.28 keeps it
      (`games_volume`), on top of Pyr's sounds (tried on 0.13.28 by its sound: half and half, a
      quarter); Sound off for every game, poker and the Bonfire too. The keyboard on a game's Keys
      button drawn as 0.13.28's (smaller than the menu bar's, in the middle).
- [x] No freezes from reading or writing Pyr's state in the middle of drawing it (0.13.2: "Playing
      craps no longer makes Pyr stop responding"; 0.13.5: the games' volume froze it): a notice
      raised while a page draws (roulette, baccarat and craps setting bets to fit a table, 0.13.0's
      code) waits for the frame to finish, and so does the games' volume moved; the invite list
      and the volume take what they need from the frame. (Found here: inviting froze a test Pyr.)
- [x] Mini-game backgrounds (0.13.4), bonus games clearer (0.13.6), Keno and Dice tweaks
  - [x] Reel Row's twelve mini games side by side with 0.13.28's (each bought on both, played
        through): 0.13.28's scenes (the places only, nothing of the pieces painted in); the
        Champagne Tower in the middle of its stage; the Treasure Match's chests in the middle
        (90 wide at least, the tally beside them); "GOOD LUCK" before the first step; at the end
        the game dimmed and the total in its middle. Cargo Run: the rocket flies the route drawn
        in the picture, its marks along it (10×, 50×, 100×, 250×, 720× BLACK HOLE), a multiple
        as far along it as its log is to 720's (the marks fall where 0.13.28's do, to a pixel),
        the way flown in gold, the cargo delivered where it got to, the panel's words ("PRESS
        LAUNCH", "CARGO WORTH", "CARGO DELIVERED") and the new description. Candy Grab: one
        claw, the machine's own and bigger, on a carriage riding the rail; across to its place
        (five along the pile, a go to every third: 0.13.28's to the pixel at two sizes), a light
        in the candy there, down, closed on what it caught, up, back; its idle sway as
        0.13.28's. Ice Fishing: one place to fish, the picture's pond, ripples while the line's
        in. Casts and goes left big, as lamps. Better than 0.13.28: what's won stays readable
        over the claw's carriage (0.13.28's carriage hid it). Not seen: the Cargo Run's route
        past 281× (taken as straight from the 250× mark to the black hole: the marks' places
        agree with it).
  - [x] Ember Respins (0.13.6), side by side with 0.13.28's: over the board a plate as wide as
        the board (620 at most), RESPINS LEFT and three big lamps (lit, or grey once used),
        the board's total, the spots; SUPER ×3 under RESPINS LEFT in the Super ones; RESET TO 3!
        over the lamps' middle whenever a fireball lands (rising and fading over a second); under
        the board "3 respins left · a new fireball puts it back to 3" (1 respin left, singular),
        gone once they're over. Measured at 1100, 1280 and 1600 wide.
  - [x] Keno (0.13.7): a win counts up to its total (about a second, slowing to it) in bigger
        words than a loss's (26 to 21), and is called out "1 OF 1 · A WIN"; playing by itself it
        waits 2.65 s after a win before the next draw (turbo too), 0.75 s after a loss (0.4 in
        turbo), from the result, as 0.13.28's (filmed: draws, waits and count to the frame). No
        sound for the count (0.13.28's recorded: none). Dice (0.13.6): rolling by itself, 0.35 s
        after a loss, 1.55 s after a win, 2.6 s after a big one (its show faded out by the next
        roll), as 0.13.28's filmed. Keno and Dice keep going through wins unless "A big win stops
        it" and it's a big one (0.13.4). Unverified: the waits at a real PC's frame rate (here a
        frame is about 80 ms, so the rebuild's came 0.05 to 0.1 s late, as anything timed by
        frames does here).

### New in the rebuild
- [x] Prop bets in chat, shared: one market per prop for everyone in the channel, Pyr Casino
      chips, odds from an LMSR market maker (credit: Maanix's PlayProps); the server keeps every prop,
      bet, cash-out, payout and refund; whoever opens one backs its odds (b ln n chips, back
      when settled), so props never make chips out of nothing; Meshy art
- [x] "Updating Pyr" (Blue: "sometimes Pyr just disappears for a moment ... I want some loading
      screen, while it's updating"): Pyr's window shows it from Update until the new Pyr's
      window is up, then closes; if the install fails or the new Pyr doesn't open, it says so
      and offers Open Pyr. Windows: the setup's /STAYOPEN puts the running pyr.exe aside
      (renamed) instead of waiting for it to quit; the new Pyr deletes pyr.exe.old. Linux: the
      password's asked for while Pyr carries on (cancelled: nothing changes). `PYR_DEV_UPDATING`
      shows the screen's stages without updating. Seen first on the update after the rebuild's.
- [ ] Approved bug reports 657 to 675 (voice up and down; stopped responding), without
      touching the echo canceller
- [ ] The key change (above): built and tried with test keys; the day's steps there

## Talking to 0.13.28 (how its messages were found)

- `rebuild/tools/fields.py <elf> <name>...`: the serde field and variant lists in the 0.13.28
  binaries (`&[&str]` statics, through their relocations). The app's has every struct the app
  reads (RecordView's 37 fields, ToApp's variants); the server's has ToServer's 94 variants.
- `PYR_DEV_FRAME_LOG=<file>`: the rebuilt app writes every message a server sends it, and the
  rebuilt server every request it gets, one a line. A rebuilt app on 0.13.28's server shows its
  messages as they are (RecordView, GainsView, a round's record); a 0.13.28 app on the rebuilt
  server shows what it asks. A request the rebuilt server can't read is answered with an error
  and the connection carries on (it used to end it).
- Levels as 0.13.28 sends them: `poker_levels_2` in the server's features (without it a 0.13.28
  app shows the old card); `RecordView` adds `hide_title`, `tier`, `stars`, `ember_back`,
  `ember_bp`, `next_reward`, `missions` (`{text, progress, target, done, xp, chips}`),
  `weekly_ready`, `weekly_building`; `GainsView` adds `chips`, `tier_up` (the level the tier
  starts at), `missions` (their texts), `all_missions`, `star` (left out when empty); a record
  after every casino round (gains only when there's something to celebrate); `claim_ember` and
  `claim_weekly` (no fields) answered with the record and gains `{chips}`.
- ToServer variants only 0.13.28 has (still to do): dom_tables, dom_watch, dom_unwatch,
  dom_sit, dom_leave, dom_ready, dom_move, dom_bot, dom_new (straddle, rabbit, hide_title and
  bounty are in). ToApp: dom_tables, dom (rabbit is in). Its achievements: 39
  (the rebuild has 32). (take_chips, staff_log_all, cms and announcement are in: see the control
  room; invite_to_table and table_invite too: see the table invites.)
- The control room as 0.13.28 talks it (found by sending it requests exactly as written,
  `crates/engine/examples/raw_probe.rs`, and reading its debug log): `{"op":"cms","req":{"op":
  "overview"}}` and the like (`CmsReq`: overview, players `{query}`, adjust `{uid, delta,
  note}`, announce `{text}`, access, set_access `{uid, on}`), answered `{"op":"cms","res":...}`
  (`CmsRes`: overview, players `{rows}`, access `{owner, people: [[name, id]]}`, done
  `{what}`); `staff_log_all` answered with the whole log; `take_chips {uid, chips, note}`;
  `announcement {from, text}` to everyone in the casino; `StaffView.cms`; who's let in kept in
  the `cms_access` meta as 0.13.28 keeps it (a server moved from 0.13.28 keeps them).

## The 0.13.28 server's feature names, in its order

From the 0.13.28 `pyr-server` binary (what `Ready::features` lists). All in the rebuild
(2026-11-14; casino_domino with Dominó) but two, left out on purpose by the ground rule for
games that can't replay 0.13.28's rounds bit for bit: `casino_0_13_2` (Reel Row Remastered) and
`casino_build` (its bonus rounds that build, Bomb Spins and Super Hold & Win). The rebuild plays
them under its own name, `reel_row_remastered` (and prop bets, `prop_bets`), so a 0.13.28 app
plays Reel Row as 0.13.0's on the rebuilt server, and a rebuilt app does on 0.13.28's.

    roles files avatars dms server_icon custom_emoji friends backup giphy channel_newest
    soundboard klipy groups idle idle_away dnd files-25mb server_gifs handshake vouches
    vouches-for-me invites poker sealed_messages channel_order move_members casino_0_12
    casino_0_13 casino_100k casino_0_13_2 poker_rabbit poker_straddle poker_pick_bot
    afk_channel poker_gm_unlimited big_slot_bets poker_levels_2 casino_cms casino_plinko
    casino_build voice_invite big_bomb_pots slots_100k hide_title bounty_poker high_tables
    hard_bots casino_domino poker_top_up_anytime craps_buy_lay dm-files-2gb

DM files (0.13.13), from the 0.13.28 binaries: each part is a `FileRequest::Put` of kind
`sealed` named `dmfile`, sent as the message's attachments; the server takes them only in
direct messages and groups ("files can only be sent in direct messages"), keeps them 7 days
(`name = 'dmfile' AND kind = 3 AND (created_at < ?1 OR (message_id IS NULL AND created_at <
?2))`), counts them apart from images, and limits them with `PYR_DM_FILES_MB` and
`PYR_USER_DM_FILES_MB` (4 GB a week): "Too many file parts waiting to be sent. Try again in a
while.", "You've sent a lot in the last hour. Try again a bit later.", "You've sent {} GB of
files this week, as much as this server takes from one person. Older files expire after 7
days.", "This server's room for files is full right now. Try again in a few days, when older
files expire." The engine's `dmfiles.rs`.

## Programs and Pyr (0.13.15 to 0.13.18), from Pyropoly 0.2.0's source (`src/pyr.rs`)

Pyr starts a program with `PYR_NAME` (my Pyr name), `PYR_BRIDGE=1` (Pyr listens), `PYR_JOIN`
(a room to join at once, from an invite's Join) and, on Windows, `PYR_EMBED` (it starts
hidden so Pyr can take its window in). Pyr writes one JSON object a line to the program's
standard input: `{"op":"friends","me":…,"list":[{"uid","name","online"}…]}`,
`{"op":"join","room":…}`, `{"op":"invited","uid":…,"ok":bool,"why":…}`. The program asks Pyr
on its standard output with lines `@pyr {"op":"invite","uid":…,"room":…}`: Pyr sends that
friend a direct message with Join. Invite links: `https://pyr.gg/<game>/<ROOM>` (0.13.18;
`pyr://play/<game>/<ROOM>` before), shown in Pyr as the game's card with Join. 0.13.28's
`plugins/programs.rs` has a `program-bridge-out` thread (it reads the program's output).
Pyropoly's embedding notes (`src/embed.rs`): Pyr takes in a top-level window of the process
that isn't a tool or helper window, the first it finds (one showing before a hidden one, then
the bigger), and while it holds one only swaps to another that shows and is about as big or
bigger; graphics drivers' hidden windows (NVIDIA's "NVOGLDC invisible", class
NVOpenGLPbuffer) are never taken in.
0.13.27's additions, from Pyropoly 0.2.4's program (its strings and code): Pyr also writes
`{"op":"pyr","can":["update"],"version":"<the Library's newest version of the program>"}`
(when it opens, and every ten minutes while it's open); Pyropoly offers "Update now" when
`version` isn't its own, and only asks when `can` has "update". It asks with
`@pyr {"op":"update","room":"<ROOM>"}`: Pyr closes it, updates it and opens it again with
`PYR_JOIN` set to that room.
