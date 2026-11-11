//! MIPC, as the stock RIL talks to the modem on ttyCMIPC1: the messages are
//! the ones the stock RIL sends (session open, NW_SET_RADIO_STATE_REQ of
//! SIM1).

use crate::log;
use crate::rpc::hex;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;

const MIPC_PORT: &str = "/dev/ccci_mipc1";
const MIPC_OPEN: &str = "8419542400000000ff000103010028000001040002000000\
                         01010f002f6465762f747479434d495043310000000000000201010001000000";
const MIPC_RADIO_STATE: (&str, &str) = ("84195424000000000100030402010800000101000", "000000");

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

/// Send one message, return the reply that arrives within 2 s
fn request(f: &mut std::fs::File, msg: &[u8]) -> Option<Vec<u8>> {
    f.write_all(msg).ok()?;
    let mut pfd = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd
    if unsafe { libc::poll(&mut pfd, 1, 2000) } <= 0 {
        return None;
    }
    let mut buf = vec![0u8; 65536];
    let n = f.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(buf)
}

/// Set the radio state of SIM1; the first call also starts the SIM
pub fn radio_state(on: bool) {
    let mut f = match OpenOptions::new().read(true).write(true).open(MIPC_PORT) {
        Ok(f) => f,
        Err(e) => {
            log(&format!("mipc: cannot open {MIPC_PORT}: {e}"));
            return;
        }
    };
    request(&mut f, &unhex(MIPC_OPEN));
    let msg = format!("{}{}{}", MIPC_RADIO_STATE.0, on as u8, MIPC_RADIO_STATE.1);
    let cnf = request(&mut f, &unhex(&msg));
    log(&format!(
        "mipc: radio state {}: {}",
        on as u8,
        cnf.map_or("no reply".into(), |c| hex(&c))
    ));
}

#[cfg(test)]
mod tests {
    #[test]
    fn messages() {
        assert_eq!(super::unhex(super::MIPC_OPEN).len(), 56);
        let m = format!("{}{}{}", super::MIPC_RADIO_STATE.0, 1, super::MIPC_RADIO_STATE.1);
        assert_eq!(m, "84195424000000000100030402010800000101000" .to_owned() + "1000000");
        assert_eq!(super::unhex(&m).len(), 24);
    }
}
