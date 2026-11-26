//! Formato "IPDict" dei pacchetti di IP Messenger 5 (v5).
//!
//! ```text
//! IP2:<lunghezza hex>:<chiave>:<lunghezza hex>:<valore>:<chiave>:...:Z
//! ```
//! I numeri sono in esadecimale, testi e byte così come sono. Un messaggio cifrato
//! è un dizionario esterno con EF/EI/EK/EB: EB contiene il dizionario interno cifrato
//! con AES-256-CTR (chiave cifrata con RSA PKCS#1 v1.5 in EK, contatore iniziale in EI).
//! Il dizionario interno termina con PUBE/PUBN/EF/EC e la firma SHA-256 in SIGN, calcolata
//! sul dizionario impacchettato senza SIGN.

use aes::cipher::{KeyIvInit, StreamCipher};
use rand::RngCore;
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};

use crate::crypto;

const HEAD: &[u8] = b"IP2:";
const FOOT: &[u8] = b":Z";

pub const NEW_VERSION: i64 = 3;
pub const IPDICT_CTR: i64 = 0x0040_0000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dict {
    items: Vec<(String, Vec<u8>)>,
}

fn hex(n: usize) -> String {
    format!("{n:x}")
}

impl Dict {
    pub fn new() -> Dict {
        Dict::default()
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn has(&self, k: &str) -> bool {
        self.items.iter().any(|(x, _)| x == k)
    }
    fn set(&mut self, k: &str, v: Vec<u8>) {
        match self.items.iter_mut().find(|(x, _)| x == k) {
            Some(it) => it.1 = v,
            None => self.items.push((k.to_string(), v)),
        }
    }
    pub fn put_int(&mut self, k: &str, v: i64) {
        let s = if v >= 0 { format!("{v:x}") } else { format!("-{:x}", -v) };
        self.set(k, s.into_bytes());
    }
    pub fn put_str(&mut self, k: &str, v: &str) {
        self.set(k, v.as_bytes().to_vec());
    }
    pub fn put_bytes(&mut self, k: &str, v: &[u8]) {
        self.set(k, v.to_vec());
    }
    pub fn put_dict_list(&mut self, k: &str, list: &[Dict]) {
        let mut out = Vec::new();
        for (i, d) in list.iter().enumerate() {
            if i > 0 {
                out.push(b':');
            }
            let c = d.content(d.len());
            out.extend_from_slice(hex(c.len()).as_bytes());
            out.push(b':');
            out.extend_from_slice(&c);
        }
        self.set(k, out);
    }
    pub fn get_bytes(&self, k: &str) -> Option<&[u8]> {
        self.items.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_slice())
    }
    pub fn get_str(&self, k: &str) -> Option<String> {
        self.get_bytes(k).map(|v| String::from_utf8_lossy(v).into_owned())
    }
    pub fn get_int(&self, k: &str) -> Option<i64> {
        let s = std::str::from_utf8(self.get_bytes(k)?).ok()?;
        match s.strip_prefix('-') {
            Some(r) => i64::from_str_radix(r, 16).ok().map(|v| -v),
            None => u64::from_str_radix(s, 16).ok().map(|v| v as i64),
        }
    }
    pub fn get_dict_list(&self, k: &str) -> Option<Vec<Dict>> {
        let v = self.get_bytes(k)?;
        let mut out = Vec::new();
        let mut s = 0;
        while s < v.len() {
            let c = v[s..].iter().position(|&b| b == b':')? + s;
            let n = usize::from_str_radix(std::str::from_utf8(&v[s..c]).ok()?, 16).ok()?;
            let start = c + 1;
            if start + n > v.len() {
                return None;
            }
            out.push(Dict::unpack_content(&v[start..start + n])?);
            s = start + n;
            if s == v.len() {
                break;
            }
            if v[s] != b':' {
                return None;
            }
            s += 1;
        }
        Some(out)
    }

