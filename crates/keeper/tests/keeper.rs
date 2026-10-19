//! The Keeper against a real server database (pyr-server's), with Claude and the sandbox
//! stood in for: what it lists, rejects and leaves for a person, and that the list it signs
//! opens the way Pyr opens it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use anyhow::Result;
use pyr_keeper::build::Builder;
use pyr_keeper::review::{Request, Reviewer, Verdict};
use pyr_keeper::{Config, Keeper};
use pyr_proto::control::{PluginSubmission, SourceFile, SubmissionStatus};
use pyr_proto::crypto::Identity;
use pyr_proto::library::{self, Entry, File, Kind, Library, Permission, SkinEntry};
use pyr_server::db::Db;

struct FakeClaude {
    answer: RefCell<Vec<&'static str>>,
    seen: RefCell<Vec<String>>,
}

impl Reviewer for FakeClaude {
    fn review(&self, req: &Request) -> Result<Verdict> {
        self.seen.borrow_mut().push(format!("{} {}→{} images:{}\n{}", req.name, req.old_version, req.new_version, req.images.len(), req.diff));
        let d = self.answer.borrow_mut().remove(0);
        Ok(Verdict { decision: d.into(), reasons: vec![format!("because {d}")], note_to_author: format!("Verdict: {d}."), whats_new: "Better colors".into() })
    }
}

struct FakeBuild;
impl Builder for FakeBuild {
    fn build_addon(&self, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
        let lib = files.iter().find(|(p, _)| p == "src/lib.rs").map(|(_, b)| b.clone()).unwrap_or_default();
        anyhow::ensure!(!String::from_utf8_lossy(&lib).contains("compile_error"), "error: compile_error!");
        Ok([b"\0asm".as_slice(), &[1, 0, 0, 0], &lib].concat())
    }
}

fn src(files: &[(&str, &[u8])]) -> Vec<SourceFile> {
    files.iter().map(|(p, b)| SourceFile { path: p.to_string(), data: pyr_proto::b64::encode(b) }).collect()
}

fn submit(db: &Db, author: &str, name: &str, kind: &str, version: &str, update_of: i64, files: Vec<SourceFile>) -> i64 {
    let sub = PluginSubmission { name: name.into(), kind: kind.into(), version: version.into(), author: "Mara".into(), author_id: author.into(), update_of, files: files.clone(), ..Default::default() };
    db.add_submission(&sub, &serde_json::to_string(&files).unwrap(), 10, "").unwrap()
}

fn skin_json(version: &str, accent: &str) -> Vec<u8> {
    let mut v: serde_json::Value = serde_json::from_str(&pyr_proto::skin::starter("Sunset", "Mara")).unwrap();
    v["version"] = version.into();
    v["colors"]["ember"] = accent.into();
    serde_json::to_vec_pretty(&v).unwrap()
}

fn file(url: &str, bytes: &[u8]) -> File {
    File { path: url.rsplit('/').next().unwrap().into(), url: url.into(), sha256: pyr_proto::release::sha256_hex(bytes), size: bytes.len() as u64 }
}

const CARGO_V1: &str = "[package]\nname = \"pyr-dice\"\nversion = \"1.0.0\"\n[dependencies]\npyr-plugin = { path = \"../kit\" }\n[package.metadata.pyr]\npermissions = [\"commands\", \"panel\"]\n";

