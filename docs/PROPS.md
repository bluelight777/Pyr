# Prop Bets, the big version (Pyr 0.15)

Blue, Nov 17 2026: "add the full features of Prop bets… NFL, NBA, NHL, MLB, College Football,
College Basketball, Topical (world events), even allow prop bets for Pyr Casino. Create a
Crypto category, weather, entertainment, science, art. It should update itself regularly…
only play money from Pyr Casino… automatically update without requiring human intervention."
"Make it beautiful and impressive." "Make it mostly automated."

His choices: **public data only, $0** (no Claude API); **a server's owner turns the casino's
own props on** (off until then); and for sports, first ESPN's public feed (Nov 17), then, told
its terms forbid automated access, "use another site that would work other than ESPN if we
can't use it" (Nov 18): of what was put to him (one paid site for everything, three free ones
with a key each, or staying on ESPN), **the three free sites**. See "The games' sites".

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
   two minutes after it ends. Whoever plays moves these numbers (someone set on it can put
   the house ahead by losing on purpose, or make a game the busiest by betting both sides
   over and over), so the casino stakes a tenth as much on them as on the feed's, which bounds
   what that can take; and nothing is asked that one bet decides (the day's biggest win, how
   many rounds are played).
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
| nfl, nba, mlb | balldontlie's games (`api.balldontlie.io`; a key) | per game: winner; the total against what's usual in the league (no bookmaker's lines on its free plan); NFL: margin and first-half leader; NBA: first-quarter leader; MLB: a run in the first |
| cfb, cbb | CollegeFootballData and CollegeBasketballData (`api.collegefootballdata.com`, `api.collegebasketballdata.com`; one key) | the Top 25's games and those with lines: winner, spread and total at the bookmakers' lines, halftime leader |
| nhl | The Odds API (`api.the-odds-api.com`; a key) | per game: winner, and the total against what's usual (5.5 goals): what a final score settles (it says nothing of overtime) |
| crypto | Coinbase's public candles, Kraken's as the spare | BTC, ETH, SOL, XRP, DOGE: up or down each day; BTC, ETH: where it is in two days (five ranges); BTC, ETH, SOL: where it is next Monday |
| weather | National Weather Service: the forecast (api.weather.gov) for the ranges, the daily climate report (CLI) to settle | ten cities: tomorrow's high (five ranges around the forecast), rain or snow |
| science | USGS earthquakes, NOAA SWPC (flares, Kp), Launch Library 2, JPL close approaches | next week: an M6.5+, how many M6+, an X flare, a G2 storm, how many launches, how many asteroids inside the Moon's orbit; each launch: off the pad on time? |
| topical | Treasury (10-year yield, debt to the penny), ECB rates (Frankfurter), the Federal Register | the 10-year on Friday, up or down tomorrow; the debt over its next $0.1 trillion by the month's end; the euro, the yen and the pound on Friday; executive orders next week |
| entertainment | Steam's player counts | the biggest games at 18:00 UTC: over or under; which of two has more on Saturday |
| art | drand (the League of Entropy's public randomness), NASA's Astronomy Picture of the Day | the Gallery Draw: which of six masterpieces is drawn tomorrow; what tomorrow's picture shows |
| casino | each server's own casino (its rounds by the day) | tomorrow: does the house come out ahead; which game takes the most in bets (the week's four biggest, or another); is a jackpot won |
| community | members | their own, in any category |

## The games' sites (Nov 18 2026)

Each league has its own `sports::Scoreboard`, and each of the three sites wants a key of the
account's own, which lives only on the VPS (`/etc/pyr-props/keys/<site>`, readable by the
maker's user alone; it goes as a request header, never in an address or an error). Blue makes
the three free accounts and gives each key on the server, where it's asked for unseen:

    ssh -t root@2.24.121.231 pyr-props-key balldontlie
    ssh -t root@2.24.121.231 pyr-props-key collegefootballdata
    ssh -t root@2.24.121.231 pyr-props-key the-odds-api

(`installer/site/pyr-props-key`: keeps the key, starts the maker again, and asks the site once
so what its leagues read is said there and then.) A league whose site has no key opens nothing
and says so in `status.json`; everything else in the feed needs no key.

| Site | Leagues | Its free plan | How it's asked |
|---|---|---|---|
| balldontlie (app.balldontlie.io) | NFL, NBA, MLB | 5 requests a minute; games with scores by quarter or inning; no lines | a request per league every 5 minutes while its games are on (13 s between two requests), every half hour else |
| CollegeFootballData (collegefootballdata.com/key; the same key at CollegeBasketballData) | college football, college basketball | 1,000 calls a month for both; results, line scores, lines, rankings; no games under way | what's coming twice a day, lines and the Top 25 with it; a day's games once the first should be over (3 h 20 min after it starts, 2 h 5 min), then every 15 and 30 minutes until they are, each settled as it's seen (the halftime leader waits up to three hours for a line score); football keeps to 440 calls a month, basketball to 480 |
| The Odds API (the-odds-api.com) | NHL | 500 credits a month; the list of games free, the scores 2 a look | the list every 6 hours; the scores once a game should be over (2 h 45 min after it starts), then every 50 minutes until it is, settled as it's seen: two looks for a night of two games in the test, 230 a month at most |

A month's calls are counted out as the month goes by (`sports::Budget`, kept in the state
file), so a busy day can't spend what the month's end needs; a game the site never calls over
is asked about for eight hours and then four times a day until it's called off. With no
bookmaker's line a game's total is against what's usual in its league (44.5 points in the NFL,
227.5 in the NBA, 5.5 goals, 8.5 runs), and its rules say so.

