//! Signed share links and QR codes (S3A-VAULT §17).
//!
//! A share token is a canonical byte string signed by the issuer's Ed25519 key.
//! The same text (`gate://<address>/share/<base64url>`) is used for the
//! tappable link and the QR payload, so a QR and a link verify identically.
//! Nothing here touches the S3A archive format; tokens live outside archives.

use ed25519_dalek::{Signature, VerifyingKey};
use s3a_core::algorithms::{check_readable, check_writable, AlgKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::address::GateAddress;
use crate::errors::VaultError;
use crate::keystore::KeyStore;

/// Domain separator: signatures over share tokens can never be confused with
/// any other S3A signature (seals, grants, access codes).
pub const SHARE_DOMAIN: &[u8] = b"S3A-VAULT-SHARE\0";
pub const SHARE_VERSION: u8 = 1;
/// Allowed clock skew for `issued_at` in the future, in seconds.
pub const MAX_CLOCK_SKEW: u64 = 300;
/// Hard cap on token lifetime (30 days); longer tokens are rejected.
pub const MAX_LIFETIME: u64 = 30 * 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareToken {
    pub sig_alg: String,
    pub issuer_pubkey: [u8; 32],
    pub item_hash: [u8; 32],
    pub issued_at: u64,
    pub expires_at: u64,
    pub nonce: [u8; 16],
    /// Optional audience: the gate address string of the intended recipient.
    pub audience: Option<String>,
    pub signature: [u8; 64],
}

/// Why `open` refused a token. Every variant means "do not act".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reject {
    Malformed(String),
    Algorithm(String),
    AddressMismatch,
    BadSignature,
    Expired,
    NotYetValid,
    LifetimeTooLong,
    WrongAudience(String),
    ItemHashMismatch,
    Replay,
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reject::Malformed(m) => write!(f, "malformed share token: {m}"),
            Reject::Algorithm(m) => write!(f, "signature algorithm rejected: {m}"),
            Reject::AddressMismatch => write!(f, "gate address does not match the signing key"),
            Reject::BadSignature => write!(f, "signature does not verify (forged or tampered)"),
            Reject::Expired => write!(f, "token expired"),
            Reject::NotYetValid => write!(f, "token issued in the future"),
            Reject::LifetimeTooLong => write!(f, "token lifetime exceeds 30 days"),
            Reject::WrongAudience(a) => write!(f, "token is addressed to {a}, not to you"),
            Reject::ItemHashMismatch => write!(f, "item content hash does not match"),
            Reject::Replay => write!(f, "nonce already seen (replay)"),
        }
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

impl ShareToken {
    pub fn address(&self) -> GateAddress {
        GateAddress::from_public_key(&self.issuer_pubkey)
    }

