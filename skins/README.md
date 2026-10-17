# Making an Ingle skin

A skin changes how [Ingle](https://firelitgames.com/ingle) looks and sounds: its colors, a
picture behind the chat, its fonts, and the sounds it plays for messages, people joining and
calls. Skins are made with Claude through the **Claude connector**, tried live in Ingle, and
sent in for the Ingle Library. Once Firelit Games looks one over and approves it, it's in the
Library for everyone, in the Skins page's Gallery (the palette under Plugins, at the bottom of
the server list), shown under the name its maker gave it.

A skin is a folder with a `skin.json` and the files it uses:

```
blood-moon/
  skin.json
  background.webp
  fonts/Body.ttf
  fonts/Bold.ttf
  fonts/OFL.txt          (a font's license travels with it)
  sounds/message.ings
  preview.png            (optional: a picture of Ingle wearing it, for the Library)
```

## skin.json

```json
{
  "name": "Blood Moon",
  "version": "1.0",
  "summary": "Deep red and black, with a moon behind the chat.",
  "author": "Blue",
  "colors": { "main": "#1a0d0f", "side": "#140a0b", "ember": "#d4202c" },
  "background": { "file": "background.webp", "show": 0.3, "under": "chat" },
  "fonts": { "body": "fonts/Body.ttf", "bold": "fonts/Bold.ttf", "scale": 1.0 },
  "sounds": { "message": "sounds/message.ings", "join": "sounds/join.ings" }
}
```

Only `name` and `version` are needed. Anything left out stays as Ingle's own skin, Campfire.

### colors

Each is written `#rrggbb`:

| Name | Where it shows |
|---|---|
| `rail` | the strip of server icons on the far left |
| `side` | the channel list and the member list |
| `side_dark` | the bars under the channel list (your name, voice) |
| `main` | the chat |
| `elev` | cards, boxes to type in, and buttons |
| `elev2` | a button or card under the pointer |
| `hover` | a row under the pointer |
| `line` | lines between things |
| `shadow_line` | the darker line under headers |
| `text` | headings and names |
| `body` | message text |
| `muted` | secondary text |
| `dim` | the faintest text (times, hints) |
| `ember` | the accent: main buttons, what's selected |
| `ember2` | the accent's lighter shade (highlights) |
| `glow` | speaking rings and glows |
| `live` | LIVE, alerts and unread badges |
| `online` | online dots |
| `owner` | the owner's crown |
| `admin` | admins' names |
| `link` | links in messages |
| `stage` | behind video and screen shares |
| `tile` | video tiles |
| `code_bg` | code blocks |
| `selection` | selected text |

Text has to stay readable: `body` and `text` on `main` (and `text` on `side`) need a contrast of
at least 4.5, and `muted` on `main` at least 3. The connector's `check_skin` measures it.

Campfire, for reference: rail `#15110f`, side `#1e1916`, side_dark `#1a1512`, main `#26201c`,
elev `#2f2823`, elev2 `#3a312b`, hover `#2a231f`, line `#3b322c`, shadow_line `#14100e`, text
`#eee6df`, body `#e6ddd5`, muted `#a8998c`, dim `#7d7067`, ember `#ff7a3d`, ember2 `#ffb347`, glow
`#ffa94d`, live `#e5484d`, online `#6cc070`, owner `#f5b041`, admin `#ec8f63`, link `#ffad7a`,
stage `#100d0b`, tile `#1d1815`, code_bg `#1a1512`, selection `#8a401c`.

### background

A PNG, JPEG or WebP (4096 pixels a side at most), seen through Ingle's colors.

- `show`: how much of it shows through, from 0.1 (a hint) to 0.6. The colors stay on top of
  it, so pick a `main` that text reads well on.
- `under`: `"chat"` (only behind the chat) or `"window"` (behind the channel and member lists
  too).

### fonts

TrueType or OpenType files (`.ttf`, `.otf`): `body` for text, `bold` for headings and names (the
body font if there's none). `scale` (0.8 to 1.25) evens out fonts that look bigger or smaller
than Ingle's own at the same size. Ingle keeps its own font for any letter or emoji a skin's
font doesn't have. Most fonts come with a license (the SIL Open Font License, say) that has to
travel with them: put its text file in the skin.

### sounds

Ingle sound files (`.ings`: Opus, 10 seconds at most). The connector's `make_sound` makes one
from an MP3, WAV, OGG or FLAC. The sounds a skin can replace:

| Name | When it plays |
|---|---|
| `message` | a new message |
| `join` | someone joins your voice channel |
| `leave` | someone leaves it |
| `stream_start` | someone starts sharing their screen |
| `stream_stop` | they stop |
| `watch_start` | you start watching a stream |
| `watch_stop` | you stop watching |
| `ring` | someone's calling you (it repeats while it rings) |
| `ringback` | you're calling someone and it's ringing |

Keep them short and gentle: they play often.

## Limits

24 files, 700 KB a file, 4 MB in all. Only `skin.json`, what it names, text files (a license,
a readme) and `preview.png`/`.jpg`/`.webp` can be in the folder.

## Trying one you're making

Skins → Make your own → **Try a skin folder** (or Settings → Appearance) wears the skin in a
folder until you stop it, and again each time you save a change to its files.

## With Claude

With the Claude connector set up (Ingle: Settings → Claude, which adds it to the Claude app),
ask Claude to make you a skin. It can:

- `new_skin`: start a skin folder with Campfire's colors written out to change,
- `make_sound`: turn an audio file into an Ingle sound,
- `check_skin`: check a folder the way Ingle and the Library will,
- `preview_skin`: show it live in your Ingle (`stop_preview` puts yours back),
- `send_skin`: send it in for the Ingle Library, and later, a new version of it,
- `my_submissions`: see whether what you sent was approved (and any note about it).

New versions can only come from the Ingle that sent the first one (it's signed with your Ingle
identity), through the connector.
