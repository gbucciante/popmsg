//! Cifratura dei messaggi compatibile con IP Messenger (RSA + AES-256-CBC).
//!
//! ```text
//! A -> B  GETPUBKEY   "<capacità>"
//! B -> A  ANSPUBKEY   "<capacità>:<e>-<n>"
//! A -> B  SENDMSG|ENCRYPTOPT  "<capacità>:<chiave di sessione cifrata>:<messaggio cifrato>"
//! ```
//! La chiave AES (32 byte) è cifrata con RSA PKCS#1 v1.5; il testo con AES-256-CBC + PKCS#7.
//! I dati binari sono in esadecimale, o in base64 se è presente `ENCODE_BASE64`.

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::Engine as _;
use rand::RngCore;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};

pub const RSA_512: u32 = 0x0000_0001;
pub const RSA_1024: u32 = 0x0000_0002;
pub const RSA_2048: u32 = 0x0000_0004;
pub const RC2_40: u32 = 0x0000_1000;
pub const BLOWFISH_128: u32 = 0x0002_0000;
pub const AES_256: u32 = 0x0010_0000;
pub const PACKETNO_IV: u32 = 0x0080_0000;
pub const ENCODE_BASE64: u32 = 0x0100_0000;
pub const SIGN_SHA1: u32 = 0x2000_0000;
pub const SIGN_SHA256: u32 = 0x4000_0000;

pub const OUR_CAPS: u32 = RSA_2048 | RSA_1024 | AES_256 | PACKETNO_IV | ENCODE_BASE64;

type Enc = cbc::Encryptor<aes::Aes256>;
type Dec = cbc::Decryptor<aes::Aes256>;

#[derive(Clone)]
pub struct PeerKey {
    pub caps: u32,
    pub key: RsaPublicKey,
}

pub fn generate() -> RsaPrivateKey {
    RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("generazione chiave RSA")
}

pub fn to_pem(k: &RsaPrivateKey) -> String {
    k.to_pkcs8_pem(LineEnding::LF).map(|s| s.to_string()).unwrap_or_default()
}

pub fn from_pem(s: &str) -> Option<RsaPrivateKey> {
    RsaPrivateKey::from_pkcs8_pem(s).ok()
}

pub fn fingerprint(k: &RsaPublicKey) -> String {
    let d = Sha256::digest(k.n().to_bytes_be());
    let h = hex::encode_upper(&d[..12]);
    h.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("-")
}

pub fn pubkey_answer(k: &RsaPrivateKey) -> String {
    let bits = if k.size() * 8 >= 2048 { RSA_2048 } else { RSA_1024 };
    let caps = (OUR_CAPS & !(RSA_512 | RSA_1024 | RSA_2048)) | bits;
    format!("{:X}:{}-{}", caps, hex::encode_upper(k.e().to_bytes_be()), hex::encode_upper(k.n().to_bytes_be()))
}

pub fn parse_pubkey_answer(s: &str) -> Option<PeerKey> {
    let s = s.trim_matches(|c| c == '\0' || c == ' ');
    let (caps, rest) = s.split_once(':')?;
    let (e, n) = rest.split_once('-')?;
    let n = n.split(':').next()?.trim();
    let caps = u32::from_str_radix(caps, 16).ok()?;
    let e = BigUint::parse_bytes(e.as_bytes(), 16)?;
    let n = BigUint::parse_bytes(n.as_bytes(), 16)?;
    Some(PeerKey { caps, key: RsaPublicKey::new(n, e).ok()? })
}

fn enc_bytes(d: &[u8], caps: u32) -> String {
    if caps & ENCODE_BASE64 != 0 {
        base64::engine::general_purpose::STANDARD.encode(d)
    } else {
        hex::encode_upper(d)
    }
}

fn dec_bytes(s: &str, caps: u32) -> Option<Vec<u8>> {
    let s = s.trim();
    let b64 = || {
        base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(s.trim_end_matches('='))
            .ok()
    };
    if caps & ENCODE_BASE64 != 0 { b64() } else { hex::decode(s).ok().or_else(b64) }
}

