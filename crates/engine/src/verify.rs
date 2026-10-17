//! Making sure people are who they say ("Make sure it's them"; see `pyr_proto::handshake`):
//! comparing words, or asking something only they'd know, through a server both are on. Then:
//! the people I made sure of (`Config::verified`), my vouches for them on the servers we share,
//! the vouches of theirs I fetch ("vouched for by Mara"), and the alarm when someone else shows
//! up under the name of someone I made sure of (`AppState::trust`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pyr_proto::control::{Event, HANDSHAKE_FEATURE, MAX_VOUCHES, MAX_VOUCHES_ASKED, Request, VOUCHES_FEATURE, VOUCHES_FOR_ME_FEATURE, Vouch};
use pyr_proto::handshake::{self, ANSWER_MAX, Asking, Handshake, QUESTION_MAX, WordsJoiner, WordsStarter};

use crate::Shared;
use crate::config::{Verified, VerifiedHow};
use crate::state::{ConnStatus, HandshakeKind, HandshakeStage, HandshakeView};

/// How long a handshake waits on the other end before it's given up.
const GIVE_UP_AFTER: Duration = Duration::from_secs(10 * 60);
/// Questions to one person whose answers didn't match, an hour (so someone pretending to be
/// them gets few guesses).
const FAILED_ASKS_AN_HOUR: usize = 3;

/// Handshakes under way, by id, and questions that didn't match lately, by person.
#[derive(Default)]
pub struct Book {
    pending: HashMap<String, Pending>,
    failed_asks: HashMap<String, Vec<Instant>>,
    /// Word comparisons I said matched, a while (by id: the server, who with, and when I made
    /// sure of them): if they then say theirs didn't, it doesn't count.
    said_match: HashMap<String, (String, String, i64, Instant)>,
}

struct Pending {
    server: String,
    with: String,
    at: Instant,
    step: Step,
}

enum Step {
    /// I asked to compare words.
    WordsStarted(WordsStarter),
    /// They asked to compare words (their commitment), and I haven't said yes yet.
    WordsOffered { commit: String },
    /// I said yes, and wait for their key.
    WordsJoined(WordsJoiner),
    /// I asked them a question.
    Asked(Asking),
    /// They asked me a question I haven't answered yet.
    AskCame { question: String, ya: String },
    /// I answered: what their confirmation is if the answers matched.
    Answered([u8; 32]),
    /// Nothing more comes (the words are showing, or it's over).
    Done,
}

/// A name as it's compared with the names of people I made sure of: lowercase, letters and
/// digits that pass for other letters as those letters, and nothing but letters and digits
/// ("Mаra" with a Cyrillic а, "M4ra", "mara!" and "Mara" are all the same).
pub fn fold(name: &str) -> String {
    let s: String = name
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            '0' | 'ο' | 'о' | 'ö' | 'ó' | 'ò' | 'ô' | 'õ' => 'o',
            '1' | '|' | 'i' | 'ı' | 'í' | 'ì' | 'ï' | 'î' | 'ι' | 'і' | 'ӏ' => 'l',
            '3' | 'е' | 'ё' | 'é' | 'è' | 'ê' | 'ë' | 'ε' => 'e',
            '4' | '@' | 'а' | 'α' | 'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            '5' | '$' | 'ѕ' => 's',
            '7' | 'τ' | 'т' => 't',
            '8' | 'в' | 'β' => 'b',
            '9' | 'ɡ' => 'g',
            'р' | 'ρ' => 'p',
            'с' | 'ç' => 'c',
            'х' | 'χ' => 'x',
            'у' | 'ý' | 'ÿ' => 'y',
            'к' | 'κ' => 'k',
            'м' => 'm',
            'н' | 'η' => 'n',
            'ν' => 'v',
            'ј' => 'j',
            'ԁ' => 'd',
            'ú' | 'ù' | 'û' | 'ü' | 'υ' => 'u',
            c => c,
        })
        .filter(|c| c.is_alphanumeric())
        .collect();
    // (And pairs that pass for one letter: "rn" for "m", "vv" for "w".)
    s.replace("rn", "m").replace("vv", "w")
}

