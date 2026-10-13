# Spellcheck dictionaries

Hunspell dictionaries for the message box's spellcheck. They are unmodified copies of
[wooorm/dictionaries](https://github.com/wooorm/dictionaries) npm packages and are built into
Ingle (`crates/app/src/spell.rs`).

| File | Package | Source | License |
|---|---|---|---|
| `en-US.*` | dictionary-en 4.0.0 | SCOWL, wordlist.sourceforge.net | MIT AND BSD (`en-US.LICENSE.txt`) |
| `en-GB.*` | dictionary-en-gb 3.0.0 | SCOWL, wordlist.sourceforge.net | MIT AND BSD (`en-GB.LICENSE.txt`) |
| `es.*` | dictionary-es 4.0.0 | RLA-ES, Santiago Bosio | Used under MPL 1.1 (offered as GPL-3.0, LGPL-3.0 or MPL-1.1; `es.LICENSE.txt`) |
| `fr.*` | dictionary-fr 3.0.0 | Grammalecte, Olivier R. | MPL-2.0 (`fr.LICENSE.txt`) |

German, Italian and Swedish are left out: their dictionaries are offered only under the GPL
or LGPL.