fn iv(caps: u32, packet_no: u64) -> [u8; 16] {
    let mut iv = [0u8; 16];
    if caps & PACKETNO_IV != 0 {
        let s = packet_no.to_string();
        let b = s.as_bytes();
        let n = b.len().min(16);
        iv[..n].copy_from_slice(&b[..n]);
    }
    iv
}

/// Opzioni da usare con un destinatario, 0 se non c'è un algoritmo in comune.
pub fn choose_caps(peer: &PeerKey) -> u32 {
    if peer.caps & AES_256 == 0 {
        return 0;
    }
    let mut caps = AES_256 | if peer.key.size() * 8 >= 2048 { RSA_2048 } else { RSA_1024 };
    if peer.caps & PACKETNO_IV != 0 {
        caps |= PACKETNO_IV;
    }
    caps
}

pub fn encrypt(peer: &PeerKey, plain: &[u8], packet_no: u64) -> Result<String, String> {
    let caps = choose_caps(peer);
    if caps == 0 {
        return Err("il destinatario non supporta AES-256".into());
    }
    let mut session = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut session);
    let ek = peer
        .key
        .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, &session)
        .map_err(|e| e.to_string())?;
    let ct = Enc::new(&session.into(), &iv(caps, packet_no).into()).encrypt_padded_vec_mut::<Pkcs7>(plain);
    Ok(format!("{:X}:{}:{}", caps, enc_bytes(&ek, caps), enc_bytes(&ct, caps)))
}

pub fn decrypt(k: &RsaPrivateKey, text: &str, packet_no: u64) -> Result<Vec<u8>, String> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() < 3 {
        return Err("formato non valido".into());
    }
    let caps = u32::from_str_radix(parts[0], 16).map_err(|_| "capacità non valide")?;
    if caps & AES_256 == 0 {
        return Err("algoritmo non supportato (solo AES-256)".into());
    }
    let ek = dec_bytes(parts[1], caps).ok_or("codifica della chiave non valida")?;
    let ct = dec_bytes(parts[2], caps).ok_or("codifica del messaggio non valida")?;
    // CryptoAPI di Windows usa l'ordine dei byte inverso: proviamo entrambi
    let mut rev = ek.clone();
    rev.reverse();
    let session = [ek, rev]
        .iter()
        .find_map(|b| k.decrypt(Pkcs1v15Encrypt, b).ok().filter(|s| s.len() == 32))
        .ok_or("impossibile decifrare la chiave di sessione")?;
    let mut key = [0u8; 32];
    key.copy_from_slice(&session);
    let dec = Dec::new(&key.into(), &iv(caps, packet_no).into());
    match dec.clone().decrypt_padded_vec_mut::<Pkcs7>(&ct) {
        Ok(v) => Ok(v),
        Err(_) => {
            // senza padding: rimuovi gli zeri finali
            let mut buf = ct.clone();
            use aes::cipher::block_padding::NoPadding;
            let out = dec.decrypt_padded_mut::<NoPadding>(&mut buf).map_err(|_| "dati cifrati non validi")?;
            let mut v = out.to_vec();
            while v.last() == Some(&0) {
                v.pop();
            }
            Ok(v)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let k = generate();
        let mut peer = parse_pubkey_answer(&pubkey_answer(&k)).unwrap();
        assert!(peer.caps & AES_256 != 0);
        for b64 in [false, true] {
            if b64 {
                peer.caps |= ENCODE_BASE64;
            }
            let ct = encrypt(&peer, "segreto àèì".as_bytes(), 123456).unwrap();
            assert_eq!(decrypt(&k, &ct, 123456).unwrap(), "segreto àèì".as_bytes());
        }
        let k2 = from_pem(&to_pem(&k)).unwrap();
        assert_eq!(fingerprint(&k.to_public_key()), fingerprint(&k2.to_public_key()));
    }
}
