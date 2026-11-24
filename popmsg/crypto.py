"""Cifratura dei messaggi compatibile con IP Messenger (RSA + AES-256-CBC).

Scambio:
    A -> B  GETPUBKEY   "<capacità hex>"
    B -> A  ANSPUBKEY   "<capacità hex>:<esponente hex>-<modulo hex>"
    A -> B  SENDMSG|ENCRYPTOPT  "<capacità hex>:<chiave di sessione cifrata>:<messaggio cifrato>[:<firma>]"

La chiave di sessione AES (32 byte) è cifrata con RSA PKCS#1 v1.5 usando la chiave
pubblica del destinatario; il messaggio è cifrato con AES-256-CBC e padding PKCS#7.
I campi binari sono in esadecimale, o in base64 se è presente ENCODE_BASE64.
"""
from __future__ import annotations

import base64
import binascii
import os
from dataclasses import dataclass

from cryptography.hazmat.primitives import hashes, padding as sympad, serialization
from cryptography.hazmat.primitives.asymmetric import padding, rsa
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

# ---- capacità (come in ipmsg.h) ----
RSA_512 = 0x00000001
RSA_1024 = 0x00000002
RSA_2048 = 0x00000004
RC2_40 = 0x00001000
BLOWFISH_128 = 0x00020000
AES_256 = 0x00100000
PACKETNO_IV = 0x00800000
ENCODE_BASE64 = 0x01000000
SIGN_SHA1 = 0x20000000
SIGN_SHA256 = 0x40000000

RSA_MASK = RSA_512 | RSA_1024 | RSA_2048
# cosa sappiamo fare: RSA 2048/1024 per la chiave, AES-256 per il testo
OUR_CAPS = RSA_2048 | RSA_1024 | AES_256 | PACKETNO_IV | ENCODE_BASE64 | SIGN_SHA256 | SIGN_SHA1


class CryptoError(Exception):
    pass


@dataclass
class PeerKey:
    caps: int
    key: rsa.RSAPublicKey

    @property
    def bits(self) -> int:
        return self.key.key_size


def generate_private_key(bits: int = 2048) -> rsa.RSAPrivateKey:
    return rsa.generate_private_key(public_exponent=65537, key_size=bits)


def private_key_to_pem(key: rsa.RSAPrivateKey) -> str:
    return key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8,
                             serialization.NoEncryption()).decode()


def private_key_from_pem(pem: str) -> rsa.RSAPrivateKey:
    return serialization.load_pem_private_key(pem.encode(), password=None)


def fingerprint(key: rsa.RSAPublicKey) -> str:
    n = key.public_numbers().n.to_bytes((key.key_size + 7) // 8, "big")
    h = hashes.Hash(hashes.SHA256())
    h.update(n)
    d = h.finalize().hex().upper()
    return "-".join(d[i:i + 4] for i in range(0, 24, 4))


# ---------------------------------------------------------------- chiavi

def build_pubkey_answer(priv: rsa.RSAPrivateKey) -> str:
    pub = priv.public_key().public_numbers()
    bits = RSA_2048 if priv.key_size >= 2048 else RSA_1024
    caps = (OUR_CAPS & ~RSA_MASK) | bits
    return f"{caps:X}:{pub.e:X}-{pub.n:X}"


def parse_pubkey_answer(text: str) -> PeerKey:
    try:
        caps_s, rest = text.strip("\0 ").split(":", 1)
        e_s, n_s = rest.split("-", 1)
        n_s = n_s.split(":")[0].strip()
        nums = rsa.RSAPublicNumbers(int(e_s, 16), int(n_s, 16))
        return PeerKey(int(caps_s, 16), nums.public_key())
    except (ValueError, TypeError) as exc:
        raise CryptoError(f"chiave pubblica non valida: {exc}") from exc


# ------------------------------------------------------------- codifiche

def _enc(data: bytes, caps: int) -> str:
    if caps & ENCODE_BASE64:
        return base64.b64encode(data).decode()
    return data.hex().upper()


def _dec(text: str, caps: int) -> bytes:
    text = text.strip()
    if caps & ENCODE_BASE64:
        return base64.b64decode(text + "=" * (-len(text) % 4))
    try:
        return bytes.fromhex(text)
    except ValueError:
        return base64.b64decode(text + "=" * (-len(text) % 4))


def _iv(caps: int, packet_no: int) -> bytes:
    if caps & PACKETNO_IV:
        return str(packet_no).encode()[:16].ljust(16, b"\0")
    return b"\0" * 16


def _aes(key: bytes, iv: bytes):
    return Cipher(algorithms.AES(key), modes.CBC(iv))


# --------------------------------------------------------------- cifra

def choose_caps(peer: PeerKey) -> int:
    """Sceglie le opzioni da usare con il destinatario, o 0 se non c'è nulla in comune."""
    if not peer.caps & AES_256:
        return 0
    caps = AES_256 | (RSA_2048 if peer.bits >= 2048 else RSA_1024)
    if peer.caps & PACKETNO_IV:
        caps |= PACKETNO_IV
    return caps


def encrypt_message(peer: PeerKey, plaintext: bytes, packet_no: int) -> str:
    caps = choose_caps(peer)
    if not caps:
        raise CryptoError("il destinatario non supporta AES-256")
    session = os.urandom(32)
    enc_key = peer.key.encrypt(session, padding.PKCS1v15())
    padder = sympad.PKCS7(128).padder()
    data = padder.update(plaintext) + padder.finalize()
    enc = _aes(session, _iv(caps, packet_no)).encryptor()
    ct = enc.update(data) + enc.finalize()
    return f"{caps:X}:{_enc(enc_key, caps)}:{_enc(ct, caps)}"


def decrypt_message(priv: rsa.RSAPrivateKey, text: str, packet_no: int) -> bytes:
    parts = text.split(":")
    if len(parts) < 3:
        raise CryptoError("formato del messaggio cifrato non valido")
    try:
        caps = int(parts[0], 16)
    except ValueError as exc:
        raise CryptoError("capacità non valide") from exc
    if not caps & AES_256:
        raise CryptoError("algoritmo non supportato (solo AES-256)")
    try:
        enc_key = _dec(parts[1], caps)
        ct = _dec(parts[2], caps)
    except (ValueError, binascii.Error) as exc:
        raise CryptoError("codifica non valida") from exc
    session = None
    # alcune implementazioni (CryptoAPI) usano l'ordine dei byte inverso
    for blob in (enc_key, enc_key[::-1]):
        try:
            session = priv.decrypt(blob, padding.PKCS1v15())
            if len(session) == 32:
                break
            session = None
        except ValueError:
            continue
    if session is None:
        raise CryptoError("impossibile decifrare la chiave di sessione")
    dec = _aes(session, _iv(caps, packet_no)).decryptor()
    data = dec.update(ct) + dec.finalize()
    try:
        unpad = sympad.PKCS7(128).unpadder()
        data = unpad.update(data) + unpad.finalize()
    except ValueError:
        data = data.rstrip(b"\0")
    return data
