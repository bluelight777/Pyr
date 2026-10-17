//! A real server lists its invites at a real directory (both in this process, on localhost),
//! and the directory hands them back by their code: as JSON and as the page.

use std::time::Duration;

use pyr_proto::control::{Event, Role};
use pyr_proto::directory::{Announce, AnnounceReply, Lookup, SignedAnnounce};

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-dir-it-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().http_status_as_error(false).proxy(None).max_redirects(0).timeout_global(Some(Duration::from_secs(10))).build().into()
}

fn get(base: &str, path: &str) -> (u16, String, Option<String>) {
    let mut r = agent().get(&format!("{base}{path}")).call().unwrap();
    let location = r.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap(), location)
}

async fn server(name: &str, base: &str) -> pyr_server::RunningServer {
    pyr_server::start_with(
        pyr_server::ServerConfig {
            data_dir: temp(name),
            bind: "127.0.0.1:0".parse().unwrap(),
            // (An address others could reach, as a VPS has: short links are only for those.)
            public_host: Some("203.0.113.5".into()),
            name: Some(name.into()),
        },
        Some(base.to_string()),
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn invites_open_by_their_code_and_go_when_taken_back() {
    let site = temp("site");
    std::fs::write(site.join("index.html"), "<html><head><title>Pyr</title>\n<meta property=\"og:title\" content=\"Pyr\">\n</head><body>pyr</body></html>").unwrap();
    let dir = pyr_directory::start(pyr_directory::Config { data_dir: temp("db"), bind: "127.0.0.1:0".parse().unwrap(), site_dir: Some(site), behind_proxy: false })
        .await
        .unwrap();
    let base = format!("http://{}", dir.local_addr);
    let camp = server("Camp", &base).await;
    let hub = camp.hub.clone();
    assert!(hub.short_links_on());

    // An invite made the way CreateInvite makes one: its short link comes back once listed.
    hub.db.create_invite("qrstuvwx", "server", None, None, Role::Member).unwrap();
    hub.db.set_listed("qrstuvwx", true).unwrap();
    let made = Event::InviteCreated { code: "qrstuvwx".into(), link: hub.invite_link("qrstuvwx"), short: None, short_problem: None, expires_at: None, max_uses: None, uses: 0 };
    let Event::InviteCreated { short, short_problem, .. } = hub.with_short_link(made).await else { panic!() };
    assert_eq!((short.as_deref(), short_problem), (Some("https://pyr.gg/qrstuvwx"), None));

    // Looked up: the full link to the server, its name and counts.
    let (status, body, _) = tokio::task::spawn_blocking({
        let base = base.clone();
        move || get(&base, "/api/v1/invites/qrstuvwx")
    })
    .await
    .unwrap();
    assert_eq!(status, 200, "{body}");
    let l: Lookup = serde_json::from_str(&body).unwrap();
    assert_eq!(l.link, hub.invite_link("qrstuvwx"));
    assert_eq!((l.name.as_str(), l.members, l.vanity), ("Camp", Some(0), false));

    // The page: the name in its title and preview; /invite/… goes to it.
    let (page, moved, missing) = tokio::task::spawn_blocking({
        let base = base.clone();
        move || (get(&base, "/qrstuvwx"), get(&base, "/invite/QRSTUVWX"), get(&base, "/nosuchxx"))
    })
    .await
    .unwrap();
    assert_eq!(page.0, 200);
    assert!(page.1.contains("<title>Join Camp on Pyr</title>") && page.1.contains("pyr-invite"), "{}", page.1);
    assert_eq!((moved.0, moved.2.as_deref()), (301, Some("/qrstuvwx")));
    assert_eq!(missing.0, 404);
    assert!(missing.1.contains("\"invite\":null"));

    // Taken back: gone from the directory too.
    hub.db.delete_invite("qrstuvwx").unwrap();
    hub.directory().unwrap().sync().await.unwrap();
    let gone = tokio::task::spawn_blocking({
        let base = base.clone();
        move || get(&base, "/api/v1/invites/qrstuvwx")
    })
    .await
    .unwrap();
    assert_eq!(gone.0, 404);

    // The community's own code; another community can't have it too.
    let owner = pyr_proto::crypto::Identity::generate().user_id();
    hub.db.insert_member(&owner, "Blue", Role::Owner, false).unwrap();
    let Ok(Event::Invites { vanity, short_links, .. }) = hub.set_vanity(&owner, Some("Camp-Fire".into())).await else { panic!() };
    assert_eq!((vanity.as_deref(), short_links), (Some("camp-fire"), true));
    assert!(hub.db.invite("camp-fire").is_some(), "it's an invite here too");
    let l: Lookup = serde_json::from_str(&tokio::task::spawn_blocking({
        let base = base.clone();
        move || get(&base, "/api/v1/invites/camp-fire").1
    })
    .await
    .unwrap())
    .unwrap();
    assert!(l.vanity && l.inviter.is_none());
    let other = server("Other", &base).await;
    let refused = other.hub.set_vanity(&owner, Some("camp-fire".into())).await.unwrap_err();
    assert!(refused.1.contains("Another community"), "{refused:?}");
    assert!(other.hub.db.invite("camp-fire").is_none(), "put back as it was");
    assert!(other.hub.set_vanity(&owner, Some("download".into())).await.is_err(), "kept for pyr.gg");
    // Changing it frees the old one (and its invite stops working).
    hub.set_vanity(&owner, Some("campfire2".into())).await.unwrap();
    assert!(hub.db.invite("camp-fire").is_none());
    other.hub.set_vanity(&owner, Some("camp-fire".into())).await.unwrap();

    // Short links off: everything of the community's goes from the directory.
    hub.db.set_meta("short_links", "0").unwrap();
    hub.directory().unwrap().sync().await.unwrap();
    let off = tokio::task::spawn_blocking({
        let base = base.clone();
        move || get(&base, "/api/v1/invites/campfire2").0
    })
    .await
    .unwrap();
    assert_eq!(off, 404);
    camp.shutdown().await;
    other.shutdown().await;
    dir.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_servers_own_key_can_speak_for_it() {
    let dir = pyr_directory::start(pyr_directory::Config { data_dir: temp("db2"), bind: "127.0.0.1:0".parse().unwrap(), site_dir: None, behind_proxy: false })
        .await
        .unwrap();
    let base = format!("http://{}", dir.local_addr);
    let tls = pyr_server::tls::load_or_create(&temp("key")).unwrap();
    let signing = pyr_server::directory::Signing::new(&tls.cert_der, &tls.key_der).unwrap();
    let a = Announce {
        ts: pyr_proto::now_ms(),
        host: "203.0.113.9".into(),
        port: 7766,
        name: "Camp".into(),
        codes: vec![pyr_proto::directory::ListedCode { code: "abcdefgh".into(), expires_at: None, inviter: None }],
        ..Default::default()
    };
    let signed = signing.sign(&a).unwrap();
    // Changed after signing: refused.
    let mut forged = signed.clone();
    forged.payload = forged.payload.replace("203.0.113.9", "198.51.100.1");
    let post = |s: SignedAnnounce| {
        let base = base.clone();
        move || {
            let mut r = agent().post(&format!("{base}/api/v1/announce")).send_json(&s).unwrap();
            (r.status().as_u16(), r.body_mut().read_json::<AnnounceReply>().unwrap())
        }
    };
    let (status, reply) = tokio::task::spawn_blocking(post(forged)).await.unwrap();
    assert_eq!(status, 401, "{reply:?}");
    // Signed by another server's key: refused too.
    let other = pyr_server::tls::load_or_create(&temp("key2")).unwrap();
    let mut wrong = pyr_server::directory::Signing::new(&other.cert_der, &other.key_der).unwrap().sign(&a).unwrap();
    wrong.cert = signed.cert.clone();
    assert_eq!(tokio::task::spawn_blocking(post(wrong)).await.unwrap().0, 401);
    // As signed: taken, and the link pins this server's certificate.
    let (status, reply) = tokio::task::spawn_blocking(post(signed.clone())).await.unwrap();
    assert_eq!((status, reply), (200, AnnounceReply::default()));
    let l = dir.db.lookup("abcdefgh").unwrap();
    let link = pyr_proto::invite::InviteLink::parse(&l.link).unwrap();
    assert!(link.matches(&tls.fingerprint) && link.host == "203.0.113.9");
    // Sent again as it was (a replay): refused.
    assert_eq!(tokio::task::spawn_blocking(post(signed)).await.unwrap().0, 409);
    dir.stop();
}
