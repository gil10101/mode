//! UUID v7 ids for frames without a dependency on the `uuid` crate: the relay
//! must stay tiny (08 §3.4). Randomness comes from `std`'s per-process
//! `RandomState` seed (OS entropy), which is enough for correlation ids.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn random_u64() -> u64 {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(n);
    h.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    h.write_u32(std::process::id());
    h.finish()
}

/// A new UUID v7 (time-ordered) as a lowercase hyphenated string. The 12
/// `rand_a` bits carry a per-process sequence so ids minted within the same
/// millisecond still sort in creation order (RFC 9562 §6.2, method 1).
pub fn uuid_v7() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        & 0x0000_FFFF_FFFF_FFFF;
    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) & 0x0FFF;
    let rb = random_u64();
    let mut bytes = [0u8; 16];
    bytes[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    bytes[6] = 0x70 | ((seq >> 8) as u8 & 0x0F); // version 7 + sequence high nibble
    bytes[7] = seq as u8;
    bytes[8] = 0x80 | ((rb >> 56) as u8 & 0x3F); // RFC 4122 variant
    bytes[9..].copy_from_slice(&rb.to_be_bytes()[1..]);
    format_uuid(&bytes)
}

fn format_uuid(b: &[u8; 16]) -> String {
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_a_v7_uuid_and_is_unique() {
        let a = uuid_v7();
        let b = uuid_v7();
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "7", "version nibble: {a}");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"), "variant: {a}");
        assert_ne!(a, b);
        assert!(
            a < b,
            "v7 ids are time-ordered, even within one millisecond: {a} {b}"
        );
        let many: Vec<String> = (0..64).map(|_| uuid_v7()).collect();
        assert!(many.windows(2).all(|w| w[0] < w[1]));
    }
}
