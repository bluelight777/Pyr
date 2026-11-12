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
- [ ] Before the swap, every name 0.13.28 stores is one the rebuild knows (from its server's
      serde lists, `rebuild/tools/fields.py`): bot levels `rookie regular hard shark genius`
      (`hard` comes with the bots' levels), games `slots blackjack roulette baccarat craps dice
      keno reels dragon plinko domino` (`domino` comes with Dominó Cubano), Bonfire prizes
      `grand blaze spark`, staff `game_master ambassador`. Until then a list holding one of
      them (the bots' ratings, a big wins list) is kept whole under `unread:` and not shown.
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
- [ ] Voice isolation takes the PC's own sound out of the microphone (0.13.2); headphones dips
      (0.13.11), its echo (0.13.12); double-talk on speakers (0.13.19); 0.13.24's try taken out
      (0.13.25)
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
- [ ] On the graphics card end to end (0.13.26, Windows): Media Foundation's hardware encoder
      (NVENC, AMF, QSV) fed by the card's video engine (scaled and made NV12 on the card), the
      card's priority for Pyr while sharing (D3DKMTSetProcessSchedulingPriorityClass), and the
      processor's way when the card's doesn't work. 0.13.28's hw.rs is about 1,000 lines; its
      messages are in its program. A/B on Blue's PC before it ships.
- [ ] Automatic reports (voice went up and down, stopped responding, sound problems): 0.13.0
      has the swells (dips) detector and the watchdog; check what 0.13.x added

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
- [ ] Plugins with art: up to 100 MB and 5,000 files, any Rust (0.13.6)
- [ ] Programs open inside Pyr, stay in their page, Opening..., NVIDIA helper windows
      (0.13.7 to 0.13.10, 0.13.17, 0.13.18)
- [x] Programs know your Pyr name and friends; PYR_BRIDGE; invite friends (0.13.15)
- [ ] Library shows plugins added since setup (0.13.7); Dominó program removed and never listed
      (0.13.12, 0.13.25)
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

- [ ] Reel Row Remastered: twelve machines with features (tumbles and bombs, Hold & Win,
      wilds that grow/freeze/walk/stay), ante, Play Feature, slowing reels, splashes, plaques,
      a sound band per machine, 99% checked by simulation (0.13.2)
- [ ] Bonus rounds that build: Bomb Spins, Super Hold & Win (0.13.6)
- [ ] Big wins: Big, Mega, Epic, Legendary, Mythic with fireworks (0.13.2/4)
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
      coming back to it. The casino's chat is at the table games only, the "funny" tags gone.
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
      made here.
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
- [ ] Bomb Pot (was Flash Pot), new art, 50,000 and 100,000 (0.13.4/6); the crown win (0.13.4)
- [ ] Bounty Poker (0.13.6)
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
- [ ] Dominó Cubano tables in the casino, to 10,000,000, El Presidente (0.13.11/12)
- [ ] Poker: rabbit (0.13.4/6/7), straddle (0.13.4), new table with seat and bot picker, 20 bots
      (0.13.4), bot levels Easy to Shark (0.13.7), bots think and bluff like people (0.13.26),
      close tables (0.13.4/24), add chips and top-up (0.13.22), high-roller blinds (0.13.22),
      top bar (0.13.22), titles and badges (0.13.4/6: GAME MASTER / OWNER on staff's seats, Title
      shown or hidden: `hide_title`, the record's field and request are in already), pre-action buttons (0.13.4), slower wins
      (0.13.4), scaling (0.13.6), table sounds (0.13.6)
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
- [ ] Emotes that move (0.13.23)
- [x] New cards and backs (0.13.22): Meshy's court figures, ace of spades and six backs (carved
      from 0.13.28); every face-down card in the back picked, and a back picked while sitting
      shows at that seat at once (Blue: "dealt green after picking red")
- [ ] Congrats / buy a drink on a big win (0.13.7)
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
- [ ] Mini-game backgrounds (0.13.4), bonus games clearer (0.13.6), Keno and Dice tweaks

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
- [ ] The key change (above)

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
- ToServer variants only 0.13.28 has (still to do): straddle, rabbit, hide_title,
  bounty, dom_tables, dom_watch, dom_unwatch, dom_sit, dom_leave, dom_ready,
  dom_move, dom_bot, dom_new. ToApp: rabbit, dom_tables, dom. Its achievements: 39
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

From the 0.13.28 `pyr-server` binary (what `Ready::features` lists). Still to add in the
rebuild (as of chunk 2; poker_levels_2, casino_plinko, casino_cms, poker_gm_unlimited since):
casino_100k, casino_0_13_2, poker_rabbit, poker_straddle, poker_pick_bot, big_slot_bets,
casino_build, big_bomb_pots, slots_100k, hide_title, bounty_poker, high_tables, hard_bots,
casino_domino, poker_top_up_anytime, craps_buy_lay, dm-files-2gb.

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
