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
- [ ] Pictures that arrive before their key keep them (0.13.4)
- [ ] AFK channel: picked in Server settings, 5/15/30/60/120 min, a channel called AFK by
      default (0.13.4); never while sharing or watching a screen, an hour by default (0.13.25);
      away vs Idle by hand, from joining the channel, the app checks before going (0.13.28)
- [ ] Invite someone to your voice channel (card with Join) (0.13.6)
- [ ] Taskbar badge with mentions and DMs, 9+ (0.13.7)
- [ ] Files in DMs and groups: up to 2 GB, E2EE, resumable, kept 7 days, 4 GB a week (0.13.13)
- [ ] New DM said "isn't end-to-end encrypted yet" (0.13.13)
- [ ] Plugins sent in with big files: parts (0.13.14)
- [ ] Game invites to friends from programs (0.13.15/16), as links pyr.gg/<game>/<code> (0.13.18)

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
- [ ] Automatic reports (voice went up and down, stopped responding, sound problems)

### Library and programs
- [ ] Plugins with art: up to 100 MB and 5,000 files, any Rust (0.13.6)
- [ ] Programs open inside Pyr, stay in their page, Opening..., NVIDIA helper windows
      (0.13.7 to 0.13.10, 0.13.17, 0.13.18)
- [ ] Programs know your Pyr name and friends; PYR_BRIDGE; invite friends (0.13.15)
- [ ] Library shows plugins added since setup (0.13.7); Dominó program removed and never listed
      (0.13.12, 0.13.25)
- [ ] One-click update of open programs, programs can ask, invites update first, list fetched
      again, every ten minutes (0.13.27)

### Pyr Casino
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
- [ ] Emotes that move (0.13.23); new cards and backs (0.13.22)
- [ ] Congrats / buy a drink on a big win (0.13.7)
- [ ] Table invites (0.13.6); volume per game (0.13.4)
- [ ] Mini-game backgrounds (0.13.4), bonus games clearer (0.13.6), Keno and Dice tweaks

### New in the rebuild
- [ ] Prop bets in chat, shared: one market per prop for everyone in the channel, Pyr Casino
      chips, odds from an LMSR market maker (credit: Maanix's PlayProps)
- [ ] Approved bug reports 657 to 675 (voice up and down; stopped responding), without
      touching the echo canceller
- [ ] The key change (above)