fn setup(dir: &Path, release: &Identity) -> (Db, Config) {
    let db = Db::open(&dir.join("server.db")).unwrap();
    let web = dir.join("web");
    std::fs::create_dir_all(&web).unwrap();
    // What's listed now (as the release key signed it): a skin, an add-on, and a program the
    // release key endorsed.
    let skin_v1 = skin_json("1.0", "#ff7a3d");
    let mut prog = Entry {
        id: "chimbrowser".into(),
        name: "Chimbrowser".into(),
        kind: Kind::Program,
        version: "0.1.3".into(),
        summary: "A browser".into(),
        description: String::new(),
        author: "Chimbita".into(),
        license: String::new(),
        source: String::new(),
        icon: None,
        permissions: vec![],
        windows: vec![file("files/chimbrowser/0.1.3/windows/chimbrowser.exe", b"MZ")],
        linux: vec![],
        wasm: None,
        run: "chimbrowser.exe".into(),
        review: String::new(),
        updated: 1,
        endorsement: String::new(),
    };
    prog.endorsement = library::endorse(release, &prog);
    let dice = Entry {
        id: "dice".into(),
        name: "Dice".into(),
        kind: Kind::Addon,
        windows: vec![],
        run: String::new(),
        wasm: Some(file("files/dice/1.0.0/pyr_dice.wasm", b"\0asm")),
        permissions: vec![Permission::Commands, Permission::Panel],
        version: "1.0.0".into(),
        endorsement: String::new(),
        ..prog.clone()
    };
    let sunset = SkinEntry {
        id: "sunset".into(),
        name: "Sunset".into(),
        version: "1.0".into(),
        summary: "Warm".into(),
        description: String::new(),
        author: "Mara".into(),
        license: String::new(),
        files: vec![file("files/sunset/1.0/skin.json", &skin_v1)],
        preview: None,
        review: String::new(),
        updated: 1,
        needs: String::new(),
    };
    let lib = Library { issued: 100, expires: i64::MAX / 2, plugins: vec![prog, dice], skins: vec![sunset] };
    std::fs::write(web.join("index3.json"), serde_json::to_string(&library::sign(release, &lib)).unwrap()).unwrap();
    let keeper = Identity::generate();
    std::fs::write(dir.join("keeper-key.txt"), pyr_proto::b64::encode(&keeper.seed())).unwrap();
    let cert = library::certify(release, &keeper.user_id(), i64::MAX / 4);
    std::fs::write(dir.join("cert.json"), serde_json::to_string(&cert).unwrap()).unwrap();
    let cfg = Config {
        guild_db: dir.join("server.db"),
        web_library: web,
        state_dir: dir.join("state"),
        key_file: dir.join("keeper-key.txt"),
        cert_file: dir.join("cert.json"),
        release_key: release.user_id(),
        api_key_file: PathBuf::new(),
        model: "test".into(),
        rustup_home: PathBuf::new(),
        cargo_home: PathBuf::new(),
        kit: PathBuf::new(),
        vendor: PathBuf::new(),
        work: PathBuf::new(),
        bwrap: PathBuf::new(),
        poll_secs: 60,
        valid_days: 30,
    };
    (db, cfg)
}

fn status(db: &Db, id: i64) -> (SubmissionStatus, String) {
    let (e, _) = db.submission(id).unwrap();
    (e.status, e.note)
}