/// Drops handshakes that waited too long; their cards say so.
fn give_up_on_old(shared: &Shared) {
    let gone: Vec<String> = {
        let mut book = shared.verify.lock();
        let gone: Vec<String> = book.pending.iter().filter(|(_, p)| p.at.elapsed() > GIVE_UP_AFTER).map(|(sid, _)| sid.clone()).collect();
        for sid in &gone {
            book.pending.remove(sid);
        }
        book.failed_asks.retain(|_, v| {
            v.retain(|t| t.elapsed() < Duration::from_secs(3600));
            !v.is_empty()
        });
        book.said_match.retain(|_, (_, _, _, at)| at.elapsed() < GIVE_UP_AFTER);
        gone
    };
    if gone.is_empty() {
        return;
    }
    let mut st = shared.state.write();
    for v in st.handshakes.iter_mut().filter(|v| gone.contains(&v.sid)) {
        if matches!(v.stage, HandshakeStage::Asked | HandshakeStage::Waiting) {
            v.stage = HandshakeStage::Failed("It went unanswered for a while, so it stopped.".into());
        }
    }
}

fn set_stage(shared: &Shared, sid: &str, stage: HandshakeStage) {
    let mut st = shared.state.write();
    if let Some(v) = st.handshakes.iter_mut().find(|v| v.sid == sid) {
        v.stage = stage;
    }
    drop(st);
    (shared.repaint)();
}

/// It's over (with what to show).
fn end(shared: &Shared, sid: &str, stage: HandshakeStage) {
    if let Some(p) = shared.verify.lock().pending.get_mut(sid) {
        p.step = Step::Done;
    }
    set_stage(shared, sid, stage);
}

/// Sends `msg` to `to` through `server`; if it can't be, the handshake ends saying why.
fn send(shared: &Arc<Shared>, server: &str, to: &str, msg: &Handshake) {
    let sid = msg.sid().to_string();
    let Some(h) = shared.servers.lock().get(server).cloned() else {
        end(shared, &sid, HandshakeStage::Failed("That server isn't connected.".into()));
        return;
    };
    let reply = h.request(Request::Handshake { to: to.to_string(), data: msg.to_json() });
    let s = shared.clone();
    shared.rt.spawn(async move {
        if let Ok(Event::Error { message, .. }) = reply.await {
            end(&s, &sid, HandshakeStage::Failed(message));
        }
    });
}

/// My id, if `with` can be reached through `key` to make sure of them (otherwise a notice says
/// why not).
fn reachable(shared: &Shared, key: &str, with: &str) -> Option<String> {
    let mut st = shared.state.write();
    let me = st.my_uid.clone();
    let why = match st.server(key) {
        None => Some("That server isn't one of yours.".to_string()),
        Some(s) if s.status != ConnStatus::Connected => Some(format!("{} isn't connected right now.", s.name)),
        Some(s) if !s.has(HANDSHAKE_FEATURE) => Some(format!("{} needs updating to Pyr 0.7.6 before people can make sure of each other through it.", s.name)),
        Some(s) if !s.members.get(with).is_some_and(|m| m.online) => Some(format!("{} isn't online right now: try when they are.", s.member_name(with))),
        _ if with == me => Some("That's you.".to_string()),
        _ => None,
    };
    match why {
        Some(why) => {
            st.toast(why, false);
            drop(st);
            (shared.repaint)();
            None
        }
        None => Some(me),
    }
}

/// A handshake I start: kept, shown, and sent.
#[allow(clippy::too_many_arguments)]
fn open(shared: &Arc<Shared>, key: &str, with: &str, kind: HandshakeKind, question: String, msg: &Handshake, step: Step) {
    give_up_on_old(shared);
    let sid = msg.sid().to_string();
    shared.verify.lock().pending.insert(sid.clone(), Pending { server: key.into(), with: with.into(), at: Instant::now(), step });
    show(shared, HandshakeView { sid, server: key.into(), with: with.into(), kind, mine: true, question, stage: HandshakeStage::Waiting, at: Instant::now() });
    send(shared, key, with, msg);
}

