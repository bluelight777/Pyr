//! Games and other programs from the Pyr Library, talking to Pyr (0.13.15 to 0.13.27; the
//! lines themselves are in `plugins::programs`): each is told my Pyr name and friends (and
//! again whenever that changes), what Pyr can do for it and the Library's newest version of it
//! (when it opens and every ten minutes). What it asks for is done here: a friend invited to
//! its room gets a direct message with the room's link (never waiting on the window), and an
//! update asked for closes it, updates it and opens it again in the same room. Joining from a
//! game's invite installs or updates the game first when it needs that.

use std::time::{Duration, Instant};

use pyr_engine::state::AppState;
use pyr_proto::control::RelationKind;
use serde_json::json;

use crate::app::{Act, PyrApp};
use crate::plugins::install::Launch;
use crate::plugins::programs::{Asked, Closed};

/// Where an update of a program stands (0.13.27).
#[derive(Clone, Debug, PartialEq)]
pub enum UpStage {
    /// Waiting for it to close.
    Closing,
    /// Fetching the Library's list again (an older list may not know the new version).
    Fetching,
    /// Installing the new version.
    Installing,
}

#[derive(Clone, Debug)]
pub struct ProgramUpdate {
    pub id: String,
    /// The room it goes back into.
    pub room: Option<String>,
    pub stage: UpStage,
    pub since: Instant,
}

/// My friends, for a program: (uid, name, online), by name.
pub fn friends_of(st: &AppState) -> Vec<(String, String, bool)> {
    let mut out: Vec<(String, String, bool)> = st
        .relations()
        .into_iter()
        .filter(|(_, k)| *k == RelationKind::Friend)
        .map(|(uid, _)| {
            let online = st.servers.iter().any(|s| s.members.get(&uid).is_some_and(|m| m.online));
            (uid.clone(), st.name_of(&uid), online)
        })
        .collect();
    out.sort_by_key(|f| f.1.to_lowercase());
    out
}

impl PyrApp {
    /// Each frame: open programs hear my friends and their newest versions; what they asked
    /// for is done; updates carry on; programs that closed are let go of.
    pub(crate) fn programs_talk(&mut self, st: &AppState, closed: Vec<Closed>, acts: &mut Vec<Act>) {
        for c in closed {
            self.program_closed(c, acts);
        }
        if !self.plugins.programs.open.is_empty() {
            let friends = friends_of(st);
            self.plugins.programs.tell_friends(&st.my_name, &friends);
            let newest: std::collections::HashMap<String, String> =
                self.plugins.programs.open.iter().filter_map(|p| self.plugins.newest_version(&p.id).map(|v| (p.id.clone(), v))).collect();
            // (The list is fetched again every ten minutes while one's open: `refresh` keeps to
            // that.)
            self.plugins.programs.tell_newest(&|id: &str| newest.get(id).cloned());
            self.plugins.refresh(false);
        }
        // Prop Bets' own window: the props of the server showing in Pyr, as they change.
        if self.plugins.programs.get(crate::props::PROP_BETS).is_some() {
            self.tell_prop_bets(st);
        } else {
            self.ui.prop_bets_told = None;
            self.ui.prop_bets_v = 0;
            self.ui.prop_bets_listed = None;
        }
        for (id, a) in self.plugins.programs.take_asked() {
            match a {
                Asked::Invite { uid, room } => self.invite_to_game(st, &id, uid, room),
                Asked::Update { room } => self.update_program(&id, room),
                Asked::Prop(ask) if id == crate::props::PROP_BETS => self.prop_bets_asked(st, ask, acts),
                Asked::Prop(_) => tracing::warn!(program = %id, "a program other than Prop Bets asked about prop bets: not done"),
            }
        }
        self.updates_step(acts);
    }