#[test]
fn new_versions_of_listed_things_are_listed_rejected_or_left_for_a_person() {
    let dir = std::env::temp_dir().join(format!("pyr-keeper-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let release = Identity::generate();
    let (db, cfg) = setup(&dir, &release);
    let mara = Identity::generate().user_id();
    // What's listed came from these submissions (listed by a person).
    let skin1 = submit(&db, &mara, "Sunset", "skin", "1.0", 0, src(&[("skin.json", &skin_json("1.0", "#ff7a3d"))]));
    db.set_submission_status(skin1, SubmissionStatus::Listed, "").unwrap();
    let dice1 = submit(&db, &mara, "Dice", "addon", "1.0.0", 0, src(&[("Cargo.toml", CARGO_V1.as_bytes()), ("src/lib.rs", b"// v1\n")]));
    db.set_submission_status(dice1, SubmissionStatus::Listed, "").unwrap();
    let chim1 = submit(&db, &mara, "Chimbrowser", "program", "0.1.3", 0, src(&[("Cargo.toml", b"[package]\nname=\"c\"\n")]));
    db.set_submission_status(chim1, SubmissionStatus::Listed, "").unwrap();

    // New versions: a skin (approved), an add-on (approved), an add-on asking for more
    // (held), a program (held), a new plugin (held), one that doesn't build (rejected), one
    // Claude rejects.
    let skin2 = submit(&db, &mara, "Sunset", "skin", "1.1", skin1, src(&[("skin.json", &skin_json("1.1", "#ff9a3d"))]));
    let dice2 = submit(&db, &mara, "Dice", "addon", "1.1.0", dice1, src(&[("Cargo.toml", CARGO_V1.replace("1.0.0", "1.1.0").as_bytes()), ("src/lib.rs", b"// v1\n// rolls faster\n")]));
    let greedy = submit(&db, &mara, "Dice", "addon", "1.2.0", dice2, src(&[("Cargo.toml", CARGO_V1.replace("1.0.0", "1.2.0").replace("\"panel\"]", "\"panel\", \"storage\"]").as_bytes()), ("src/lib.rs", b"// v3\n")]));
    let chim2 = submit(&db, &mara, "Chimbrowser", "program", "0.1.4", chim1, src(&[("Cargo.toml", b"[package]\nname=\"c\"\nversion=\"0.1.4\"\n")]));
    let fresh = submit(&db, &mara, "Coins", "addon", "1.0.0", 0, src(&[("Cargo.toml", CARGO_V1.as_bytes())]));
    let broken = submit(&db, &mara, "Dice", "addon", "1.3.0", dice1, src(&[("Cargo.toml", CARGO_V1.replace("1.0.0", "1.3.0").as_bytes()), ("src/lib.rs", b"compile_error!(\"x\");\n")]));
    let shady = submit(&db, &mara, "Sunset", "skin", "1.2", skin1, src(&[("skin.json", &skin_json("1.2", "#123456"))]));
    let _ = fresh;

    let claude = FakeClaude { answer: RefCell::new(vec!["approve", "approve", "reject"]), seen: RefCell::new(vec![]) };
    let mut k = Keeper::new(cfg.clone(), &claude, &FakeBuild).unwrap();
    k.pass().unwrap();

    assert_eq!(status(&db, skin2).0, SubmissionStatus::Listed, "{:?}", status(&db, skin2));
    assert!(status(&db, skin2).1.contains("Listed by the Pyr Library Keeper"));
    assert_eq!(status(&db, dice2).0, SubmissionStatus::Listed);
    let (s, note) = status(&db, greedy);
    assert_eq!(s, SubmissionStatus::New);
    assert!(note.contains("new permission"), "{note}");
    let (s, note) = status(&db, chim2);
    assert_eq!(s, SubmissionStatus::New);
    assert!(note.contains("programs"), "{note}");
    assert_eq!(status(&db, broken).0, SubmissionStatus::Rejected);
    assert!(status(&db, broken).1.contains("didn't build"));
    assert_eq!(status(&db, shady).0, SubmissionStatus::Rejected);
    assert_eq!(status(&db, shady).1, "Verdict: reject.");
    // Claude saw what changed (the diff), not just the new files.
    let seen = claude.seen.borrow();
    assert!(seen[0].contains("Sunset 1.0→1.1") && seen[0].contains("-  \"version\": \"1.0\"") , "{}", seen[0]);
    assert!(seen[1].contains("+// rolls faster"), "{}", seen[1]);

    // The list on the website: signed by the Keeper, opened as Pyr opens it.
    let text = std::fs::read_to_string(cfg.web_library.join("index3.json")).unwrap();
    let lib = library::open(&text, &release.user_id()).unwrap();
    assert_eq!(lib.find_skin("sunset").unwrap().version, "1.1");
    let dice = lib.find("dice").unwrap();
    assert_eq!(dice.version, "1.1.0");
    assert!(dice.review.contains("Better colors"));
    assert_eq!(lib.find("chimbrowser").unwrap().version, "0.1.3", "the program stays as it was, endorsed");
    // Its files are where the list says, with the hashes it gives.
    for f in lib.find_skin("sunset").unwrap().files.iter().chain(dice.wasm.iter()) {
        let bytes = std::fs::read(cfg.web_library.join(&f.url)).unwrap();
        assert_eq!(pyr_proto::release::sha256_hex(&bytes), f.sha256);
    }
    // Held ones aren't looked at again (until released); nothing else is waiting.
    let claude2 = FakeClaude { answer: RefCell::new(vec![]), seen: RefCell::new(vec![]) };
    let mut k2 = Keeper::new(cfg.clone(), &claude2, &FakeBuild).unwrap();
    assert_eq!(k2.pass().unwrap(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
