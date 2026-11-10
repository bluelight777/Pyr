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
        for (id, a) in self.plugins.programs.take_asked() {
            match a {
                Asked::Invite { uid, room } => self.invite_to_game(st, &id, uid, room),
                Asked::Update { room } => self.update_program(&id, room),
            }
        }
        self.updates_step(acts);
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
