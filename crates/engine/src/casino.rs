//! Pyr Casino as this Pyr sees it on a server: my chips there, the house's fingerprint for my
//! next round, the jackpot, big wins, my blackjack hand and craps table, and every round that
//! came back, each one checked (this Pyr plays it again from the seeds and compares).

use std::collections::{BTreeMap, VecDeque};

use pyr_poker::casino::{self, blackjack, craps};
use pyr_poker::net::{BigWin, CasinoRes, RoundView};
use pyr_poker::progress::CasinoStats;

/// Rounds kept for the history panes.
pub const ROUNDS_KEPT: usize = 80;

#[derive(Clone, Debug, Default)]
pub struct CasinoLive {
    /// The casino's state came (it's open on this server).
    pub open: bool,
    pub chips: u64,
    /// The fingerprint of the house's seed for my next round, and the round's number.
    pub commit: Option<[u8; 32]>,
    pub nonce: u64,
    /// Every fingerprint the house showed me, by round (each round is checked against the one
    /// shown before it).
    pub shown: BTreeMap<u64, [u8; 32]>,
    pub jackpot: u64,
    pub friday_ms: u64,
    pub friday_prize: u64,
    /// Newest first.
    pub big_wins: Vec<BigWin>,
    /// Big wins heard since I opened the casino (the app shows each once).
    pub big_win_seq: u64,
    pub blackjack: Option<blackjack::View>,
    pub craps: craps::Table,
    pub stats: BTreeMap<String, CasinoStats>,
    /// Rounds that came back, oldest first, and how many have come altogether.
    pub rounds: VecDeque<RoundView>,
    pub round_seq: u64,
    /// Rounds this Pyr played again and found exactly as the house said; and any that weren't.
    pub checked: u64,
    pub problems: Vec<String>,
    /// A request is on its way (the app waits before the next).
    pub busy: bool,
    /// Answers to my requests (or refusals) so far: the app's wait is over when it changes.
    pub answers: u64,
}

impl CasinoLive {
    fn remember(&mut self, nonce: u64, commit: [u8; 32]) {
        self.commit = Some(commit);
        self.nonce = nonce;
        self.shown.insert(nonce, commit);
        while self.shown.len() > 200 {
            let first = *self.shown.keys().next().unwrap();
            self.shown.remove(&first);
        }
    }

    pub fn apply(&mut self, res: CasinoRes) {
        if !matches!(res, CasinoRes::BigWin { .. } | CasinoRes::Jackpot { .. }) {
            self.answers += 1;
        }
        match res {
            CasinoRes::State { view } => {
                self.open = true;
                self.chips = view.chips;
                self.remember(view.nonce, view.commit);
                self.jackpot = view.jackpot;
                self.friday_ms = view.friday_ms;
                self.friday_prize = view.friday_prize;
                self.big_wins = view.big_wins;
                self.blackjack = view.blackjack;
                self.craps = view.craps;
                self.stats = view.stats;
                self.busy = false;
            }
            CasinoRes::Round { round } => {
                let round = *round;
                match self.shown.get(&round.nonce) {
                    Some(c) => match casino::verify(c, &round.server_seed, &round.client_seed, round.nonce, &round.outcome) {
                        Ok(()) => self.checked += 1,
                        Err(e) => self.problems.push(format!("Round {}: {e}", round.nonce)),
                    },
                    // (A round from before this Pyr opened the casino: nothing to check it by.)
                    None => {}
                }
                self.chips = round.chips;
                self.jackpot = round.jackpot;
                self.remember(round.next_nonce, round.next_commit);
                match round.outcome.game() {
                    casino::Game::Blackjack => self.blackjack = None,
                    casino::Game::Craps => {
                        if let Some(t) = &round.craps {
                            self.craps = t.clone();
                        }
                    }
                    _ => {}
                }
                let st = self.stats.entry(round.outcome.game().id().to_string()).or_default();
                st.rounds += 1;
                st.wagered += round.outcome.wagered();
                st.paid += round.outcome.paid();
                self.rounds.push_back(round);
                while self.rounds.len() > ROUNDS_KEPT {
                    self.rounds.pop_front();
                }
                self.round_seq += 1;
                self.busy = false;
            }
            CasinoRes::Blackjack { view, chips, next_commit, next_nonce } => {
                self.blackjack = Some(view);
                self.chips = chips;
                self.remember(next_nonce, next_commit);
                self.busy = false;
            }
            CasinoRes::Craps { table, chips } => {
                self.craps = table;
                self.chips = chips;
                self.busy = false;
            }
            CasinoRes::BigWin { win } => {
                self.big_wins.insert(0, win);
                self.big_wins.truncate(30);
                self.big_win_seq += 1;
            }
            CasinoRes::Jackpot { pool } => self.jackpot = pool,
        }
    }

    /// The last round of a game.
    pub fn last(&self, game: casino::Game) -> Option<&RoundView> {
        self.rounds.iter().rev().find(|r| r.outcome.game() == game)
    }
}