/// Shows a handshake: one card per person (a newer one takes the last one's place).
fn show(shared: &Shared, view: HandshakeView) {
    let mut st = shared.state.write();
    st.handshakes.retain(|v| v.with != view.with);
    st.handshakes.push(view);
    drop(st);
    (shared.repaint)();
}

/// Start comparing words with `with`, through server `key`.
pub fn compare_words(shared: &Arc<Shared>, key: &str, with: &str) {
    let Some(me) = reachable(shared, key, with) else { return };
    let (starter, msg) = WordsStarter::start(&me, with);
    open(shared, key, with, HandshakeKind::Words, String::new(), &msg, Step::WordsStarted(starter));
}

/// Ask `with` something only they'd know, expecting `answer`.
pub fn ask(shared: &Arc<Shared>, key: &str, with: &str, question: &str, answer: &str) {
    let question: String = question.trim().chars().take(QUESTION_MAX).collect();
    let answer: String = answer.chars().take(ANSWER_MAX).collect();
    if question.is_empty() || handshake::normalize(&answer).is_empty() {
        shared.state.write().toast("Write a question, and the answer you expect.", false);
        (shared.repaint)();
        return;
    }
    give_up_on_old(shared);
    if shared.verify.lock().failed_asks.get(with).is_some_and(|v| v.len() >= FAILED_ASKS_AN_HOUR) {
        let name = shared.state.read_recursive().name_of(with);
        shared.state.write().toast(format!("The answers didn't match {FAILED_ASKS_AN_HOUR} times in the last hour, so wait a while before asking {name} again (so does anyone guessing)."), false);
        (shared.repaint)();
        return;
    }
    let Some(me) = reachable(shared, key, with) else { return };
    let (asking, msg) = Asking::start(&me, with, &question, &answer);
    open(shared, key, with, HandshakeKind::Ask, question, &msg, Step::Asked(asking));
}

/// Takes the step of handshake `sid` (leaving `Done`), with its server and who it's with.
fn take(shared: &Shared, sid: &str) -> Option<(String, String, Step)> {
    let mut book = shared.verify.lock();
    let p = book.pending.get_mut(sid)?;
    Some((p.server.clone(), p.with.clone(), std::mem::replace(&mut p.step, Step::Done)))
}

fn put_back(shared: &Shared, sid: &str, step: Step) {
    if let Some(p) = shared.verify.lock().pending.get_mut(sid) {
        p.step = step;
        p.at = Instant::now();
    }
}

/// Yes to comparing words they asked for.
pub fn accept_words(shared: &Arc<Shared>, sid: &str) {
    let Some((server, with, step)) = take(shared, sid) else { return };
    let Step::WordsOffered { commit } = step else {
        put_back(shared, sid, step);
        return;
    };
    let me = shared.state.read_recursive().my_uid.clone();
    match WordsJoiner::join(&me, &with, sid, &commit) {
        Some((joiner, msg)) => {
            put_back(shared, sid, Step::WordsJoined(joiner));
            set_stage(shared, sid, HandshakeStage::Waiting);
            send(shared, &server, &with, &msg);
        }
        None => end(shared, sid, HandshakeStage::Failed("Their Pyr's request couldn't be read.".into())),
    }
}

/// My answer to the question they asked.
pub fn answer(shared: &Arc<Shared>, sid: &str, answer: &str) {
    let answer: String = answer.chars().take(ANSWER_MAX).collect();
    if handshake::normalize(&answer).is_empty() {
        return;
    }
    let Some((server, with, step)) = take(shared, sid) else { return };
    let Step::AskCame { question, ya } = step else {
        put_back(shared, sid, step);
        return;
    };
    let me = shared.state.read_recursive().my_uid.clone();
    match handshake::answer(&with, &me, sid, &question, &ya, &answer) {
        Some((reply, expected)) => {
            put_back(shared, sid, Step::Answered(expected));
            set_stage(shared, sid, HandshakeStage::Waiting);
            send(shared, &server, &with, &reply);
        }
        None => end(shared, sid, HandshakeStage::Failed("Their Pyr's question couldn't be read.".into())),
    }
}

