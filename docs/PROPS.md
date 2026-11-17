# Prop Bets, the big version (Pyr 0.15)

Blue, Nov 17 2026: "add the full features of Prop bets… NFL, NBA, NHL, MLB, College Football,
College Basketball, Topical (world events), even allow prop bets for Pyr Casino. Create a
Crypto category, weather, entertainment, science, art. It should update itself regularly…
only play money from Pyr Casino… automatically update without requiring human intervention."
"Make it beautiful and impressive." "Make it mostly automated."

His choices: **public data only, $0** (no Claude API); **ESPN's public feed** for sports;
**a server's owner turns the casino's own props on** (off until then).

## How the big sites do it (what's copied)

Kalshi and Polymarket don't write props one at a time: a *series* repeats ("the high in New
York", "Bitcoin at midnight", every game on the schedule), each day or game is an *event*, and
each event gets a few props from a template. Every prop names its source and its rules up
front and is settled from that source, nobody deciding. Manifold adds what makes play money
fun: the odds as a line over time, what you hold and what it's worth, the best bettors.

## The pieces

1. **The feed maker**, `crates/props-feed` (`pyr-props`), a service on the VPS. Every source
   has its own clock. It keeps what it opened in a state file, asks the sources how things came
   out, and writes one signed file, `https://pyr.gg/props/feed.json`
   (`pyr_proto::props_feed`), and beside it `status.json` (each source's health: read it to see
   what's broken).
2. **The server** (`crates/server/src/props_feed.rs`, `poker/props.rs`): while the casino's own
   props are on, it reads the feed every two minutes (ETag), checks the release key's
   certificate, and follows it: opens a house prop for each prop there, moves what changed,
   settles or calls off what came out. House props belong to no channel; the casino backs them
   (the most it can lose on one: `b ln(1/p)` of its least likely outcome).
3. **Pyr Casino's props** are each server's own, never the feed's
   (`crates/server/src/poker/props/own.rs`): made from its casino's own count of rounds, a day
   (UTC) at a time. Tomorrow's open today (once someone has played there in the last week),
   close as the day starts, say how the day is going while it's on, and settle by themselves
   two minutes after it ends. Whoever plays moves these numbers, so the casino stakes a
   quarter as much on them as on the feed's, and nothing is asked that one bet decides (the
   day's biggest win, how many rounds are played).
4. **The app** relays to Prop Bets' window (the bridge: `game_bridge.rs`); the window
   (`crates/prop-bets`) shows categories, events, a prop's page, what I hold, the best at props.

## Rules every automatic prop keeps

- **Betting closes before the answer can be known**: a game's props at its start; a "next
  week" prop when that week starts; a price at a moment, a day before. A house market pays for
  information, so what's bet after the answer is knowable is taken from the casino.
- A prop's id never changes meaning. A game put back keeps its id (its times move); one
  postponed or canceled is called off (chips back).
- A result is published only once the source is final (a game final for five minutes, a week
  over for a day), and a prop whose result can't be had is called off, not guessed.
- Questions are 150 characters at most and outcomes 30 (`pyr_poker::props::check_new`).

## Categories and their sources

| Category | Source | Props |
|---|---|---|
| nfl, nba, nhl, mlb, cfb, cbb | ESPN's scoreboard (`site.api.espn.com/apis/site/v2/sports/…/scoreboard`) | per game: winner; spread and total when the scoreboard has odds; one game prop (NFL: margin and first-half leader; CFB, CBB: halftime leader; NBA: first-quarter leader; NHL: overtime or shootout; MLB: a run in the first) |
| crypto | Coinbase's public candles, Kraken's as the spare | BTC, ETH, SOL, XRP, DOGE: up or down each day; BTC, ETH: where it is in two days (five ranges); BTC, ETH, SOL: where it is next Monday |
| weather | National Weather Service: the forecast (api.weather.gov) for the ranges, the daily climate report (CLI) to settle | ten cities: tomorrow's high (five ranges around the forecast), rain or snow |
| science | USGS earthquakes, NOAA SWPC (flares, Kp), Launch Library 2, JPL close approaches | next week: an M6.5+, how many M6+, an X flare, a G2 storm, how many launches, how many asteroids inside the Moon's orbit; each launch: off the pad on time? |
| topical | Treasury (10-year yield, debt to the penny), ECB rates (Frankfurter), the Federal Register | the 10-year on Friday, up or down tomorrow; the debt over its next $0.1 trillion by the month's end; the euro, the yen and the pound on Friday; executive orders next week |
| entertainment | Steam's player counts | the biggest games at 18:00 UTC: over or under; which of two has more on Saturday |
| art | drand (the League of Entropy's public randomness), NASA's Astronomy Picture of the Day | the Gallery Draw: which of six masterpieces is drawn tomorrow; what tomorrow's picture shows |
| casino | each server's own casino (its rounds by the day) | tomorrow: does the house come out ahead; which game takes the most in bets (the week's four biggest, or another); is a jackpot won |
| community | members | their own, in any category |

## What's checked and what isn't (Nov 17 2026)

Read live through the fetch tool, shapes as in `crates/props-feed/samples/`: ESPN (NFL, NBA,
NHL, college basketball), Coinbase ticker and candles, Kraken ticker and OHLC, USGS (query and
count), SWPC flares and Kp, Launch Library 2, JPL, Treasury (both), Frankfurter, the Federal
Register, drand.

Not readable from here (the fetch tool is refused by their robots rules, so nothing here was
tried another way): api.weather.gov, Steam's API, BLS. Written from their documented shapes;
`pyr-props check` on the VPS says whether they read, and `status.json` after.

Not used: CoinGecko (its terms forbid gambling use by name), Netflix Top 10 and Billboard
(automated collection forbidden), Wikipedia's pageviews (its API answered "not found" to every
try from here: not verified), Apple's chart feeds (terms unread), BLS (unread), the New York
Fed's rates (its newest was three weeks old).

Terms: ESPN's (Disney's) forbid automated access outright; Blue was told so (Nov 17) and the
sports source sits behind one trait (`sports::Scoreboard`) so another can take its place.
Coinbase's and Kraken's market-data terms couldn't be read. Steam's API terms (read) have no
word against this use, ask for Valve's name beside its data, and allow 100,000 calls a day.
Launch Library 2: fifteen calls an hour without a key. NWS, USGS, NOAA, NASA, Treasury, the
Federal Register: public domain.
