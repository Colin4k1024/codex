use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use superagent_marketplace::{
    FileKind, MarketError, Marketplace, PackFile, PackManifest, SignedPack, Source,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_root() -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("sa-mkt-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn regular(path: &str, content: &str) -> PackFile {
    PackFile {
        path: path.into(),
        content: content.into(),
        kind: FileKind::Regular,
        symlink_target: None,
    }
}

fn manifest(name: &str, version: &str, source: Source, files: Vec<PackFile>) -> PackManifest {
    PackManifest {
        name: name.into(),
        version: version.into(),
        source,
        publisher_key_id: "pub1".into(),
        licenses: vec!["Apache-2.0".into()],
        files,
    }
}

fn setup() -> Marketplace {
    let mut mp = Marketplace::new();
    mp.register_key("pub1", "publisher-secret");
    mp
}

#[test]
fn third_party_install_upgrade_rollback() {
    let root = temp_root();
    let mut mp = setup();
    let s1 = mp
        .sign_pack(manifest(
            "pack",
            "1.0.0",
            Source::ThirdParty,
            vec![regular("skill.md", "v1")],
        ))
        .unwrap();
    let d1 = s1.digest.clone();
    let r1 = mp.install(&s1, &root).unwrap();
    assert_eq!(r1.version, "1.0.0");
    assert!(Path::new(&r1.install_dir).join("skill.md").exists());

    let s2 = mp
        .sign_pack(manifest(
            "pack",
            "2.0.0",
            Source::ThirdParty,
            vec![regular("skill.md", "v2")],
        ))
        .unwrap();
    let d2 = s2.digest.clone();
    assert_ne!(d1, d2);
    let r2 = mp.upgrade(&s2, &root).unwrap();
    assert_eq!(r2.version, "2.0.0");
    assert_eq!(mp.current("pack").unwrap().digest, d2);

    let rb = mp.rollback("pack").unwrap();
    assert_eq!(rb.version, "1.0.0");
    assert_eq!(mp.current("pack").unwrap().digest, d1);
    let events: Vec<&str> = mp
        .audit()
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert!(
        events.contains(&"install") && events.contains(&"upgrade") && events.contains(&"rollback")
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn run_binding_pinned_across_upgrade() {
    let root = temp_root();
    let mut mp = setup();
    let s1 = mp
        .sign_pack(manifest(
            "pack",
            "1.0.0",
            Source::ThirdParty,
            vec![regular("a.txt", "1")],
        ))
        .unwrap();
    let d1 = s1.digest.clone();
    mp.install(&s1, &root).unwrap();
    let pinned = mp.bind_run("run-1", "pack").unwrap();
    assert_eq!(pinned, d1);
    let s2 = mp
        .sign_pack(manifest(
            "pack",
            "2.0.0",
            Source::ThirdParty,
            vec![regular("a.txt", "2")],
        ))
        .unwrap();
    let d2 = s2.digest.clone();
    mp.upgrade(&s2, &root).unwrap();
    // run binding still resolves the OLD digest despite the upgrade.
    assert_eq!(mp.resolve_run_binding("run-1").unwrap(), &d1);
    assert_ne!(mp.resolve_run_binding("run-1").unwrap(), &d2);
    assert_eq!(mp.current("pack").unwrap().digest, d2);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn signature_and_digest_verification() {
    let root = temp_root();
    let mut mp = setup();
    let signed = mp
        .sign_pack(manifest(
            "pack",
            "1.0.0",
            Source::Curated,
            vec![regular("a.txt", "ok")],
        ))
        .unwrap();
    assert!(mp.verify_pack(&signed).is_ok());
    assert!(mp.install(&signed, &root).is_ok());

    // tampered manifest -> digest mismatch.
    let mut tampered = signed.clone();
    tampered.manifest.version = "9.9.9".into();
    assert!(matches!(
        mp.verify_pack(&tampered),
        Err(MarketError::DigestMismatch)
    ));
    assert!(matches!(
        mp.install(&tampered, &root),
        Err(MarketError::DigestMismatch)
    ));

    // flipped signature byte -> bad signature.
    let mut bad_sig = signed.clone();
    bad_sig.signature[0] ^= 0xff;
    assert!(matches!(
        mp.verify_pack(&bad_sig),
        Err(MarketError::BadSignature)
    ));

    // unknown publisher key.
    let mut ghost = manifest(
        "pack",
        "1.0.0",
        Source::Curated,
        vec![regular("a.txt", "x")],
    );
    ghost.publisher_key_id = "ghost".into();
    assert!(matches!(
        mp.sign_pack(ghost),
        Err(MarketError::UnknownKey(_))
    ));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn malicious_path_traversal_rejected() {
    let root = temp_root();
    let mut mp = setup();
    let evil = manifest(
        "pack",
        "1.0.0",
        Source::ThirdParty,
        vec![regular("../evil.txt", "pwned")],
    );
    let signed: SignedPack = mp.sign_pack(evil).unwrap();
    assert!(matches!(
        mp.install(&signed, &root),
        Err(MarketError::PathTraversal(_))
    ));
    // nothing escaped the install root, and the pack is not installed.
    assert!(!root.join("evil.txt").exists());
    assert!(!root.parent().unwrap().join("evil.txt").exists());
    assert!(mp.current("pack").is_none());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn malicious_symlink_rejected() {
    let root = temp_root();
    let mut mp = setup();
    let link = PackFile {
        path: "link".into(),
        content: String::new(),
        kind: FileKind::Symlink,
        symlink_target: Some("../../etc/passwd".into()),
    };
    let signed = mp
        .sign_pack(manifest("pack", "1.0.0", Source::ThirdParty, vec![link]))
        .unwrap();
    assert!(matches!(
        mp.install(&signed, &root),
        Err(MarketError::SymlinkRejected(_))
    ));
    assert!(!root.join("pack").join("1.0.0").join("link").exists());
    assert!(mp.current("pack").is_none());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn curated_vs_thirdparty_and_licenses() {
    let root = temp_root();
    let mut mp = setup();
    let curated = mp
        .sign_pack(manifest(
            "curated-pack",
            "1.0.0",
            Source::Curated,
            vec![regular("c.md", "c")],
        ))
        .unwrap();
    let third = mp
        .sign_pack(manifest(
            "third-pack",
            "1.0.0",
            Source::ThirdParty,
            vec![regular("t.md", "t")],
        ))
        .unwrap();
    let rc = mp.install(&curated, &root).unwrap();
    let rt = mp.install(&third, &root).unwrap();
    assert_eq!(rc.source, Source::Curated);
    assert_eq!(rt.source, Source::ThirdParty);
    // license list travels with the pack.
    assert_eq!(rc.licenses, vec!["Apache-2.0".to_string()]);
    assert_eq!(rt.licenses, vec!["Apache-2.0".to_string()]);
    let install_events: Vec<&serde_json::Value> = mp
        .audit()
        .iter()
        .filter(|e| e["event"] == "install")
        .collect();
    assert_eq!(install_events.len(), 2);
    assert!(install_events
        .iter()
        .all(|e| e["licenses"].as_array().unwrap().len() == 1));
    std::fs::remove_dir_all(&root).ok();
}