/// What I say about the words: they match (then it's them, unless they said theirs didn't), or
/// they don't.
pub fn words_said(shared: &Arc<Shared>, sid: &str, matched: bool) {
    let Some((server, with)) = shared.verify.lock().pending.get(sid).map(|p| (p.server.clone(), p.with.clone())) else { return };
    let they_said = shared.state.read_recursive().handshakes.iter().find(|v| v.sid == sid).and_then(|v| match &v.stage {
        HandshakeStage::Compare { they_said, .. } => Some(*they_said),
        _ => None,
    });
    let Some(they_said) = they_said else { return };
    if matched && they_said == Some(false) {
        // One of you is wrong, or someone's in between: it can't count.
        let name = shared.state.read_recursive().server(&server).map(|s| s.member_name(&with)).unwrap_or_else(|| "They".into());
        end(shared, sid, HandshakeStage::Failed(format!("{name} said the words didn't match, so this can't count. Compare them again, slowly, when you can talk.")));
    } else if matched {
        let at_ms = mark(shared, &server, &with, VerifiedHow::Words);
        shared.verify.lock().said_match.insert(sid.to_string(), (server.clone(), with.clone(), at_ms, Instant::now()));
        end(shared, sid, HandshakeStage::Sure);
    } else {
        end(
            shared,
            sid,
            HandshakeStage::Failed("The words didn't match: someone may be in between you, or it isn't them. Don't trust this conversation with anything private.".into()),
        );
    }
    send(shared, &server, &with, &Handshake::WordsSaid { sid: sid.to_string(), matched });
}

/// The answers to their question matched, and only they could have known it: it's them.
pub fn it_is_them(shared: &Arc<Shared>, sid: &str) {
    let found = shared.state.read_recursive().handshakes.iter().find(|v| v.sid == sid && v.stage == HandshakeStage::AnswersMatched).map(|v| (v.server.clone(), v.with.clone()));
    if let Some((server, with)) = found {
        mark(shared, &server, &with, VerifiedHow::Question);
        end(shared, sid, HandshakeStage::Sure);
    }
}

/// Not now (or no more): the other end hears so, and the card goes.
pub fn decline(shared: &Arc<Shared>, sid: &str) {
    let p = shared.verify.lock().pending.remove(sid);
    if let Some(p) = p {
        if !matches!(p.step, Step::Done) {
            send(shared, &p.server, &p.with, &Handshake::Decline { sid: sid.to_string() });
        }
    }
    dismiss(shared, sid);
}

/// The card goes (a finished handshake).
pub fn dismiss(shared: &Shared, sid: &str) {
    shared.verify.lock().pending.remove(sid);
    shared.state.write().handshakes.retain(|v| v.sid != sid);
    (shared.repaint)();
}