    /// Canonical signed bytes. Fixed order, fixed widths, length-prefixed strings.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(160);
        b.extend_from_slice(SHARE_DOMAIN);
        b.push(SHARE_VERSION);
        put_str(&mut b, &self.sig_alg);
        b.extend_from_slice(&self.issuer_pubkey);
        let a = self.address();
        b.extend_from_slice(&a.prefix);
        b.push(a.checksum);
        b.extend_from_slice(&self.item_hash);
        b.extend_from_slice(&self.issued_at.to_be_bytes());
        b.extend_from_slice(&self.expires_at.to_be_bytes());
        b.extend_from_slice(&self.nonce);
        match &self.audience {
            None => b.push(0),
            Some(a) => {
                b.push(1);
                put_str(&mut b, a);
            }
        }
        b
    }

    /// Wire bytes = canonical bytes || 64-byte signature.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.canonical_bytes();
        b.extend_from_slice(&self.signature);
        b
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, Reject> {
        let m = |s: &str| Reject::Malformed(s.into());
        let mut r = Cursor { b, i: 0 };
        if r.take(SHARE_DOMAIN.len())? != SHARE_DOMAIN {
            return Err(m("bad domain"));
        }
        if r.take(1)?[0] != SHARE_VERSION {
            return Err(m("unsupported version"));
        }
        let sig_alg = r.string()?;
        let issuer_pubkey: [u8; 32] = r.take(32)?.try_into().unwrap();
        let prefix: [u8; 8] = r.take(8)?.try_into().unwrap();
        let checksum = r.take(1)?[0];
        let item_hash: [u8; 32] = r.take(32)?.try_into().unwrap();
        let issued_at = u64::from_be_bytes(r.take(8)?.try_into().unwrap());
        let expires_at = u64::from_be_bytes(r.take(8)?.try_into().unwrap());
        let nonce: [u8; 16] = r.take(16)?.try_into().unwrap();
        let audience = match r.take(1)?[0] {
            0 => None,
            1 => Some(r.string()?),
            _ => return Err(m("bad audience flag")),
        };
        let signature: [u8; 64] = r.take(64)?.try_into().unwrap();
        if r.i != b.len() {
            return Err(m("trailing bytes"));
        }
        let t = ShareToken {
            sig_alg,
            issuer_pubkey,
            item_hash,
            issued_at,
            expires_at,
            nonce,
            audience,
            signature,
        };
        let a = t.address();
        if a.prefix != prefix || a.checksum != checksum {
            return Err(Reject::AddressMismatch);
        }
        Ok(t)
    }

    /// `gate://<address>/share/<base64url(wire bytes)>` — the link and the QR payload.
    pub fn to_link(&self) -> String {
        format!(
            "{}/share/{}",
            self.address().to_uri(),
            b64url_encode(&self.to_bytes())
        )
    }

    pub fn from_link(link: &str) -> Result<Self, Reject> {
        let link = link.trim();
        let rest = link
            .strip_prefix("gate://")
            .ok_or_else(|| Reject::Malformed("not a gate:// link".into()))?;
        let (host, payload) = rest
            .split_once("/share/")
            .ok_or_else(|| Reject::Malformed("missing /share/ path".into()))?;
        let host_addr = GateAddress::parse_address(host)
            .map_err(|_| Reject::Malformed("bad gate address in link".into()))?;
        let bytes =
            b64url_decode(payload).ok_or_else(|| Reject::Malformed("bad base64url".into()))?;
        let t = Self::from_bytes(&bytes)?;
        // The visible host must be the address the key derives; otherwise a
        // link could display one address while carrying another issuer.
        if !host_addr.matches_key(&t.issuer_pubkey) {
            return Err(Reject::AddressMismatch);
        }
        Ok(t)
    }
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Reject> {
        if self.i + n > self.b.len() {
            return Err(Reject::Malformed("truncated".into()));
        }
        let s = &self.b[self.i..self.i + n];
        self.i += n;
        Ok(s)
    }
    fn string(&mut self) -> Result<String, Reject> {
        let n = u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as usize;
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| Reject::Malformed("bad utf-8".into()))
    }
}

/// Issues a signed share token for `item_hash` with the identity's key.
#[allow(clippy::too_many_arguments)]
pub fn issue(
    keystore: &dyn KeyStore,
    identity_id: &[u8; 16],
    item_hash: [u8; 32],
    issued_at: u64,
    ttl: u64,
    nonce: [u8; 16],
    audience: Option<String>,
) -> Result<ShareToken, VaultError> {
    let sig_alg = "ed25519".to_string();
    check_writable(AlgKind::Signature, &sig_alg)
        .map_err(|e| VaultError::KeyStoreError(e.to_string()))?;
    if let Some(a) = &audience {
        let aa = GateAddress::parse_address(a)
            .map_err(|_| VaultError::KeyStoreError("bad audience address".into()))?;
        // New tokens must name the audience by its full 256-bit address.
        if aa.is_legacy() {
            return Err(VaultError::KeyStoreError(
                "audience must be a full s3a2- address (legacy 64-bit addresses are too weak)"
                    .into(),
            ));
        }
    }
    let mut t = ShareToken {
        sig_alg,
        issuer_pubkey: keystore.get_public_key(identity_id)?,
        item_hash,
        issued_at,
        expires_at: issued_at.saturating_add(ttl.min(MAX_LIFETIME)),
        nonce,
        audience,
        signature: [0; 64],
    };
    t.signature = keystore.sign(identity_id, &t.canonical_bytes())?;
    Ok(t)
}

