//! Making sure people are who they say: comparing words, asking something only they'd know,
//! vouches from people you made sure of, and the alarm for someone else under a name you know.

use std::time::{Duration, Instant};

use ingle_engine::config::VerifiedHow;
use ingle_engine::state::{ConnStatus, HandshakeKind, HandshakeStage, Trust};
use ingle_engine::{Engine, EngineOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-verify-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn client(name: &str) -> Engine {
    let dir = temp_dir(name);
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    Engine::start(EngineOptions { config_dir: Some(dir), push_to_talk: false, ..Default::default() }).unwrap()
}

async fn wait_for(what: &str, secs: u64, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// `e`'s card for making sure of (or by) `with`: (id, stage).
fn card(e: &Engine, with: &str) -> Option<(String, HandshakeStage, HandshakeKind, String)> {
    e.state().handshakes.iter().find(|v| v.with == with).map(|v| (v.sid.clone(), v.stage.clone(), v.kind, v.question.clone()))
}

fn stage(e: &Engine, with: &str) -> Option<HandshakeStage> {
    card(e, with).map(|c| c.1)
}

fn words(e: &Engine, with: &str) -> Option<Vec<(String, String)>> {
    match stage(e, with)? {
        HandshakeStage::Compare { words, .. } => Some(words),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn making_sure_of_people() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Trust Camp".into()),
    })
    .await
    .unwrap();
    let (a, b, c) = (client("Alice"), client("Bob"), client("Carol"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice", 10, || a.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
    for e in [&b, &c] {
        e.add_server(&server.invite_link()).unwrap();
        wait_for("joined", 10, || e.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
    }
    wait_for("everyone known", 10, || [&a, &b, &c].iter().all(|e| e.state().servers[0].members.values().filter(|m| m.online).count() == 3)).await;
    let key = a.state().servers[0].key.clone();
    let (ua, ub, uc) = (a.state().my_uid.clone(), b.state().my_uid.clone(), c.state().my_uid.clone());
    assert_eq!(a.state().trust(&ub, "Bob"), Trust::Unknown);

    // Alice and Bob compare words: the same six on both, both say they match, both are sure.
    a.compare_words(&key, &ub);
    wait_for("bob is asked", 10, || stage(&b, &ua) == Some(HandshakeStage::Asked)).await;
    let (sid_b, _, kind, _) = card(&b, &ua).unwrap();
    assert_eq!(kind, HandshakeKind::Words);
    b.accept_words(&sid_b);
    wait_for("words on both", 10, || words(&a, &ub).is_some() && words(&b, &ua).is_some()).await;
    let (wa, wb) = (words(&a, &ub).unwrap(), words(&b, &ua).unwrap());
    assert_eq!(wa.len(), 6);
    assert_eq!(wa, wb, "the same words at both ends");
    let sid_a = card(&a, &ub).unwrap().0;
    a.words_said(&sid_a, true);
    b.words_said(&sid_b, true);
    wait_for("both sure", 10, || stage(&a, &ub) == Some(HandshakeStage::Sure) && stage(&b, &ua) == Some(HandshakeStage::Sure)).await;
    assert_eq!(a.state().verified.get(&ub).map(|v| (v.name.as_str(), v.how)), Some(("Bob", VerifiedHow::Words)));
    assert_eq!(b.state().trust(&ua, "Alice"), Trust::Verified);

    // Alice asks Carol something only Carol would know.
    a.ask_them(&key, &uc, "What did we name the raid boss?", "Old Smoky");
    wait_for("carol is asked", 10, || stage(&c, &ua) == Some(HandshakeStage::Asked)).await;
    let (sid_c, _, kind, question) = card(&c, &ua).unwrap();
    assert_eq!((kind, question.as_str()), (HandshakeKind::Ask, "What did we name the raid boss?"));
    c.answer_question(&sid_c, "old smoky!");
    wait_for("alice is sure of carol", 10, || stage(&a, &uc) == Some(HandshakeStage::Sure)).await;
    assert_eq!(a.state().trust(&uc, "Carol"), Trust::Verified);
    // Carol's answer matched: she says whether only Alice could have known it.
    wait_for("carol hears it matched", 10, || stage(&c, &ua) == Some(HandshakeStage::AnswersMatched)).await;
    assert!(!c.state().verified.contains_key(&ua), "not until she says so");
    c.it_is_them(&sid_c);
    wait_for("carol is sure of alice", 10, || c.state().verified.contains_key(&ua)).await;

    // A wrong answer: no match at either end, and nobody made sure of.
    b.ask_them(&key, &uc, "Where did we meet?", "Moonglade");
    wait_for("carol is asked again", 10, || card(&c, &ub).is_some_and(|x| x.1 == HandshakeStage::Asked)).await;
    c.answer_question(&card(&c, &ub).unwrap().0, "Stormwind");
    wait_for("no match", 10, || matches!(stage(&b, &uc), Some(HandshakeStage::Failed(_))) && matches!(stage(&c, &ub), Some(HandshakeStage::Failed(_)))).await;
    assert!(!b.state().verified.contains_key(&uc));

    // Bob made sure of Alice, and Alice of Carol: Carol is vouched for, to Bob, by Alice.
    b.refresh_vouches(&key);
    wait_for("vouched for", 10, || b.state().trust(&uc, "Carol") == Trust::Vouched(vec![ua.clone()])).await;

    // Someone else calling themselves Bob: not the Bob Alice made sure of.
    let d = client("B0b");
    d.add_server(&server.invite_link()).unwrap();
    wait_for("dave", 10, || d.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
    let ud = d.state().my_uid.clone();
    wait_for("alice sees him", 10, || a.state().servers[0].members.contains_key(&ud)).await;
    assert_eq!(a.state().trust(&ud, "B0b"), Trust::Impostor(ub.clone()));

    // Saying no: the other end hears so.
    d.compare_words(&key, &ua);
    wait_for("alice is asked by dave", 10, || stage(&a, &ud) == Some(HandshakeStage::Asked)).await;
    a.decline_handshake(&card(&a, &ud).unwrap().0);
    wait_for("dave hears no", 10, || matches!(stage(&d, &ua), Some(HandshakeStage::Failed(_)))).await;
    assert!(card(&a, &ud).is_none(), "the card's gone");

    // The words match for one, not for the other: it doesn't count, whichever said so first.
    // Carol says they match (and closes the card), then Dave says they don't.
    c.compare_words(&key, &ud);
    wait_for("dave is asked by carol", 10, || stage(&d, &uc) == Some(HandshakeStage::Asked)).await;
    d.accept_words(&card(&d, &uc).unwrap().0);
    wait_for("words for carol and dave", 10, || words(&c, &ud).is_some() && words(&d, &uc).is_some()).await;
    let sid_cd = card(&c, &ud).unwrap().0;
    c.words_said(&sid_cd, true);
    assert!(c.state().verified.contains_key(&ud));
    c.dismiss_handshake(&sid_cd);
    d.words_said(&card(&d, &uc).unwrap().0, false);
    wait_for("taken back", 10, || !c.state().verified.contains_key(&ud) && matches!(stage(&c, &ud), Some(HandshakeStage::Failed(_)))).await;
    // Dave says they don't first: Carol saying they do can't count.
    c.compare_words(&key, &ud);
    wait_for("dave is asked again", 10, || stage(&d, &uc) == Some(HandshakeStage::Asked)).await;
    d.accept_words(&card(&d, &uc).unwrap().0);
    wait_for("words again", 10, || words(&c, &ud).is_some() && words(&d, &uc).is_some()).await;
    d.words_said(&card(&d, &uc).unwrap().0, false);
    wait_for("carol hears they don't", 10, || matches!(stage(&c, &ud), Some(HandshakeStage::Compare { they_said: Some(false), .. }))).await;
    c.words_said(&card(&c, &ud).unwrap().0, true);
    assert!(matches!(stage(&c, &ud), Some(HandshakeStage::Failed(_))));
    assert!(!c.state().verified.contains_key(&ud));

    // Forgetting Bob: no longer made sure of (and his vouches go with him).
    a.forget_verified(&ub);
    assert_eq!(a.state().trust(&ub, "Bob"), Trust::Unknown);
    for e in [&a, &b, &c, &d] {
        e.shutdown();
    }
    server.shutdown().await;
}