    /// Tells Prop Bets' own window the props of the server showing in Pyr (every channel I can
    /// read, and the casino's own: asked for when it opens or the server changes, and again
    /// every few minutes), my chips, its text channels, the best at props and how the casino's
    /// own props stand there: when any of it changed since it was last told. A window that
    /// said so (`props_hello`) is told only the props that changed; one that didn't, every
    /// prop each time.
    fn tell_prop_bets(&mut self, st: &AppState) {
        use pyr_proto::control::{PROPS_FEATURE, PROPS_HOUSE_FEATURE};
        let s = self.ui.server.as_deref().and_then(|k| st.server(k));
        let now = s.map(|s| (s.key.clone(), s.poker.props_version, s.poker.me.as_ref().map(|m| m.chips), s.has(PROPS_FEATURE)));
        // (The server tells this Pyr of the casino's own props as they change only for a
        // quarter of an hour after it asked for them all.)
        if let Some(s) = s.filter(|s| s.has(PROPS_HOUSE_FEATURE))
            && self.ui.prop_bets_listed.is_some_and(|t| t.elapsed() > Duration::from_secs(300))
        {
            self.engine.prop_list_all(&s.key);
            self.ui.prop_bets_listed = Some(Instant::now());
        }
        if self.ui.prop_bets_told.as_ref() == Some(&now) {
            return;
        }
        let first = self.ui.prop_bets_told.is_none();
        let before = self.ui.prop_bets_told.replace(now).flatten();
        // A server it hasn't seen yet: every prop, my chips, and how the casino's own stand.
        let new_server = s.filter(|s| before.as_ref().map(|t| &t.0) != Some(&s.key));
        if let Some(s) = new_server {
            self.engine.prop_list_all(&s.key);
            self.ui.prop_bets_listed = Some(Instant::now());
            if s.poker.me.is_none() && s.has(PROPS_FEATURE) {
                self.engine.poker(&s.key, pyr_poker::net::ToServer::Lobby);
            }
            self.engine.prop_auto(&s.key, None);
        }
        let line = match s {
            Some(s) => {
                let channels: Vec<serde_json::Value> = s
                    .channels
                    .iter()
                    .filter(|c| c.kind == pyr_proto::control::ChannelKind::Text)
                    .map(|c| json!({"id": c.id, "name": c.name}))
                    .collect();
                let full = self.ui.prop_bets_v < 2 || first || new_server.is_some();
                let since = before.as_ref().map_or(0, |t| t.1);
                let props: Vec<&pyr_poker::props::PropView> = if full {
                    s.poker.props.values().rev().collect()
                } else {
                    s.poker.props.iter().rev().filter(|(id, _)| s.poker.props_at.get(id).is_some_and(|at| *at > since)).map(|(_, p)| p).collect()
                };
                json!({
                    "op": "props",
                    "full": full,
                    "server": {"key": s.key, "name": s.info.name},
                    "props_here": s.has(PROPS_FEATURE),
                    "house_here": s.has(PROPS_HOUSE_FEATURE),
                    "me": st.my_uid,
                    "chips": s.poker.me.as_ref().map(|m| m.chips),
                    "channels": channels,
                    "channel": self.ui.channel.get(&s.key),
                    "list": props,
                    "stats": s.poker.prop_stats,
                    "auto": s.poker.prop_auto,
                })
            }
            None => json!({"op": "props", "full": true, "server": null, "props_here": false, "list": []}),
        };
        self.plugins.programs.tell(crate::props::PROP_BETS, line);
    }