/// Stateless checks: algorithm, self-certifying address, signature, time, audience, item.
pub fn verify(
    t: &ShareToken,
    now: u64,
    my_address: Option<&str>,
    item_hash: Option<&[u8; 32]>,
) -> Result<(), Reject> {
    check_readable(AlgKind::Signature, &t.sig_alg).map_err(|e| Reject::Algorithm(e.to_string()))?;
    if t.sig_alg != "ed25519" {
        // "none" and future IDs are not acceptable for share tokens.
        return Err(Reject::Algorithm(format!(
            "'{}' not accepted for share tokens",
            t.sig_alg
        )));
    }
    let vk = VerifyingKey::from_bytes(&t.issuer_pubkey).map_err(|_| Reject::BadSignature)?;
    vk.verify_strict(&t.canonical_bytes(), &Signature::from_bytes(&t.signature))
        .map_err(|_| Reject::BadSignature)?;
    if t.expires_at <= t.issued_at || t.expires_at - t.issued_at > MAX_LIFETIME {
        return Err(Reject::LifetimeTooLong);
    }
    if t.issued_at > now + MAX_CLOCK_SKEW {
        return Err(Reject::NotYetValid);
    }
    if now >= t.expires_at {
        return Err(Reject::Expired);
    }
    if let Some(aud) = &t.audience {
        let ok = my_address
            .and_then(|m| GateAddress::parse_address(m).ok())
            .zip(GateAddress::parse_address(aud).ok())
            .map(|(a, b)| a.same_identity(&b))
            .unwrap_or(false);
        if !ok {
            return Err(Reject::WrongAudience(aud.clone()));
        }
    }
    if let Some(h) = item_hash {
        if h != &t.item_hash {
            return Err(Reject::ItemHashMismatch);
        }
    }
    Ok(())
}

/// Local replay store: nonce (hex of issuer||nonce) -> expiry. Expired entries are pruned.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SeenStore {
    pub seen: BTreeMap<String, u64>,
}