/// A message from `from`'s Pyr, through server `key`.
pub(crate) fn on_handshake(shared: &Arc<Shared>, key: &str, from: &str, data: &str) {
    if shared.state.read_recursive().blocked.contains(from) {
        return;
    }
    let Some(msg) = Handshake::from_json(data) else { return };
    give_up_on_old(shared);
    let sid = msg.sid().to_string();
    // Anything for a handshake under way comes from whoever it's with, through its server.
    let ours = {
        let book = shared.verify.lock();
        match book.pending.get(&sid) {
            Some(p) if p.server == key && p.with == from => Some(true),
            Some(_) => Some(false),
            None => None,
        }
    };
    match msg {
        Handshake::Words { commit, .. } if ours.is_none() => {
            offered(shared, key, from, &sid, HandshakeKind::Words, String::new(), Step::WordsOffered { commit });
        }
        Handshake::Ask { question, ya, .. } if ours.is_none() => {
            let question: String = question.chars().take(QUESTION_MAX).collect();
            offered(shared, key, from, &sid, HandshakeKind::Ask, question.clone(), Step::AskCame { question, ya });
        }
        // (Theirs didn't match, after I said mine did and closed the card.)
        Handshake::WordsSaid { matched: false, .. } if ours.is_none() => took_back(shared, key, from, &sid),
        _ if ours != Some(true) => {}
        Handshake::WordsKey { key: theirs, .. } => {
            let Some((server, with, step)) = take(shared, &sid) else { return };
            let Step::WordsStarted(starter) = step else { return };
            match starter.finish(&theirs) {
                Some((reveal, words)) => {
                    compare(shared, &sid, words);
                    send(shared, &server, &with, &reveal);
                }
                None => end(shared, &sid, HandshakeStage::Failed("Their Pyr sent a key that won't do: someone may be in between you.".into())),
            }
        }
        Handshake::WordsReveal { key: theirs, .. } => {
            let Some((_, _, step)) = take(shared, &sid) else { return };
            let Step::WordsJoined(joiner) = step else { return };
            match joiner.finish(&theirs) {
                Some(words) => compare(shared, &sid, words),
                None => end(
                    shared,
                    &sid,
                    HandshakeStage::Failed("Their key wasn't the one their Pyr promised: someone may be in between you. Don't trust this conversation with anything private.".into()),
                ),
            }
        }
        Handshake::WordsSaid { matched, .. } => {
            if matched {
                // They made sure of me: their vouch for me is on its way.
                refetch_for_me_soon(shared);
            }
            let mut st = shared.state.write();
            let mut sure = false;
            if let Some(v) = st.handshakes.iter_mut().find(|v| v.sid == sid) {
                match &mut v.stage {
                    HandshakeStage::Compare { they_said, .. } => *they_said = Some(matched),
                    HandshakeStage::Sure => sure = true,
                    _ => {}
                }
            }
            drop(st);
            if sure && !matched {
                took_back(shared, key, from, &sid);
            }
            (shared.repaint)();
        }
        Handshake::Answer { yb, mac, .. } => {
            let Some((server, with, step)) = take(shared, &sid) else { return };
            let Step::Asked(asking) = step else { return };
            match asking.finish(&yb, &mac) {
                Some(confirm) => {
                    mark(shared, &server, &with, VerifiedHow::Question);
                    end(shared, &sid, HandshakeStage::Sure);
                    send(shared, &server, &with, &confirm);
                }
                None => {
                    shared.verify.lock().failed_asks.entry(with.clone()).or_default().push(Instant::now());
                    end(shared, &sid, HandshakeStage::Failed("The answers didn't match. A typo, or another way of saying it? You can ask again.".into()));
                    send(shared, &server, &with, &Handshake::Mismatch { sid: sid.clone() });
                }
            }
        }
        Handshake::Confirm { mac, .. } => {
            let Some((_, _, step)) = take(shared, &sid) else { return };
            let Step::Answered(expected) = step else { return };
            let stage = if handshake::confirmed(&expected, &mac) { HandshakeStage::AnswersMatched } else { HandshakeStage::Failed("Their Pyr's reply didn't check out.".into()) };
            if stage == HandshakeStage::AnswersMatched {
                // My answer matched: they made sure of me.
                refetch_for_me_soon(shared);
            }
            end(shared, &sid, stage);
        }
        Handshake::Mismatch { .. } => end(shared, &sid, HandshakeStage::Failed("Your answer wasn't the one they expected.".into())),
        Handshake::Decline { .. } => {
            shared.verify.lock().pending.remove(&sid);
            set_stage(shared, &sid, HandshakeStage::Failed("Not now, they said.".into()));
        }
        Handshake::Words { .. } | Handshake::Ask { .. } => {}
    }
}

/// They say the words didn't match, after I said they did: one of us is wrong, or someone's in
/// between. It doesn't count (unless I made sure of them again since), and a card says so.
fn took_back(shared: &Arc<Shared>, key: &str, from: &str, sid: &str) {
    let said = shared.verify.lock().said_match.remove(sid);
    let Some((_, _, at_ms, _)) = said.filter(|(k, w, _, at)| k == key && w == from && at.elapsed() < GIVE_UP_AFTER) else { return };
    if shared.state.read_recursive().verified.get(from).is_some_and(|v| v.at_ms == at_ms) {
        forget(shared, from);
    }
    shared.verify.lock().pending.remove(sid);
    let name = shared.state.read_recursive().server(key).map(|s| s.member_name(from)).unwrap_or_else(|| "They".into());
    let why = format!("{name} says the words didn't match, so Pyr doesn't count it as them after all. Compare them again, slowly, when you can talk.");
    show(
        shared,
        HandshakeView {
            sid: sid.into(),
            server: key.into(),
            with: from.into(),
            kind: HandshakeKind::Words,
            mine: false,
            question: String::new(),
            stage: HandshakeStage::Failed(why),
            at: Instant::now(),
        },
    );
}

