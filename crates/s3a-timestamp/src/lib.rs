//! RFC 3161 multi-TSA anchoring for a SHA-256 digest (e.g. a run-ledger head).
//!
//! Checks performed on every token: PKIStatus granted, ContentInfo is SignedData,
//! eContentType is id-ct-TSTInfo, message imprint is SHA-256 of *our* digest,
//! nonce matches (when expected), and the signed `messageDigest` attribute equals
//! the hash of the TSTInfo bytes (catches tampering with genTime/imprint).
//!
//! GAP: the CMS signature itself and the TSA certificate chain are NOT verified.
//! A party who can mint arbitrary CMS blobs could forge a token. Verify offline
//! with `openssl ts -verify` until a vetted crate path is added.
#![forbid(unsafe_code)]
use sha2::{Digest, Sha256, Sha384, Sha512};

#[derive(Debug, PartialEq, Eq)]
pub enum Error { Der(&'static str), Status(u64), Imprint, Nonce, MessageDigest, Policy { have: usize, need: usize }, Format(&'static str) }
impl std::fmt::Display for Error { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{self:?}") } }
impl std::error::Error for Error {}
type R<T> = Result<T, Error>;

const OID_SHA256: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
const OID_SHA384: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02];
const OID_SHA512: &[u8] = &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03];
const OID_SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
const OID_TSTINFO: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x10, 0x01, 0x04];
const OID_MSG_DIGEST: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x09, 0x04];

// ---- minimal DER reader ----
#[derive(Clone, Copy)]
struct Tlv<'a> { tag: u8, val: &'a [u8], raw: &'a [u8] }
struct Rd<'a>(&'a [u8]);
impl<'a> Rd<'a> {
    fn done(&self) -> bool { self.0.is_empty() }
    fn peek(&self) -> Option<u8> { self.0.first().copied() }
    fn next(&mut self) -> R<Tlv<'a>> {
        let b = self.0;
        if b.len() < 2 { return Err(Error::Der("short")); }
        let tag = b[0];
        if tag & 0x1f == 0x1f { return Err(Error::Der("high tag")); }
        let (len, hl) = match b[1] {
            l if l < 0x80 => (l as usize, 2),
            0x81..=0x84 => { let n = (b[1] & 0x7f) as usize; if b.len() < 2 + n { return Err(Error::Der("short len")); }
                (b[2..2 + n].iter().fold(0usize, |a, &x| (a << 8) | x as usize), 2 + n) }
            _ => return Err(Error::Der("bad len")),
        };
        let end = hl.checked_add(len).filter(|&e| e <= b.len()).ok_or(Error::Der("truncated"))?;
        self.0 = &b[end..];
        Ok(Tlv { tag, val: &b[hl..end], raw: &b[..end] })
    }
    fn want(&mut self, tag: u8) -> R<Tlv<'a>> { let t = self.next()?; if t.tag == tag { Ok(t) } else { Err(Error::Der("unexpected tag")) } }
    fn opt(&mut self, tag: u8) -> R<Option<Tlv<'a>>> { if self.peek() == Some(tag) { self.next().map(Some) } else { Ok(None) } }
}
fn uint(v: &[u8]) -> R<u64> { let v = strip(v); if v.len() > 8 { return Err(Error::Der("int too big")); } Ok(v.iter().fold(0, |a, &x| (a << 8) | x as u64)) }
fn strip(mut v: &[u8]) -> &[u8] { while v.len() > 1 && v[0] == 0 { v = &v[1..]; } v }
fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut o = vec![tag]; let n = body.len();
    if n < 0x80 { o.push(n as u8) } else { let b = n.to_be_bytes(); let b = strip(&b); o.push(0x80 | b.len() as u8); o.extend_from_slice(b); }
    o.extend_from_slice(body); o
}

/// Build a DER TimeStampReq (v1, SHA-256 imprint, nonce, certReq=true).
pub fn build_request(digest: &[u8; 32], nonce: &[u8]) -> Vec<u8> {
    let alg = der(0x30, &[der(0x06, OID_SHA256), vec![0x05, 0x00]].concat());
    let imprint = der(0x30, &[alg, der(0x04, digest)].concat());
    let mut n = strip(nonce).to_vec(); if n.first().map_or(true, |b| b & 0x80 != 0) { n.insert(0, 0); }
    der(0x30, &[vec![0x02, 0x01, 0x01], imprint, der(0x02, &n), vec![0x01, 0x01, 0xff]].concat())
}

/// A parsed, checked timestamp token.
#[derive(Debug, Clone)]
pub struct TokenInfo { pub gen_time: String, pub tsa_id: Vec<u8>, pub nonce: Option<Vec<u8>>, pub serial: Vec<u8> }

/// Extract the TimeStampToken (ContentInfo DER) from a TimeStampResp, checking status.
pub fn token_from_response(resp: &[u8]) -> R<Vec<u8>> {
    let mut top = Rd(resp); let seq = top.want(0x30)?; let mut r = Rd(seq.val);
    let status = r.want(0x30)?; let s = uint(Rd(status.val).want(0x02)?.val)?;
    if s > 1 { return Err(Error::Status(s)); }
    Ok(r.want(0x30)?.raw.to_vec())
}

