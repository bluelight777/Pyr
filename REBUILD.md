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
- [ ] New DM said "isn't end-to-end encrypted yet" (0.13.13): 0.13.0 already checks the
      server's sealed feature; check a new DM's intro on screen
- [x] Plugins sent in with big files: parts (0.13.14)
- [x] Game invites to friends from programs (0.13.15/16), as links pyr.gg/<game>/<code> (0.13.18)

### Voice, video, sounds
- [ ] Voice isolation takes the PC's own sound out of the microphone (0.13.2); headphones dips
      (0.13.11), its echo (0.13.12); double-talk on speakers (0.13.19); 0.13.24's try taken out
      (0.13.25)
- [ ] Headset that drops out and comes back isn't an error (0.13.2)
- [ ] Streamer mode only when turned on (0.13.2)
- [ ] Sound profiles Classic / Ember; ten new ringtones; soundboard redrawn (0.13.2/3)
- [ ] Join chime in Classic climbs and rings (0.13.23)
- [ ] Mute shortcut: any key combo, then any single key (0.13.20/22); soundboard keys the same
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
- [ ] Floor: chapters, Stake-like tiles, studio, players, best win, menu bar with Games menu
      (0.13.2/4/6/7)
- [ ] Levels: 100 levels, ten tiers and crests, XP by time played, level chips, tier chests,
      frames and felts, missions (0.13.5)
- [ ] VIP Club: Ember Back, daily chips by tier, weekly bonus, VIP page (0.13.5)
- [ ] Plinko (0.13.5)
- [ ] The control room (0.13.5); GMs give/take chips, staff log and download (0.13.4)
- [ ] Bomb Pot (was Flash Pot), new art, 50,000 and 100,000 (0.13.4/6); the crown win (0.13.4)
- [ ] Bounty Poker (0.13.6)
- [ ] High stakes and higher-than-high tables, 500K/1M/5M chips (0.13.6)
- [ ] Craps: lay odds (0.13.6), buy and lay (0.13.23, 0.13.26), chips stay as piled (0.13.24),
      lay win callout (0.13.25)
- [ ] Dominó Cubano tables in the casino, to 10,000,000, El Presidente (0.13.11/12)
- [ ] Poker: rabbit (0.13.4/6/7), straddle (0.13.4), new table with seat and bot picker, 20 bots
      (0.13.4), bot levels Easy to Shark (0.13.7), bots think and bluff like people (0.13.26),
      close tables (0.13.4/24), add chips and top-up (0.13.22), high-roller blinds (0.13.22),
      top bar (0.13.22), titles and badges (0.13.4/6), pre-action buttons (0.13.4), slower wins
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
- [ ] Approved bug reports 657 to 675 (voice up and down; stopped responding), without
      touching the echo canceller
- [ ] The key change (above)

## The 0.13.28 server's feature names, in its order

From the 0.13.28 `pyr-server` binary (what `Ready::features` lists). Still to add in the
rebuild (as of chunk 2): casino_100k, casino_0_13_2, poker_rabbit, poker_straddle,
poker_pick_bot, poker_gm_unlimited, big_slot_bets, poker_levels_2, casino_cms, casino_plinko,
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