    /// What Prop Bets' own window asked: done on the server it was last told about, as a
    /// prop's card in chat does it (the server checks it all).
    fn prop_bets_asked(&mut self, st: &AppState, ask: crate::plugins::programs::PropAsk, acts: &mut Vec<Act>) {
        use crate::plugins::programs::PropAsk;
        if let PropAsk::Hello { v } = ask {
            self.ui.prop_bets_v = v;
            return;
        }
        let Some(key) = self.ui.prop_bets_told.as_ref().and_then(|t| t.as_ref()).map(|t| t.0.clone()) else { return };
        acts.push(match ask {
            PropAsk::Bet { id, outcome, chips, min_shares } => Act::PropBet { key, id, outcome, chips, min_shares },
            PropAsk::CashOut { id, outcome } => Act::PropCashOut { key, id, outcome },
            PropAsk::Open { channel, question, outcomes, depth, closes_in, category } => Act::PropOpen { key, channel, question, outcomes, depth, closes_in, category },
            PropAsk::Settle { id, outcome } => Act::PropSettle { key, id, outcome },
            PropAsk::Void { id } => Act::PropVoid { key, id },
            PropAsk::Refresh => {
                self.engine.prop_list_all(&key);
                self.ui.prop_bets_listed = Some(Instant::now());
                return;
            }
            PropAsk::Hello { .. } => return,
            PropAsk::Look { id } => {
                self.engine.prop_look(&key, id);
                return;
            }
            PropAsk::Stats { days } => {
                self.engine.prop_stats(&key, days);
                return;
            }
            PropAsk::Auto { set } => {
                self.engine.prop_auto(&key, set);
                return;
            }
            PropAsk::Share { id } => {
                // Its card, in the text channel showing in Pyr.
                let Some(s) = st.server(&key) else { return };
                let Some(prop) = s.poker.props.get(&id) else { return };
                let showing = self.ui.channel.get(&key).and_then(|c| s.channels.iter().find(|ch| ch.id == *c && ch.kind == pyr_proto::control::ChannelKind::Text));
                let said = match showing {
                    Some(ch) => {
                        acts.push(Act::Send { key: key.clone(), channel: ch.id, text: crate::props::message_for(prop), reply_to: None, files: Vec::new() });
                        json!({"op": "said", "text": format!("Shared in #{}.", ch.name), "error": false})
                    }
                    None => json!({"op": "said", "text": "Go to a text channel in Pyr first: it's shared there.", "error": true}),
                };
                self.plugins.programs.tell(crate::props::PROP_BETS, said);
                return;
            }
        });
    }

    /// A friend invited to a game's room: a direct message from me with its link (shown to
    /// them as the game's card with Join). The game hears how it went.
    fn invite_to_game(&mut self, st: &AppState, game: &str, uid: String, room: String) {
        let name = self.plugins.installed.get(game).map(|p| p.name.clone()).unwrap_or_else(|| game.to_string());
        let text = crate::play_invites::invite_text(&st.my_name, &name, game, &room);
        let teller = self.plugins.programs.teller(game);
        let who = uid.clone();
        self.engine.message_friend(&uid, text, move |r| {
            if let Err(e) = &r {
                tracing::warn!("Couldn't send the game invite to {who}: {e}");
            }
            if let Some(t) = teller {
                let line = match r {
                    Ok(()) => json!({"op": "invited", "uid": who, "ok": true}),
                    Err(e) => json!({"op": "invited", "uid": who, "ok": false, "why": e}),
                };
                let _ = t.send(line.to_string());
            }
        });
    }

    /// Updates a program (0.13.27): closes it if it's open, fetches the Library's list again,
    /// installs the newer version and opens it again (in `room`). With nothing newer, it opens
    /// again as it was, and Pyr says so.
    pub(crate) fn update_program(&mut self, id: &str, room: Option<String>) {
        if self.ui.program_updates.iter().any(|u| u.id == id) {
            return;
        }
        let open = self.plugins.programs.get(id).is_some();
        if open {
            self.plugins.programs.close_to_update(id, room.clone());
        }
        self.plugins.refetch();
        self.ui.program_updates.push(ProgramUpdate { id: id.to_string(), room, stage: if open { UpStage::Closing } else { UpStage::Fetching }, since: Instant::now() });
    }