Their terms, as read on their own pages (the-odds-api.com/terms-and-conditions.html of Aug 31
2026, balldontlie.io/terms of Oct 20 2026, collegefootballdata.com/terms of Aug 12 2026): all
three allow showing their data in an app, commercial or not, and keeping it; all three forbid
passing it on as a data product or standing in for their API (the feed carries props made
from the games, with who settles them named: not their answers). CollegeFootballData allows
betting-related uses in so many words. balldontlie adds two things Blue was told before he
chose it: its data mustn't be called the leagues' official data (the props say "as
balldontlie reports it"), and it says not to settle wagers on it alone without checking
elsewhere; these are play chips, and nothing here checks its scores against a second source.

What was looked at and not taken: TheSportsDB (one site for all six leagues with a clear yes
to apps and services, but its free key gives three games a day and a published app needs its
$9 a month); the leagues' own feeds (MLB's, the NHL's, the NFL's: personal, non-commercial use
only) and theScore's (the same); the open data sets on GitHub (nflverse, sportsdataverse:
theirs to give, but updated daily at best, and last published in August or September);
SportsBlaze (no terms to read). ESPN's adapter is still in the tree (`sources/espn.rs`, its
tests with it) behind `pyr-props run --espn`, which nothing sets.

## What's checked and what isn't (Nov 17 2026)

Read live through the fetch tool, shapes as in `crates/props-feed/samples/`: ESPN (NFL, NBA,
NHL, college basketball: no longer used), Coinbase ticker and candles, Kraken ticker and OHLC,
USGS (query and count), SWPC flares and Kp, Launch Library 2, JPL, Treasury (both),
Frankfurter, the Federal Register, drand.

**The three games' sites (Nov 18): written from their documentation alone, then read for
real the same night** once Blue had given their keys: all six leagues read at the first try
(359 props in the feed, each well formed; `REBUILD.md` has what was in it). What that found:
a tournament game against "TBD" had been opened; a game with a side that isn't a team yet is
left alone now, and one already open is called off. Before the keys, no answer of theirs had
been read: the tests' answers are made to the field names and values their documentation
shows. Each adapter takes everything as optional but a
game's id, start and teams, says so when games are there and none can be read (rather than
opening nothing quietly), and `pyr-props-key` shows what a site's leagues read the moment its
key is given. Known since: balldontlie's free key reads all three of its leagues, and
CollegeBasketballData's spread is signed as football's (negative: the home team favored). Not
seen yet: a game settling from any of the three, and how soon CollegeFootballData marks one
completed and fills its line scores.

A newer maker goes up without a Pyr release: `scripts/make-props-upload.sh` (the maker, the
set-up and a fresh certificate in `dist/props-upload`; no key in it) and, on Blue's PC,
`publish-props.cmd`.

The Weather Service's daily climate report (forecast.weather.gov's product page, what settles
the weather's props): its layout asked of the fetch tool line by line (the heading's "CLIMATE
SUMMARY FOR NOVEMBER 9 2026", "TEMPERATURE (F)" then "YESTERDAY" then "MAXIMUM 58 …",
"PRECIPITATION (IN)" then "YESTERDAY 0.00 …", version 1 the newest): as `weather::climate`
reads it. (The copy the tool had was a week old: its cache, by the page's own dates.)

Not readable from here (the fetch tool is refused by their robots rules, so nothing here was
tried another way): api.weather.gov (the forecast the weather's props open from), Steam's API,
NASA's picture API, BLS. Written from their documented shapes; `pyr-props check` on the VPS
says whether they read, and `status.json` after (`pyr-props status <file>` for people).

Tried end to end with no network at all: `cargo run -p pyr-props --example demo_feed` makes a
signed, made-up feed in every category (a few props close in minutes; `--results` has them
come out), a test server follows it (`PYR_PROPS_FEED`, `PYR_PROPS_FEED_KEY`), and Pyr with
Prop Bets' window shows, bets, cashes out and is paid.

The VPS's set-up (`installer/site/setup-props.sh`) dry-run with no network and stand-ins for
nginx, curl and systemctl, the rest real (its user, the key's and the website's permissions,
the release's own program, key and certificate; the service started as its user with
everything read-only but its two folders): the first time, again, a wrong certificate, no
key. What it found is in `REBUILD.md` (0.15.0). A source that can't be reached (all of them,
there) is said in `status.json` and asked again in two minutes, then less and less often (a
half hour at most) until it answers; a request is given 25 seconds. The others carry on, and
props that open without asking anyone (the Gallery Draw, the picture of the day, the coins' up
or down) still open.

What a full feed asks (`demo_feed --many 1500`: the real one runs to several hundred props,
1,500 is the most a server lists to an app): a server opens them all in 0.2 s the first time
and looks at them all again in 0.03 s after a restart, on 16 to 26 MB; Prop Bets' window does
5.5 ms of its own work a frame on a board of 375 props; and the three together (the server,
Pyr, the window inside it) show them, take a bet and pay for it. The numbers are in
`REBUILD.md` (0.15.0).

Not used: CoinGecko (its terms forbid gambling use by name), Netflix Top 10 and Billboard
(automated collection forbidden), Wikipedia's pageviews (its API answered "not found" to every
try from here: not verified), Apple's chart feeds (terms unread), BLS (unread), the New York
Fed's rates (its newest was three weeks old).

Terms: ESPN's (Disney's) forbid automated access outright; Blue was told so (Nov 17), and on
Nov 18 had the games moved to sites that allow it ("The games' sites" above).
Coinbase's and Kraken's market-data terms couldn't be read. Steam's API terms (read) have no
word against this use, ask for Valve's name beside its data, and allow 100,000 calls a day.
Launch Library 2: fifteen calls an hour without a key. NWS, USGS, NOAA, NASA, Treasury, the
Federal Register: public domain.
