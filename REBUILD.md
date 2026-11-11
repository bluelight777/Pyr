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
- [ ] Share picker redesign (0.13.14); still screens keep quality, High profile, crisp text
      (0.13.21); keep the size, lower the frame rate instead (0.13.22); on the graphics card
      end to end with NVENC/AMF/QSV, priority, 60 fps pacing, encoder in stats (0.13.26)
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
      through -99 dB). Still to do: Pyr's own echo test on Blue's PC, A/B against 0.13.28.

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
- [ ] Bets to 100,000 on blackjack, baccarat, keno, slots (0.13.2/4/6); Pyr Sevens' Phoenix
      Jackpot in proportion (0.13.6); new players 100,000 (0.13.6)
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
- [ ] Floor: chapters with the bar to jump to each, Stake-like tiles, studio, players, best win
      (0.13.2/4/6/7); the casino's chat at the table games only (0.13.2)
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
- [ ] High stakes and higher-than-high tables, 500K/1M/5M chips (0.13.6)
- [ ] Craps: lay odds (0.13.6), buy and lay (0.13.23, 0.13.26), chips stay as piled (0.13.24),
      lay win callout (0.13.25)
- [ ] Dominó Cubano tables in the casino, to 10,000,000, El Presidente (0.13.11/12)
- [ ] Poker: rabbit (0.13.4/6/7), straddle (0.13.4), new table with seat and bot picker, 20 bots
      (0.13.4), bot levels Easy to Shark (0.13.7), bots think and bluff like people (0.13.26),
      close tables (0.13.4/24), add chips and top-up (0.13.22), high-roller blinds (0.13.22),
      top bar (0.13.22), titles and badges (0.13.4/6: GAME MASTER / OWNER on staff's seats, Title
      shown or hidden: `hide_title`, the record's field and request are in already), pre-action buttons (0.13.4), slower wins
      (0.13.4), scaling (0.13.6), table sounds (0.13.6)
- [ ] Keys for every game (0.13.26)
- [ ] Emotes that move (0.13.23)
- [x] New cards and backs (0.13.22): Meshy's court figures, ace of spades and six backs (carved
      from 0.13.28); every face-down card in the back picked, and a back picked while sitting
      shows at that seat at once (Blue: "dealt green after picking red")
- [ ] Congrats / buy a drink on a big win (0.13.7)
- [ ] Table invites (0.13.6); volume per game (0.13.4)
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
  invite_to_table, bounty, dom_tables, dom_watch, dom_unwatch, dom_sit, dom_leave, dom_ready,
  dom_move, dom_bot, dom_new. ToApp: rabbit, dom_tables, dom, table_invite. Its achievements: 39
  (the rebuild has 32). (take_chips, staff_log_all, cms and announcement are in: see the control
  room.)
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
