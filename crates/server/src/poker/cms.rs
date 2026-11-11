//! Pyr Casino's control room (0.13.5), as 0.13.28 has it: the casino at a glance, every player
//! (look them up, give or take chips: always in the log, with the balance before and after),
//! the whole log, a word to everyone in the casino, and who else may open it. It's the Owner of
//! the Game's, and whoever they let in (`cms_access`, kept as 0.13.28 keeps it).

use pyr_poker::levels;
use pyr_poker::net::{CmsOverview, CmsPlayer, CmsReq, CmsRes, PeriodStats, StaffLog, ToApp};
use tracing::info;

use super::{Poker, STAFF_LOG, State, casino, progress_chips};
use crate::hub::Hub;

/// Who else may open the control room (their ids, in the order they were let in).
const ACCESS: &str = "cms_access";
/// What the overview shows of the biggest wins and of the log, and the most players a look-up
/// lists (most chips first).
const BIG_WINS: usize = 8;
const RECENT: usize = 12;
const PLAYERS: usize = 100;
/// The longest note for the log, and the longest word to everyone.
const NOTE_MAX: usize = 140;
const TEXT_MAX: usize = 280;

/// A note as the log quotes it: ` (“note”)`, or nothing.
pub(super) fn quoted(note: &str) -> String {
    if note.is_empty() { String::new() } else { format!(" (\u{201c}{note}\u{201d})") }
}

impl Poker {
    /// Who else may open the control room.
    pub(super) fn cms_people(hub: &Hub) -> Vec<String> {
        hub.db.meta_json(ACCESS).unwrap_or_default()
    }

    /// Whether `uid` may open the control room: the Owner of the Game, or someone they let in.
    pub(super) fn can_cms(hub: &Hub, uid: &str) -> bool {
        Self::is_game_owner(hub, uid) || Self::cms_people(hub).iter().any(|p| p == uid)
    }

    fn the_log(hub: &Hub) -> Vec<StaffLog> {
        hub.db.meta_json(STAFF_LOG).unwrap_or_default()
    }

    /// The whole log, newest first: the Owner of the Game's (and the control room's).
    pub(super) fn staff_log_all(hub: &Hub, uid: &str) -> Result<Vec<StaffLog>, String> {
        if !Self::can_cms(hub, uid) {
            return Err("only the Owner of the Game (and the control room) reads the staff log".into());
        }
        Ok(Self::the_log(hub))
    }

