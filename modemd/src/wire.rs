//! CCCI packet header and the parameter list of FS/RPC requests.
//!
//! Packets on /dev/ccci_fs and /dev/ccci_rpc keep their 16 byte header:
//!
//! ```text
//!     u32 data0      FS: bit 31 = more fragments follow
//!     u32 len        packet length including the header
//!     u16 channel
//!     u16 seq
//!     u32 reserved   buffer slot of the request, echoed in the reply
//! ```
//!
//! followed by a request: `u32 op, u32 count, count x {u32 len, data, pad
//! to 4}`. Replies carry `op | 0xffff0000` and the result parameters.

pub const HDR_LEN: usize = 16;
pub const RESP: u32 = 0xffff_0000;

#[derive(Clone, Copy, Debug)]
pub struct Header {
    pub data0: u32,
    pub len: u32,
    pub slot: u32,
}

pub fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

pub fn parse_header(pkt: &[u8]) -> Option<Header> {
    Some(Header {
        data0: u32_at(pkt, 0)?,
        len: u32_at(pkt, 4)?,
        slot: u32_at(pkt, 12)?,
    })
}

pub fn header(data0: u32, slot: u32) -> [u8; HDR_LEN] {
    let mut h = [0u8; HDR_LEN];
    h[0..4].copy_from_slice(&data0.to_le_bytes());
    h[12..16].copy_from_slice(&slot.to_le_bytes());
    h
}

/// `op` and the parameters of a request; a truncated parameter is clipped.
pub fn parse_params(buf: &[u8]) -> (u32, Vec<Vec<u8>>) {
    let op = u32_at(buf, 0).unwrap_or(0);
    let n = u32_at(buf, 4).unwrap_or(0);
    let mut off = 8;
    let mut args = Vec::new();
    for _ in 0..n {
        let Some(len) = u32_at(buf, off) else { break };
        let len = len as usize;
        let start = (off + 4).min(buf.len());
        let end = (off + 4).saturating_add(len).min(buf.len());
        args.push(buf[start..end].to_vec());
        off = off + 4 + ((len + 3) & !3);
    }
    (op, args)
}

pub fn build_params(op: u32, params: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(params.len() as u32).to_le_bytes());
    for p in params {
        out.extend_from_slice(&(p.len() as u32).to_le_bytes());
        out.extend_from_slice(p);
        out.resize(out.len() + (p.len().wrapping_neg() & 3), 0);
    }
    out
}

pub fn i32p(v: i32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

pub fn u32p(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_roundtrip() {
        let p = vec![vec![1, 2, 3], vec![], vec![9; 8]];
        let b = build_params(0x1001, &p);
        assert_eq!(b.len(), 8 + 4 + 4 + 4 + 4 + 8);
        let (op, a) = parse_params(&b);
        assert_eq!(op, 0x1001);
        assert_eq!(a, p);
    }
}