impl SeenStore {
    pub fn load(path: &Path) -> Result<Self, VaultError> {
        match fs::read(path) {
            Ok(b) => {
                serde_json::from_slice(&b).map_err(|e| VaultError::KeyStoreError(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), VaultError> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap())?;
        fs::rename(tmp, path)?;
        Ok(())
    }
    fn key(t: &ShareToken) -> String {
        format!("{}{}", hex::encode(t.issuer_pubkey), hex::encode(t.nonce))
    }
    /// Records the token; returns Err(Replay) if its nonce was already seen.
    pub fn check_and_record(&mut self, t: &ShareToken, now: u64) -> Result<(), Reject> {
        self.seen.retain(|_, exp| *exp > now);
        let k = Self::key(t);
        if self.seen.contains_key(&k) {
            return Err(Reject::Replay);
        }
        self.seen.insert(k, t.expires_at);
        Ok(())
    }
}

/// Pinned contacts: full public key hex -> petname. Petnames are shown only on
/// an exact 32-byte key match, never on an address match (legacy 64-bit addresses are spoofable).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Contacts {
    pub by_key: BTreeMap<String, String>,
}

impl Contacts {
    pub fn load(path: &Path) -> Result<Self, VaultError> {
        match fs::read(path) {
            Ok(b) => {
                serde_json::from_slice(&b).map_err(|e| VaultError::KeyStoreError(e.to_string()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), VaultError> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        fs::write(path, serde_json::to_vec_pretty(self).unwrap())?;
        Ok(())
    }
    pub fn petname(&self, pk: &[u8; 32]) -> Option<&str> {
        self.by_key.get(&hex::encode(pk)).map(|s| s.as_str())
    }
}

/// Full verification including replay. Records the nonce only if every other check passed.
pub fn open(
    t: &ShareToken,
    now: u64,
    my_address: Option<&str>,
    item_hash: Option<&[u8; 32]>,
    seen: &mut SeenStore,
) -> Result<(), Reject> {
    verify(t, now, my_address, item_hash)?;
    seen.check_and_record(t, now)
}

/// Human-readable result line block.
pub fn describe(
    t: &ShareToken,
    result: &Result<(), Reject>,
    contacts: &Contacts,
    item_checked: bool,
) -> String {
    match result {
        Err(r) => format!("REJECTED: {r}\nDo not trust or act on this code."),
        Ok(()) => {
            let who = match contacts.petname(&t.issuer_pubkey) {
                Some(p) => format!("{p} (pinned contact)"),
                None => {
                    "UNKNOWN ISSUER - not in your contacts; compare the key fingerprint out of band"
                        .into()
                }
            };
            let fp = hex::encode(Sha256::digest(t.issuer_pubkey));
            format!(
                "VERIFIED\nissuer:      {who}\naddress:     {}\nkey sha256:  {}\nitem sha256: {}{}\nexpires:     {} (unix)\n",
                t.address().to_uri(),
                fp,
                hex::encode(t.item_hash),
                if item_checked { " (matches local item)" } else { " (item not supplied; not compared)" },
                t.expires_at
            )
        }
    }
}

// ---- QR encode / decode -------------------------------------------------

/// SVG QR code of `payload` (EC level M; quiet zone included).
pub fn qr_svg(payload: &str) -> Result<String, VaultError> {
    let code = qrcode::QrCode::with_error_correction_level(payload.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| VaultError::KeyStoreError(format!("qr encode: {e}")))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(320, 320)
        .build())
}

/// Grayscale PNG QR code of `payload`.
pub fn qr_png(payload: &str, module_px: u32) -> Result<Vec<u8>, VaultError> {
    let code = qrcode::QrCode::with_error_correction_level(payload.as_bytes(), qrcode::EcLevel::M)
        .map_err(|e| VaultError::KeyStoreError(format!("qr encode: {e}")))?;
    let img = code
        .render::<image::Luma<u8>>()
        .module_dimensions(module_px, module_px)
        .build();
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| VaultError::KeyStoreError(format!("png: {e}")))?;
    Ok(out.into_inner())
}

/// Decodes every QR code in a grayscale image; returns the payload strings.
/// Tries the raw image, then a contrast-stretched and a binarised copy, so
/// washed-out phone screenshots still decode. Decoding never affects trust:
/// whatever is decoded must still pass `verify`.
pub fn qr_decode_luma(img: &image::GrayImage) -> Vec<String> {
    let try_one = |im: image::GrayImage| -> Vec<String> {
        let mut p = rqrr::PreparedImage::prepare(im);
        p.detect_grids()
            .into_iter()
            .filter_map(|g| g.decode().ok().map(|(_, s)| s))
            .collect()
    };
    let r = try_one(img.clone());
    if !r.is_empty() {
        return r;
    }
    let (lo, hi) = img
        .pixels()
        .fold((255u8, 0u8), |(l, h), p| (l.min(p.0[0]), h.max(p.0[0])));
    if hi <= lo {
        return r;
    }
    let span = (hi - lo) as u32;
    let mut st = img.clone();
    for p in st.pixels_mut() {
        p.0[0] = ((p.0[0] - lo) as u32 * 255 / span) as u8;
    }
    let r = try_one(st.clone());
    if !r.is_empty() {
        return r;
    }
    let mid = lo + (hi - lo) / 2;
    for (p, q) in st.pixels_mut().zip(img.pixels()) {
        p.0[0] = if q.0[0] > mid { 255 } else { 0 };
    }
    try_one(st)
}

/// Decodes QR payloads from an image file (PNG).
pub fn qr_decode_file(path: &Path) -> Result<Vec<String>, VaultError> {
    let img = image::open(path)
        .map_err(|e| VaultError::KeyStoreError(format!("image: {e}")))?
        .to_luma8();
    Ok(qr_decode_luma(&img))
}

/// Accepts a pasted link or a path to an image containing the QR code.
/// Returns the first payload that parses as a share token (or the parse error).
pub fn token_from_input(input: &str) -> Result<ShareToken, Reject> {
    if input.trim().starts_with("gate://") {
        return ShareToken::from_link(input);
    }
    let payloads =
        qr_decode_file(Path::new(input)).map_err(|e| Reject::Malformed(e.to_string()))?;
    if payloads.is_empty() {
        return Err(Reject::Malformed("no QR code found in image".into()));
    }
    if payloads.len() > 1 {
        // A sticker partly over another code could yield two symbols; refuse to guess.
        return Err(Reject::Malformed(format!(
            "{} QR codes in image; expected exactly one",
            payloads.len()
        )));
    }
    ShareToken::from_link(&payloads[0])
}

// ---- strict base64url (RFC 4648 §5, no padding) --------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

pub fn b64url_encode(d: &[u8]) -> String {
    let mut s = String::with_capacity(d.len() * 4 / 3 + 3);
    for c in d.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..=c.len() {
            s.push(B64[(n >> (18 - 6 * k) & 63) as usize] as char);
        }
    }
    s
}

/// Rejects non-alphabet characters and non-canonical trailing bits.
pub fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        let v = B64.iter().position(|&c| c == ch)? as u32;
        acc = acc << 6 | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if acc != 0 {
        return None;
    }
    Some(out)
}