/// Verify a token (ContentInfo DER) against our digest and optional expected nonce.
pub fn verify_token(token: &[u8], digest: &[u8; 32], nonce: Option<&[u8]>) -> R<TokenInfo> {
    let mut top = Rd(token); let ci = top.want(0x30)?; if !top.done() { return Err(Error::Der("trailing")); }
    let mut r = Rd(ci.val);
    if r.want(0x06)?.val != OID_SIGNED_DATA { return Err(Error::Format("not SignedData")); }
    let sd = Rd(r.want(0xa0)?.val).want(0x30)?; let mut s = Rd(sd.val);
    s.want(0x02)?; s.want(0x31)?;
    let mut e = Rd(s.want(0x30)?.val);
    if e.want(0x06)?.val != OID_TSTINFO { return Err(Error::Format("not TSTInfo")); }
    let tst = Rd(e.want(0xa0)?.val).want(0x04)?.val;
    s.opt(0xa0)?; s.opt(0xa1)?;
    let mut sis = Rd(s.want(0x31)?.val);
    let mut tsa_id = None; let mut md_ok = false;
    while !sis.done() {
        let mut si = Rd(sis.want(0x30)?.val);
        si.want(0x02)?; let sid = si.next()?;
        let alg = Rd(Rd(si.want(0x30)?.val).want(0x06)?.val.as_ref()).0;
        let h: Vec<u8> = match alg { a if a == OID_SHA256 => Sha256::digest(tst).to_vec(), a if a == OID_SHA384 => Sha384::digest(tst).to_vec(), a if a == OID_SHA512 => Sha512::digest(tst).to_vec(), _ => return Err(Error::Format("signer digest alg")) };
        let attrs = si.want(0xa0)?; let mut a = Rd(attrs.val);
        while !a.done() { let mut at = Rd(a.want(0x30)?.val); if at.want(0x06)?.val == OID_MSG_DIGEST {
            if Rd(at.want(0x31)?.val).want(0x04)?.val != h.as_slice() { return Err(Error::MessageDigest); } md_ok = true; } }
        tsa_id.get_or_insert(sid.raw.to_vec());
    }
    if !md_ok { return Err(Error::MessageDigest); }
    // TSTInfo
    let mut t = Rd(Rd(tst).want(0x30)?.val);
    t.want(0x02)?; t.want(0x06)?;
    let mut mi = Rd(t.want(0x30)?.val);
    let alg = Rd(mi.want(0x30)?.val).want(0x06)?;
    if alg.val != OID_SHA256 || mi.want(0x04)?.val != digest { return Err(Error::Imprint); }
    let serial = t.want(0x02)?.val.to_vec();
    let gen_time = String::from_utf8_lossy(t.want(0x18)?.val).into_owned();
    t.opt(0x30)?; t.opt(0x01)?;
    let got_nonce = t.opt(0x02)?.map(|n| strip(n.val).to_vec());
    if let Some(want) = nonce { if got_nonce.as_deref() != Some(strip(want)) { return Err(Error::Nonce); } }
    // Prefer explicit tsa GeneralName; else the signer identifier.
    let tsa = t.opt(0xa0)?.map(|x| x.val.to_vec());
    Ok(TokenInfo { gen_time, tsa_id: tsa.or(tsa_id).ok_or(Error::Format("no signer"))?, nonce: got_nonce, serial })
}

/// Digest + DER tokens. File format: b"S3ATSA1\n" | digest[32] | u32 n | n × (u32 len | DER).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor { pub digest: [u8; 32], pub tokens: Vec<Vec<u8>> }
const MAGIC: &[u8; 8] = b"S3ATSA1\n";
impl Anchor {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = MAGIC.to_vec(); o.extend_from_slice(&self.digest); o.extend_from_slice(&(self.tokens.len() as u32).to_be_bytes());
        for t in &self.tokens { o.extend_from_slice(&(t.len() as u32).to_be_bytes()); o.extend_from_slice(t); } o
    }
    pub fn from_bytes(b: &[u8]) -> R<Self> {
        let f = Error::Format("anchor file");
        if b.len() < 44 || &b[..8] != MAGIC { return Err(f); }
        let digest: [u8; 32] = b[8..40].try_into().unwrap();
        let n = u32::from_be_bytes(b[40..44].try_into().unwrap()) as usize; let mut p = 44; let mut tokens = vec![];
        for _ in 0..n { let l = b.get(p..p + 4).ok_or(Error::Format("anchor file"))?; let l = u32::from_be_bytes(l.try_into().unwrap()) as usize; p += 4;
            tokens.push(b.get(p..p.checked_add(l).ok_or(Error::Format("anchor file"))?).ok_or(Error::Format("anchor file"))?.to_vec()); p += l; }
        if p != b.len() { return Err(f); }
        Ok(Anchor { digest, tokens })
    }
    /// Count tokens that verify AND come from distinct TSA identities; require >= k.
    pub fn verify(&self, k: usize) -> R<Vec<TokenInfo>> {
        let mut ok: Vec<TokenInfo> = vec![];
        for t in &self.tokens { if let Ok(i) = verify_token(t, &self.digest, None) { if !ok.iter().any(|o| o.tsa_id == i.tsa_id) { ok.push(i); } } }
        if ok.len() < k { return Err(Error::Policy { have: ok.len(), need: k }); }
        Ok(ok)
    }
}
pub const DEFAULT_K: usize = 2;

/// Fetch a token from one TSA URL (feature `fetch`).
#[cfg(feature = "fetch")]
pub fn fetch(url: &str, digest: &[u8; 32]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    use std::io::Read;
    let nonce: [u8; 8] = Sha256::digest([digest.as_slice(), &std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_be_bytes(), url.as_bytes()].concat())[..8].try_into()?;
    let resp = ureq::post(url).set("Content-Type", "application/timestamp-query").timeout(std::time::Duration::from_secs(20)).send_bytes(&build_request(digest, &nonce))?;
    let mut body = vec![]; resp.into_reader().take(1 << 20).read_to_end(&mut body)?;
    let tok = token_from_response(&body)?; verify_token(&tok, digest, Some(&nonce))?; Ok(tok)
}