/// They want to make sure of me: their request waits for me (one per person: a newer one takes
/// the last one's place).
fn offered(shared: &Shared, key: &str, from: &str, sid: &str, kind: HandshakeKind, question: String, step: Step) {
    {
        let mut book = shared.verify.lock();
        book.pending.retain(|_, p| !(p.with == from && matches!(p.step, Step::WordsOffered { .. } | Step::AskCame { .. })));
        book.pending.insert(sid.to_string(), Pending { server: key.into(), with: from.into(), at: Instant::now(), step });
    }
    show(shared, HandshakeView { sid: sid.into(), server: key.into(), with: from.into(), kind, mine: false, question, stage: HandshakeStage::Asked, at: Instant::now() });
    shared.play(crate::sfx::Sound::Message);
}

fn compare(shared: &Shared, sid: &str, words: Vec<handshake::Word>) {
    let words = words.into_iter().map(|(e, w)| (e.to_string(), w.to_string())).collect();
    set_stage(shared, sid, HandshakeStage::Compare { words, they_said: None });
}

/// I made sure of `uid` (as server `key` shows them): remembered, and vouched for. When, in
/// milliseconds.
fn mark(shared: &Arc<Shared>, key: &str, uid: &str, how: VerifiedHow) -> i64 {
    let name = {
        let st = shared.state.read_recursive();
        st.server(key).map(|s| s.member_name(uid)).unwrap_or_else(|| st.name_of(uid))
    };
    let at_ms = pyr_proto::now_ms();
    let v = Verified { name, how, at_ms };
    {
        let mut cfg = shared.config.lock();
        cfg.verified.insert(uid.to_string(), v.clone());
        shared.save_config_locked(&cfg);
    }
    {
        let mut st = shared.state.write();
        st.verified.insert(uid.to_string(), v);
        // Vouched for or not, they're made sure of now.
        st.vouched.remove(uid);
    }
    everywhere(shared);
    // They're likely making sure of me too, just now.
    refetch_for_me_soon(shared);
    at_ms
}

/// No longer counted as made sure of (their vouches go too).
pub fn forget(shared: &Arc<Shared>, uid: &str) {
    {
        let mut cfg = shared.config.lock();
        cfg.verified.remove(uid);
        shared.save_config_locked(&cfg);
    }
    {
        let mut st = shared.state.write();
        st.verified.remove(uid);
        for by in st.vouched.values_mut() {
            by.retain(|b| b != uid);
        }
        st.vouched.retain(|_, by| !by.is_empty());
    }
    everywhere(shared);
}

/// Vouches published and fetched again on every server (after the people I made sure of, or
/// whether I vouch, changed).
pub(crate) fn everywhere(shared: &Arc<Shared>) {
    let keys: Vec<String> = shared.state.read_recursive().servers.iter().map(|s| s.key.clone()).collect();
    for key in keys {
        publish_vouches(shared, &key);
        fetch_vouches(shared, &key);
        fetch_vouches_for_me(shared, &key);
    }
    (shared.repaint)();
}

/// Who made sure it's me, asked again on every server in a few seconds and a little later
/// (after making sure of each other: their vouch for me takes a moment to arrive).
fn refetch_for_me_soon(shared: &Arc<Shared>) {
    let s = shared.clone();
    shared.rt.spawn(async move {
        for wait in [4, 10] {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            let keys: Vec<String> = s.state.read_recursive().servers.iter().map(|x| x.key.clone()).collect();
            for key in keys {
                fetch_vouches_for_me(&s, &key);
            }
        }
    });
}

