//! Minimal DNS over TCP, so name lookups happen *inside* the tunnel.
//!
//! The system resolver would query the local ISP, which defeats the point of a
//! regional exit. Instead we open a TCP connection to the resolver PIA assigned
//! us and speak DNS over TCP (RFC 1035 §4.2.2) through the tunnel.

use crate::error::{ProxyError, Result};
use std::net::Ipv4Addr;

/// Wrap a DNS query for an A record with the two byte TCP length prefix.
pub fn build_query(name: &str, transaction_id: u16) -> Result<Vec<u8>> {
    let mut message = Vec::with_capacity(64);
    message.extend_from_slice(&transaction_id.to_be_bytes());
    message.extend_from_slice(&0x0100u16.to_be_bytes()); // standard query, recursion desired
    message.extend_from_slice(&1u16.to_be_bytes()); // one question
    message.extend_from_slice(&0u16.to_be_bytes()); // no answers
    message.extend_from_slice(&0u16.to_be_bytes()); // no authority
    message.extend_from_slice(&0u16.to_be_bytes()); // no additional

    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() {
            continue;
        }
        if label.len() > 63 {
            return Err(ProxyError::Protocol(format!(
                "DNS label too long in '{name}'"
            )));
        }
        message.push(label.len() as u8);
        message.extend_from_slice(label.as_bytes());
    }
    message.push(0);
    message.extend_from_slice(&1u16.to_be_bytes()); // QTYPE = A
    message.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = IN

    let len = message.len() as u16;
    let mut framed = Vec::with_capacity(2 + message.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(&message);
    Ok(framed)
}

fn skip_name(message: &[u8], mut offset: usize) -> Result<usize> {
    loop {
        let len = *message
            .get(offset)
            .ok_or_else(|| ProxyError::Protocol("truncated DNS name".into()))?;
        if len == 0 {
            return Ok(offset + 1);
        }
        if len & 0xC0 == 0xC0 {
            return Ok(offset + 2); // compression pointer ends the name
        }
        offset += 1 + len as usize;
    }
}

/// Parse a DNS response body (without the TCP length prefix) for A records.
pub fn parse_response(message: &[u8], transaction_id: u16) -> Result<Vec<Ipv4Addr>> {
    if message.len() < 12 {
        return Err(ProxyError::Protocol("DNS response shorter than a header".into()));
    }
    let id = u16::from_be_bytes([message[0], message[1]]);
    if id != transaction_id {
        return Err(ProxyError::Protocol(format!(
            "DNS transaction id mismatch: expected {transaction_id}, got {id}"
        )));
    }
    let flags = u16::from_be_bytes([message[2], message[3]]);
    let rcode = flags & 0x000F;
    if rcode != 0 {
        return Err(ProxyError::Protocol(format!(
            "DNS server returned error code {rcode}"
        )));
    }

    let question_count = u16::from_be_bytes([message[4], message[5]]) as usize;
    let answer_count = u16::from_be_bytes([message[6], message[7]]) as usize;

    let mut offset = 12;
    for _ in 0..question_count {
        offset = skip_name(message, offset)?;
        offset += 4; // qtype + qclass
    }

    let mut addresses = Vec::new();
    for _ in 0..answer_count {
        offset = skip_name(message, offset)?;
        if message.len() < offset + 10 {
            break;
        }
        let record_type = u16::from_be_bytes([message[offset], message[offset + 1]]);
        let record_class = u16::from_be_bytes([message[offset + 2], message[offset + 3]]);
        let data_len = u16::from_be_bytes([message[offset + 8], message[offset + 9]]) as usize;
        offset += 10;

        if message.len() < offset + data_len {
            break;
        }
        if record_type == 1 && record_class == 1 && data_len == 4 {
            addresses.push(Ipv4Addr::new(
                message[offset],
                message[offset + 1],
                message[offset + 2],
                message[offset + 3],
            ));
        }
        offset += data_len;
    }

    if addresses.is_empty() {
        return Err(ProxyError::Protocol(
            "DNS response contained no A records".into(),
        ));
    }
    Ok(addresses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_response(transaction_id: u16, address: [u8; 4]) -> Vec<u8> {
        let query = build_query("example.com", transaction_id).expect("query");
        let message = &query[2..];

        let mut response = Vec::new();
        response.extend_from_slice(&transaction_id.to_be_bytes());
        response.extend_from_slice(&[0x81, 0x80]); // response, recursion available
        response.extend_from_slice(&1u16.to_be_bytes()); // one question
        response.extend_from_slice(&1u16.to_be_bytes()); // one answer
        // NSCOUNT and ARCOUNT are both zero: four bytes, not six.
        response.extend_from_slice(&[0, 0, 0, 0]);
        response.extend_from_slice(&message[12..]); // copy the question verbatim
        response.extend_from_slice(&[0xC0, 0x0C]); // pointer back to the question name
        response.extend_from_slice(&[0, 1]); // TYPE A
        response.extend_from_slice(&[0, 1]); // CLASS IN
        response.extend_from_slice(&[0, 0, 0, 60]); // TTL
        response.extend_from_slice(&[0, 4]); // RDLENGTH
        response.extend_from_slice(&address);
        response
    }

    #[test]
    fn query_is_length_prefixed_and_well_formed() {
        let query = build_query("ifconfig.me", 0x1234).expect("query");
        let declared = u16::from_be_bytes([query[0], query[1]]) as usize;
        assert_eq!(declared, query.len() - 2);
        assert_eq!(&query[2..4], &0x1234u16.to_be_bytes());
        // Name labels: 8"ifconfig" 2"me" 0
        assert_eq!(query[14], 8);
        assert_eq!(&query[15..23], b"ifconfig");
        assert_eq!(query[23], 2);
        assert_eq!(&query[24..26], b"me");
        assert_eq!(query[26], 0);
    }

    #[test]
    fn parses_an_a_record() {
        let response = synthetic_response(0x1234, [203, 0, 113, 7]);
        let addresses = parse_response(&response, 0x1234).expect("parse");
        assert_eq!(addresses, vec![Ipv4Addr::new(203, 0, 113, 7)]);
    }

    #[test]
    fn rejects_a_mismatched_transaction_id() {
        let response = synthetic_response(0x1234, [203, 0, 113, 7]);
        let error = parse_response(&response, 0x9999).expect_err("mismatch must fail");
        assert!(error.to_string().contains("transaction id"), "{error}");
    }

    #[test]
    fn rejects_a_response_with_no_answers() {
        let mut response = synthetic_response(0x1234, [203, 0, 113, 7]);
        // Claim zero answers.
        response[6] = 0;
        response[7] = 0;
        let error = parse_response(&response, 0x1234).expect_err("no answers must fail");
        assert!(error.to_string().contains("no A records"), "{error}");
    }
}