    /// Contenuto dei primi `n` elementi (senza intestazione).
    fn content(&self, n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, (k, v)) in self.items.iter().take(n).enumerate() {
            if i > 0 {
                out.push(b':');
            }
            out.extend_from_slice(k.as_bytes());
            out.push(b':');
            out.extend_from_slice(hex(v.len()).as_bytes());
            out.push(b':');
            out.extend_from_slice(v);
        }
        out
    }

    /// Pacchetto completo dei primi `n` elementi.
    pub fn pack_n(&self, n: usize) -> Vec<u8> {
        let c = self.content(n);
        let mut out = Vec::with_capacity(c.len() + 16);
        out.extend_from_slice(HEAD);
        out.extend_from_slice(hex(c.len()).as_bytes());
        out.push(b':');
        out.extend_from_slice(&c);
        out.extend_from_slice(FOOT);
        out
    }
    pub fn pack(&self) -> Vec<u8> {
        self.pack_n(self.len())
    }

    fn unpack_content(v: &[u8]) -> Option<Dict> {
        let mut d = Dict::new();
        let mut s = 0;
        while s < v.len() {
            let kc = v[s..].iter().position(|&b| b == b':')? + s;
            if kc == s {
                return None;
            }
            let key = std::str::from_utf8(&v[s..kc]).ok()?.to_string();
            let lc = v[kc + 1..].iter().position(|&b| b == b':')? + kc + 1;
            let n = usize::from_str_radix(std::str::from_utf8(&v[kc + 1..lc]).ok()?, 16).ok()?;
            let start = lc + 1;
            if start + n > v.len() {
                return None;
            }
            d.items.push((key, v[start..start + n].to_vec()));
            s = start + n;
            if s == v.len() {
                break;
            }
            if v[s] != b':' {
                return None;
            }
            s += 1;
        }
        Some(d)
    }

    /// Legge un pacchetto IP2 (che deve occupare tutti i dati).
    pub fn unpack(data: &[u8]) -> Option<Dict> {
        let data = data.strip_suffix(b"\0").unwrap_or(data);
        let rest = data.strip_prefix(HEAD)?;
        let c = rest.iter().position(|&b| b == b':')?;
        let n = usize::from_str_radix(std::str::from_utf8(&rest[..c]).ok()?, 16).ok()?;
        let body = rest.get(c + 1..c + 1 + n)?;
        if rest.get(c + 1 + n..)? != FOOT {
            return None;
        }
        Dict::unpack_content(body)
    }
}

pub fn is_ipdict(data: &[u8]) -> bool {
    data.starts_with(HEAD)
}

type Ctr = ctr::Ctr128BE<aes::Aes256>;

/// Firma e cifra il dizionario interno per `peer` (come MsgMng::EncIPDict).
pub fn encrypt(inner: &Dict, me: &RsaPrivateKey, peer: &RsaPublicKey, caps: u32) -> Result<Vec<u8>, String> {
    let mut d = inner.clone();
    d.items.retain(|(k, _)| k != "SIGN");
    d.put_int("PUBE", u64::from_str_radix(&me.e().to_str_radix(16), 16).unwrap_or(65537) as i64);
    d.put_bytes("PUBN", &me.n().to_bytes_be());
    d.put_int("EF", crypto::SIGN_SHA256 as i64);
    d.put_int("EC", caps as i64);
    let sig = crypto::sign(me, &d.pack()).ok_or("firma non riuscita")?;
    d.put_bytes("SIGN", &sig);
    let mut body = d.pack();

    let mut key = [0u8; 32];
    let mut iv = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut key);
    rand::thread_rng().fill_bytes(&mut iv);
    Ctr::new(&key.into(), &iv.into()).apply_keystream(&mut body);
    let ek = peer.encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, &key).map_err(|e| e.to_string())?;

    let mut out = Dict::new();
    out.put_int("EF", (crypto::RSA_2048 | crypto::AES_256) as i64 | IPDICT_CTR);
    out.put_bytes("EI", &iv);
    out.put_bytes("EK", &ek);
    out.put_bytes("EB", &body);
    Ok(out.pack())
}

