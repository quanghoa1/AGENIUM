use std::sync::Arc;

use agenium_secrets::*;

fn payloads() -> Vec<(&'static str, Vec<u8>)> {
    let oauth = |n: usize| format!("{{\"access_token\":\"{}\"}}", "a".repeat(n)).into_bytes();
    vec![
        (
            "provider/anthropic",
            b"sk-ant-api03-CANARY-0123456789abcdefghijklmnopqr".to_vec(),
        ),
        ("mcp/github/oauth-1k", oauth(1024)),
        ("mcp/github/oauth-2k", oauth(2048)),
        ("mcp/github/oauth-4k", oauth(4096)),
        (
            "mcp/burp/credential",
            br#"{"user":"dev","password":"CANARY-pass"}"#.to_vec(),
        ),
        (
            "ssh/ed25519",
            format!(
                "-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\n",
                "b".repeat(340)
            )
            .into_bytes(),
        ),
        ("ssh/rsa4096", vec![0xA5; 3272]),
    ]
}

#[test]
fn vault_round_trips_every_secret_type_without_plaintext_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let kr = Arc::new(MemoryKeyring::default());
    let path = tmp.path().join("vault.bin");
    let s = open_store(Backend::Vault, kr.clone(), &path, "root1");
    for (name, v) in payloads() {
        s.set(name, &v).unwrap();
    }
    for (name, v) in payloads() {
        assert_eq!(s.get(name).unwrap().unwrap().expose(), &v[..], "{name}");
    }
    assert_eq!(s.list().unwrap().len(), 7);
    let disk = std::fs::read(&path).unwrap();
    for needle in [&b"CANARY"[..], b"sk-ant", b"OPENSSH", b"access_token"] {
        assert!(
            !disk.windows(needle.len()).any(|w| w == needle),
            "plaintext on disk"
        );
    }
    s.delete("ssh/rsa4096").unwrap();
    assert!(s.get("ssh/rsa4096").unwrap().is_none());
    // A second store object (new process) reads the same vault with the key from the keyring.
    let s2 = open_store(Backend::Vault, kr, &path, "root1");
    assert_eq!(s2.list().unwrap().len(), 6);
    assert_eq!(
        format!("{:?}", s2.get("provider/anthropic").unwrap().unwrap()),
        "Secret([48 bytes])"
    );
}

#[test]
fn vault_detects_tampering_and_missing_key() {
    let tmp = tempfile::tempdir().unwrap();
    let kr = Arc::new(MemoryKeyring::default());
    let path = tmp.path().join("vault.bin");
    let s = open_store(Backend::Vault, kr.clone(), &path, "r");
    s.set("a", b"secret-value").unwrap();
    let mut b = std::fs::read(&path).unwrap();
    let last = b.len() - 1;
    b[last] ^= 1;
    std::fs::write(&path, &b).unwrap();
    assert!(matches!(s.get("a"), Err(SecretError::Corrupt(_))));
    // Key lost: never silently create a new vault over the old one.
    let other = open_store(Backend::Vault, kr, &path, "other-root");
    assert!(matches!(other.set("b", b"x"), Err(SecretError::Corrupt(_))));
    assert!(matches!(other.get("b"), Err(SecretError::Corrupt(_))));
}

#[test]
fn keyring_errors_surface_and_names_are_checked() {
    let kr = Arc::new(MemoryKeyring::default());
    *kr.fail.lock().unwrap() = Some(SecretError::Timeout("locked".into()));
    let tmp = tempfile::tempdir().unwrap();
    let s = open_store(Backend::Vault, kr.clone(), &tmp.path().join("v"), "r");
    assert!(matches!(s.set("a", b"x"), Err(SecretError::Timeout(_))));
    *kr.fail.lock().unwrap() = None;
    assert!(matches!(
        s.set("bad name", b"x"),
        Err(SecretError::InvalidName(_))
    ));
    assert!(matches!(
        s.set("a:b", b"x"),
        Err(SecretError::InvalidName(_))
    ));
    let k = open_store(Backend::Keyring, kr, &tmp.path().join("unused"), "r");
    k.set("provider/x", b"1").unwrap();
    k.set("provider/y", b"2").unwrap();
    k.delete("provider/x").unwrap();
    assert_eq!(k.list().unwrap(), vec!["provider/y".to_string()]);
}

/// A malformed master key in the keyring (64 bytes, but a non-ASCII character where a hex pair should be) is reported
/// as corrupt; decoding it used to panic on a slice that split the character.
#[test]
fn a_malformed_master_key_is_corrupt_not_a_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let kr = Arc::new(MemoryKeyring::default());
    let account = "vault-master/r";
    let key = format!("a\u{e9}{}", "a".repeat(61));
    assert_eq!(key.len(), 64);
    kr.set(store::SERVICE, account, key.as_bytes()).unwrap();
    let s = VaultStore::new(&tmp.path().join("vault.bin"), kr, account);
    assert!(matches!(s.get("a"), Err(SecretError::Corrupt(_))));
}

/// Real OS keyring (Windows Credential Manager here). Touches the user's keyring, so opt-in:
/// `AGENIUM_TEST_OS_KEYRING=1 cargo test -p agenium-secrets --test secrets os_keyring -- --nocapture`
#[test]
fn os_keyring_round_trip_and_blob_limit() {
    if std::env::var("AGENIUM_TEST_OS_KEYRING").as_deref() != Ok("1") {
        eprintln!("skipped (set AGENIUM_TEST_OS_KEYRING=1)");
        return;
    }
    OsKeyring::status().unwrap();
    let kr = OsKeyring::default();
    let account = format!("selftest/{}", std::process::id());
    for size in [32usize, 1024, 2048, 2560, 4096] {
        let v = vec![b'x'; size];
        let r = kr.set(store::SERVICE, &account, &v);
        let back = r
            .as_ref()
            .ok()
            .and_then(|_| kr.get(store::SERVICE, &account).ok().flatten());
        eprintln!(
            "direct keyring blob {size:>5} B: set={:?} round_trip={}",
            r.as_ref().map(|_| "ok"),
            back.as_deref() == Some(&v[..])
        );
    }
    kr.delete(store::SERVICE, &account).unwrap();
    // The hybrid vault only stores a 64-byte hex key in the keyring, so any secret size works.
    let tmp = tempfile::tempdir().unwrap();
    let s = open_store(
        Backend::Vault,
        Arc::new(OsKeyring::default()),
        &tmp.path().join("vault.bin"),
        &format!("selftest-{}", std::process::id()),
    );
    for (name, v) in payloads() {
        s.set(name, &v).unwrap();
        assert_eq!(s.get(name).unwrap().unwrap().expose(), &v[..]);
    }
    kr.delete(
        store::SERVICE,
        &format!("vault-master/selftest-{}", std::process::id()),
    )
    .unwrap();
}
