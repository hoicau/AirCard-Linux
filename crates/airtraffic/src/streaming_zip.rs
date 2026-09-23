//! StreamingZip uses BE32 framing; ATC uses LE32. Never reuse the ATC frame codec.
use crate::handshake::DuplexTransport;
use aircard_core::{decode_binary, encode_binary};
use plist::{Dictionary, Value};
use serde::Serialize;
use std::{
    io,
    time::{Duration, Instant},
};

#[derive(Debug, Serialize)]
pub struct Failure {
    pub stage: &'static str,
    pub kind: &'static str,
    pub sent: usize,
    pub received: usize,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub event: &'static str,
    pub sent: usize,
    pub received: usize,
    pub complete: bool,
    pub failure: Option<Failure>,
}
pub fn stage<T: DuplexTransport>(
    mut transport: T,
    source: &str,
    archive: &[u8],
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Report {
    let mut report = Report {
        event: "streaming_zip_complete",
        sent: 0,
        received: 0,
        complete: false,
        failure: None,
    };
    let start = Instant::now();
    let result = (|| -> Result<(), (&'static str, &'static str)> {
        let transaction = source
            .strip_prefix("AirCard-Linux-Theme-")
            .ok_or(("validate", "source"))?;
        aircard_core::staging::validate_transaction(transaction)
            .map_err(|_| ("validate", "source"))?;
        if archive.is_empty() || archive.len() > 64 * 1024 * 1024 {
            return Err(("validate", "archive_limit"));
        }
        let payload = encode_binary(&Value::Dictionary(Dictionary::from_iter([(
            "MediaSubdir",
            Value::String(source.into()),
        )])))
        .map_err(|_| ("encode", "plist"))?;
        let header = (payload.len() as u32).to_be_bytes();
        for bytes in [&header[..], &payload, archive] {
            let mut offset = 0;
            while offset < bytes.len() {
                let budget = budget(start, timeout, cancelled).map_err(|e| ("send", e))?;
                let n = transport
                    .send(&bytes[offset..bytes.len().min(offset + 65536)], budget)
                    .map_err(|e| ("send", classify(e)))?;
                if n == 0 || n > (bytes.len() - offset).min(65536) {
                    return Err(("send", "disconnected_or_invalid_count"));
                }
                offset += n;
                report.sent += n;
            }
        }
        let mut header = [0; 4];
        receive(
            &mut transport,
            &mut header,
            start,
            timeout,
            cancelled,
            &mut report.received,
        )
        .map_err(|e| ("response_header", e))?;
        let length = u32::from_be_bytes(header) as usize;
        if !(8..=65536).contains(&length) {
            return Err(("response_header", "length_limit"));
        }
        let mut data = vec![0; length];
        receive(
            &mut transport,
            &mut data,
            start,
            timeout,
            cancelled,
            &mut report.received,
        )
        .map_err(|e| ("response_body", e))?;
        let value = decode_binary(&data).map_err(|_| ("response_body", "invalid_binary_plist"))?;
        let d = value
            .as_dictionary()
            .ok_or(("response", "invalid_dictionary"))?;
        if d.contains_key("Error") || d.contains_key("ErrorDescription") {
            return Err(("response", "device_rejected"));
        }
        if d.get("Status").and_then(Value::as_string) != Some("DataComplete") {
            return Err(("response", "unexpected_status"));
        }
        Ok(())
    })();
    match result {
        Ok(()) => report.complete = true,
        Err((stage, kind)) => {
            report.failure = Some(Failure {
                stage,
                kind,
                sent: report.sent,
                received: report.received,
            })
        }
    }
    report
}
fn budget(
    start: Instant,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<u32, &'static str> {
    if cancelled() {
        return Err("cancelled");
    }
    let left = timeout.saturating_sub(start.elapsed());
    if left.is_zero() {
        return Err("timeout");
    }
    Ok(left.as_millis().clamp(1, 250) as u32)
}
fn classify(e: io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "timeout",
        io::ErrorKind::UnexpectedEof
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe => "disconnected",
        _ => "transport",
    }
}
fn receive<T: DuplexTransport>(
    t: &mut T,
    buffer: &mut [u8],
    start: Instant,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
    count: &mut usize,
) -> Result<(), &'static str> {
    let mut offset = 0;
    while offset < buffer.len() {
        let budget = budget(start, timeout, cancelled)?;
        match t.receive(&mut buffer[offset..], budget) {
            Ok(0) => return Err("disconnected"),
            Ok(n) if n <= buffer.len() - offset => {
                offset += n;
                *count += n;
            }
            Ok(_) => return Err("invalid_count"),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(e) => return Err(classify(e)),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};
    struct Mock {
        input: VecDeque<u8>,
        output: Rc<RefCell<Vec<u8>>>,
    }
    impl crate::ReceiveTransport for Mock {
        fn receive(&mut self, b: &mut [u8], _: u32) -> io::Result<usize> {
            let n = b.len().min(3).min(self.input.len());
            for x in &mut b[..n] {
                *x = self.input.pop_front().unwrap();
            }
            Ok(n)
        }
    }
    impl DuplexTransport for Mock {
        fn send(&mut self, b: &[u8], _: u32) -> io::Result<usize> {
            let n = b.len().min(5);
            self.output.borrow_mut().extend(&b[..n]);
            Ok(n)
        }
    }
    fn run(data: Vec<u8>) -> (Report, Vec<u8>) {
        let output = Rc::new(RefCell::new(vec![]));
        let result = stage(
            Mock {
                input: data.into(),
                output: output.clone(),
            },
            "AirCard-Linux-Theme-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            b"PK-test",
            Duration::from_secs(1),
            &|| false,
        );
        let bytes = output.borrow().clone();
        (result, bytes)
    }
    #[test]
    fn fragmented_io_uses_big_endian_and_requires_complete_status() {
        let body = encode_binary(&Value::Dictionary(Dictionary::from_iter([(
            "Status",
            Value::String("DataComplete".into()),
        )])))
        .unwrap();
        let mut input = (body.len() as u32).to_be_bytes().to_vec();
        input.extend(body);
        let (r, b) = run(input);
        assert!(r.complete);
        let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
        assert_eq!(&b[4 + n..], b"PK-test");
        assert!(decode_binary(&b[4..4 + n]).is_ok());
    }
    #[test]
    fn malformed_length_disconnect_and_status_fail_closed() {
        for input in [vec![], vec![0xff; 4], vec![0, 0, 0, 8, 1, 2]] {
            let (r, _) = run(input);
            assert!(!r.complete);
            assert!(r.failure.is_some());
        }
        let body = encode_binary(&Value::Dictionary(Dictionary::from_iter([(
            "Status",
            Value::String("Receiving".into()),
        )])))
        .unwrap();
        let mut input = (body.len() as u32).to_be_bytes().to_vec();
        input.extend(body);
        assert_eq!(run(input).0.failure.unwrap().kind, "unexpected_status");
    }
    #[test]
    fn cancellation_and_zero_deadline_send_nothing() {
        for cancel in [false, true] {
            let output = Rc::new(RefCell::new(vec![]));
            let r = stage(
                Mock {
                    input: VecDeque::new(),
                    output: output.clone(),
                },
                "AirCard-Linux-Theme-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                b"x",
                Duration::ZERO,
                &|| cancel,
            );
            assert!(!r.complete);
            assert!(output.borrow().is_empty());
        }
    }
}