/// Risultato della lettura di un pacchetto IP2.
pub struct Opened {
    pub dict: Dict,
    pub encrypted: bool,
    /// firma: None = assente, Some(ok)
    pub signed: Option<bool>,
    /// chiave pubblica del mittente contenuta nel messaggio
    pub sender_key: Option<RsaPublicKey>,
}

/// Decifra (se serve) e verifica un pacchetto IP2 (come MsgMng::ResolveDictMsg).
pub fn open(data: &[u8], me: Option<&RsaPrivateKey>) -> Result<Opened, String> {
    let outer = Dict::unpack(data).ok_or("pacchetto IP2 non valido")?;
    let (dict, encrypted) = if outer.has("EB") {
        let me = me.ok_or("cifratura disattivata")?;
        let iv: [u8; 16] = outer.get_bytes("EI").and_then(|v| v.try_into().ok()).ok_or("EI non valido")?;
        let ek = outer.get_bytes("EK").ok_or("EK mancante")?;
        let mut rev = ek.to_vec();
        rev.reverse();
        let key = [ek.to_vec(), rev]
            .iter()
            .find_map(|b| me.decrypt(Pkcs1v15Encrypt, b).ok().filter(|k| k.len() == 32))
            .ok_or("impossibile decifrare la chiave di sessione")?;
        let key: [u8; 32] = key.try_into().map_err(|_| "chiave non valida")?;
        let mut body = outer.get_bytes("EB").unwrap_or_default().to_vec();
        Ctr::new(&key.into(), &iv.into()).apply_keystream(&mut body);
        (Dict::unpack(&body).ok_or("contenuto cifrato non valido")?, true)
    } else {
        (outer, false)
    };
    let sender_key = match (dict.get_bytes("PUBN"), dict.get_int("PUBE")) {
        (Some(n), Some(e)) => RsaPublicKey::new(BigUint::from_bytes_be(n), BigUint::from(e as u64)).ok(),
        _ => None,
    };
    let signed = match (dict.get_bytes("SIGN"), &sender_key) {
        (Some(sig), Some(k)) if dict.items.last().map(|x| x.0 == "SIGN").unwrap_or(false) => {
            Some(crypto::verify(k, &dict.pack_n(dict.len() - 1), sig))
        }
        (Some(_), _) => Some(false),
        _ => None,
    };
    Ok(Opened { dict, encrypted, signed, sender_key })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_unpack_encrypt() {
        let mut d = Dict::new();
        d.put_int("VER", NEW_VERSION);
        d.put_int("PKT", 0x1234);
        d.put_str("UID", "anna");
        d.put_int("NEG", -5);
        let mut f = Dict::new();
        f.put_int("FI", 0);
        f.put_str("FN", "a:b.png");
        d.put_dict_list("FILE", &[f.clone(), f.clone()]);
        d.put_str("BODY", "ciao: è");
        let p = d.pack();
        assert!(p.starts_with(b"IP2:"));
        assert!(p.ends_with(b":Z"));
        let u = Dict::unpack(&p).unwrap();
        assert_eq!(u, d);
        assert_eq!(u.get_int("PKT"), Some(0x1234));
        assert_eq!(u.get_int("NEG"), Some(-5));
        assert_eq!(u.get_dict_list("FILE").unwrap(), vec![f.clone(), f]);
        // formato esatto di un elemento
        let mut x = Dict::new();
        x.put_int("CMD", 0x20);
        assert_eq!(x.pack(), b"IP2:8:CMD:2:20:Z".to_vec());

        let me = crypto::generate();
        let peer = crypto::generate();
        let enc = encrypt(&d, &me, &peer.to_public_key(), crypto::OUR_CAPS).unwrap();
        let o = open(&enc, Some(&peer)).unwrap();
        assert!(o.encrypted);
        assert_eq!(o.signed, Some(true));
        assert_eq!(o.dict.get_str("BODY").unwrap(), "ciao: è");
        assert_eq!(o.sender_key.unwrap(), me.to_public_key());
    }
}