/// Who made sure it's me on server `key`: their signed vouches for me (from members, not
/// bots), for the shield by my own name (`AppState::made_sure_of_me`).
pub(crate) fn fetch_vouches_for_me(shared: &Arc<Shared>, key: &str) {
    let ready = shared.state.read_recursive().server(key).is_some_and(|s| s.status == ConnStatus::Connected && s.has(VOUCHES_FOR_ME_FEATURE));
    if !ready {
        return;
    }
    let Some(h) = shared.servers.lock().get(key).cloned() else { return };
    let reply = h.request(Request::GetVouchesForMe {});
    let s = shared.clone();
    let key = key.to_string();
    shared.rt.spawn(async move {
        let Ok(Event::Vouches { vouches }) = reply.await else { return };
        let mut st = s.state.write();
        let me = st.my_uid.clone();
        let Some(srv) = st.server(&key) else { return };
        let by: Vec<String> = vouches.into_iter().filter(|v| v.for_uid == me && v.by != me && srv.members.contains_key(&v.by) && !srv.is_bot(&v.by) && v.valid()).map(|v| v.by).collect();
        st.set_made_sure_of_me(&key, by);
        drop(st);
        (s.repaint)();
    });
}

/// My vouches on server `key`: for the people I made sure of who are members there (none when
/// I don't vouch).
pub(crate) fn publish_vouches(shared: &Arc<Shared>, key: &str) {
    let vouches: Vec<Vouch> = {
        let st = shared.state.read_recursive();
        let Some(s) = st.server(key).filter(|s| s.status == ConnStatus::Connected && s.has(VOUCHES_FEATURE)) else { return };
        if !st.settings.vouch {
            Vec::new()
        } else {
            st.verified.keys().filter(|u| s.members.contains_key(*u)).take(MAX_VOUCHES).map(|u| Vouch::new(&shared.identity, u)).collect()
        }
    };
    if let Some(h) = shared.servers.lock().get(key) {
        h.notify(Request::SetVouches { vouches });
    }
}

/// The vouches the people I made sure of made on server `key`.
pub(crate) fn fetch_vouches(shared: &Arc<Shared>, key: &str) {
    let by: Vec<String> = {
        let st = shared.state.read_recursive();
        let Some(s) = st.server(key).filter(|s| s.status == ConnStatus::Connected && s.has(VOUCHES_FEATURE)) else { return };
        st.verified.keys().filter(|u| s.members.contains_key(*u)).take(MAX_VOUCHES_ASKED).cloned().collect()
    };
    if by.is_empty() {
        return;
    }
    let Some(h) = shared.servers.lock().get(key).cloned() else { return };
    // (A server too old to say who made sure of me: the ones among these do.)
    let older = !shared.state.read_recursive().server(key).is_some_and(|s| s.has(VOUCHES_FOR_ME_FEATURE));
    let reply = h.request(Request::GetVouches { by });
    let s = shared.clone();
    let key = key.to_string();
    shared.rt.spawn(async move {
        let Ok(Event::Vouches { vouches }) = reply.await else { return };
        let mut st = s.state.write();
        let me = st.my_uid.clone();
        if older {
            let for_me: Vec<String> = vouches.iter().filter(|v| v.for_uid == me && st.verified.contains_key(&v.by) && v.valid()).map(|v| v.by.clone()).collect();
            st.set_made_sure_of_me(&key, for_me);
        }
        for v in vouches {
            if v.for_uid == me || st.verified.contains_key(&v.for_uid) || !st.verified.contains_key(&v.by) || !v.valid() {
                continue;
            }
            let by = st.vouched.entry(v.for_uid).or_default();
            if !by.contains(&v.by) {
                by.push(v.by);
            }
        }
        drop(st);
        (s.repaint)();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn look_alike_names_fold_to_the_same() {
        let mara = fold("Mara");
        for other in ["mara", "MARA", "M4ra", "Mаra", "Mara!", " M a r a ", "Mαra"] {
            assert_eq!(fold(other), mara, "{other}");
        }
        assert_eq!(fold("Kim"), fold("Kirn"));
        assert_eq!(fold("Will"), fold("VVi11"));
        assert_ne!(fold("Mara"), fold("Maria"));
        assert_eq!(fold("🔥"), "");
    }
}
