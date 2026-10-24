//! usage: s3a-timestamp anchor <hex-sha256> <out.tsa> <url>...   |   s3a-timestamp verify <file.tsa> [k]
use s3a_timestamp::*;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(String::as_str) {
        Some("anchor") if a.len() >= 5 => {
            let h = &a[2]; if h.len() != 64 { return Err("digest must be 64 hex chars".into()); }
            let mut d = [0u8; 32]; for i in 0..32 { d[i] = u8::from_str_radix(&h[2 * i..2 * i + 2], 16)?; }
            let mut tokens = vec![];
            for u in &a[4..] { match fetch(u, &d) { Ok(t) => { eprintln!("ok   {u}"); tokens.push(t) } Err(e) => eprintln!("fail {u}: {e}") } }
            std::fs::write(&a[3], Anchor { digest: d, tokens }.to_bytes())?;
        }
        Some("verify") if a.len() >= 3 => {
            let k = a.get(3).map(|s| s.parse()).transpose()?.unwrap_or(DEFAULT_K);
            for i in Anchor::from_bytes(&std::fs::read(&a[2])?)?.verify(k)? { println!("{}", i.gen_time); }
        }
        _ => return Err("usage: anchor <hex> <out> <url>... | verify <file> [k]".into()),
    }
    Ok(())
}
