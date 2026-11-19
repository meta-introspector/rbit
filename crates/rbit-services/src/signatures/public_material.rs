//! Reject secret, encrypted, compressed and indefinite `OpenPGP` packets before staging.
//! Public key enrollment accepts a deliberately narrow, fully length-delimited armor profile.
use base64::{Engine, engine::general_purpose::STANDARD};

pub(super) fn public_packets(armor: &str) -> bool {
    let Some(body) = armor
        .trim()
        .strip_prefix("-----BEGIN PGP PUBLIC KEY BLOCK-----")
        .and_then(|text| text.strip_suffix("-----END PGP PUBLIC KEY BLOCK-----"))
    else {
        return false;
    };
    let mut lines = body.trim_start_matches(['\r', '\n']).lines();
    let mut encoded = String::new();
    let mut payload = false;
    for line in &mut lines {
        let line = line.trim();
        if !payload {
            if line.is_empty() {
                payload = true;
                continue;
            }
            if line.contains(':') {
                continue;
            }
            payload = true;
        }
        if !line.starts_with('=') {
            encoded.push_str(line);
        }
    }
    let Ok(bytes) = STANDARD.decode(encoded) else { return false };
    let mut offset = 0;
    let mut primary = false;
    while offset < bytes.len() {
        let header = bytes[offset];
        offset += 1;
        if header & 0x80 == 0 {
            return false;
        }
        let new_format = header & 0x40 != 0;
        let tag = if new_format { header & 63 } else { (header >> 2) & 15 };
        // Only signature, public primary/subkey, marker, trust, UID and user-attribute packets.
        // In particular tags 5/7 (secret keys) never reach GPG or the filesystem.
        if !matches!(tag, 2 | 6 | 10 | 12 | 13 | 14 | 17) {
            return false;
        }
        if tag == 6 && primary {
            return false; // One trust entry is one primary key, including its public subkeys.
        }
        primary |= tag == 6;
        let Some(length) = packet_length(&bytes, &mut offset, header, new_format) else { return false };
        let Some(end) = offset.checked_add(length).filter(|end| *end <= bytes.len()) else { return false };
        offset = end;
    }
    primary
}
fn packet_length(bytes: &[u8], offset: &mut usize, header: u8, new_format: bool) -> Option<usize> {
    let count = if new_format {
        let first = *bytes.get(*offset)?;
        *offset += 1;
        match first {
            0..=191 => return Some(usize::from(first)),
            192..=223 => {
                let second = *bytes.get(*offset)?;
                *offset += 1;
                return Some((usize::from(first) - 192) * 256 + usize::from(second) + 192);
            }
            255 => 4,
            _ => return None,
        }
    } else {
        match header & 3 {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => return None,
        }
    };
    let mut length = 0usize;
    for _ in 0..count {
        length = length.checked_mul(256)?.checked_add(usize::from(*bytes.get(*offset)?))?;
        *offset += 1;
    }
    Some(length)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn armor(bytes: &[u8]) -> String {
        format!("-----BEGIN PGP PUBLIC KEY BLOCK-----\n\n{}\n-----END PGP PUBLIC KEY BLOCK-----", STANDARD.encode(bytes))
    }
    #[test]
    fn signatures_public_armor_does_not_hide_secret_or_nested_packets() {
        assert!(public_packets(&armor(&[0xc6, 1, 0])));
        for bytes in
            [&[0xc5, 1, 0][..], &[0xc6, 1, 0, 0xc7, 1, 0], &[0xc8, 1, 0], &[0xc6, 224, 0], &[0xc6, 10, 0], &[0xc6, 1, 0, 0xc6, 1, 0]]
        {
            assert!(!public_packets(&armor(bytes)));
        }
    }
}
