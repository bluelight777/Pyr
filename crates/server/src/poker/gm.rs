//! Game Masters for the whole game: named only on the home server (`HOME_SERVER`, Brethren of
//! Blood), Game Masters on every server. The home server signs each of them (and its owner, the
//! game's owner) a certificate with its TLS key, the key its invite links pin; their Pyr shows
//! it to every other server, which checks it here: signed by the certificate whose SHA-256 is
//! the home's fingerprint, for the one showing it, and not run out. A certificate lasts
//! `GM_DAYS` and is renewed whenever its holder's Pyr opens the casino at home, so someone
//! no longer a Game Master stops being one everywhere within that time.

use pyr_poker::net::{GM_CONTEXT, GM_DAYS, GmCert, HOME_SERVER, Role, SignedGm};

use crate::directory::Signing;

/// The home server's fingerprint, if there is one.
pub fn home() -> Option<[u8; 32]> {
    pyr_proto::b64::decode_array::<32>(HOME_SERVER)
}

/// A certificate for `uid`, signed with this (the home) server's key.
pub fn sign(signing: &Signing, uid: &str, home_owner: bool, role: Option<Role>, now_ms: u64) -> Result<SignedGm, String> {
    let cert = GmCert { uid: uid.to_string(), home_owner, role: role.unwrap_or(Role::GameMaster), issued_ms: now_ms, expires_ms: now_ms + GM_DAYS * 86_400_000 };
    let payload = serde_json::to_string(&cert).map_err(|e| e.to_string())?;
    let mut msg = GM_CONTEXT.to_vec();
    msg.extend_from_slice(payload.as_bytes());
    let (cert, scheme, sig) = signing.sign_raw(&msg).map_err(|e| format!("{e:#}"))?;
    Ok(SignedGm { cert, scheme, payload, sig })
}

/// What a certificate shown by `uid` vouches for, if it's the home server's (`home`), theirs,
/// and still good at `now_ms`.
pub fn verify(signed: &SignedGm, home: [u8; 32], uid: &str, now_ms: u64) -> Result<GmCert, String> {
    use pyr_proto::directory::{SCHEME_ECDSA_P256, SCHEME_ED25519};
    let cert = pyr_proto::b64::decode(&signed.cert).map_err(|_| "that certificate isn't base64".to_string())?;
    let sig = pyr_proto::b64::decode(&signed.sig).map_err(|_| "that signature isn't base64".to_string())?;
    if cert.len() > 8192 || sig.len() > 1024 || signed.payload.len() > 4096 {
        return Err("that certificate is too big".into());
    }
    if pyr_proto::crypto::fingerprint(&cert) != home {
        return Err("that certificate isn't from the Game Masters' home server".into());
    }
    let alg = match signed.scheme.as_str() {
        SCHEME_ECDSA_P256 => webpki::ring::ECDSA_P256_SHA256,
        SCHEME_ED25519 => webpki::ring::ED25519,
        _ => return Err("that certificate's signature isn't one Pyr knows".into()),
    };
    let der = rustls::pki_types::CertificateDer::from(cert.as_slice());
    let ee = webpki::EndEntityCert::try_from(&der).map_err(|e| format!("the home server's certificate can't be read: {e}"))?;
    let mut msg = GM_CONTEXT.to_vec();
    msg.extend_from_slice(signed.payload.as_bytes());
    ee.verify_signature(alg, &msg, &sig).map_err(|_| "that Game Master certificate's signature doesn't match".to_string())?;
    let gm: GmCert = serde_json::from_str(&signed.payload).map_err(|e| format!("that certificate can't be read: {e}"))?;
    if gm.uid != uid {
        return Err("that Game Master certificate is someone else's".into());
    }
    if gm.expires_ms <= now_ms {
        return Err("that Game Master certificate has run out (open Pyr Casino on the home server to renew it)".into());
    }
    Ok(gm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_homes_certificates_for_the_one_showing_them_count_until_they_run_out() {
        let dir = std::env::temp_dir().join(format!("pyr-gm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(dir.join("home")).unwrap();
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let home_tls = crate::tls::load_or_create(&dir.join("home")).unwrap();
        let other_tls = crate::tls::load_or_create(&dir.join("other")).unwrap();
        let home = Signing::new(&home_tls.cert_der, &home_tls.key_der).unwrap();
        let other = Signing::new(&other_tls.cert_der, &other_tls.key_der).unwrap();
        let now = 1_000_000;
        let s = sign(&home, "gee", false, Some(Role::GameMaster), now).unwrap();
        let ok = verify(&s, home_tls.fingerprint, "gee", now + 1).unwrap();
        assert!(!ok.home_owner && ok.uid == "gee");
        assert!(verify(&s, home_tls.fingerprint, "mallory", now).is_err(), "someone else's");
        assert!(verify(&s, home_tls.fingerprint, "gee", now + GM_DAYS * 86_400_000).is_err(), "run out");
        // Signed by another server: no good, whatever it says.
        let fake = sign(&other, "gee", true, None, now).unwrap();
        assert!(verify(&fake, home_tls.fingerprint, "gee", now).is_err());
        // The home's certificate on another's say-so: the signature doesn't match.
        let mut forged = fake.clone();
        forged.cert = s.cert.clone();
        assert!(verify(&forged, home_tls.fingerprint, "gee", now).is_err());
        // Changed after signing: no good.
        let mut changed = s.clone();
        changed.payload = changed.payload.replace("\"home_owner\":false", "\"home_owner\":true");
        assert!(verify(&changed, home_tls.fingerprint, "gee", now).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