    fn updates_step(&mut self, acts: &mut Vec<Act>) {
        let mut done = Vec::new();
        for i in 0..self.ui.program_updates.len() {
            let u = self.ui.program_updates[i].clone();
            match u.stage {
                UpStage::Closing => {
                    if self.plugins.programs.get(&u.id).is_none() {
                        self.ui.program_updates[i].stage = UpStage::Fetching;
                    } else if u.since.elapsed() > Duration::from_secs(15) {
                        // (Asked to close, it didn't: it's ended.)
                        self.plugins.programs.kill(&u.id);
                    }
                }
                UpStage::Fetching => {
                    if self.plugins.fetching() {
                        continue;
                    }
                    let entry = self.plugins.installed.get(&u.id).cloned().and_then(|p| self.plugins.update_for(&p).cloned());
                    match entry {
                        Some(e) => {
                            self.plugins.install(&e);
                            self.ui.program_updates[i].stage = UpStage::Installing;
                        }
                        None => {
                            let (name, version) = self.plugins.installed.get(&u.id).map(|p| (p.name.clone(), p.version.clone())).unwrap_or_default();
                            acts.push(Act::Toast(format!("{name} {version} is the newest in the Pyr Library: it opens again as it was."), false));
                            done.push((u.id.clone(), u.room.clone()));
                        }
                    }
                }
                UpStage::Installing => {
                    if self.plugins.progress(&u.id).is_none() {
                        done.push((u.id.clone(), u.room.clone()));
                    }
                }
            }
        }
        for (id, room) in done {
            self.ui.program_updates.retain(|u| u.id != id);
            acts.push(Act::OpenProgram { id, room });
        }
    }

    /// A program closed: said, if it was by mistake (with its last errors in pyr.log).
    fn program_closed(&mut self, c: Closed, acts: &mut Vec<Act>) {
        if c.mistake {
            tracing::warn!(program = %c.id, code = ?c.code, "{} closed by mistake. The last it wrote to its errors:\n{}", c.name, c.errors.join("\n"));
            let code = c.code.map(|c| format!(" ({c})")).unwrap_or_default();
            acts.push(Act::Toast(format!("{} closed by mistake{code}.", c.name), true));
        } else {
            tracing::info!(program = %c.id, updating = c.updating.is_some(), "{} closed", c.name);
        }
    }

    /// Join from a game's invite (`https://pyr.gg/<game>/<ROOM>`): opens the game in that room,
    /// updating it first when the Library has a newer version; one that isn't installed is
    /// shown in the Library, and opens in that room once it's installed.
    pub(crate) fn play_invite(&mut self, game: &str, room: &str, acts: &mut Vec<Act>) {
        let installed = self.plugins.installed.get(game).cloned();
        match installed {
            Some(p) => {
                if self.plugins.update_for(&p).is_some() {
                    // (0.13.28's words, as it goes.)
                    acts.push(Act::Toast(format!("Updating {} first, then into the room.", p.name), false));
                    self.update_program(game, Some(room.to_string()));
                } else {
                    acts.push(Act::OpenProgram { id: game.to_string(), room: Some(room.to_string()) });
                }
            }
            None => match self.plugins.library.as_ref().and_then(|l| l.find(game)).cloned() {
                Some(e) => {
                    self.ui.pending_play = Some((game.to_string(), room.to_string()));
                    self.plugins.detail = Some(game.to_string());
                    self.ui.plugins_open = true;
                    self.ui.home = false;
                    self.plugins.program_page = None;
                    acts.push(Act::Toast(format!("You're invited to play in room {room}. Install {} and it opens in that room.", e.name), false));
                }
                None => {
                    self.plugins.refetch();
                    acts.push(Act::Toast("That game isn't in the Pyr Library here (yet): try again in a moment.".into(), true));
                }
            },
        }
    }

    /// A plugin finished installing: a game someone invited me to opens in their room.
    pub(crate) fn plugin_installed(&mut self, id: &str, acts: &mut Vec<Act>) {
        if let Some((game, room)) = self.ui.pending_play.clone() {
            if game == id && !self.ui.program_updates.iter().any(|u| u.id == id) {
                self.ui.pending_play = None;
                acts.push(Act::OpenProgram { id: game, room: Some(room) });
            }
        }
    }

    /// Opens a program from Pyr (in a room, from an invite or after an update).
    pub(crate) fn open_program(&mut self, id: &str, room: Option<String>, my_name: &str) -> Result<(), String> {
        let launch = Launch { name: my_name.to_string(), join: room };
        self.plugins.open_program(id, &launch)
    }
}