    pub(super) fn cms(&self, hub: &Hub, st: &mut State, uid: &str, req: CmsReq) -> Result<CmsRes, String> {
        if !Self::can_cms(hub, uid) {
            return Err("the control room is the Owner of the Game's (and whoever they let in)".into());
        }
        match req {
            CmsReq::Overview => Ok(CmsRes::Overview(Self::cms_overview(hub, st))),
            CmsReq::Players { query } => {
                let query = query.trim().to_lowercase();
                let mut rows: Vec<CmsPlayer> = hub
                    .db
                    .poker_records()
                    .into_iter()
                    .filter_map(|(them, rec, _)| {
                        let name = hub.db.get_user(&them)?.name;
                        if !query.is_empty() && !name.to_lowercase().contains(&query) {
                            return None;
                        }
                        let (rounds, wagered, paid) = rec.casino.values().fold((0, 0, 0), |(r, w, p), c| (r + c.rounds, w + c.wagered, p + c.paid));
                        Some(CmsPlayer {
                            at_tables: st.held(&them),
                            role: Self::role(hub, &them),
                            uid: them,
                            name,
                            chips: rec.chips,
                            level: rec.level(),
                            hands: rec.hands,
                            rounds,
                            wagered,
                            paid,
                            ember_claimed: rec.ember_claimed,
                        })
                    })
                    .collect();
                rows.sort_by(|a, b| b.chips.cmp(&a.chips).then_with(|| a.name.cmp(&b.name)));
                rows.truncate(PLAYERS);
                Ok(CmsRes::Players { rows })
            }
            CmsReq::Adjust { uid: them, delta, note } => {
                if delta == 0 {
                    return Err("give or take some chips".into());
                }
                if hub.db.get_user(&them).is_none() {
                    return Err("there's nobody by that id here".into());
                }
                let note: String = note.trim().chars().take(NOTE_MAX).collect();
                let (mut rec, _) = hub.db.poker_record(&them);
                let before = rec.chips;
                // (Taken: as many as they have, at most.)
                let n = if delta > 0 { delta.unsigned_abs() } else { delta.unsigned_abs().min(rec.chips) };
                rec.chips = if delta > 0 { rec.chips.saturating_add(n) } else { rec.chips - n };
                let held = st.held(&them);
                hub.db.set_poker_record(&them, &rec, held).map_err(|e| e.to_string())?;
                hub.send_poker(&them, ToApp::Record { me: Self::view(hub, &them, &rec, held), gains: None });
                let name = Self::name_of(hub, &them);
                let what = format!(
                    "{} {} chips {} {name} (control room){} · {} → {}",
                    if delta > 0 { "gave" } else { "took" },
                    progress_chips(n),
                    if delta > 0 { "to" } else { "from" },
                    quoted(&note),
                    progress_chips(before),
                    progress_chips(rec.chips)
                );
                self.log_staff(hub, uid, what.clone());
                Ok(CmsRes::Done { what })
            }
            CmsReq::Announce { text } => {
                let text: String = text.trim().chars().take(TEXT_MAX).collect();
                if text.is_empty() {
                    return Err("say something".into());
                }
                info!("poker control room: announced");
                self.log_staff(hub, uid, format!("announced: \u{201c}{text}\u{201d}"));
                hub.broadcast_poker_from(pyr_poker::net::CASINO_013, ToApp::Announcement { from: Self::name_of(hub, uid), text });
                Ok(CmsRes::Done { what: "Said to everyone in the casino".into() })
            }
            CmsReq::Access => Ok(Self::cms_access_view(hub, uid)),
            CmsReq::SetAccess { uid: them, on } => {
                if !Self::is_game_owner(hub, uid) {
                    return Err("only the Owner of the Game lets people into the control room".into());
                }
                if hub.db.get_user(&them).is_none() {
                    return Err("there's nobody by that id here".into());
                }
                let mut people = Self::cms_people(hub);
                let had = people.iter().any(|p| *p == them);
                if on != had {
                    if on {
                        people.push(them.clone());
                    } else {
                        people.retain(|p| *p != them);
                    }
                    hub.db.set_meta(ACCESS, &serde_json::to_string(&people).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                    let name = Self::name_of(hub, &them);
                    self.log_staff(hub, uid, if on { format!("let {name} into the control room") } else { format!("took {name} out of the control room") });
                    // (Their Pyr learns it at once: the control room shows, or goes.)
                    hub.send_poker(&them, ToApp::Staff { view: Self::staff_view(hub, &them, false) });
                }
                Ok(Self::cms_access_view(hub, uid))
            }
        }
    }

    fn cms_access_view(hub: &Hub, uid: &str) -> CmsRes {
        CmsRes::Access { owner: Self::is_game_owner(hub, uid), people: Self::cms_people(hub).into_iter().map(|p| (Self::name_of(hub, &p), p)).collect() }
    }

    fn cms_overview(hub: &Hub, st: &State) -> CmsOverview {
        let records = hub.db.poker_records();
        let mut by_tier = vec![0u64; levels::TIERS.len()];
        let mut tier_players = vec![0u64; levels::TIERS.len()];
        let (mut chips, mut at_tables) = (0u64, 0u64);
        for (them, rec, _) in &records {
            chips += rec.chips;
            at_tables += st.held(them);
            let t = rec.tier().min(by_tier.len() - 1);
            by_tier[t] += rec.chips;
            tier_players[t] += 1;
        }
        // Every game's all-time numbers (the Reel Row machines each their own).
        let (mut wagered, mut paid, mut rounds) = (0u64, 0u64, 0u64);
        for (_, v) in hub.db.meta_with_prefix(casino::ALL_TIME) {
            if let Ok(p) = serde_json::from_str::<PeriodStats>(&v) {
                wagered += p.wagered;
                paid += p.paid;
                rounds += p.rounds;
            }
        }
        let log = Self::the_log(hub);
        CmsOverview {
            players: records.len() as u64,
            chips,
            at_tables,
            wagered,
            paid,
            rounds,
            rake_week: Self::rake_week(hub),
            friday_pool: Self::friday_pool(hub),
            jackpot: Self::jackpot(hub),
            tables: st.rooms.len() as u64,
            tourneys: st.tourneys.len() as u64,
            staff: Self::roles(hub).len() as u64,
            log: log.len() as u64,
            by_tier,
            tier_players,
            big_wins: Self::big_wins(hub).into_iter().take(BIG_WINS).collect(),
            recent: log.into_iter().take(RECENT).collect(),
        }
    }
}
